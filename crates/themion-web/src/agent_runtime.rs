use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use themion_core::agent::{Agent, AgentEvent, BuildIdentity};
use themion_core::client::{ChatBackend, ChatClient};
use themion_core::client_codex::CodexClient;
use themion_core::db::DbHandle;
use themion_core::CodexAuth;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

const OPENROUTER_DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const OPENROUTER_DEFAULT_MODEL: &str = "minimax/minimax-m2.7";
const LLAMACPP_DEFAULT_BASE_URL: &str = "http://localhost:8080/v1";
const LLAMACPP_DEFAULT_MODEL: &str = "local";
const CODEX_DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const CODEX_DEFAULT_MODEL: &str = "gpt-5.4";
const DEFAULT_SYSTEM_PROMPT: &str =
    "You are an expert coding assistant operating inside Themion, a terminal-based coding agent.";

#[derive(Clone)]
pub struct AgentRuntimeService {
    request_tx: mpsc::UnboundedSender<AgentRuntimeRequest>,
}

enum AgentRuntimeRequest {
    Snapshot {
        response_tx: oneshot::Sender<Result<AgentRosterSnapshot>>,
    },
    SubmitPrompt {
        agent_id: String,
        prompt: String,
        response_tx: oneshot::Sender<Result<()>>,
    },
    Subscribe {
        agent_id: String,
        response_tx: oneshot::Sender<Result<mpsc::UnboundedReceiver<AgentRuntimeEvent>>>,
    },
    CreateAgent {
        label: Option<String>,
        roles: Vec<String>,
        response_tx: oneshot::Sender<Result<CreatedAgent>>,
    },
    DeleteAgent {
        agent_id: String,
        response_tx: oneshot::Sender<Result<DeletedAgent>>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentRosterSnapshot {
    pub agents: Vec<AgentSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentSummary {
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
    pub busy: bool,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub warning: Option<String>,
}

#[derive(Clone, Debug)]
pub enum AgentRuntimeEvent {
    Snapshot(AgentSnapshot),
    RosterUpdated(AgentRosterSnapshot),
    Busy { agent_id: String, busy: bool },
    TranscriptDelta(TranscriptDelta),
    Completed { agent_id: String },
    Failed { agent_id: String, message: String },
    Deleted { agent_id: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentSnapshot {
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
    pub busy: bool,
    pub provider: String,
    pub model: String,
    pub transcript: Vec<TranscriptEntry>,
    pub status: String,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TranscriptEntry {
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct TranscriptDelta {
    pub agent_id: String,
    pub kind: String,
    pub text: String,
    pub replace_last: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreatedAgent {
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeletedAgent {
    pub agent_id: String,
}

struct AgentRuntimeState {
    db: Arc<DbHandle>,
    project_dir: PathBuf,
    bootstrap: WebAgentBootstrap,
    agents: HashMap<String, AgentEntry>,
}

struct AgentEntry {
    summary: AgentSummary,
    session_id: Uuid,
    transcript: Vec<TranscriptEntry>,
    subscribers: Vec<mpsc::UnboundedSender<AgentRuntimeEvent>>,
}

#[derive(Clone)]
struct WebAgentBootstrap {
    provider: String,
    base_url: String,
    api_key: Option<String>,
    model: String,
    system_prompt: String,
    active_profile: String,
    configured_profile: String,
    profiles: HashMap<String, WebProfileConfig>,
}

#[derive(Clone)]
struct WebProfileConfig {
    provider: Option<String>,
}

pub async fn start_agent_runtime() -> Result<AgentRuntimeService> {
    let db_path = resolve_system_db_path();
    let db = DbHandle::open(&db_path)
        .with_context(|| format!("failed to open themion db at {}", db_path.display()))?;
    let bootstrap = load_bootstrap()?;
    let project_dir = std::env::current_dir().context("failed to resolve project dir")?;
    let mut agents = HashMap::new();
    agents.insert(
        "master".to_string(),
        AgentEntry {
            summary: AgentSummary {
                agent_id: "master".to_string(),
                label: "master".to_string(),
                roles: vec!["master".to_string(), "interactive".to_string()],
                busy: false,
                provider: bootstrap.provider.clone(),
                model: bootstrap.model.clone(),
                status: "idle".to_string(),
                warning: None,
            },
            session_id: Uuid::new_v4(),
            transcript: Vec::new(),
            subscribers: Vec::new(),
        },
    );

    let state = Arc::new(Mutex::new(AgentRuntimeState {
        db,
        project_dir,
        bootstrap,
        agents,
    }));
    let (request_tx, mut request_rx) = mpsc::unbounded_channel();
    let state_for_loop = Arc::clone(&state);
    tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            handle_request(&state_for_loop, request).await;
        }
    });

    Ok(AgentRuntimeService { request_tx })
}

async fn handle_request(state: &Arc<Mutex<AgentRuntimeState>>, request: AgentRuntimeRequest) {
    match request {
        AgentRuntimeRequest::Snapshot { response_tx } => {
            let _ = response_tx.send(snapshot_state(state));
        }
        AgentRuntimeRequest::Subscribe {
            agent_id,
            response_tx,
        } => {
            let _ = response_tx.send(subscribe_agent(state, &agent_id));
        }
        AgentRuntimeRequest::SubmitPrompt {
            agent_id,
            prompt,
            response_tx,
        } => {
            let result = begin_agent_turn(state, &agent_id, prompt).await;
            let _ = response_tx.send(result);
        }
        AgentRuntimeRequest::CreateAgent {
            label,
            roles,
            response_tx,
        } => {
            let _ = response_tx.send(create_agent(state, label, roles));
        }
        AgentRuntimeRequest::DeleteAgent {
            agent_id,
            response_tx,
        } => {
            let _ = response_tx.send(delete_agent(state, &agent_id));
        }
    }
}

fn snapshot_state(state: &Arc<Mutex<AgentRuntimeState>>) -> Result<AgentRosterSnapshot> {
    let state = state
        .lock()
        .map_err(|_| anyhow!("agent runtime poisoned"))?;
    Ok(AgentRosterSnapshot {
        agents: collect_sorted_summaries(&state.agents),
    })
}

fn subscribe_agent(
    state: &Arc<Mutex<AgentRuntimeState>>,
    agent_id: &str,
) -> Result<mpsc::UnboundedReceiver<AgentRuntimeEvent>> {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut state = state
        .lock()
        .map_err(|_| anyhow!("agent runtime poisoned"))?;
    let entry = state
        .agents
        .get_mut(agent_id)
        .ok_or_else(|| anyhow!("unknown agent_id {}", agent_id))?;
    tx.send(AgentRuntimeEvent::Snapshot(build_agent_snapshot(entry)))
        .ok();
    entry.subscribers.push(tx);
    Ok(rx)
}

fn create_agent(
    state: &Arc<Mutex<AgentRuntimeState>>,
    label: Option<String>,
    roles: Vec<String>,
) -> Result<CreatedAgent> {
    let mut state = state
        .lock()
        .map_err(|_| anyhow!("agent runtime poisoned"))?;
    let roles = normalize_created_agent_roles(roles);
    if roles
        .iter()
        .any(|role| normalize_primary_role(role) == "master")
    {
        anyhow::bail!("cannot create another master agent");
    }

    let agent_id = allocate_default_agent_id(&state.agents);
    let label = label
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| agent_id.clone());
    let warning = warning_for_roles(&roles);
    let entry = AgentEntry {
        summary: AgentSummary {
            agent_id: agent_id.clone(),
            label: label.clone(),
            roles: roles.clone(),
            busy: false,
            provider: state.bootstrap.provider.clone(),
            model: state.bootstrap.model.clone(),
            status: "idle".to_string(),
            warning,
        },
        session_id: Uuid::new_v4(),
        transcript: Vec::new(),
        subscribers: Vec::new(),
    };
    state.agents.insert(agent_id.clone(), entry);
    let snapshot = AgentRosterSnapshot {
        agents: collect_sorted_summaries(&state.agents),
    };
    broadcast_roster(&mut state.agents, snapshot.clone());
    Ok(CreatedAgent {
        agent_id,
        label,
        roles,
    })
}

fn delete_agent(state: &Arc<Mutex<AgentRuntimeState>>, agent_id: &str) -> Result<DeletedAgent> {
    let mut state = state
        .lock()
        .map_err(|_| anyhow!("agent runtime poisoned"))?;
    if agent_id == "master" {
        anyhow::bail!("cannot delete the predefined leader agent");
    }
    if state.agents.values().any(|entry| entry.summary.busy) {
        anyhow::bail!("cannot delete web agents while a web agent is busy");
    }
    let removed = state
        .agents
        .remove(agent_id)
        .ok_or_else(|| anyhow!("unknown agent_id {}", agent_id))?;
    for subscriber in removed.subscribers {
        let _ = subscriber.send(AgentRuntimeEvent::Deleted {
            agent_id: agent_id.to_string(),
        });
    }
    let snapshot = AgentRosterSnapshot {
        agents: collect_sorted_summaries(&state.agents),
    };
    broadcast_roster(&mut state.agents, snapshot.clone());
    Ok(DeletedAgent {
        agent_id: agent_id.to_string(),
    })
}

async fn begin_agent_turn(
    state: &Arc<Mutex<AgentRuntimeState>>,
    agent_id: &str,
    prompt: String,
) -> Result<()> {
    let agent_id_owned = agent_id.to_string();
    let (session_id, bootstrap, db, project_dir, roles, label, warning) = {
        let mut runtime = state
            .lock()
            .map_err(|_| anyhow!("agent runtime poisoned"))?;
        let bootstrap = runtime.bootstrap.clone();
        let db = Arc::clone(&runtime.db);
        let project_dir = runtime.project_dir.clone();
        let entry = runtime
            .agents
            .get_mut(&agent_id_owned)
            .ok_or_else(|| anyhow!("unknown agent_id {}", agent_id_owned))?;
        if entry.summary.busy {
            return Err(anyhow!("agent {} is already busy", agent_id_owned));
        }
        entry.summary.busy = true;
        entry.summary.status = "running".to_string();
        if let Some(warning) = entry.summary.warning.clone() {
            entry.transcript.push(TranscriptEntry {
                kind: "warning".to_string(),
                text: warning.clone(),
            });
            broadcast(
                entry,
                AgentRuntimeEvent::TranscriptDelta(TranscriptDelta {
                    agent_id: agent_id_owned.clone(),
                    kind: "warning".to_string(),
                    text: warning.clone(),
                    replace_last: false,
                }),
            );
        }
        entry.transcript.push(TranscriptEntry {
            kind: "user".to_string(),
            text: prompt.clone(),
        });
        broadcast(
            entry,
            AgentRuntimeEvent::Busy {
                agent_id: agent_id_owned.clone(),
                busy: true,
            },
        );
        broadcast(
            entry,
            AgentRuntimeEvent::TranscriptDelta(TranscriptDelta {
                agent_id: agent_id_owned.clone(),
                kind: "user".to_string(),
                text: prompt.clone(),
                replace_last: false,
            }),
        );
        (
            entry.session_id,
            bootstrap,
            db,
            project_dir,
            entry.summary.roles.clone(),
            entry.summary.label.clone(),
            entry.summary.warning.clone(),
        )
    };

    let state_for_task = Arc::clone(state);
    tokio::spawn(async move {
        let result = run_agent_turn(
            Arc::clone(&state_for_task),
            agent_id_owned.clone(),
            prompt,
            session_id,
            bootstrap,
            db,
            project_dir,
            roles,
            label,
            warning,
        )
        .await;
        if let Err(error) = result {
            let _ = finalize_failure(&state_for_task, &agent_id_owned, error.to_string());
        }
    });

    Ok(())
}

async fn run_agent_turn(
    state: Arc<Mutex<AgentRuntimeState>>,
    agent_id: String,
    prompt: String,
    session_id: Uuid,
    bootstrap: WebAgentBootstrap,
    db: Arc<DbHandle>,
    project_dir: PathBuf,
    roles: Vec<String>,
    label: String,
    _warning: Option<String>,
) -> Result<()> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let mut agent = build_agent(
        &bootstrap,
        session_id,
        project_dir,
        db,
        &agent_id,
        &label,
        roles,
    )?;
    agent.set_event_tx(event_tx);

    let state_for_events = Arc::clone(&state);
    let agent_id_for_events = agent_id.clone();
    let event_task = tokio::spawn(async move {
        let mut assistant_open = false;
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::AssistantChunk(chunk) => {
                    let _ = apply_transcript_delta(
                        &state_for_events,
                        &agent_id_for_events,
                        TranscriptDelta {
                            agent_id: agent_id_for_events.clone(),
                            kind: "assistant".to_string(),
                            text: chunk,
                            replace_last: assistant_open,
                        },
                    );
                    assistant_open = true;
                }
                AgentEvent::Status(text) | AgentEvent::Stats(text) => {
                    let _ = apply_transcript_delta(
                        &state_for_events,
                        &agent_id_for_events,
                        TranscriptDelta {
                            agent_id: agent_id_for_events.clone(),
                            kind: "status".to_string(),
                            text,
                            replace_last: false,
                        },
                    );
                }
                AgentEvent::ToolStart { name, .. } => {
                    let _ = apply_transcript_delta(
                        &state_for_events,
                        &agent_id_for_events,
                        TranscriptDelta {
                            agent_id: agent_id_for_events.clone(),
                            kind: "status".to_string(),
                            text: format!("tool started: {name}"),
                            replace_last: false,
                        },
                    );
                }
                AgentEvent::ToolEnd => {
                    let _ = apply_transcript_delta(
                        &state_for_events,
                        &agent_id_for_events,
                        TranscriptDelta {
                            agent_id: agent_id_for_events.clone(),
                            kind: "status".to_string(),
                            text: "tool finished".to_string(),
                            replace_last: false,
                        },
                    );
                }
                AgentEvent::TurnDone(_) => break,
                AgentEvent::AssistantText(_)
                | AgentEvent::LlmStart
                | AgentEvent::WorkflowStateChanged(_) => {}
            }
        }
    });

    let result = agent.run_loop(&prompt).await;
    let _ = event_task.await;
    match result {
        Ok((response, _stats)) => finalize_success(&state, &agent_id, response),
        Err(error) => finalize_failure(&state, &agent_id, error.to_string()),
    }
}

fn finalize_success(
    state: &Arc<Mutex<AgentRuntimeState>>,
    agent_id: &str,
    response: String,
) -> Result<()> {
    let mut state = state
        .lock()
        .map_err(|_| anyhow!("agent runtime poisoned"))?;
    let entry = state
        .agents
        .get_mut(agent_id)
        .ok_or_else(|| anyhow!("unknown agent_id {}", agent_id))?;
    entry.summary.busy = false;
    entry.summary.status = "idle".to_string();
    if let Some(last) = entry.transcript.last_mut() {
        if last.kind == "assistant" {
            last.text = response.clone();
        } else {
            entry.transcript.push(TranscriptEntry {
                kind: "assistant".to_string(),
                text: response.clone(),
            });
        }
    } else {
        entry.transcript.push(TranscriptEntry {
            kind: "assistant".to_string(),
            text: response.clone(),
        });
    }
    broadcast(
        entry,
        AgentRuntimeEvent::TranscriptDelta(TranscriptDelta {
            agent_id: agent_id.to_string(),
            kind: "assistant".to_string(),
            text: response,
            replace_last: true,
        }),
    );
    broadcast(
        entry,
        AgentRuntimeEvent::Busy {
            agent_id: agent_id.to_string(),
            busy: false,
        },
    );
    broadcast(
        entry,
        AgentRuntimeEvent::Completed {
            agent_id: agent_id.to_string(),
        },
    );
    Ok(())
}

fn finalize_failure(
    state: &Arc<Mutex<AgentRuntimeState>>,
    agent_id: &str,
    message: String,
) -> Result<()> {
    let mut state = state
        .lock()
        .map_err(|_| anyhow!("agent runtime poisoned"))?;
    let entry = state
        .agents
        .get_mut(agent_id)
        .ok_or_else(|| anyhow!("unknown agent_id {}", agent_id))?;
    entry.summary.busy = false;
    entry.summary.status = "error".to_string();
    entry.transcript.push(TranscriptEntry {
        kind: "error".to_string(),
        text: message.clone(),
    });
    broadcast(
        entry,
        AgentRuntimeEvent::Busy {
            agent_id: agent_id.to_string(),
            busy: false,
        },
    );
    broadcast(
        entry,
        AgentRuntimeEvent::Failed {
            agent_id: agent_id.to_string(),
            message,
        },
    );
    Ok(())
}

fn apply_transcript_delta(
    state: &Arc<Mutex<AgentRuntimeState>>,
    agent_id: &str,
    delta: TranscriptDelta,
) -> Result<()> {
    let mut state = state
        .lock()
        .map_err(|_| anyhow!("agent runtime poisoned"))?;
    let entry = state
        .agents
        .get_mut(agent_id)
        .ok_or_else(|| anyhow!("unknown agent_id {}", agent_id))?;
    if delta.replace_last {
        if let Some(last) = entry.transcript.last_mut() {
            if last.kind == delta.kind {
                last.text.push_str(&delta.text);
            } else {
                entry.transcript.push(TranscriptEntry {
                    kind: delta.kind.clone(),
                    text: delta.text.clone(),
                });
            }
        } else {
            entry.transcript.push(TranscriptEntry {
                kind: delta.kind.clone(),
                text: delta.text.clone(),
            });
        }
    } else {
        entry.transcript.push(TranscriptEntry {
            kind: delta.kind.clone(),
            text: delta.text.clone(),
        });
    }
    broadcast(entry, AgentRuntimeEvent::TranscriptDelta(delta));
    Ok(())
}

fn build_agent_snapshot(entry: &AgentEntry) -> AgentSnapshot {
    AgentSnapshot {
        agent_id: entry.summary.agent_id.clone(),
        label: entry.summary.label.clone(),
        roles: entry.summary.roles.clone(),
        busy: entry.summary.busy,
        provider: entry.summary.provider.clone(),
        model: entry.summary.model.clone(),
        transcript: entry.transcript.clone(),
        status: entry.summary.status.clone(),
        warning: entry.summary.warning.clone(),
    }
}

fn collect_sorted_summaries(agents: &HashMap<String, AgentEntry>) -> Vec<AgentSummary> {
    let mut entries: Vec<_> = agents.values().map(|entry| entry.summary.clone()).collect();
    entries.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    entries
}

fn broadcast(entry: &mut AgentEntry, event: AgentRuntimeEvent) {
    entry
        .subscribers
        .retain(|subscriber| subscriber.send(event.clone()).is_ok());
}

fn broadcast_roster(agents: &mut HashMap<String, AgentEntry>, snapshot: AgentRosterSnapshot) {
    for entry in agents.values_mut() {
        broadcast(entry, AgentRuntimeEvent::RosterUpdated(snapshot.clone()));
    }
}

fn normalize_primary_role(value: &str) -> &str {
    if value == "main" {
        "master"
    } else {
        value
    }
}

fn normalize_created_agent_roles(mut roles: Vec<String>) -> Vec<String> {
    roles = roles
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    roles.sort();
    roles.dedup();
    if roles.is_empty() {
        roles.push("executor".to_string());
    }
    roles
}

fn allocate_default_agent_id(agents: &HashMap<String, AgentEntry>) -> String {
    let mut n = 1usize;
    loop {
        let candidate = format!("smith-{n}");
        if !agents.contains_key(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn warning_for_roles(roles: &[String]) -> Option<String> {
    if roles.iter().any(|role| role == "interactive") {
        None
    } else {
        Some(format!(
            "selected agent roles are {}. This agent is not interactive; prompts may still run, but the browser surface is optimized for interactive agents.",
            roles.join(", ")
        ))
    }
}

fn build_agent(
    bootstrap: &WebAgentBootstrap,
    session_id: Uuid,
    project_dir: PathBuf,
    db: Arc<DbHandle>,
    agent_id: &str,
    label: &str,
    roles: Vec<String>,
) -> Result<Agent> {
    let client: Box<dyn ChatBackend + Send + Sync> = match bootstrap.provider.as_str() {
        "openai-codex" => {
            let profile_name = bootstrap.active_profile.clone();
            let auth = resolve_codex_auth(bootstrap)?.ok_or_else(|| {
                anyhow!(
                    "no Codex auth for profile '{}'; run the themion CLI Codex login first",
                    bootstrap.active_profile
                )
            })?;
            Box::new(CodexClient::new(
                bootstrap.base_url.clone(),
                auth,
                Box::new(move |auth: &CodexAuth| save_codex_auth_for_profile(&profile_name, auth)),
            ))
        }
        _ => {
            let mut client = ChatClient::new(bootstrap.base_url.clone(), bootstrap.api_key.clone());
            if bootstrap.provider == "openrouter" {
                client = client.with_headers([
                    (
                        "HTTP-Referer".to_string(),
                        "https://github.com/tasanakorn".to_string(),
                    ),
                    ("X-Title".to_string(), "themion-web".to_string()),
                    ("X-OpenRouter-Title".to_string(), "themion-web".to_string()),
                    (
                        "X-OpenRouter-Categories".to_string(),
                        "developer-tools".to_string(),
                    ),
                ]);
            }
            Box::new(client)
        }
    };

    let mut agent = Agent::new_with_db(
        client,
        bootstrap.model.clone(),
        Some(bootstrap.provider.clone()),
        Some(bootstrap.active_profile.clone()),
        bootstrap.system_prompt.clone(),
        session_id,
        project_dir,
        db,
    );
    agent.set_build_identity(Some(BuildIdentity {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        app_version_hash: option_env!("THEMION_BUILD_HASH")
            .unwrap_or("dev")
            .to_string(),
        app_version_dirty: option_env!("THEMION_BUILD_DIRTY")
            .map(|value| value == "true")
            .unwrap_or(true),
    }));
    agent.set_api_log_enabled(false);
    agent.set_local_agent_role_context(agent_id.to_string(), label.to_string(), roles);
    Ok(agent)
}

fn load_bootstrap() -> Result<WebAgentBootstrap> {
    let active_profile = std::env::var("THEMION_PROFILE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string());
    let provider = std::env::var("THEMION_PROVIDER")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "openrouter".to_string());

    let (base_url, api_key, model) = match provider.as_str() {
        "llamacpp" => (
            std::env::var("LLAMACPP_BASE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| LLAMACPP_DEFAULT_BASE_URL.to_string()),
            None,
            std::env::var("LLAMACPP_MODEL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| LLAMACPP_DEFAULT_MODEL.to_string()),
        ),
        "openai-codex" => (
            std::env::var("CODEX_BASE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| CODEX_DEFAULT_BASE_URL.to_string()),
            None,
            std::env::var("CODEX_MODEL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| CODEX_DEFAULT_MODEL.to_string()),
        ),
        _ => {
            let api_key = std::env::var("OPENROUTER_API_KEY")
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    anyhow!("OPENROUTER_API_KEY is required for themion-web agent runtime")
                })?;
            (
                std::env::var("OPENROUTER_BASE_URL")
                    .ok()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| OPENROUTER_DEFAULT_BASE_URL.to_string()),
                Some(api_key),
                std::env::var("OPENROUTER_MODEL")
                    .ok()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| OPENROUTER_DEFAULT_MODEL.to_string()),
            )
        }
    };

    let mut profiles = HashMap::new();
    profiles.insert(
        active_profile.clone(),
        WebProfileConfig {
            provider: Some(provider.clone()),
        },
    );

    Ok(WebAgentBootstrap {
        provider,
        base_url,
        api_key,
        model,
        system_prompt: std::env::var("SYSTEM_PROMPT")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string()),
        active_profile: active_profile.clone(),
        configured_profile: active_profile,
        profiles,
    })
}

fn resolve_system_db_path() -> PathBuf {
    if let Ok(path) = std::env::var("THEMION_WEB_DB_PATH") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if let Ok(xdg_data_home) = std::env::var("XDG_DATA_HOME") {
        let trimmed = xdg_data_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("themion").join("system.db");
        }
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("themion")
        .join("system.db")
}

fn resolve_codex_auth(bootstrap: &WebAgentBootstrap) -> Result<Option<CodexAuth>> {
    if let Some(auth) = load_codex_auth_for_profile(&bootstrap.active_profile)? {
        return Ok(Some(auth));
    }

    let mut obvious_targets = Vec::new();
    if bootstrap
        .profiles
        .get(&bootstrap.configured_profile)
        .is_some_and(|profile| profile.provider.as_deref() == Some("openai-codex"))
    {
        obvious_targets.push(bootstrap.configured_profile.clone());
    }
    if bootstrap
        .profiles
        .get("codex")
        .is_some_and(|profile| profile.provider.as_deref() == Some("openai-codex"))
        && !obvious_targets.iter().any(|name| name == "codex")
    {
        obvious_targets.push("codex".to_string());
    }

    let codex_profile_names: Vec<String> = bootstrap
        .profiles
        .iter()
        .filter_map(|(name, profile)| {
            (profile.provider.as_deref() == Some("openai-codex")).then(|| name.clone())
        })
        .collect();
    if codex_profile_names.len() == 1
        && !obvious_targets
            .iter()
            .any(|name| name == &codex_profile_names[0])
    {
        obvious_targets.push(codex_profile_names[0].clone());
    }

    if obvious_targets.len() == 1 && obvious_targets[0] == bootstrap.active_profile {
        return migrate_legacy_codex_auth_to_profile(&bootstrap.active_profile);
    }

    Ok(None)
}

fn themion_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("themion"))
}

fn profile_auth_path(profile: &str) -> Option<PathBuf> {
    themion_config_dir().map(|dir| {
        dir.join("auth")
            .join(format!("{}.json", sanitize_profile_name(profile)))
    })
}

fn legacy_auth_path() -> Option<PathBuf> {
    themion_config_dir().map(|dir| dir.join("auth.json"))
}

fn sanitize_profile_name(profile: &str) -> String {
    let mut out = String::with_capacity(profile.len());
    for ch in profile.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "profile".to_string()
    } else {
        out
    }
}

fn load_codex_auth_for_profile(profile: &str) -> Result<Option<CodexAuth>> {
    let Some(path) = profile_auth_path(profile) else {
        return Ok(None);
    };
    load_auth_file(&path)
}

fn save_codex_auth_for_profile(profile: &str, auth: &CodexAuth) -> Result<()> {
    let path = profile_auth_path(profile).ok_or_else(|| anyhow!("cannot determine config dir"))?;
    save_auth_file(&path, auth)
}

fn migrate_legacy_codex_auth_to_profile(profile: &str) -> Result<Option<CodexAuth>> {
    let Some(auth) = load_legacy_auth()? else {
        return Ok(None);
    };
    if load_codex_auth_for_profile(profile)?.is_none() {
        save_codex_auth_for_profile(profile, &auth)
            .with_context(|| format!("saving migrated auth for profile '{}'", profile))?;
    }
    Ok(Some(auth))
}

fn load_legacy_auth() -> Result<Option<CodexAuth>> {
    let Some(path) = legacy_auth_path() else {
        return Ok(None);
    };
    load_auth_file(&path)
}

fn load_auth_file(path: &std::path::Path) -> Result<Option<CodexAuth>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&raw)?))
}

