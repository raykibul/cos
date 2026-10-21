//! A record of every answered request, and the report section "Approved on
//! your behalf (N)" built from them.

use std::fmt::Write as _;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::request::Role;
use crate::verdict::Tier;

/// Longest request summary kept in a record.
const SUMMARY_CHARS: usize = 160;

/// Who answered a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decider {
    /// Rust rules: the always-allowed tier.
    Rules,
    /// The quick-check layer (later).
    Jev,
    Chief,
    User,
    /// Nobody answered within five minutes.
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Allowed,
    Denied,
}

/// One answered request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub request_id: String,
    pub at: Timestamp,
    pub role: Role,
    pub tool: String,
    /// What was asked, short enough for one line; see [`summarize`].
    pub summary: String,
    pub tier: Tier,
    /// Why the policy put it in that tier.
    pub rule: String,
    pub decided_by: Decider,
    pub outcome: Outcome,
    /// The decider's own reason, such as the chief's.
    pub reason: String,
}

impl AuditRecord {
    fn approved_on_your_behalf(&self) -> bool {
        self.outcome == Outcome::Allowed && matches!(self.decided_by, Decider::Chief | Decider::Jev)
    }
}

/// A one-line summary of a request: the command for Bash, the path for file
/// tools, the URL for WebFetch.
pub fn summarize(tool: &str, input: &Value) -> String {
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    let text = match tool {
        "Bash" => field("command").map(str::to_owned),
        "WebFetch" => field("url").map(|url| format!("WebFetch {url}")),
        "WebSearch" => field("query").map(|q| format!("WebSearch {q}")),
        _ => field("file_path")
            .or_else(|| field("notebook_path"))
            .or_else(|| field("path"))
            .map(|path| format!("{tool} {path}")),
    };
    let text = text.unwrap_or_else(|| format!("{tool} {input}"));
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > SUMMARY_CHARS {
        let cut: String = one_line.chars().take(SUMMARY_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        one_line
    }
}

/// Renders the report section listing what the chief (or jev) approved for
/// you, with counts of everything else. Times are shown in `zone`.
pub fn render_report_section(records: &[AuditRecord], zone: &TimeZone) -> String {
    let approved: Vec<&AuditRecord> = records.iter().filter(|r| r.approved_on_your_behalf()).collect();
    let mut out = format!("## Approved on your behalf ({})\n\n", approved.len());
    if approved.is_empty() {
        out.push_str("Nothing was approved on your behalf.\n");
    }
    for (n, record) in approved.iter().enumerate() {
        let time = record.at.to_zoned(zone.clone()).strftime("%H:%M");
        let who = match record.decided_by {
            Decider::Jev => "jev",
            _ => "the chief",
        };
        let _ = writeln!(
            out,
            "{}. {} — {}, approved by {who} ({}) at {time}. Reason: {}",
            n + 1,
            inline_code(&record.summary),
            record.tool,
            kind(record.tier),
            one_line(&record.reason),
        );
    }
    let count = |f: &dyn Fn(&AuditRecord) -> bool| records.iter().filter(|r| f(r)).count();
    let rules = count(&|r| r.decided_by == Decider::Rules);
    let refused = count(&|r| r.decided_by == Decider::Chief && r.outcome == Outcome::Denied);
    let by_you = count(&|r| r.decided_by == Decider::User);
    let unanswered = count(&|r| r.decided_by == Decider::Timeout);
    let mut notes = Vec::new();
    if rules > 0 {
        notes.push(format!("Rust rules allowed {rules} routine requests"));
    }
    if refused > 0 {
        notes.push(format!("the chief refused {refused}"));
    }
    if by_you > 0 {
        notes.push(format!("you answered {by_you}"));
    }
    if unanswered > 0 {
        notes.push(format!("{unanswered} went unanswered and were refused"));
    }
    if !notes.is_empty() {
        let mut line = notes.join(", ");
        line[..1].make_ascii_uppercase();
        let _ = write!(out, "\n{line}.\n");
    }
    out
}

/// What a delegated approval covered.
fn kind(tier: Tier) -> &'static str {
    match tier {
        Tier::Chief { needs } if needs.install => "install",
        Tier::Chief { needs } if needs.network => "network",
        _ => "worktree",
    }
}

fn one_line(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        "(none given)".to_owned()
    } else {
        text
    }
}

/// Wraps `text` in a Markdown code span that survives backticks inside it.
fn inline_code(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::verdict::Needs;

    fn record(decided_by: Decider, outcome: Outcome, summary: &str) -> AuditRecord {
        AuditRecord {
            request_id: "r1".into(),
            at: "2026-10-22T14:03:00Z".parse().unwrap(),
            role: Role::Worker,
            tool: "Bash".into(),
            summary: summary.into(),
            tier: Tier::Chief {
                needs: Needs::INSTALL,
            },
            rule: "changes or downloads dependencies (npm install)".into(),
            decided_by,
            outcome,
            reason: "the task adds\nzod for validation".into(),
        }
    }

    #[test]
    fn summarizes_requests() {
        assert_eq!(
            summarize("Bash", &json!({"command": "cargo   test\n -p x"})),
            "cargo test -p x"
        );
        assert_eq!(
            summarize("Edit", &json!({"file_path": "/w/a.rs"})),
            "Edit /w/a.rs"
        );
        let long = "x".repeat(500);
        assert_eq!(
            summarize("Bash", &json!({ "command": long })).chars().count(),
            SUMMARY_CHARS
        );
    }

    #[test]
    fn renders_the_report_section() {
        let records = vec![
            record(Decider::Chief, Outcome::Allowed, "npm install zod"),
            record(Decider::Chief, Outcome::Denied, "curl -X POST x"),
            record(Decider::Rules, Outcome::Allowed, "cargo test"),
            record(Decider::Timeout, Outcome::Denied, "git push"),
            record(Decider::Chief, Outcome::Allowed, "echo `id`"),
        ];
        let section = render_report_section(&records, &TimeZone::UTC);
        assert_eq!(
            section,
            "## Approved on your behalf (2)\n\n\
             1. `npm install zod` — Bash, approved by the chief (install) at 14:03. Reason: the task adds zod for validation\n\
             2. `` echo `id` `` — Bash, approved by the chief (install) at 14:03. Reason: the task adds zod for validation\n\
             \nRust rules allowed 1 routine requests, the chief refused 1, 1 went unanswered and were refused.\n"
        );
        assert_eq!(
            render_report_section(&[], &TimeZone::UTC),
            "## Approved on your behalf (0)\n\nNothing was approved on your behalf.\n"
        );
    }
}
