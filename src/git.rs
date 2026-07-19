use std::collections::HashSet;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;

use crate::model::{Evidence, EvidenceRelation, Reason, ReasonStatus, Reference, Report, Target};
use crate::{AppError, Result};

const HISTORY_LIMIT: usize = 6;
const MAX_BLOB_BYTES: u64 = 5 * 1024 * 1024;
const MAX_COMMIT_BYTES: u64 = 1024 * 1024;
const MAX_GIT_OUTPUT_BYTES: u64 = MAX_BLOB_BYTES;

pub fn validate_target(spec: &str) -> std::result::Result<String, String> {
    parse_target(spec)
        .map(|_| spec.to_string())
        .map_err(|error| error.0)
}

pub fn analyze(spec: &str) -> Result<Report> {
    let (raw_path, line) = parse_target(spec)?;
    let current_dir = env::current_dir()
        .map_err(|error| AppError(format!("could not read the current directory: {error}")))?;
    let root_output = run_git(&current_dir, ["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(trimmed_utf8(&root_output.stdout, "repository path")?);
    let path = repository_path(&root, &current_dir, raw_path)?;
    let revision = git_text(
        &root,
        ["rev-parse", "--verify", "HEAD^{commit}"],
        "repository has no HEAD commit",
    )?;

    let object = format!("{revision}:{path}");
    run_git_checked(
        &root,
        ["cat-file", "-e", object.as_str()],
        format!("{path:?} is not a tracked file at HEAD"),
    )?;
    let size = git_text(
        &root,
        ["cat-file", "-s", object.as_str()],
        "could not read the file size",
    )?
    .parse::<u64>()
    .map_err(|_| AppError("Git returned an invalid file size".into()))?;
    if size > MAX_BLOB_BYTES {
        return Err(AppError(format!(
            "{path:?} is larger than the 5 MiB safety limit"
        )));
    }

    let blob = run_git_checked(
        &root,
        ["cat-file", "blob", object.as_str()],
        format!("could not read {path:?} at HEAD"),
    )?
    .stdout;
    if blob.contains(&0) {
        return Err(AppError(format!(
            "{path:?} is binary; only UTF-8 text files are supported"
        )));
    }
    let text = std::str::from_utf8(&blob)
        .map_err(|_| AppError(format!("{path:?} is not valid UTF-8 text")))?;
    let source_line = text.lines().nth(line - 1).ok_or_else(|| {
        AppError(format!(
            "line {line} is outside {path:?} (1-{})",
            text.lines().count()
        ))
    })?;

    let dirty = !git_output(&root, ["status", "--porcelain=v1", "--", &path])?
        .stdout
        .is_empty();
    let shallow = git_text(
        &root,
        ["rev-parse", "--is-shallow-repository"],
        "could not determine whether repository history is shallow",
    )? == "true";

    let line_range = format!("{line},{line}");
    let blame = run_git_checked(
        &root,
        [
            "blame",
            "-w",
            "-M",
            "-C",
            "--root",
            "-l",
            "-s",
            "-L",
            line_range.as_str(),
            revision.as_str(),
            "--",
            path.as_str(),
        ],
        format!("could not trace {path}:{line}"),
    )?;
    let blamed_commit = parse_blame_commit(&blame.stdout)?;

    let history_arg = format!("{line},{line}:{path}");
    let max_history = (HISTORY_LIMIT + 1).to_string();
    let history_output = git_output(
        &root,
        [
            "log",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-patch",
            "--format=%H",
            "--max-count",
            max_history.as_str(),
            "-L",
            history_arg.as_str(),
            revision.as_str(),
        ],
    )?;

    let mut warnings = Vec::new();
    if dirty {
        warnings.push("working-tree changes were ignored; evidence is for HEAD".into());
    }
    if shallow {
        warnings.push("this is a shallow clone, so older evidence may be missing".into());
    }

    let (history, fallback_used) = if history_output.status.success() {
        (parse_hash_lines(&history_output.stdout), false)
    } else {
        warnings.push("exact line history was unavailable; showing file history".into());
        let fallback = run_git_checked(
            &root,
            [
                "log",
                "--follow",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-patch",
                "--format=%H",
                "--max-count",
                max_history.as_str(),
                revision.as_str(),
                "--",
                path.as_str(),
            ],
            format!("could not read history for {path:?}"),
        )?;
        (parse_hash_lines(&fallback.stdout), true)
    };

    let history_truncated = history.len() > HISTORY_LIMIT;
    if history_truncated {
        warnings.push(format!(
            "history output was limited to {HISTORY_LIMIT} commits"
        ));
    }

    let mut hashes = Vec::with_capacity(HISTORY_LIMIT);
    let mut seen = HashSet::new();
    if seen.insert(blamed_commit.clone()) {
        hashes.push(blamed_commit.clone());
    }
    for hash in history.into_iter().take(HISTORY_LIMIT) {
        if hashes.len() == HISTORY_LIMIT {
            break;
        }
        if seen.insert(hash.clone()) {
            hashes.push(hash);
        }
    }

    let github_repo = github_remote(&root);
    let mut evidence = Vec::with_capacity(hashes.len());
    for (index, hash) in hashes.iter().enumerate() {
        let mut item = commit_metadata(&root, hash, github_repo.as_deref())?;
        item.relation = if index == 0 {
            EvidenceRelation::LastChanged
        } else if fallback_used {
            EvidenceRelation::FileHistory
        } else {
            EvidenceRelation::LineHistory
        };
        evidence.push(item);
    }

    let reason = recorded_reason(evidence.first());
    Ok(Report {
        schema_version: 1,
        target: Target {
            path,
            revision,
            line,
            text: source_line.trim_end_matches('\r').to_string(),
            dirty,
        },
        reason,
        evidence,
        history_complete: !shallow && !history_truncated && !fallback_used,
        warnings,
    })
}

fn parse_target(spec: &str) -> Result<(&str, usize)> {
    let (path, raw_line) = spec.rsplit_once(':').ok_or_else(|| {
        AppError("target must end with a line number, for example src/auth.rs:42".into())
    })?;
    if path.is_empty() {
        return Err(AppError("target path cannot be empty".into()));
    }
    let line = raw_line
        .parse::<usize>()
        .map_err(|_| AppError(format!("invalid line number {raw_line:?}")))?;
    if line == 0 {
        return Err(AppError("line numbers start at 1".into()));
    }
    Ok((path, line))
}

fn repository_path(root: &Path, current_dir: &Path, raw_path: &str) -> Result<String> {
    let root = root
        .canonicalize()
        .map_err(|error| AppError(format!("could not resolve repository root: {error}")))?;
    let current_dir = current_dir
        .canonicalize()
        .map_err(|error| AppError(format!("could not resolve the current directory: {error}")))?;
    let supplied = Path::new(raw_path);
    let candidate = normalize_path(if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        current_dir.join(supplied)
    });
    let relative = candidate
        .strip_prefix(&root)
        .map_err(|_| {
            AppError(format!(
                "file {raw_path:?} is outside the current Git repository"
            ))
        })?
        .to_path_buf();

    let mut component_path = root.clone();
    for component in relative.components() {
        component_path.push(component);
        if component_path
            .symlink_metadata()
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(AppError(format!(
                "file {raw_path:?} passes through a symbolic link; use its tracked path instead"
            )));
        }
    }

    let candidate = candidate
        .canonicalize()
        .map_err(|_| AppError(format!("file {raw_path:?} does not exist")))?;
    candidate.strip_prefix(&root).map_err(|_| {
        AppError(format!(
            "file {raw_path:?} is outside the current Git repository"
        ))
    })?;
    if !candidate.is_file() {
        return Err(AppError(format!("{raw_path:?} is not a file")));
    }
    let path = relative
        .to_str()
        .ok_or_else(|| AppError("non-UTF-8 file paths are not supported".into()))?;
    Ok(path.replace(std::path::MAIN_SEPARATOR, "/"))
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn parse_blame_commit(bytes: &[u8]) -> Result<String> {
    let first_line = trimmed_utf8(bytes, "Git blame output")?
        .lines()
        .next()
        .unwrap_or_default();
    let hash = first_line.split_whitespace().next().unwrap_or_default();
    if hash.len() < 7 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError("Git returned invalid blame output".into()));
    }
    Ok(hash.to_string())
}

