//! Running a task's test command: `sh -c` in its own process group, with a
//! timeout, keeping the tail of stdout and stderr interleaved.

use std::collections::VecDeque;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::net::unix::pipe;
use tokio::process::{Child, Command};

use super::{TaskGitError, TaskWorktree};

pub const DEFAULT_TEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// How long the command gets between SIGTERM and SIGKILL after a timeout.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// How long to keep reading output once the command's processes are gone. A
/// process that left the group (`setsid`, a daemon) can hold the pipe open.
const DRAIN: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOptions {
    pub timeout: Duration,
    /// Only the last this-many bytes of output are kept.
    pub max_output_bytes: usize,
}

impl Default for TestOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TEST_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
    Passed,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestOutcome {
    pub status: TestStatus,
    /// The shell's exit code; `None` if a signal ended it.
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    /// The tail of stdout and stderr, interleaved as written.
    pub output: String,
    /// Whether earlier output was dropped to stay within the limit.
    pub truncated: bool,
    pub duration_ms: u64,
}

impl TaskWorktree {
    /// Runs the task's test command in its worktree. See [`run_tests`].
    pub async fn run_tests(&self, command: &str, options: &TestOptions) -> Result<TestOutcome, TaskGitError> {
        run_tests(&self.path, command, options).await
    }
}

/// Runs `sh -c <command>` in `dir`, with stdin closed, in a new process group.
///
/// On a timeout the whole group gets SIGTERM, then SIGKILL after a short
/// grace. Whatever the command leaves running in its group when it exits
/// (servers started in the background) is killed too. An `Err` means the
/// command couldn't be started; a failing command is an `Ok` outcome.
pub async fn run_tests(
    dir: &Path,
    command: &str,
    options: &TestOptions,
) -> Result<TestOutcome, TaskGitError> {
    if command.trim().is_empty() {
        return Err(TaskGitError::Invalid("the test command is empty".into()));
    }
    let cannot_start = |error: std::io::Error| {
        TaskGitError::Invalid(format!("could not run the test command {command:?}: {error}"))
    };

    // One pipe for both streams, so the output reads in the order it was written.
    let (reader, writer) = std::io::pipe().map_err(cannot_start)?;
    let mut shell = Command::new("sh");
    shell
        .arg("-c")
        .arg(command)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(writer.try_clone().map_err(cannot_start)?)
        .stderr(writer)
        .process_group(0)
        .kill_on_drop(true);
    let started = Instant::now();
    let mut child = shell.spawn().map_err(cannot_start)?;
    // Our copies of the write end must close, or the pipe never ends.
    drop(shell);
    let mut group = Group(child.id().and_then(|id| Pid::from_raw(id as i32)));
    let mut output = pipe::Receiver::from_owned_fd(reader.into()).map_err(cannot_start)?;

    let mut tail = Tail::new(options.max_output_bytes);
    let mut buf = vec![0; 16 * 1024];
    let mut open = true;
    let deadline = tokio::time::sleep(options.timeout);
    tokio::pin!(deadline);
    let finished = loop {
        tokio::select! {
            status = child.wait() => break Some(status.map_err(cannot_start)?),
            read = output.read(&mut buf), if open => match read {
                Ok(0) | Err(_) => open = false,
                Ok(n) => tail.push(&buf[..n]),
            },
            () = &mut deadline => break None,
        }
    };

    let status = match finished {
        Some(status) => {
            // The shell is done; nothing it started may outlive it.
            group.signal(Signal::KILL);
            status
        }
        None => stop(&mut child, &group).await.map_err(cannot_start)?,
    };
    group.0 = None;

    if open {
        let drain = async {
            while let Ok(n @ 1..) = output.read(&mut buf).await {
                tail.push(&buf[..n]);
            }
        };
        let _ = tokio::time::timeout(DRAIN, drain).await;
    }

    let (output, truncated) = tail.finish();
    Ok(TestOutcome {
        status: match finished {
            None => TestStatus::TimedOut,
            Some(status) if status.success() => TestStatus::Passed,
            Some(_) => TestStatus::Failed,
        },
        exit_code: status.code(),
        signal: status.signal(),
        output,
        truncated,
        duration_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
    })
}

/// SIGTERM to the group, a grace period for the shell to exit, then SIGKILL
/// to whatever is left.
async fn stop(child: &mut Child, group: &Group) -> std::io::Result<ExitStatus> {
    group.signal(Signal::TERM);
    let exited = tokio::time::timeout(KILL_GRACE, child.wait()).await;
    group.signal(Signal::KILL);
    match exited {
        Ok(status) => status,
        Err(_) => child.wait().await,
    }
}

/// The command's process group. Dropping it while armed (the future was
/// cancelled) kills the group.
struct Group(Option<Pid>);

impl Group {
    fn signal(&self, signal: Signal) {
        if let Some(pid) = self.0 {
            // Fails only if the group is already gone.
            let _ = kill_process_group(pid, signal);
        }
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.signal(Signal::KILL);
    }
}

/// The last `max` bytes written.
struct Tail {
    bytes: VecDeque<u8>,
    max: usize,
    truncated: bool,
}

impl Tail {
    fn new(max: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            max,
            truncated: false,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        let chunk = if chunk.len() > self.max {
            self.truncated = true;
            &chunk[chunk.len() - self.max..]
        } else {
            chunk
        };
        let overflow = (self.bytes.len() + chunk.len()).saturating_sub(self.max);
        if overflow > 0 {
            self.truncated = true;
            self.bytes.drain(..overflow);
        }
        self.bytes.extend(chunk);
    }

    fn finish(self) -> (String, bool) {
        let bytes: Vec<u8> = self.bytes.into();
        (String::from_utf8_lossy(&bytes).into_owned(), self.truncated)
    }
}
