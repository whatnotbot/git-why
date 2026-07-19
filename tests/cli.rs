use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

struct Repository {
    directory: TempDir,
}

impl Repository {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-q"]);
        git(directory.path(), &["config", "user.name", "Test Author"]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(directory.path(), &["config", "core.autocrlf", "false"]);
        Self { directory }
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }

    fn write(&self, path: &str, contents: &[u8]) {
        let path = self.path().join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn commit(&self, subject: &str, body: Option<&str>) {
        git(self.path(), &["add", "--all"]);
        let mut arguments = vec!["commit", "-q", "--no-gpg-sign", "-m", subject];
        if let Some(body) = body {
            arguments.extend(["-m", body]);
        }
        git(self.path(), &arguments);
    }
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_cli(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-why"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "git-why failed: {}",
        stderr(output)
    );
}

fn parse_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_keys(value: &Value, expected: &[&str]) {
    let mut actual: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn root_and_subdirectory_invocations_report_the_recorded_reason() {
    let repository = Repository::new();
    repository.write("src/auth.rs", b"let allowed_skew = 30;\n");
    repository.commit(
        "Fix token clock skew",
        Some("Accept small clock differences between hosts."),
    );

    let root = run_cli(repository.path(), &["src/auth.rs:1"]);
    let nested = run_cli(&repository.path().join("src"), &["auth.rs:1"]);
    assert_success(&root);
    assert_success(&nested);

    let root = stdout(&root);
    assert_eq!(root, stdout(&nested));
    assert!(root.contains("src/auth.rs:1"));
    assert!(root.contains("RECORDED REASON"));
    assert!(root.contains("Accept small clock differences between hosts."));
}

#[test]
fn generic_commit_metadata_reports_an_unknown_reason() {
    let repository = Repository::new();
    repository.write("notes.txt", b"something\n");
    repository.commit("update", None);

    let human = run_cli(repository.path(), &["notes.txt:1"]);
    let json = run_cli(repository.path(), &["--json", "notes.txt:1"]);
    assert_success(&human);
    assert_success(&json);

    assert!(stdout(&human).contains("NO RECORDED REASON"));
    let document = parse_json(&json);
    assert_eq!(document["reason"]["status"], "unknown");
    assert!(document["reason"]["text"].is_null());
}

#[test]
fn json_is_pure_and_matches_human_output_with_a_github_reference() {
    let repository = Repository::new();
    git(
        repository.path(),
        &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
    );
    repository.write("widget.rs", b"pub const RETRIES: u8 = 3;\n");
    repository.commit(
        "Retry transient widget failures (#42)",
        Some("Avoid failing on a single temporary error."),
    );

    let human = run_cli(repository.path(), &["widget.rs:1"]);
    let json = run_cli(repository.path(), &["--json", "widget.rs:1"]);
    assert_success(&human);
    assert_success(&json);
    assert!(
        json.stderr.is_empty(),
        "unexpected stderr: {}",
        stderr(&json)
    );

    let human = stdout(&human);
    let document = parse_json(&json);
    let reason = document["reason"]["text"].as_str().unwrap();
    let commit = document["evidence"][0]["commit"].as_str().unwrap();
    let url = document["evidence"][0]["references"][0]["url"]
        .as_str()
        .unwrap();
    assert_eq!(document["schema_version"], 1);
    assert_eq!(document["target"]["path"], "widget.rs");
    assert_keys(
        &document,
        &[
            "schema_version",
            "target",
            "reason",
            "evidence",
            "history_complete",
            "warnings",
        ],
    );
    assert_keys(
        &document["target"],
        &["path", "revision", "line", "text", "dirty"],
    );
    assert_keys(&document["reason"], &["status", "text", "source_commit"]);
    assert_keys(
        &document["evidence"][0],
        &[
            "relation",
            "commit",
            "authored_at",
            "author_name",
            "subject",
            "body",
            "references",
        ],
    );
    assert_keys(
        &document["evidence"][0]["references"][0],
        &["number", "url"],
    );
    assert!(human.contains(reason));
    assert!(human.contains(&commit[..8]));
    assert_eq!(url, "https://github.com/acme/widgets/issues/42");
    assert!(human.contains(url));
}

#[test]
fn accepts_spaces_unicode_and_leading_dash_paths() {
    let repository = Repository::new();
    let paths = ["with space.rs", "café.rs", "-leading.rs"];
    for path in paths {
        repository.write(path, "let café = true;\n".as_bytes());
    }
    repository.commit("Add unusual but valid paths", None);

    for path in paths {
        let target = format!("{path}:1");
        let arguments: Vec<&str> = if path.starts_with('-') {
            vec!["--", &target]
        } else {
            vec![&target]
        };
        let output = run_cli(repository.path(), &arguments);
        assert_success(&output);
        let output = stdout(&output);
        assert!(output.contains(&format!("{path}:1")));
        assert!(output.contains("let café = true;"));
    }
}

#[test]
fn rejects_malformed_and_out_of_range_targets_with_clean_diagnostics() {
    let repository = Repository::new();
    repository.write("short.txt", b"one line\n");
    repository.commit("Add short example", None);

    let malformed = run_cli(repository.path(), &["short.txt:not-a-line"]);
    assert_eq!(malformed.status.code(), Some(2));
    assert!(stdout(&malformed).is_empty());
    assert!(stderr(&malformed).contains("invalid line number"));

    let out_of_range = run_cli(repository.path(), &["short.txt:2"]);
    assert!(!out_of_range.status.success());
    assert!(stdout(&out_of_range).is_empty());
    assert!(stderr(&out_of_range).contains("line 2 is outside \"short.txt\" (1-1)"));

    let missing_argument = run_cli(repository.path(), &[]);
    assert_eq!(missing_argument.status.code(), Some(2));
    assert!(stdout(&missing_argument).is_empty());
    assert!(stderr(&missing_argument).contains("required arguments were not provided"));
}

#[test]
fn rejects_nul_bytes_as_binary_input() {
    let repository = Repository::new();
    repository.write("binary.dat", b"text\0more text\n");
    repository.commit("Add binary data", None);

    let output = run_cli(repository.path(), &["binary.dat:1"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).contains("binary; only UTF-8 text files are supported"));
}

#[test]
fn dirty_worktree_is_explicit_while_evidence_remains_at_head() {
    let repository = Repository::new();
    repository.write("config.rs", b"const MODE: &str = \"safe\";\n");
    repository.commit(
        "Choose safe mode",
        Some("Keep production defaults conservative."),
    );
    repository.write("config.rs", b"const MODE: &str = \"fast\";\n");

    let human = run_cli(repository.path(), &["config.rs:1"]);
    let json = run_cli(repository.path(), &["--json", "config.rs:1"]);
    assert_success(&human);
    assert_success(&json);

    let human = stdout(&human);
    assert!(human.contains("[uncommitted]"));
    assert!(human.contains("const MODE: &str = \"safe\";"));
    assert!(human.contains("working-tree changes were ignored; evidence is for HEAD"));

    let document = parse_json(&json);
    assert_eq!(document["target"]["dirty"], true);
    assert_eq!(document["target"]["text"], "const MODE: &str = \"safe\";");
    assert!(document["warnings"][0]
        .as_str()
        .unwrap()
        .contains("working-tree changes were ignored"));
}

#[test]
fn follows_a_line_across_a_rename() {
    let repository = Repository::new();
    repository.write("before.rs", b"const TIMEOUT: u8 = 5;\n");
    repository.commit(
        "Set the request timeout",
        Some("Avoid hanging forever on an unavailable peer."),
    );
    git(repository.path(), &["mv", "before.rs", "after.rs"]);
    repository.commit("Rename the timeout module", None);
    repository.write("after.rs", b"const TIMEOUT: u8 = 6;\n");
    repository.commit(
        "Increase the request timeout",
        Some("Allow slower peers one additional second."),
    );

    let output = run_cli(repository.path(), &["--json", "after.rs:1"]);
    assert_success(&output);
    let document = parse_json(&output);

    assert_eq!(document["target"]["path"], "after.rs");
    assert_eq!(
        document["reason"]["text"],
        "Allow slower peers one additional second."
    );
    assert_eq!(
        document["evidence"][0]["subject"],
        "Increase the request timeout"
    );
    assert!(document["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["subject"] == "Set the request timeout"));
}

#[test]
fn shallow_clone_marks_history_incomplete() {
    let origin = Repository::new();
    origin.write("policy.rs", b"const RETRIES: u8 = 3;\n");
    origin.commit("Set retry policy", Some("Tolerate two transient failures."));
    origin.write("unrelated.txt", b"newer commit\n");
    origin.commit("Add unrelated note", None);

    let parent = tempfile::tempdir().unwrap();
    let clone = parent.path().join("shallow");
    let source = format!("file://{}", origin.path().display());
    let output = Command::new("git")
        .args(["clone", "-q", "--depth", "1", &source])
        .arg(&clone)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "shallow clone failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = run_cli(&clone, &["--json", "policy.rs:1"]);
    assert_success(&output);
    let document = parse_json(&output);
    assert_eq!(document["history_complete"], false);
    assert!(document["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap().contains("shallow clone")));
}

#[cfg(unix)]
#[test]
fn rejects_symbolic_link_aliases() {
    use std::os::unix::fs::symlink;

    let repository = Repository::new();
    repository.write("target.rs", b"const SAFE: bool = true;\n");
    repository.write("nested/target.rs", b"const NESTED: bool = true;\n");
    repository.commit("Add safe target", None);
    symlink("target.rs", repository.path().join("alias.rs")).unwrap();
    symlink("nested", repository.path().join("alias-directory")).unwrap();

    let output = run_cli(repository.path(), &["alias.rs:1"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).contains("symbolic link"));

    let output = run_cli(repository.path(), &["alias-directory/target.rs:1"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).contains("symbolic link"));
}

#[cfg(not(windows))]
#[test]
fn wildcard_characters_in_filenames_are_literal() {
    let repository = Repository::new();
    repository.write("literal*.rs", b"const TARGET: bool = true;\n");
    repository.write("literal-other.rs", b"const OTHER: bool = true;\n");
    repository.commit("Add literal wildcard path", None);
    repository.write("literal-other.rs", b"const OTHER: bool = false;\n");

    let output = run_cli(repository.path(), &["--json", "literal*.rs:1"]);
    assert_success(&output);
    let document = parse_json(&output);
    assert_eq!(document["target"]["path"], "literal*.rs");
    assert_eq!(document["target"]["dirty"], false);
    assert!(document["warnings"].as_array().unwrap().is_empty());
}

#[cfg(not(windows))]
#[test]
fn accepts_colons_in_the_path_before_the_final_line_separator() {
    let repository = Repository::new();
    repository.write("clock:zone.rs", b"const ZONE: &str = \"UTC\";\n");
    repository.commit("Record the clock zone", None);

    let output = run_cli(repository.path(), &["--json", "clock:zone.rs:1"]);
    assert_success(&output);
    let document = parse_json(&output);
    assert_eq!(document["target"]["path"], "clock:zone.rs");
    assert_eq!(document["target"]["text"], "const ZONE: &str = \"UTC\";");
}

#[test]
fn handles_crlf_empty_and_no_final_newline_files() {
    let repository = Repository::new();
    repository.write("crlf.txt", b"first\r\nsecond\r\n");
    repository.write("single.txt", b"only line");
    repository.write("empty.txt", b"");
    repository.commit("Add text shapes", None);

    let crlf = run_cli(repository.path(), &["--json", "crlf.txt:2"]);
    let single = run_cli(repository.path(), &["--json", "single.txt:1"]);
    assert_success(&crlf);
    assert_success(&single);
    assert_eq!(parse_json(&crlf)["target"]["text"], "second");
    assert_eq!(parse_json(&single)["target"]["text"], "only line");

    let empty = run_cli(repository.path(), &["empty.txt:1"]);
    assert_eq!(empty.status.code(), Some(1));
    assert!(stdout(&empty).is_empty());
    assert!(stderr(&empty).contains("line 1 is outside \"empty.txt\" (1-0)"));
}

#[test]
fn root_commit_is_valid_complete_evidence() {
    let repository = Repository::new();
    repository.write("root.rs", b"const ROOT: bool = true;\n");
    repository.commit(
        "Create root evidence",
        Some("Establish the initial invariant."),
    );

    let output = run_cli(repository.path(), &["--json", "root.rs:1"]);
    assert_success(&output);
    let document = parse_json(&output);
    assert_eq!(document["history_complete"], true);
    assert_eq!(document["evidence"].as_array().unwrap().len(), 1);
    assert_eq!(document["evidence"][0]["relation"], "last_changed");
    assert_eq!(
        document["reason"]["text"],
        "Establish the initial invariant."
    );
}

#[test]
fn multi_commit_history_is_newest_first_and_truncated_explicitly() {
    let repository = Repository::new();
    for value in 0..8 {
        repository.write(
            "counter.rs",
            format!("const VALUE: u8 = {value};\n").as_bytes(),
        );
        repository.commit(&format!("Set counter value {value}"), None);
    }

    let output = run_cli(repository.path(), &["--json", "counter.rs:1"]);
    assert_success(&output);
    let document = parse_json(&output);
    let evidence = document["evidence"].as_array().unwrap();
    assert_eq!(evidence.len(), 6);
    let subjects: Vec<&str> = evidence
        .iter()
        .map(|item| item["subject"].as_str().unwrap())
        .collect();
    assert_eq!(
        subjects,
        [
            "Set counter value 7",
            "Set counter value 6",
            "Set counter value 5",
            "Set counter value 4",
            "Set counter value 3",
            "Set counter value 2",
        ]
    );
    assert_eq!(document["history_complete"], false);
    assert!(document["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap().contains("limited to 6 commits")));
}

#[test]
fn non_repository_and_missing_git_fail_without_stdout() {
    let directory = tempfile::tempdir().unwrap();
    let not_repository = run_cli(directory.path(), &["missing.rs:1"]);
    assert_eq!(not_repository.status.code(), Some(1));
    assert!(stdout(&not_repository).is_empty());
    assert!(stderr(&not_repository).contains("not a git repository"));

    let empty_path = tempfile::tempdir().unwrap();
    let missing_git = Command::new(env!("CARGO_BIN_EXE_git-why"))
        .current_dir(directory.path())
        .env("PATH", empty_path.path())
        .arg("missing.rs:1")
        .output()
        .unwrap();
    assert_eq!(missing_git.status.code(), Some(1));
    assert!(stdout(&missing_git).is_empty());
    assert!(stderr(&missing_git).contains("could not run Git"));
}
