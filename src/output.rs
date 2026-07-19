use std::fmt::Write;

use crate::model::{EvidenceRelation, ReasonStatus, Report};
use crate::{AppError, Result};

/// Remove terminal control characters from values read from a repository.
pub fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '\u{061c}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{2028}'
                        | '\u{2029}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                )
            {
                ' '
            } else {
                character
            }
        })
        .collect()
}

pub fn human(report: &Report) -> String {
    let mut rendered = String::new();
    let dirty = if report.target.dirty {
        " [uncommitted]"
    } else {
        ""
    };

    let _ = writeln!(
        rendered,
        "{}:{} at {}{}",
        sanitize(&report.target.path),
        report.target.line,
        sanitize(&report.target.revision),
        dirty
    );
    let _ = writeln!(rendered, "    {}", sanitize(&report.target.text));

    match &report.reason.status {
        ReasonStatus::Recorded => {
            rendered.push_str("\nRECORDED REASON\n");
            let reason = report
                .reason
                .text
                .as_deref()
                .unwrap_or("No reason text was recorded.");
            let _ = writeln!(rendered, "    {}", sanitize(reason));
        }
        ReasonStatus::Unknown => {
            rendered.push_str("\nNO RECORDED REASON\n");
            rendered.push_str("    The available commit history does not explain why.\n");
        }
    }

    rendered.push_str("\nEVIDENCE\n");
    if report.evidence.is_empty() {
        rendered.push_str("    No relevant commits found.\n");
    }

    for evidence in &report.evidence {
        let relation = match &evidence.relation {
            EvidenceRelation::LastChanged => "LAST CHANGED",
            EvidenceRelation::LineHistory => "LINE HISTORY",
            EvidenceRelation::FileHistory => "FILE HISTORY",
        };
        let short_commit: String = evidence.commit.chars().take(8).collect();
        let _ = writeln!(
            rendered,
            "    {relation}  {}  {}  {}",
            sanitize(&short_commit),
            sanitize(&evidence.authored_at),
            sanitize(&evidence.author_name)
        );
        let _ = writeln!(rendered, "      {}", sanitize(&evidence.subject));
        for reference in &evidence.references {
            match &reference.url {
                Some(url) => {
                    let _ = writeln!(
                        rendered,
                        "      Reference #{}  {}",
                        reference.number,
                        sanitize(url)
                    );
                }
                None => {
                    let _ = writeln!(rendered, "      Reference #{}", reference.number);
                }
            }
        }
    }

    let completeness = if report.history_complete {
        "complete"
    } else {
        "incomplete"
    };
    let _ = writeln!(rendered, "\nHistory: {completeness}");
    for warning in &report.warnings {
        let _ = writeln!(rendered, "Warning: {}", sanitize(warning));
    }

    rendered
}

pub fn json(report: &Report) -> Result<String> {
    serde_json::to_string_pretty(report)
        .map(|mut json| {
            json.push('\n');
            json
        })
        .map_err(|error| AppError(format!("could not encode JSON: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Evidence, Reason, Reference, Target};

    fn report() -> Report {
        Report {
            schema_version: 1,
            target: Target {
                path: "src/\u{1b}[31mauth.rs".into(),
                revision: "HEAD".into(),
                line: 42,
                text: "allow_skew = 30;\nforged".into(),
                dirty: false,
            },
            reason: Reason {
                status: ReasonStatus::Recorded,
                text: Some("Handle clock drift.".into()),
                source_commit: Some("0123456789abcdef".into()),
            },
            evidence: vec![Evidence {
                relation: EvidenceRelation::LastChanged,
                commit: "0123456789abcdef".into(),
                authored_at: "2026-01-02T03:04:05Z".into(),
                author_name: "Ada\rLovelace".into(),
                subject: "fix token validation".into(),
                body: String::new(),
                references: vec![Reference {
                    number: 12,
                    url: Some("https://example.com/pull/12".into()),
                }],
            }],
            history_complete: true,
            warnings: vec![],
        }
    }

    #[test]
    fn human_output_is_accessible_and_sanitized() {
        let rendered = human(&report());

        assert!(rendered.contains("RECORDED REASON"));
        assert!(rendered.contains("LAST CHANGED  01234567"));
        assert!(rendered.contains("Reference #12  https://example.com/pull/12"));
        assert!(rendered.contains("allow_skew = 30; forged"));
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\r'));
    }

    #[test]
    fn json_output_is_valid_and_contains_no_literal_escape_character() {
        let rendered = json(&report()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["target"]["line"], 42);
        assert!(rendered.ends_with('\n'));
        assert!(!rendered.contains('\u{1b}'));
    }
}
