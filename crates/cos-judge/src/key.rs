//! Finding the API key, `TYPESAFE_API_KEY`. The daemon often runs under
//! launchd or systemd, which never read your shell's rc files, so it looks
//! in order:
//!
//! 1. its own environment;
//! 2. `daemon.env` in the cos home (`NAME=value` lines), refused if other
//!    users can read or write it;
//! 3. your login shell, once: `$SHELL -lic` printing the variable, with no
//!    stdin, stderr discarded and a 3 s timeout.
//!
//! The value is never logged, printed or put in an error; only where it was
//! found.

use std::collections::BTreeMap;
use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

/// The variable that holds the key.
pub const KEY_ENV: &str = "TYPESAFE_API_KEY";
/// How long the login shell gets to print it.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Around the value in the probe's output, so whatever the rc files print
/// (banners, prompts, escape codes) is ignored.
const START: &str = "<<cos-key-start>>";
const END: &str = "<<cos-key-end>>";
/// Most bytes read from the probe.
const MAX_PROBE_OUTPUT: u64 = 64 * 1024;

/// An API key. Its `Debug` and `Display` never show the value.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(Arc<str>);

impl ApiKey {
    /// `None` for an empty or blank value.
    pub fn new(value: &str) -> Option<Self> {
        let value = value.trim();
        (!value.is_empty()).then(|| Self(value.into()))
    }

    /// The value, for the `Authorization` header only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Where the key was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    /// The daemon's own environment.
    Environment,
    /// `daemon.env` in the cos home.
    DaemonEnv,
    /// Your login shell.
    Shell,
    Missing,
}

impl KeySource {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Environment => "the daemon's environment",
            Self::DaemonEnv => "daemon.env",
            Self::Shell => "your login shell",
            Self::Missing => "nowhere",
        }
    }
}

/// `daemon.env`, read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvFile {
    values: BTreeMap<String, String>,
}

impl EnvFile {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// The names it sets, for logs. Never the values.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Why `daemon.env` wasn't read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvFileError {
    #[error("there is no {0}")]
    Missing(String),
    #[error(
        "{path} is readable or writable by other users (mode {mode:o}), so it wasn't read; run `chmod 600 {path}`"
    )]
    Unsafe { path: String, mode: u32 },
    #[error("could not read {path}: {reason}")]
    Unreadable { path: String, reason: String },
}

/// Reads `NAME=value` lines from `path`, refusing a file other users can
/// read or write.
pub fn read_env_file(path: &Path) -> Result<EnvFile, EnvFileError> {
    let shown = path.display().to_string();
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(EnvFileError::Missing(shown));
        }
        Err(error) => {
            return Err(EnvFileError::Unreadable {
                path: shown,
                reason: error.to_string(),
            });
        }
    };
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(EnvFileError::Unsafe { path: shown, mode });
    }
    let text = std::fs::read_to_string(path).map_err(|error| EnvFileError::Unreadable {
        path: shown,
        reason: error.to_string(),
    })?;
    Ok(EnvFile {
        values: parse_env_file(&text),
    })
}

/// `NAME=value` lines as systemd's `EnvironmentFile` reads them: blank lines
/// and `#` or `;` comments skipped, an optional `export `, and one pair of
/// matching quotes around the value removed.
pub fn parse_env_file(text: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !is_name(name) {
            continue;
        }
        let value = value.trim();
        let value = ['"', '\'']
            .iter()
            .find_map(|quote| {
                value
                    .strip_prefix(*quote)
                    .and_then(|inner| inner.strip_suffix(*quote))
            })
            .unwrap_or(value);
        values.insert(name.to_owned(), value.to_owned());
    }
    values
}

