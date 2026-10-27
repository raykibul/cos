//! pf redirects for forwards of ports below 1024 (`cos_bridge::redirect`).
//! The first one asks for the Mac's password, through macOS's own prompt,
//! to start this app again as a root helper (`--pf-redirect <socket>`). The
//! helper connects back to a socket in the user's temporary folder, applies
//! each table it is sent, and removes every rule when the app quits or
//! crashes. So the prompt comes once per launch, and nothing is installed.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use cos_bridge::redirect::{self, ANCHOR, Pf, Redirector, Table};

/// The flag that runs this binary as the helper.
pub(crate) const HELPER_FLAG: &str = "--pf-redirect";
/// How long the helper has to connect once the password is given.
const CONNECT_WAIT: Duration = Duration::from_secs(15);
/// How long the helper has to answer a table.
const REPLY_WAIT: Duration = Duration::from_secs(10);
const PFCTL: &str = "/sbin/pfctl";

/// The app's side: starts the helper when a redirect is first needed.
#[derive(Default)]
pub(crate) struct PfRedirector {
    helper: Mutex<Option<Helper>>,
}

impl Redirector for PfRedirector {
    fn apply(&self, table: &Table) -> Result<(), String> {
        let mut helper = self.helper.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(running) = helper.as_mut() {
            match running.send(table) {
                Ok(()) => return Ok(()),
                // Gone (killed, or pf failed it): start it again below.
                Err(_) => *helper = None,
            }
        }
        if table.is_empty() {
            // No helper, so no rules either.
            return Ok(());
        }
        let mut started = Helper::start()?;
        started.send(table)?;
        *helper = Some(started);
        Ok(())
    }
}

/// The connection to a running helper. Dropping it (or the app exiting)
/// makes the helper remove every rule and exit.
struct Helper {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Helper {
    /// Asks for the password and starts the helper, then waits for it to
    /// connect and say `ready`.
    fn start() -> Result<Self, String> {
        let socket = std::env::temp_dir().join(format!("cos-redirect-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)
            .map_err(|error| format!("couldn't make {}: {error}", socket.display()))?;
        let result = Self::launch(&socket).and_then(|()| Self::accept(&listener));
        let _ = std::fs::remove_file(&socket);
        let mut helper = result?;
        helper.reply()?;
        Ok(helper)
    }

    fn launch(socket: &Path) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let command = helper_command(&exe, socket);
        let output = Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "do shell script (item 1 of argv) with prompt (item 2 of argv) with administrator privileges",
                "-e",
                "end run",
                &command,
                "cos wants to redirect localhost ports below 1024 to the ports it forwards. It removes them when it quits.",
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("couldn't run osascript: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        let error = String::from_utf8_lossy(&output.stderr);
        if error.contains("-128") {
            return Err("you cancelled the password prompt".into());
        }
        Err(error.trim().to_owned())
    }

    fn accept(listener: &UnixListener) -> Result<Self, String> {
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + CONNECT_WAIT;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).map_err(|error| error.to_string())?;
                    stream
                        .set_read_timeout(Some(REPLY_WAIT))
                        .map_err(|error| error.to_string())?;
                    let reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
                    return Ok(Self {
                        writer: stream,
                        reader,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    return Err("the redirect helper didn't start".into());
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    fn send(&mut self, table: &Table) -> Result<(), String> {
        writeln!(self.writer, "{}", redirect::format_table(table)).map_err(|error| error.to_string())?;
        self.reply()
    }

    fn reply(&mut self) -> Result<(), String> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Err("the redirect helper stopped".into()),
            Ok(_) => redirect::read_reply(&line),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// The shell command the password prompt runs: the helper, in the
/// background so the prompt returns, with nothing on the terminal.
fn helper_command(exe: &Path, socket: &Path) -> String {
    format!(
        "{} {HELPER_FLAG} {} </dev/null >/dev/null 2>&1 &",
        shell_quote(&exe.to_string_lossy()),
        shell_quote(&socket.to_string_lossy())
    )
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The helper, as root: connects to the app's socket and serves it until
/// the app goes. Returns the process's exit code.
pub(crate) fn helper_main(socket: Option<PathBuf>) -> i32 {
    let Some(socket) = socket else { return 2 };
    let Ok(stream) = UnixStream::connect(&socket) else {
        return 1;
    };
    let Ok(reader) = stream.try_clone() else { return 1 };
    redirect::serve(BufReader::new(reader), stream, &mut Pfctl::default());
    0
}

/// pf through `pfctl`.
#[derive(Default)]
struct Pfctl {
    /// From `pfctl -E`, given back to `pfctl -X` so pf goes back to how it was.
    token: Option<String>,
}

impl Pf for Pfctl {
    fn enable(&mut self) -> Result<(), String> {
        // pf may be off with no rules loaded, and then nothing evaluates
        // com.apple/*: load macOS's own /etc/pf.conf, as booting would.
        let nat = pfctl(&["-s", "nat"], None)?;
        if !nat.contains("com.apple/*") {
            pfctl(&["-f", "/etc/pf.conf"], None)?;
        }
        let enabled = pfctl(&["-E"], None)?;
        self.token = token(&enabled);
        Ok(())
    }

    fn load(&mut self, rules: &str) -> Result<(), String> {
        if rules.is_empty() {
            pfctl(&["-a", ANCHOR, "-F", "all"], None).map(drop)
        } else {
            pfctl(&["-a", ANCHOR, "-f", "-"], Some(rules)).map(drop)
        }
    }

    fn disable(&mut self) {
        if let Some(token) = self.token.take() {
            let _ = pfctl(&["-X", &token], None);
        }
    }
}

/// Runs pfctl, with `input` on stdin, and returns stdout and stderr
/// together (pfctl says most things on stderr).
fn pfctl(args: &[&str], input: Option<&str>) -> Result<String, String> {
    let mut child = Command::new(PFCTL)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("couldn't run pfctl: {error}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(text)
    } else {
        Err(format!("pfctl {}: {}", args.join(" "), text.trim()))
    }
}

/// The token in `pfctl -E`'s output: `Token : 12345`.
fn token(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "Token")
            .then(|| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_runs_the_helper_quoted_and_in_the_background() {
        let command = helper_command(
            Path::new("/Applications/cos.app/Contents/MacOS/cos"),
            Path::new("/var/folders/x/T/it's.sock"),
        );
        assert_eq!(
            command,
            r"'/Applications/cos.app/Contents/MacOS/cos' --pf-redirect '/var/folders/x/T/it'\''s.sock' </dev/null >/dev/null 2>&1 &"
        );
    }

    #[test]
    fn the_token_is_read_from_pfctl() {
        let output = "No ALTQ support in kernel\nALTQ related functions disabled\npf enabled\nToken : 13281929583218417559\n";
        assert_eq!(token(output).as_deref(), Some("13281929583218417559"));
        assert_eq!(token("pf already enabled\n"), None);
    }

    #[test]
    fn no_table_needs_no_helper() {
        let redirector = PfRedirector::default();
        assert_eq!(redirector.apply(&Table::new()), Ok(()));
        assert!(redirector.helper.lock().unwrap().is_none());
    }
}
