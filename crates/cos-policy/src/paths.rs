//! Where a path points, relative to the worktree: inside it (and inside the
//! task's scope or not), outside it, at a secret, or somewhere cos can't tell.
//!
//! Paths are normalized lexically (`~`, `..`, relative to the current
//! directory). Symlinks are only followed through a [`PathProbe`]; the default
//! [`Lexical`] probe follows none, so the daemon should pass [`Filesystem`]
//! when the worktree exists on disk.

use std::path::{Component, Path, PathBuf};

/// Stands in for `~` when the request doesn't say where home is. No worktree
/// lives under it, so such paths count as outside.
const UNKNOWN_HOME: &str = "/~";

/// Resolves symlinks for the path checks.
pub trait PathProbe {
    /// Where `path` really points once symlinks are followed, or `None` when
    /// that can't be known (for example a dangling symlink).
    fn resolve(&self, path: &Path) -> Option<PathBuf>;
}

/// Takes paths at face value and follows no symlinks.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lexical;

impl PathProbe for Lexical {
    fn resolve(&self, path: &Path) -> Option<PathBuf> {
        Some(path.to_path_buf())
    }
}

/// Follows symlinks on disk: the longest existing ancestor is canonicalized
/// and the rest of the path appended, so files that don't exist yet resolve
/// too.
#[derive(Debug, Clone, Copy, Default)]
pub struct Filesystem;

impl PathProbe for Filesystem {
    fn resolve(&self, path: &Path) -> Option<PathBuf> {
        let mut existing = path.to_path_buf();
        let mut rest = Vec::new();
        loop {
            if std::fs::symlink_metadata(&existing).is_ok() {
                // An existing entry that won't canonicalize is a dangling or
                // looping symlink; nobody can say where it leads.
                let mut real = std::fs::canonicalize(&existing).ok()?;
                real.extend(rest.iter().rev());
                return Some(real);
            }
            let name = existing.file_name()?.to_owned();
            rest.push(name);
            if !existing.pop() {
                return None;
            }
        }
    }
}

/// Where a path points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Location {
    /// `/dev/null` and friends.
    Device,
    /// A key, token or credential file, wherever it is.
    Secret { path: String, kind: &'static str },
    /// Outside the worktree. `via_symlink` when it only looked inside.
    Outside { path: String, via_symlink: bool },
    /// Depends on something only known when the command runs.
    Unknown,
    Inside {
        /// Relative to the worktree root, `/`-separated; empty for the root.
        rel: String,
        /// Set for files that steer agents or hold settings.
        protected: Option<&'static str>,
        in_scope: bool,
    },
}

/// The facts the path checks need from the request.
pub(crate) struct PathRules<'a> {
    root: PathBuf,
    real_root: Option<PathBuf>,
    home: PathBuf,
    scope: Option<&'a [String]>,
    probe: &'a dyn PathProbe,
}

