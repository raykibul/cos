//! Trimming what goes to jev: short strings, and paths that don't name the
//! machine's folders.

use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// `text` cut to at most `max` characters, with `…` when cut.
pub fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// `text` with the worktree's absolute path written as `.` and the home
/// folder as `~`, so jev sees `./src/a.rs` and `~/.ssh` rather than where
/// they are on this machine.
pub fn relative_paths(text: &str, worktree_root: &Path, home: Option<&Path>) -> String {
    let mut out = text.to_owned();
    let root = worktree_root.to_string_lossy();
    let root = root.trim_end_matches('/');
    if !root.is_empty() {
        out = replace_path(&out, root, ".");
    }
    if let Some(home) = home {
        let home = home.to_string_lossy();
        let home = home.trim_end_matches('/');
        if !home.is_empty() {
            out = replace_path(&out, home, "~");
        }
    }
    out
}

/// Replaces `path` where it stands as a whole path (followed by `/`, the
/// end, or a character that can't continue a file name).
fn replace_path(text: &str, path: &str, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(path) {
        let after = &rest[at + path.len()..];
        let whole = after
            .chars()
            .next()
            .is_none_or(|next| next == '/' || !(next.is_alphanumeric() || "._-".contains(next)));
        out.push_str(&rest[..at]);
        out.push_str(if whole { with } else { path });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// A SHA-256 of the input as JSON, hex: the same input always gets the same
/// hash, so repeats and labelled examples can be matched up.
pub fn input_hash<T: Serialize>(input: &T) -> String {
    let json = serde_json::to_string(input).unwrap_or_default();
    let digest = Sha256::digest(json.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_on_characters() {
        assert_eq!(cut("  héllo  ", 10), "héllo");
        assert_eq!(cut("héllo", 2), "hé…");
    }

    #[test]
    fn paths_become_relative() {
        let root = Path::new("/Users/me/.cos/worktrees/app/oslo");
        let home = Some(Path::new("/Users/me"));

        assert_eq!(
            relative_paths("cat /Users/me/.cos/worktrees/app/oslo/src/a.rs", root, home),
            "cat ./src/a.rs"
        );
        assert_eq!(relative_paths("ls /Users/me/.ssh", root, home), "ls ~/.ssh");
        assert_eq!(relative_paths("cd /Users/me", root, home), "cd ~");
        // Another folder that merely starts the same way stays as it is.
        assert_eq!(
            relative_paths("ls /Users/me/.cos/worktrees/app/oslo-v2", root, home),
            "ls ~/.cos/worktrees/app/oslo-v2"
        );
    }

    #[test]
    fn the_hash_is_stable() {
        let a = input_hash(&serde_json::json!({ "b": 1, "a": 2 }));
        let b = input_hash(&serde_json::json!({ "a": 2, "b": 1 }));

        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }
}