/// `text` with `name` set to `value`: the line that sets it replaced, else
/// one added at the end. Other lines are kept as they are.
pub fn set_in_env_file(text: &str, name: &str, value: &str) -> String {
    let mut found = false;
    let mut lines = Vec::new();
    for existing in text.lines() {
        let trimmed = existing.trim_start();
        let trimmed = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim_start();
        let sets = trimmed
            .split_once('=')
            .is_some_and(|(existing, _)| existing.trim() == name);
        if !sets {
            lines.push(existing.to_owned());
        } else if !found {
            found = true;
            lines.push(format!("{name}={value}"));
        }
        // A later line setting it again would win in systemd; it goes.
    }
    if !found {
        lines.push(format!("{name}={value}"));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// What the login shell said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Found(ApiKey),
    /// The shell ran but the variable is empty or unset.
    NotSet,
    /// The shell couldn't be run, timed out or printed nothing usable.
    Failed(String),
}

/// Asks your login shell for `name`: `shell -lic` with a `printf` of the
/// variable between markers, no stdin, stderr discarded, killed after
/// `timeout`.
pub async fn probe_shell(shell: &Path, name: &str, timeout: Duration) -> Probe {
    if !is_name(name) {
        return Probe::Failed(format!("`{name}` isn't a variable name"));
    }
    let script = format!("printf '%s%s%s' '{START}' \"${name}\" '{END}'");
    let child = tokio::process::Command::new(shell)
        .arg("-lic")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return Probe::Failed(format!("could not run {}: {error}", shell.display())),
    };
    let mut stdout = child.stdout.take().expect("stdout is piped");
    // Stops at the end marker: something the rc files started in the
    // background may hold stdout open long after.
    let read = async {
        let mut output = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let read = stdout.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            output.extend_from_slice(&chunk[..read]);
            if output.len() as u64 > MAX_PROBE_OUTPUT || contains(&output, END.as_bytes()) {
                break;
            }
        }
        Ok::<_, std::io::Error>(output)
    };
    let output = match tokio::time::timeout(timeout, read).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Probe::Failed(format!("could not read {}'s output: {error}", shell.display()));
        }
        Err(_) => {
            let _ = child.start_kill();
            return Probe::Failed(format!(
                "{} didn't answer within {}s",
                shell.display(),
                timeout.as_secs()
            ));
        }
    };
    let _ = child.start_kill();
    let output = String::from_utf8_lossy(&output);
    // The last pair: rc files can't fake it after our printf ran.
    let Some(end) = output.rfind(END) else {
        return Probe::Failed(format!("{} printed nothing usable", shell.display()));
    };
    let Some(start) = output[..end].rfind(START) else {
        return Probe::Failed(format!("{} printed nothing usable", shell.display()));
    };
    match ApiKey::new(&output[start + START.len()..end]) {
        Some(key) => Probe::Found(key),
        None => Probe::NotSet,
    }
}