impl<'a> PathRules<'a> {
    pub fn new(
        root: &Path,
        home: Option<&Path>,
        scope: Option<&'a [String]>,
        probe: &'a dyn PathProbe,
    ) -> Self {
        let root = lexical(root);
        let real_root = probe.resolve(&root);
        Self {
            root,
            real_root,
            home: home.map(lexical).unwrap_or_else(|| PathBuf::from(UNKNOWN_HOME)),
            scope,
            probe,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Turns `raw` into an absolute, normalized path. `base` is the directory
    /// relative paths start from, `None` when it isn't known.
    pub fn absolute(&self, base: Option<&Path>, raw: &str) -> Option<PathBuf> {
        let path = if raw == "~" {
            self.home.clone()
        } else if let Some(rest) = raw.strip_prefix("~/") {
            self.home.join(rest)
        } else if raw.starts_with('~') {
            // `~user/...`: someone else's home.
            return None;
        } else if raw.starts_with('/') {
            PathBuf::from(raw)
        } else {
            base?.join(raw)
        };
        Some(lexical(&path))
    }

    /// Where `raw` points. `glob` says it contains unquoted wildcards, in
    /// which case the folder before the first wildcard decides inside or
    /// outside and the whole pattern is checked against secret names.
    pub fn locate(&self, base: Option<&Path>, raw: &str, glob: bool) -> Location {
        if raw.is_empty() {
            return Location::Unknown;
        }
        let Some(path) = self.absolute(base, raw) else {
            return Location::Unknown;
        };
        if is_device(&path) {
            return Location::Device;
        }
        let shown = path.display().to_string();
        if let Some(kind) = secret_kind(&path, &self.home, glob) {
            return Location::Secret { path: shown, kind };
        }
        let fixed = if glob { before_glob(&path) } else { path.clone() };
        let lexically_inside = fixed.starts_with(&self.root);
        let (Some(real), Some(real_root)) = (self.probe.resolve(&fixed), self.real_root.as_ref()) else {
            return Location::Unknown;
        };
        if real != fixed
            && let Some(kind) = secret_kind(&real, &self.home, false)
        {
            return Location::Secret {
                path: real.display().to_string(),
                kind,
            };
        }
        let Ok(real_rel) = real.strip_prefix(real_root) else {
            return Location::Outside {
                path: shown,
                via_symlink: lexically_inside,
            };
        };
        // Name the file by where it looked like it was, so a glob keeps its
        // wildcard part for the scope check.
        let rel = match path.strip_prefix(&self.root) {
            Ok(rel) => slash_path(rel),
            Err(_) => slash_path(real_rel),
        };
        Location::Inside {
            protected: protected_kind(&rel),
            in_scope: self.in_scope(&rel),
            rel,
        }
    }

    fn in_scope(&self, rel: &str) -> bool {
        match self.scope {
            None => true,
            Some(globs) => globs.iter().any(|pattern| scope_matches(pattern, rel)),
        }
    }
}

/// Normalizes `.` and `..` without touching the disk. `..` at the root stays
/// at the root, as the shell does.
pub(crate) fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if out.parent().is_some() {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn slash_path(path: &Path) -> String {
    let parts: Vec<_> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    parts.join("/")
}

fn is_device(path: &Path) -> bool {
    const DEVICES: &[&str] = &[
        "/dev/null",
        "/dev/stdout",
        "/dev/stderr",
        "/dev/stdin",
        "/dev/tty",
        "/dev/zero",
        "/dev/random",
        "/dev/urandom",
    ];
    DEVICES.iter().any(|device| path == Path::new(device)) || path.starts_with("/dev/fd")
}

fn has_glob(text: &str) -> bool {
    text.contains(['*', '?', '[', '{'])
}

/// The path up to (not including) its first wildcard component.
fn before_glob(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        if has_glob(&component.as_os_str().to_string_lossy()) {
            break;
        }
        out.push(component);
    }
    out
}

/// Folders that hold keys or credentials wherever they are.
const SECRET_DIRS: &[&str] = &[".ssh", ".aws", ".gnupg", ".password-store"];
/// Folders that hold credentials when they're directly in your home.
const HOME_SECRET_DIRS: &[&str] = &[
    ".docker",
    ".kube",
    ".azure",
    ".config/gh",
    ".config/gcloud",
    ".config/op",
    "Library/Keychains",
    ".cargo/credentials",
    ".cargo/credentials.toml",
];
const SECRET_FILES: &[&str] = &[
    ".envrc",
    ".netrc",
    "_netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".pgpass",
    ".htpasswd",
    "credentials",
    "credentials.json",
    "secrets.json",
    "secrets.yml",
    "secrets.yaml",
];
const SECRET_EXTENSIONS: &[&str] = &[
    ".pem",
    ".key",
    ".p12",
    ".pfx",
    ".jks",
    ".keystore",
    ".ppk",
    ".kdbx",
    ".keychain",
    ".keychain-db",
];
const SSH_KEY_PREFIXES: &[&str] = &["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"];
/// Committed templates of `.env`, without real values.
const ENV_TEMPLATES: &[&str] = &[
    ".env.example",
    ".env.sample",
    ".env.template",
    ".env.dist",
    ".env.defaults",
    ".env.schema",
];
/// Dotfiles a wildcard like `.*` or `.env*` could match.
const SECRET_DOTFILES: &[&str] = &[".env", ".env.local", ".envrc", ".netrc", ".npmrc", ".pypirc"];

/// Names the kind of secret `path` holds, if it looks like one.
pub(crate) fn secret_kind(path: &Path, home: &Path, glob: bool) -> Option<&'static str> {
    let components: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if components.iter().any(|c| SECRET_DIRS.contains(&c.as_str())) {
        return Some("a folder of keys or credentials");
    }
    if let Ok(in_home) = path.strip_prefix(home) {
        let in_home = slash_path(in_home);
        if HOME_SECRET_DIRS
            .iter()
            .any(|dir| in_home == *dir || in_home.starts_with(&format!("{dir}/")))
        {
            return Some("a folder of keys or credentials");
        }
    }
    let name = components.last()?;
    if secret_name(name) {
        return Some("a key, token or credential file");
    }
    if glob
        && has_glob(name)
        && SECRET_DOTFILES
            .iter()
            .any(|file| glob_matches_segment(name, file))
    {
        return Some("a key, token or credential file");
    }
    None
}

pub(crate) fn secret_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if ENV_TEMPLATES.contains(&lower.as_str()) {
        return false;
    }
    lower == ".env"
        || lower.starts_with(".env.")
        || (lower.ends_with(".env") && lower.len() > 4)
        || SECRET_FILES.contains(&lower.as_str())
        || SECRET_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
        || SSH_KEY_PREFIXES.iter().any(|prefix| lower.starts_with(prefix))
}

