//! Reading the verdict line a worker or reviewer ends its last message with
//! (see "Verdicts" in `docs/orchestration.md`).
//!
//! - Worker: `DONE` or `STUCK: <why>`.
//! - Reviewer: `VERDICT: APPROVE` or `VERDICT: CHANGES: <what to fix>`.
//!
//! Models dress lines up, so markdown emphasis, backticks, a leading `>` or
//! list marker and a trailing full stop are ignored, and so is letter case
//! in the keywords. The verdict must be the last non-empty line, except that
//! a reviewer's `CHANGES` notes may run on over the following lines.

/// What a worker's last message says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkerVerdict {
    Done,
    Stuck(String),
}

/// What a reviewer's last message says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewerVerdict {
    Approve,
    Changes(String),
}

pub(crate) fn worker(message: &str) -> Option<WorkerVerdict> {
    let line = clean(message.lines().rev().find(|line| !clean(line).is_empty())?);
    if line.trim_end_matches('.').eq_ignore_ascii_case("done") {
        return Some(WorkerVerdict::Done);
    }
    let why = strip_keyword(line, "stuck")?;
    if !(why.is_empty() || why.starts_with(':')) {
        return None;
    }
    let why = why.trim_start_matches(':').trim();
    Some(WorkerVerdict::Stuck(if why.is_empty() {
        "the worker gave no reason".into()
    } else {
        why.to_owned()
    }))
}

pub(crate) fn reviewer(message: &str) -> Option<ReviewerVerdict> {
    let lines: Vec<&str> = message.lines().collect();
    // The last line that starts with `VERDICT:`; anything after it is notes.
    let (at, rest) = lines.iter().enumerate().rev().find_map(|(at, line)| {
        let rest = strip_keyword(clean(line), "verdict")?;
        rest.starts_with(':').then(|| (at, rest[1..].trim()))
    })?;
    let rest = clean(rest);
    if rest.trim_end_matches('.').eq_ignore_ascii_case("approve") {
        return (lines[at + 1..].iter().all(|line| clean(line).is_empty()))
            .then_some(ReviewerVerdict::Approve);
    }
    let notes = strip_keyword(rest, "changes")?;
    let mut notes = notes.trim_start_matches(':').trim().to_owned();
    for line in &lines[at + 1..] {
        let line = line.trim();
        if !line.is_empty() {
            if !notes.is_empty() {
                notes.push('\n');
            }
            notes.push_str(line);
        }
    }
    Some(ReviewerVerdict::Changes(if notes.is_empty() {
        "the reviewer gave no notes".into()
    } else {
        notes
    }))
}

/// `line` without markdown decoration around it.
fn clean(line: &str) -> &str {
    let mut line = line.trim();
    loop {
        let before = line;
        line = line
            .trim_start_matches(['>', '-', '*', '_', '`', '#'])
            .trim_end_matches(['*', '_', '`'])
            .trim();
        if line == before {
            return line;
        }
    }
}

/// The rest of `line` after `keyword` (any letter case), if it starts with it.
fn strip_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let head = line.get(..keyword.len())?;
    head.eq_ignore_ascii_case(keyword).then(|| {
        line[keyword.len()..]
            .trim_start_matches(['*', '_', '`'])
            .trim_start()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_verdicts() {
        assert_eq!(worker("Added the limiter.\n\nDONE"), Some(WorkerVerdict::Done));
        assert_eq!(worker("**DONE**\n\n"), Some(WorkerVerdict::Done));
        assert_eq!(worker("done."), Some(WorkerVerdict::Done));
        assert_eq!(
            worker("Tried.\nSTUCK: the API key is missing"),
            Some(WorkerVerdict::Stuck("the API key is missing".into()))
        );
        assert_eq!(
            worker("`STUCK`"),
            Some(WorkerVerdict::Stuck("the worker gave no reason".into()))
        );
        assert_eq!(worker("DONE\nand one more thing"), None);
        assert_eq!(worker("I am done with it"), None);
        assert_eq!(worker("Stuckness: none"), None);
        assert_eq!(worker(""), None);
    }

    #[test]
    fn reviewer_verdicts() {
        assert_eq!(
            reviewer("Looks right.\nVERDICT: APPROVE"),
            Some(ReviewerVerdict::Approve)
        );
        assert_eq!(reviewer("**Verdict: approve.**"), Some(ReviewerVerdict::Approve));
        assert_eq!(
            reviewer("Some notes.\nVERDICT: CHANGES: handle the empty list"),
            Some(ReviewerVerdict::Changes("handle the empty list".into()))
        );
        assert_eq!(
            reviewer("VERDICT: CHANGES:\n- add a test\n- fix the typo"),
            Some(ReviewerVerdict::Changes("- add a test\n- fix the typo".into()))
        );
        assert_eq!(
            reviewer("VERDICT: CHANGES"),
            Some(ReviewerVerdict::Changes("the reviewer gave no notes".into()))
        );
        assert_eq!(reviewer("VERDICT: APPROVE\nbut also fix x"), None);
        assert_eq!(reviewer("I approve"), None);
        assert_eq!(reviewer("VERDICT: maybe"), None);
    }
}