fn parse_hash_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|line| line.len() >= 7 && line.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_string)
        .collect()
}

fn commit_metadata(root: &Path, hash: &str, github_repo: Option<&str>) -> Result<Evidence> {
    let commit_size = git_text(
        root,
        ["cat-file", "-s", hash],
        format!("could not read commit {hash}"),
    )?
    .parse::<u64>()
    .map_err(|_| AppError(format!("Git returned an invalid size for commit {hash}")))?;
    if commit_size > MAX_COMMIT_BYTES {
        return Err(AppError(format!(
            "commit {hash} exceeds the 1 MiB metadata safety limit"
        )));
    }
    let output = run_git_checked(
        root,
        [
            "show",
            "-s",
            "--no-color",
            "--no-notes",
            "--format=format:%H%x00%aI%x00%an%x00%s%x00%b%x00",
            hash,
        ],
        format!("could not read commit {hash}"),
    )?;
    let fields: Vec<&[u8]> = output.stdout.split(|byte| *byte == 0).collect();
    if fields.len() != 6 || !fields[5].is_empty() {
        return Err(AppError(format!(
            "Git returned invalid metadata for commit {hash}"
        )));
    }
    let field = |index, label| {
        std::str::from_utf8(fields[index])
            .map(str::trim)
            .map(str::to_string)
            .map_err(|_| AppError(format!("commit {hash} has non-UTF-8 {label}")))
    };
    let commit = field(0, "hash")?;
    let authored_at = field(1, "date")?;
    let author_name = field(2, "author")?;
    let subject = field(3, "subject")?;
    let body = field(4, "body")?;
    let references = extract_references(&format!("{subject}\n{body}"), github_repo);

    Ok(Evidence {
        relation: EvidenceRelation::LineHistory,
        commit,
        authored_at,
        author_name,
        subject,
        body,
        references,
    })
}