/// Files every agent in the worktree reads as instructions. The task-branch
/// check doesn't cover them, but an agent that rewrites them steers the
/// chief, the reviewer and every later worker, so changing them is never
/// delegated.
const INSTRUCTION_FILES: &[&str] = &["claude.md", "claude.local.md", "agents.md"];

/// Names files inside the worktree that change how agents behave or what
/// they may do. Editing them is never delegated. The folder and `.env*`
/// rules mirror `cos_core::taskgit::is_protected` (any depth, any letter
/// case), which rejects a task branch that touches them anyway.
fn protected_kind(rel: &str) -> Option<&'static str> {
    let parts: Vec<String> = rel.split('/').map(str::to_lowercase).collect();
    for part in &parts {
        match part.as_str() {
            ".claude" => return Some("Claude Code settings, hooks and commands (.claude/)"),
            ".git" => return Some("git's own files (.git)"),
            ".codex" | ".cos" => return Some("agent configuration"),
            ".mcp.json" => return Some("the MCP server configuration (.mcp.json)"),
            name if name.starts_with(".env") => return Some("an environment file (.env*)"),
            _ => {}
        }
    }
    match parts.last() {
        Some(name) if INSTRUCTION_FILES.contains(&name.as_str()) => {
            Some("instructions every agent in this worktree reads")
        }
        _ => None,
    }
}

/// Whether `pattern` (relative to the worktree, `/`-separated) covers `rel`
/// or one of its folders, so `src` and `src/**` both cover `src/a/b.rs`. The
/// rules are `cos_core::taskgit::Scope`'s: `*` and `?` within a segment, `**`
/// for any depth, `[` literal, anchored at the root, `..` never matches.
pub(crate) fn scope_matches(pattern: &str, rel: &str) -> bool {
    let pattern = pattern.trim();
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    let pattern: Vec<&str> = pattern
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if pattern.is_empty() || pattern.contains(&"..") {
        return false;
    }
    let path: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    (1..=path.len()).any(|end| scope_glob(&pattern, &path[..end]))
}

/// Matches path segments against pattern segments, where `**` stands for any
/// number of segments.
fn scope_glob(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| scope_glob(rest, &path[skip..])),
        Some((first, rest)) => path.split_first().is_some_and(|(name, others)| {
            let first: Vec<char> = first.chars().collect();
            let name: Vec<char> = name.chars().collect();
            match_chars(&first, &name, false) && scope_glob(rest, others)
        }),
    }
}

/// Matches one name the way the shell expands a wildcard: `*`, `?` and
/// `[...]`, and a leading dot has to be matched explicitly.
pub(crate) fn glob_matches_segment(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    match_chars(&pattern, &name, true)
}

