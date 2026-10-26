//! The Chief of Staff's tools, as the daemon runs them. `cos mcp` forwards
//! each call here with the calling agent's id; the tool list and its schemas
//! live in [`crate::mcp`]. Every tool works only on the chief's own project.

use std::time::Duration;

use axum::http::StatusCode;
use cos_core::staff::MAX_NOTES;
use cos_core::{Agent, AgentState, Base, Role, StaffNote, Workspace};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{Staff, error, prompt};
use crate::daemon::agents::StartAgent;
use crate::daemon::api::ApiError;
use crate::daemon::attachments;

/// `last_message` in `workspaces_list` is cut to this many characters.
const LIST_MESSAGE_CHARS: usize = 500;
const MAX_SEND_CHARS: usize = 100_000;
/// How often `agent_send` checks whether a resumed agent is ready.
const READY_POLL: Duration = Duration::from_millis(100);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceCreate {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    from: Option<Base>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentSpawn {
    prompt: String,
    #[serde(default)]
    workspace_id: Option<i64>,
    #[serde(default)]
    new_workspace: Option<WorkspaceCreate>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    permission_mode: Option<String>,
    /// Files in the chief's attachments, by path or name, copied into the
    /// agent's workspace and named in its prompt.
    #[serde(default)]
    attachments: Vec<String>,
    /// False: its news skips the chief's inbox.
    #[serde(default = "yes")]
    report: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentSend {
    agent_id: i64,
    message: String,
    #[serde(default)]
    attachments: Vec<String>,
    /// Changes whether it reports to the chief, from this message on.
    #[serde(default)]
    report: Option<bool>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentRead {
    agent_id: i64,
    #[serde(default)]
    screen: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentStop {
    agent_id: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BriefUpdate {
    brief: String,
}

/// `memory_read` and `memory_delete`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRead {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryWrite {
    name: String,
    description: String,
    body: String,
}

impl Staff {
    /// Runs one tool for the Chief of Staff `agent_id` and returns the text
    /// it reads. Errors are meant for it too.
    pub(crate) async fn call_tool(
        &self,
        agent_id: i64,
        name: &str,
        arguments: Value,
    ) -> Result<String, ApiError> {
        let inner = &self.inner;
        let agent = inner.agents.get(agent_id)?;
        let project_id = match agent.role {
            Role::Staff => inner.books.project_of(agent_id)?,
            _ => None,
        };
        let Some(project_id) = project_id else {
            return Err(error(
                StatusCode::FORBIDDEN,
                format!("agent {agent_id} is not a project's Chief of Staff"),
            ));
        };
        match name {
            "workspaces_list" => self.workspaces_list(project_id).await,
            "workspace_create" => {
                let workspace = self.workspace_create(project_id, parse(name, arguments)?).await?;
                Ok(pretty(&workspace))
            }
            "agent_spawn" => {
                self.agent_spawn(project_id, agent_id, parse(name, arguments)?)
                    .await
            }
            "agent_send" => {
                self.agent_send(project_id, agent_id, parse(name, arguments)?)
                    .await
            }
            "agent_read" => self.agent_read(project_id, parse(name, arguments)?).await,
            "agent_stop" => self.agent_stop(project_id, parse(name, arguments)?),
            "brief_update" => self.brief_update(project_id, parse(name, arguments)?).await,
            "memory_list" => self.memory_list(project_id),
            "memory_read" => self.memory_read(project_id, parse(name, arguments)?),
            "memory_write" => self.memory_write(project_id, parse(name, arguments)?).await,
            "memory_delete" => self.memory_delete(project_id, parse(name, arguments)?).await,
            _ => Err(ApiError::not_found(&format!("there is no tool {name}"))),
        }
    }

    async fn workspaces_list(&self, project_id: i64) -> Result<String, ApiError> {
        let inner = &self.inner;
        let project = inner.projects.get(project_id).await?;
        let mut workspaces = Vec::new();
        for workspace in &project.workspaces {
            let agents: Vec<Value> = inner
                .agents
                .list(Some(workspace.id), None)?
                .iter()
                .map(|agent| {
                    json!({
                        "id": agent.id,
                        "title": agent.title,
                        "state": agent.state,
                        "model": agent.model,
                        "spawned_by": agent.spawned_by,
                        "report": agent.report,
                        "last_message": agent.last_message.as_deref().map(|text| cut_chars(text, LIST_MESSAGE_CHARS)),
                    })
                })
                .collect();
            workspaces.push(json!({
                "id": workspace.id,
                "title": workspace.title,
                "city": workspace.city,
                "branch": workspace.branch,
                "path": workspace.path,
                "agents": agents,
            }));
        }
        Ok(pretty(&json!({ "workspaces": workspaces })))
    }

    async fn workspace_create(
        &self,
        project_id: i64,
        create: WorkspaceCreate,
    ) -> Result<Workspace, ApiError> {
        let projects = &self.inner.projects;
        let base = match create.from {
            Some(base) => base,
            None if projects.get(project_id).await?.has_origin => Base::Origin,
            None => Base::Head,
        };
        Ok(projects
            .create_workspace(project_id, base, create.title.as_deref())
            .await?)
    }

    async fn agent_spawn(
        &self,
        project_id: i64,
        staff_id: i64,
        spawn: AgentSpawn,
    ) -> Result<String, ApiError> {
        let inner = &self.inner;
        let task = spawn.prompt.trim();
        if task.is_empty() {
            return Err(error(StatusCode::BAD_REQUEST, "the prompt is empty".into()));
        }
        let workspace = match (spawn.workspace_id, spawn.new_workspace) {
            (Some(id), None) => self.own_workspace(project_id, id)?,
            (None, Some(create)) => self.workspace_create(project_id, create).await?,
            _ => {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "give exactly one of workspace_id or new_workspace".into(),
                ));
            }
        };
        let project = inner.projects.get(project_id).await?;
        let attached = self.hand_over(staff_id, &spawn.attachments, &workspace.path)?;
        let task_and_files = attachments::with_attachments(task, &attached);
        let request = StartAgent {
            prompt: Some(format!(
                "{task_and_files}\n\n{}",
                prompt::spawned_line(&project.name)
            )),
            // Named after the task, not the line cos adds.
            title: spawn.title.or_else(|| Some(task.to_owned())),
            model: spawn.model,
            permission_mode: spawn.permission_mode,
            ..StartAgent::default()
        };
        let launch = cos_core::Launch {
            spawned_by: Some(staff_id),
            report: spawn.report,
            ..cos_core::Launch::default()
        };
        let agent = inner
            .agents
            .start_agent_with(workspace.id, Role::User, &request, &launch)?;
        Ok(pretty(
            &json!({ "agent_id": agent.id, "workspace_id": workspace.id }),
        ))
    }

    async fn agent_send(&self, project_id: i64, staff_id: i64, send: AgentSend) -> Result<String, ApiError> {
        let inner = &self.inner;
        let agent = self.own_agent(project_id, send.agent_id)?;
        let message = send.message.trim();
        if message.is_empty() || message.chars().count() > MAX_SEND_CHARS {
            return Err(error(
                StatusCode::BAD_REQUEST,
                format!("the message must be 1 to {MAX_SEND_CHARS} characters"),
            ));
        }
        let attached = self.hand_over(staff_id, &send.attachments, &inner.agents.folder(agent.id)?)?;
        let message = attachments::with_attachments(message, &attached);
        let message = message.as_str();
        // Before the message goes in, so the turn it starts follows the new setting.
        let report = send.report.filter(|&report| report != agent.report);
        if let Some(report) = report {
            inner.agents.set_report(agent.id, report)?;
        }
        let resumed = !agent.state.is_running();
        if resumed {
            inner.agents.resume(agent.id)?;
        }
        self.until_ready(agent.id).await?;
        inner.agents.paste_message(agent.id, message).await?;
        let sent = if resumed {
            format!("Resumed agent {} and sent the message.", agent.id)
        } else {
            format!("Sent to agent {}.", agent.id)
        };
        Ok(match report {
            Some(true) => format!("{sent} It reports to you again."),
            Some(false) => format!("{sent} It no longer reports to you."),
            None => sent,
        })
    }

    /// Waits until a (re)starting agent can take a message.
    async fn until_ready(&self, agent_id: i64) -> Result<(), ApiError> {
        let deadline = tokio::time::Instant::now() + self.inner.config.ready_wait;
        loop {
            let agent = self.inner.agents.get(agent_id)?;
            match agent.state {
                AgentState::Starting if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(READY_POLL).await;
                }
                AgentState::Starting => {
                    return Err(error(
                        StatusCode::CONFLICT,
                        format!("agent {agent_id} is still starting; try again shortly"),
                    ));
                }
                AgentState::NeedsTrust => {
                    return Err(error(
                        StatusCode::CONFLICT,
                        format!("agent {agent_id} waits for the user to trust its folder"),
                    ));
                }
                AgentState::NeedsLogin => {
                    return Err(error(
                        StatusCode::CONFLICT,
                        format!("agent {agent_id} waits for the user to log in to Claude in its terminal"),
                    ));
                }
                AgentState::Exited | AgentState::Interrupted => {
                    return Err(error(
                        StatusCode::CONFLICT,
                        format!("agent {agent_id} exited before it could take the message"),
                    ));
                }
                AgentState::Idle | AgentState::Working | AgentState::WaitingApproval => return Ok(()),
            }
        }
    }

    async fn agent_read(&self, project_id: i64, read: AgentRead) -> Result<String, ApiError> {
        let inner = &self.inner;
        let agent = self.own_agent(project_id, read.agent_id)?;
        let from_transcript = inner
            .agents
            .transcript(agent.id, 0)
            .await
            .ok()
            .and_then(|transcript| transcript.last_reply().map(str::to_owned));
        let mut result = json!({
            "state": agent.state,
            "last_message": from_transcript.or(agent.last_message),
        });
        if read.screen {
            result["screen"] = json!(inner.agents.screen(agent.id)?.text);
        }
        Ok(pretty(&result))
    }

    fn agent_stop(&self, project_id: i64, stop: AgentStop) -> Result<String, ApiError> {
        let agent = self.own_agent(project_id, stop.agent_id)?;
        self.inner.agents.stop(agent.id)?;
        Ok(format!("Stopping agent {}.", agent.id))
    }

    async fn brief_update(&self, project_id: i64, update: BriefUpdate) -> Result<String, ApiError> {
        let brief = self.inner.books.set_brief(project_id, &update.brief)?;
        self.inner.brief_or_notes_changed(project_id).await?;
        Ok(format!("Brief saved ({} bytes).", brief.len()))
    }

    fn memory_list(&self, project_id: i64) -> Result<String, ApiError> {
        let notes: Vec<Value> = self
            .inner
            .books
            .notes(project_id)?
            .iter()
            .map(|note| {
                json!({
                    "name": note.name,
                    "description": note.description,
                    "bytes": note.body.len(),
                    "updated": updated(note.updated_ms),
                })
            })
            .collect();
        Ok(pretty(&json!({ "notes": notes, "max_notes": MAX_NOTES })))
    }

    fn memory_read(&self, project_id: i64, read: MemoryRead) -> Result<String, ApiError> {
        let note = self.note(project_id, &read.name)?;
        Ok(pretty(&json!({
            "name": note.name,
            "description": note.description,
            "updated": updated(note.updated_ms),
            "body": note.body,
        })))
    }

    async fn memory_write(&self, project_id: i64, write: MemoryWrite) -> Result<String, ApiError> {
        let existed = self.inner.books.note(project_id, &write.name)?.is_some();
        let note = self
            .inner
            .books
            .write_note(project_id, &write.name, &write.description, &write.body)?;
        self.inner.brief_or_notes_changed(project_id).await?;
        let verb = if existed { "Replaced" } else { "Saved" };
        Ok(format!(
            "{verb} the note {} ({} bytes).",
            note.name,
            note.body.len()
        ))
    }

    async fn memory_delete(&self, project_id: i64, delete: MemoryRead) -> Result<String, ApiError> {
        let note = self.note(project_id, &delete.name)?;
        self.inner.books.delete_note(project_id, &note.name)?;
        self.inner.brief_or_notes_changed(project_id).await?;
        Ok(format!("Deleted the note {}.", note.name))
    }

    /// One of the project's notes, or a 404 that lists the names there are.
    fn note(&self, project_id: i64, name: &str) -> Result<StaffNote, ApiError> {
        if let Some(note) = self.inner.books.note(project_id, name)? {
            return Ok(note);
        }
        let names: Vec<String> = self
            .inner
            .books
            .notes(project_id)?
            .into_iter()
            .map(|note| note.name)
            .collect();
        let known = if names.is_empty() {
            "there are no notes yet".to_owned()
        } else {
            format!("the notes are: {}", names.join(", "))
        };
        Err(ApiError::not_found(&format!(
            "there is no note {:?}; {known}",
            name.trim()
        )))
    }

    /// Copies files from the chief's attachments into `folder`, where an
    /// agent it starts or messages can read them.
    fn hand_over(
        &self,
        staff_id: i64,
        names: &[String],
        folder: &std::path::Path,
    ) -> Result<Vec<attachments::Attachment>, ApiError> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let own = self.inner.agents.folder(staff_id)?;
        attachments::find_all(&own, names)?
            .iter()
            .map(|attachment| attachments::copy_into(attachment, folder))
            .collect()
    }

    /// An agent in one of the project's workspaces, not the chief itself.
    fn own_agent(&self, project_id: i64, agent_id: i64) -> Result<Agent, ApiError> {
        let agent = self.inner.agents.get(agent_id)?;
        let workspace = self.inner.projects.workspace(agent.workspace_id)?;
        if workspace.project_id != project_id || workspace.staff {
            return Err(error(
                StatusCode::FORBIDDEN,
                format!("agent {agent_id} is not in one of this project's workspaces"),
            ));
        }
        Ok(agent)
    }

    /// One of the project's workspaces.
    fn own_workspace(&self, project_id: i64, workspace_id: i64) -> Result<Workspace, ApiError> {
        let workspace = self.inner.projects.listed_workspace(workspace_id)?;
        if workspace.project_id != project_id {
            return Err(error(
                StatusCode::FORBIDDEN,
                format!("workspace {workspace_id} belongs to another project"),
            ));
        }
        Ok(workspace)
    }
}

/// When a note last changed, as an RFC 3339 time the chief can read.
fn updated(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|at| at.round(jiff::Unit::Second).unwrap_or(at).to_string())
        .unwrap_or_default()
}

fn pretty(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn cut_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max).collect();
    cut.push('…');
    cut
}

fn parse<T: DeserializeOwned>(tool: &str, arguments: Value) -> Result<T, ApiError> {
    let arguments = if arguments.is_null() { json!({}) } else { arguments };
    serde_json::from_value(arguments).map_err(|failure| {
        error(
            StatusCode::BAD_REQUEST,
            format!("bad arguments for {tool}: {failure}"),
        )
    })
}