fn recorded_reason(evidence: Option<&Evidence>) -> Reason {
    let Some(evidence) = evidence else {
        return Reason {
            status: ReasonStatus::Unknown,
            text: None,
            source_commit: None,
        };
    };
    let body_reason = evidence
        .body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty());
    let text = body_reason
        .map(str::to_string)
        .or_else(|| substantive_subject(&evidence.subject).then(|| evidence.subject.clone()));
    Reason {
        status: if text.is_some() {
            ReasonStatus::Recorded
        } else {
            ReasonStatus::Unknown
        },
        text,
        source_commit: Some(evidence.commit.clone()),
    }
}

fn substantive_subject(subject: &str) -> bool {
    let normalized = subject
        .trim()
        .trim_matches(|character: char| !character.is_alphanumeric())
        .to_ascii_lowercase();
    normalized.len() >= 8
        && !matches!(
            normalized.as_str(),
            "initial commit" | "cleanup" | "update" | "changes" | "refactor" | "miscellaneous"
        )
}

fn extract_references(text: &str, github_repo: Option<&str>) -> Vec<Reference> {
    let bytes = text.as_bytes();
    let mut references = Vec::new();
    let mut seen = HashSet::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'#' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end > start {
            if let Ok(number) = text[start..end].parse::<u64>() {
                if number > 0 && seen.insert(number) {
                    references.push(Reference {
                        number,
                        url: github_repo.map(|repository| {
                            format!("https://github.com/{repository}/issues/{number}")
                        }),
                    });
                }
            }
        }
        index = end.max(index + 1);
    }
    references
}

fn github_remote(root: &Path) -> Option<String> {
    let output = git_output(root, ["config", "--get", "remote.origin.url"]).ok()?;
    if !output.status.success() {
        return None;
    }
    parse_github_remote(trimmed_utf8(&output.stdout, "Git remote URL").ok()?)
}