/// The key from the first two places, without running a shell: `env` is
/// the daemon's own value of the variable, `file` what reading `daemon.env`
/// gave. A value in the environment that equals the file's came from the
/// file (systemd loads it into the environment), so it's reported as
/// `daemon.env`.
pub fn resolve_without_shell(
    env: Option<&str>,
    file: &Result<EnvFile, EnvFileError>,
) -> Option<(ApiKey, KeySource)> {
    let from_file = file
        .as_ref()
        .ok()
        .and_then(|file| file.get(KEY_ENV))
        .and_then(ApiKey::new);
    match (env.and_then(ApiKey::new), from_file) {
        (Some(env), Some(file)) if env == file => Some((env, KeySource::DaemonEnv)),
        (Some(env), _) => Some((env, KeySource::Environment)),
        (None, Some(file)) => Some((file, KeySource::DaemonEnv)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn the_key_never_shows() {
        let key = ApiKey::new("  sk-secret  ").unwrap();

        assert_eq!(key.expose(), "sk-secret");
        assert!(!format!("{key:?} {key}").contains("secret"));
        assert_eq!(ApiKey::new("   "), None);
    }

    #[test]
    fn parses_env_lines_like_systemd() {
        let values = parse_env_file(
            "# comment\n\nA=1\nexport B = \"two words\"\nC='x=y'\n; other\nnot a line\n9X=bad\nD=\n",
        );

        assert_eq!(values.get("A").map(String::as_str), Some("1"));
        assert_eq!(values.get("B").map(String::as_str), Some("two words"));
        assert_eq!(values.get("C").map(String::as_str), Some("x=y"));
        assert_eq!(values.get("D").map(String::as_str), Some(""));
        assert!(!values.contains_key("9X"));
        assert_eq!(values.len(), 4);
    }

    #[test]
    fn sets_a_value_keeping_other_lines() {
        assert_eq!(set_in_env_file("", "K", "v"), "K=v\n");
        assert_eq!(
            set_in_env_file("# keys\nA=1\nK=old\nB=2\nexport K=dup\n", "K", "new"),
            "# keys\nA=1\nK=new\nB=2\n"
        );
        assert_eq!(set_in_env_file("A=1", "K", "v"), "A=1\nK=v\n");
    }

    #[test]
    fn refuses_an_env_file_others_can_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.env");
        fs::write(&path, format!("{KEY_ENV}=sk-file\n")).unwrap();

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let unsafe_file = read_env_file(&path);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let safe = read_env_file(&path).unwrap();

        let error = unsafe_file.unwrap_err();
        assert!(
            matches!(error, EnvFileError::Unsafe { mode: 0o644, .. }),
            "{error}"
        );
        assert!(error.to_string().contains("chmod 600"));
        assert_eq!(safe.get(KEY_ENV), Some("sk-file"));
        assert!(matches!(
            read_env_file(&dir.path().join("none")),
            Err(EnvFileError::Missing(_))
        ));
    }

    #[test]
    fn resolves_in_order() {
        let file = |value: &str| {
            Ok(EnvFile {
                values: [(KEY_ENV.to_owned(), value.to_owned())].into(),
            })
        };
        let none: Result<EnvFile, EnvFileError> = Err(EnvFileError::Missing("x".into()));

        let source = |env: Option<&str>, file: &Result<EnvFile, EnvFileError>| {
            resolve_without_shell(env, file).map(|(key, source)| (key.expose().to_owned(), source))
        };
        assert_eq!(
            source(Some("sk-env"), &file("sk-file")),
            Some(("sk-env".into(), KeySource::Environment))
        );
        assert_eq!(
            source(None, &file("sk-file")),
            Some(("sk-file".into(), KeySource::DaemonEnv))
        );
        // systemd loaded the file into the environment.
        assert_eq!(
            source(Some("sk-file"), &file("sk-file")),
            Some(("sk-file".into(), KeySource::DaemonEnv))
        );
        assert_eq!(source(Some(" "), &none), None);
        assert_eq!(source(None, &file("")), None);
    }

    /// A fake login shell: a script that prints noise, then runs the probe's
    /// `printf` with the variable set as the test says.
    fn fake_shell(dir: &Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("fake-shell");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn the_shell_probe_finds_the_key_among_noise() {
        let dir = tempfile::tempdir().unwrap();
        // $1 is -lic, $2 the script.
        let shell = fake_shell(
            dir.path(),
            &format!(
                "echo 'Welcome!'; printf '\\033[0m'; echo oops >&2; {KEY_ENV}=sk-shell; export {KEY_ENV}; eval \"$2\"; echo trailing"
            ),
        );

        let probe = probe_shell(&shell, KEY_ENV, Duration::from_secs(3)).await;

        assert_eq!(probe, Probe::Found(ApiKey::new("sk-shell").unwrap()));
    }

    #[tokio::test]
    async fn the_shell_probe_reports_an_unset_variable() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), &format!("unset {KEY_ENV}; eval \"$2\""));

        assert_eq!(
            probe_shell(&shell, KEY_ENV, Duration::from_secs(3)).await,
            Probe::NotSet
        );
    }

    #[tokio::test]
    async fn the_shell_probe_times_out_and_never_reads_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let hangs = fake_shell(dir.path(), "sleep 30");
        let reads = dir.path().join("reads");
        fs::write(
            &reads,
            "#!/bin/sh\nread line; printf '%s' \"<<cos-key-start>>$line<<cos-key-end>>\"\n",
        )
        .unwrap();
        fs::set_permissions(&reads, fs::Permissions::from_mode(0o755)).unwrap();

        let started = std::time::Instant::now();
        let slow = probe_shell(&hangs, KEY_ENV, Duration::from_millis(300)).await;
        let from_stdin = probe_shell(&reads, KEY_ENV, Duration::from_secs(3)).await;
        let missing = probe_shell(Path::new("/nonexistent/shell"), KEY_ENV, Duration::from_secs(1)).await;

        assert!(
            matches!(slow, Probe::Failed(ref why) if why.contains("didn't answer")),
            "{slow:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        // stdin is /dev/null, so `read` gets nothing.
        assert_eq!(from_stdin, Probe::NotSet);
        assert!(matches!(missing, Probe::Failed(_)));
    }
}