fn save_auth_file(path: &std::path::Path, auth: &CodexAuth) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("cannot determine auth directory"))?;
    std::fs::create_dir_all(parent)?;
    let json = serde_json::to_string_pretty(auth)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

impl AgentRuntimeService {
    pub async fn snapshot(&self) -> Result<AgentRosterSnapshot> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(AgentRuntimeRequest::Snapshot { response_tx })
            .map_err(|_| anyhow!("agent runtime unavailable"))?;
        response_rx
            .await
            .context("agent runtime dropped snapshot response")?
    }

    pub async fn subscribe(
        &self,
        agent_id: String,
    ) -> Result<mpsc::UnboundedReceiver<AgentRuntimeEvent>> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(AgentRuntimeRequest::Subscribe {
                agent_id,
                response_tx,
            })
            .map_err(|_| anyhow!("agent runtime unavailable"))?;
        response_rx
            .await
            .context("agent runtime dropped subscribe response")?
    }

    pub async fn submit_prompt(&self, agent_id: String, prompt: String) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(AgentRuntimeRequest::SubmitPrompt {
                agent_id,
                prompt,
                response_tx,
            })
            .map_err(|_| anyhow!("agent runtime unavailable"))?;
        response_rx
            .await
            .context("agent runtime dropped prompt response")?
    }

    pub async fn create_agent(
        &self,
        label: Option<String>,
        roles: Vec<String>,
    ) -> Result<CreatedAgent> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(AgentRuntimeRequest::CreateAgent {
                label,
                roles,
                response_tx,
            })
            .map_err(|_| anyhow!("agent runtime unavailable"))?;
        response_rx
            .await
            .context("agent runtime dropped create response")?
    }

    pub async fn delete_agent(&self, agent_id: String) -> Result<DeletedAgent> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(AgentRuntimeRequest::DeleteAgent {
                agent_id,
                response_tx,
            })
            .map_err(|_| anyhow!("agent runtime unavailable"))?;
        response_rx
            .await
            .context("agent runtime dropped delete response")?
    }
}