fn parse_github_remote(remote: &str) -> Option<String> {
    let path = if let Some(path) = remote.strip_prefix("https://github.com/") {
        path
    } else if let Some(path) = remote.strip_prefix("http://github.com/") {
        path
    } else if let Some(path) = remote.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = remote.strip_prefix("ssh://git@github.com/") {
        path
    } else {
        return None;
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    if owner.is_empty()
        || repository.is_empty()
        || parts.next().is_some()
        || !owner.chars().all(safe_github_name)
        || !repository.chars().all(safe_github_name)
    {
        return None;
    }
    Some(format!("{owner}/{repository}"))
}

fn safe_github_name(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
}

fn run_git<const N: usize>(directory: &Path, arguments: [&str; N]) -> Result<Output> {
    git_output(directory, arguments).and_then(|output| {
        if output.status.success() {
            Ok(output)
        } else {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if detail.is_empty() {
                Err(AppError("Git command failed".into()))
            } else {
                Err(AppError(detail))
            }
        }
    })
}

fn run_git_checked<const N: usize>(
    directory: &Path,
    arguments: [&str; N],
    message: String,
) -> Result<Output> {
    let output = git_output(directory, arguments)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(AppError(message))
    }
}

fn git_text<const N: usize>(
    directory: &Path,
    arguments: [&str; N],
    message: impl Into<String>,
) -> Result<String> {
    let output = run_git_checked(directory, arguments, message.into())?;
    Ok(trimmed_utf8(&output.stdout, "Git output")?.to_string())
}

fn git_output<const N: usize>(directory: &Path, arguments: [&str; N]) -> Result<Output> {
    let mut child = Command::new("git")
        .arg("--no-pager")
        .arg("--no-optional-locks")
        .arg("--literal-pathspecs")
        .arg("-c")
        .arg("core.fsmonitor=")
        .arg("-c")
        .arg("log.showSignature=false")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| AppError(format!("could not run Git: {error}")))?;

    let stdout = child.stdout.take().expect("Git stdout is piped");
    let stderr = child.stderr.take().expect("Git stderr is piped");
    let stdout_reader = thread::spawn(move || read_git_output(stdout));
    let stderr_reader = thread::spawn(move || read_git_output(stderr));
    let status = child
        .wait()
        .map_err(|error| AppError(format!("could not wait for Git: {error}")))?;
    let read = |reader: thread::JoinHandle<std::io::Result<Vec<u8>>>| {
        reader
            .join()
            .map_err(|_| AppError("Git output reader failed".into()))?
            .map_err(|error| AppError(format!("could not read Git output: {error}")))
    };
    Ok(Output {
        status,
        stdout: read(stdout_reader)?,
        stderr: read(stderr_reader)?,
    })
}

fn read_git_output(reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take(MAX_GIT_OUTPUT_BYTES + 1)
        .read_to_end(&mut output)?;
    if output.len() as u64 > MAX_GIT_OUTPUT_BYTES {
        return Err(std::io::Error::other(
            "output exceeded the 5 MiB safety limit",
        ));
    }
    Ok(output)
}

fn trimmed_utf8<'a>(bytes: &'a [u8], label: &str) -> Result<&'a str> {
    std::str::from_utf8(bytes)
        .map(|value| value.trim_end_matches(['\r', '\n']))
        .map_err(|_| AppError(format!("{label} is not valid UTF-8")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_uses_the_last_colon() {
        assert_eq!(
            parse_target("C:\\work\\auth.rs:42").unwrap(),
            ("C:\\work\\auth.rs", 42)
        );
        assert!(parse_target("src/auth.rs").is_err());
        assert!(parse_target("src/auth.rs:0").is_err());
    }

    #[test]
    fn parses_supported_github_remotes() {
        assert_eq!(
            parse_github_remote("git@github.com:acme/widgets.git").as_deref(),
            Some("acme/widgets")
        );
        assert_eq!(
            parse_github_remote("https://github.com/acme/widgets").as_deref(),
            Some("acme/widgets")
        );
        assert_eq!(
            parse_github_remote("https://github.com/acme/widgets/").as_deref(),
            Some("acme/widgets")
        );
        assert!(parse_github_remote("https://example.com/acme/widgets").is_none());
    }

    #[test]
    fn rejects_oversized_git_output() {
        assert!(read_git_output(std::io::repeat(0)).is_err());
    }

    #[test]
    fn extracts_unique_references() {
        let references =
            extract_references("Fixes #12 and duplicates #12; see #7", Some("acme/app"));
        assert_eq!(references.len(), 2);
        assert_eq!(references[0].number, 12);
        assert_eq!(
            references[1].url.as_deref(),
            Some("https://github.com/acme/app/issues/7")
        );
    }
}