/// `*` and `?`, plus `[...]` classes when `classes` is set; otherwise `[` is
/// literal.
fn match_chars(pattern: &[char], name: &[char], classes: bool) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|skip| match_chars(rest, &name[skip..], classes)),
        Some(('?', rest)) => !name.is_empty() && match_chars(rest, &name[1..], classes),
        Some(('[', rest)) if classes => {
            let Some(close) = rest.iter().skip(1).position(|&c| c == ']').map(|i| i + 1) else {
                return name.first() == Some(&'[') && match_chars(rest, &name[1..], classes);
            };
            let Some(&c) = name.first() else {
                return false;
            };
            let class = &rest[..close];
            let (negate, class) = match class.split_first() {
                Some(('!' | '^', tail)) => (true, tail),
                _ => (false, class),
            };
            let mut hit = false;
            let mut i = 0;
            while i < class.len() {
                if i + 2 < class.len() && class[i + 1] == '-' {
                    hit |= class[i] <= c && c <= class[i + 2];
                    i += 3;
                } else {
                    hit |= class[i] == c;
                    i += 1;
                }
            }
            hit != negate && match_chars(&rest[close + 1..], &name[1..], classes)
        }
        Some((p, rest)) => name.first() == Some(p) && match_chars(rest, &name[1..], classes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(scope: Option<&[String]>) -> PathRules<'_> {
        PathRules::new(
            Path::new("/wt/repo"),
            Some(Path::new("/home/me")),
            scope,
            &Lexical,
        )
    }

    #[test]
    fn normalizes_lexically() {
        let rules = rules(None);
        let cwd = Some(Path::new("/wt/repo/src"));
        assert_eq!(
            rules.absolute(cwd, "../a/./b"),
            Some(PathBuf::from("/wt/repo/a/b"))
        );
        assert_eq!(rules.absolute(cwd, "~/x"), Some(PathBuf::from("/home/me/x")));
        assert_eq!(rules.absolute(cwd, "/../../etc"), Some(PathBuf::from("/etc")));
        assert_eq!(rules.absolute(None, "rel"), None);
        assert_eq!(rules.absolute(cwd, "~root/x"), None);
    }

    #[test]
    fn locates_paths() {
        let scope = vec!["src/**".to_owned(), "docs/*.md".to_owned()];
        let rules = rules(Some(&scope));
        let cwd = Some(Path::new("/wt/repo"));
        assert!(matches!(
            rules.locate(cwd, "src/a/b.rs", false),
            Location::Inside {
                in_scope: true,
                protected: None,
                ..
            }
        ));
        assert!(matches!(
            rules.locate(cwd, "README.md", false),
            Location::Inside { in_scope: false, .. }
        ));
        assert!(matches!(
            rules.locate(cwd, "../other/x", false),
            Location::Outside { .. }
        ));
        assert!(matches!(
            rules.locate(cwd, ".env", false),
            Location::Secret { .. }
        ));
        assert!(matches!(
            rules.locate(cwd, ".env.example", false),
            Location::Inside { .. }
        ));
        assert!(matches!(
            rules.locate(cwd, ".env*", true),
            Location::Secret { .. }
        ));
        assert!(matches!(rules.locate(cwd, "*", true), Location::Inside { .. }));
        assert!(matches!(
            rules.locate(cwd, "~/.ssh/config", false),
            Location::Secret { .. }
        ));
        assert!(matches!(rules.locate(cwd, "/dev/null", false), Location::Device));
        assert!(matches!(
            rules.locate(cwd, ".claude/settings.json", false),
            Location::Inside {
                protected: Some(_),
                ..
            }
        ));
        assert!(matches!(rules.locate(None, "x", false), Location::Unknown));
    }

    #[test]
    fn matches_scope_globs() {
        assert!(scope_matches("src/**", "src/a/b.rs"));
        assert!(scope_matches("src", "src/a/b.rs"));
        assert!(scope_matches("docs/*.md", "docs/a.md"));
        assert!(!scope_matches("docs/*.md", "docs/a.txt"));
        assert!(scope_matches("**/*.rs", "a/b/c.rs"));
        assert!(!scope_matches("src/**", "srcx/a.rs"));
        assert!(scope_matches("app/[id].tsx", "app/[id].tsx"));
        assert!(!scope_matches("app/[id].tsx", "app/i.tsx"));
        assert!(scope_matches("*", ".hidden"));
        assert!(!scope_matches("../x", "x"));
        assert!(glob_matches_segment(".[e]nv", ".env"));
        assert!(!glob_matches_segment("*", ".env"));
    }

    struct FakeLinks;

    impl PathProbe for FakeLinks {
        fn resolve(&self, path: &Path) -> Option<PathBuf> {
            match path.strip_prefix("/wt/repo/escape") {
                Ok(rest) => Some(Path::new("/etc").join(rest)),
                Err(_) => Some(path.to_path_buf()),
            }
        }
    }

    #[test]
    fn follows_symlinks_through_the_probe() {
        let rules = PathRules::new(Path::new("/wt/repo"), None, None, &FakeLinks);
        assert_eq!(
            rules.locate(Some(Path::new("/wt/repo")), "escape/passwd", false),
            Location::Outside {
                path: "/wt/repo/escape/passwd".into(),
                via_symlink: true
            }
        );
    }

    #[test]
    fn filesystem_probe_resolves_missing_tails() {
        let dir = std::env::temp_dir();
        let real = Filesystem.resolve(&dir.join("cos-policy-missing/x")).unwrap();
        assert!(real.ends_with("cos-policy-missing/x"));
    }
}
