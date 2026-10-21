#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("could not start {binary}: {source}")]
    Spawn {
        binary: String,
        #[source]
        source: std::io::Error,
    },

    #[error("the agent exited{}", stderr_suffix(.stderr))]
    Exited { stderr: String },

    #[error("timed out waiting for the agent to answer `{request}`{}", stderr_suffix(.stderr))]
    Timeout { request: String, stderr: String },

    #[error("the agent refused `{request}`: {message}")]
    Rejected { request: String, message: String },

    #[error("no open permission request with id {0}")]
    UnknownRequest(String),

    #[error("the agent's input is already closed")]
    InputClosed,

    #[error("terminal: {0}")]
    Terminal(String),

    #[error("could not write to the agent: {0}")]
    Write(#[source] std::io::Error),
}

fn stderr_suffix(stderr: &str) -> String {
    if stderr.trim().is_empty() {
        String::new()
    } else {
        format!(". Its stderr:\n{}", stderr.trim_end())
    }
}
