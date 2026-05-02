use crate::config::Config;
use crate::tui::{AppEvent, Entry, FrameRequester};
#[cfg(feature = "stylos")]
use crate::app_runtime::{start_watchdog_task, WatchdogRuntimeState};
use crate::runtime_domains::RuntimeDomains;
use crate::tui::App;
use crate::Session;
use std::path::PathBuf;
use std::sync::Arc;
use themion_core::agent::Agent;
use themion_core::client::ChatClient;
use themion_core::client_codex::CodexClient;
use themion_core::db::DbHandle;
#[cfg(feature = "stylos")]
use themion_core::db::{CreateNoteArgs, NoteColumn, NoteKind};
use themion_core::tools::{
    SystemInspectionProvider, SystemInspectionResult, SystemInspectionRuntime,
    SystemInspectionTaskRuntime, SystemInspectionTools,
};
use themion_core::ChatBackend;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

pub(crate) enum AppRuntimeEvent {
    Agent(Uuid, themion_core::agent::AgentEvent),
    AgentReady(Box<themion_core::agent::Agent>, Uuid),
    #[cfg(feature = "stylos")]
    StylosCmd(crate::stylos::StylosCmdRequest),
    #[cfg(feature = "stylos")]
    IncomingPrompt(crate::stylos::IncomingPromptRequest),
    #[cfg(feature = "stylos")]
    WatchdogDispatchLog {
        agent_id: Option<String>,
        text: String,
    },
    #[cfg(feature = "stylos")]
    StylosEvent(String),
    ShellComplete {
        output: String,
        exit_code: Option<i32>,
    },
}



#[derive(Clone)]
pub(crate) enum AgentActivity {
    PreparingRequest,
    WaitingForModel,
    StreamingResponse,
    RunningTool(String),
    WaitingAfterTool,
    LoginStarting,
    WaitingForLoginBrowser,
    RunningShellCommand,
    Finishing,
}

impl AgentActivity {
    pub(crate) fn label(&self, stream_chunks: u64, stream_chars: u64) -> String {
        match self {
            Self::PreparingRequest => "preparing request…".to_string(),
            Self::WaitingForModel => "waiting for model…".to_string(),
            Self::StreamingResponse => format!(
                "receiving response… chunks:{} chars:{}",
                stream_chunks, stream_chars
            ),
            Self::RunningTool(detail) => format!("running tool… {}", detail),
            Self::WaitingAfterTool => "tool finished, waiting for model…".to_string(),
            Self::LoginStarting => "starting login…".to_string(),
            Self::WaitingForLoginBrowser => "waiting for login confirmation…".to_string(),
            Self::RunningShellCommand => "running shell command…".to_string(),
            Self::Finishing => "finalizing…".to_string(),
        }
    }

    pub(crate) fn status_bar(&self, stream_chunks: u64, stream_chars: u64) -> String {
        match self {
            Self::PreparingRequest => "preparing".to_string(),
            Self::WaitingForModel => "waiting-model".to_string(),
            Self::StreamingResponse => format!("streaming c:{} ch:{}", stream_chunks, stream_chars),
            Self::RunningTool(_) => "running-tool".to_string(),
            Self::WaitingAfterTool => "waiting-after-tool".to_string(),
            Self::LoginStarting => "login-start".to_string(),
            Self::WaitingForLoginBrowser => "login-wait".to_string(),
            Self::RunningShellCommand => "shell".to_string(),
            Self::Finishing => "finalizing".to_string(),
        }
    }
}

pub(crate) fn activity_status_value(
    activity: Option<&AgentActivity>,
    idle_since: Option<std::time::Instant>,
    stream_chunks: u64,
    stream_chars: u64,
) -> String {
    const NAP_AFTER: std::time::Duration = std::time::Duration::from_secs(5 * 60);

    if let Some(activity) = activity {
        return activity.status_bar(stream_chunks, stream_chars);
    }

    match idle_since {
        Some(idle_since) if idle_since.elapsed() > NAP_AFTER => "nap".to_string(),
        _ => "idle".to_string(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppSnapshotAgent {
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
    pub busy: bool,
    pub incoming: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppSnapshot {
    pub primary_session_id: Option<Uuid>,
    pub primary_agent_id: Option<String>,
    pub busy: bool,
    pub activity_status: Option<String>,
    pub local_agents: Vec<AppSnapshotAgent>,
    #[cfg(feature = "stylos")]
    pub stylos_status: Option<String>,
    #[cfg(feature = "stylos")]
    pub pending_watchdog_note: bool,
    #[cfg(feature = "stylos")]
    pub active_incoming_prompt_count: usize,
    #[cfg(feature = "stylos")]
    pub aggregate_busy_agents: bool,
}

#[derive(Clone)]
pub struct AppSnapshotHub {
    sender: watch::Sender<AppSnapshot>,
}

impl AppSnapshotHub {
    pub fn new(initial: AppSnapshot) -> Self {
        let (sender, _) = watch::channel(initial);
        Self { sender }
    }

    pub fn subscribe(&self) -> watch::Receiver<AppSnapshot> {
        self.sender.subscribe()
    }

    pub fn current(&self) -> AppSnapshot {
        self.sender.borrow().clone()
    }

    pub fn publish(&self, snapshot: AppSnapshot) {
        let _ = self.sender.send(snapshot);
    }
}

pub struct AppState {
    pub runtime_domains: Arc<RuntimeDomains>,
    pub session: Session,
    pub db: Arc<DbHandle>,
    pub project_dir: PathBuf,
    pub session_id: Uuid,
    pub snapshot_hub: AppSnapshotHub,
    #[cfg(feature = "stylos")]
    pub watchdog_state: Arc<WatchdogRuntimeState>,
    #[cfg(feature = "stylos")]
    pub stylos_config: crate::config::StylosConfig,
}

#[cfg(feature = "stylos")]
pub struct DoneMentionRequest {
    pub note_id: String,
    pub note_slug: String,
    pub from_instance: String,
    pub from_agent_id: String,
    pub completed_by_instance: String,
    pub completed_by_agent_id: String,
    pub result_summary: String,
}

impl AppState {
    pub fn for_tui(cfg: Config, project_dir_override: Option<PathBuf>) -> anyhow::Result<Self> {
        Self::build(cfg, project_dir_override, true)
    }

    pub fn for_headless(
        cfg: Config,
        project_dir_override: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::build(cfg, project_dir_override, false)
    }

    fn build(
        cfg: Config,
        project_dir_override: Option<PathBuf>,
        interactive: bool,
    ) -> anyhow::Result<Self> {
        #[cfg(feature = "stylos")]
        let stylos_config = cfg.stylos.clone();

        let runtime_domains = Arc::new(if interactive {
            RuntimeDomains::for_tui_mode()?
        } else {
            RuntimeDomains::for_print_mode()?
        });
        let project_dir = resolve_project_dir(project_dir_override);
        let db = open_history_db(interactive);
        let session = Session::from_config(cfg);
        let session_id = Uuid::new_v4();
        let _ = db.insert_session(session_id, &project_dir, interactive);
        let snapshot_hub = AppSnapshotHub::new(AppSnapshot {
            primary_session_id: Some(session_id),
            primary_agent_id: Some("master".to_string()),
            busy: false,
            activity_status: Some("idle".to_string()),
            local_agents: vec![AppSnapshotAgent {
                agent_id: "master".to_string(),
                label: "master".to_string(),
                roles: vec!["master".to_string(), "interactive".to_string()],
                busy: false,
                incoming: false,
            }],
            #[cfg(feature = "stylos")]
            stylos_status: Some("off".to_string()),
            #[cfg(feature = "stylos")]
            pending_watchdog_note: false,
            #[cfg(feature = "stylos")]
            active_incoming_prompt_count: 0,
            #[cfg(feature = "stylos")]
            aggregate_busy_agents: false,
        });

        Ok(Self {
            runtime_domains,
            session,
            db,
            project_dir,
            session_id,
            snapshot_hub,
            #[cfg(feature = "stylos")]
            watchdog_state: Arc::new(WatchdogRuntimeState::default()),
            #[cfg(feature = "stylos")]
            stylos_config,
        })
    }

    pub fn system_inspection_snapshot(&self) -> SystemInspectionResult {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut provider = SystemInspectionProvider {
            status: "ok".to_string(),
            active_profile: Some(self.session.active_profile.clone()),
            provider: Some(self.session.provider.clone()),
            model: Some(self.session.model.clone()),
            auth_configured: Some(match self.session.provider.as_str() {
                "openai-codex" => resolve_codex_auth(&self.session).ok().flatten().is_some(),
                _ => self
                    .session
                    .api_key
                    .as_ref()
                    .map(|v| !v.is_empty())
                    .unwrap_or(false),
            }),
            base_url_present: Some(!self.session.base_url.trim().is_empty()),
            rate_limits: None,
            warnings: Vec::new(),
            issues: Vec::new(),
        };
        if provider.auth_configured == Some(false) {
            provider.status = "degraded".to_string();
            provider
                .issues
                .push("provider authentication is not configured".to_string());
        }
        if provider.base_url_present == Some(false) {
            provider.status = "degraded".to_string();
            provider
                .issues
                .push("provider base_url is empty".to_string());
        }

        let tool_names = themion_core::tools::tool_definitions()
            .as_array()
            .into_iter()
            .flat_map(|defs| defs.iter())
            .filter_map(|entry| entry.get("function")?.get("name")?.as_str())
            .map(str::to_string)
            .collect::<Vec<_>>();

        let tools = SystemInspectionTools {
            status: "ok".to_string(),
            tool_count: tool_names.len(),
            available_names: tool_names,
            warnings: Vec::new(),
            issues: Vec::new(),
        };

        let runtime = SystemInspectionRuntime {
            status: "ok".to_string(),
            pid: Some(std::process::id()),
            now_ms,
            session_id: self.session_id.to_string(),
            project_dir: self.project_dir.display().to_string(),
            workflow_name: None,
            phase_name: None,
            workflow_status: None,
            debug_runtime_lines: vec![
                "debug runtime snapshot unavailable outside the TUI app loop".to_string(),
            ],
            task_runtime: Some(SystemInspectionTaskRuntime {
                status: "partial".to_string(),
                current_activity: None,
                current_activity_detail: None,
                busy: None,
                activity_status: None,
                activity_status_changed_at_ms: None,
                process_started_at_ms: None,
                uptime_ms: None,
                recent_window_ms: None,
                runtime_notes: vec![
                    "task runtime inspection is unavailable outside the TUI app loop".to_string(),
                ],
            }),
            warnings: vec!["runtime inspection is partial outside the TUI app loop".to_string()],
            issues: Vec::new(),
        };

        let mut warnings = Vec::new();
        let mut issues = Vec::new();
        if provider.status != "ok" {
            warnings.push("provider readiness is degraded".to_string());
            issues.extend(provider.issues.clone());
        }
        let overall_status = if issues.is_empty() { "ok" } else { "degraded" }.to_string();
        let summary = if overall_status == "ok" {
            "local inspection snapshot available".to_string()
        } else {
            format!("local inspection found {} issue(s)", issues.len())
        };

        SystemInspectionResult {
            overall_status,
            summary,
            runtime,
            tools,
            provider,
            warnings,
            issues,
        }
    }

    pub fn build_agent(&self) -> anyhow::Result<Agent> {
        build_agent(
            &self.session,
            self.session_id,
            self.project_dir.clone(),
            self.db.clone(),
            #[cfg(feature = "stylos")]
            None,
            #[cfg(feature = "stylos")]
            None,
            #[cfg(feature = "stylos")]
            "master",
            None,
            Some(self.system_inspection_snapshot()),
            false,
        )
    }
}


pub fn start_tick_loop<T, F>(
    runtime_domains: &Arc<RuntimeDomains>,
    app_tx: mpsc::UnboundedSender<T>,
    mut make_tick: F,
) where
    T: Send + 'static,
    F: FnMut() -> T + Send + 'static,
{
    let tui_domain = runtime_domains
        .tui()
        .expect("tui runtime available in TUI mode");
    let tui_domain_for_tick = tui_domain.clone();
    tui_domain_for_tick.spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(150));
        loop {
            interval.tick().await;
            if app_tx.send(make_tick()).is_err() {
                break;
            }
        }
    });
}

#[cfg(feature = "stylos")]
pub fn start_tui_watchdog_loop(
    app_state: &AppState,
    runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
) {
    let tui_domain = app_state
        .runtime_domains
        .tui()
        .expect("tui runtime available in TUI mode");
    start_watchdog_task(&tui_domain, runtime_tx, app_state.watchdog_state.clone());
}


pub(crate) fn set_agent_activity(app: &mut App, activity: AgentActivity) {
    let activity_changed = app
        .agent_activity
        .as_ref()
        .map(|current| std::mem::discriminant(current) != std::mem::discriminant(&activity))
        .unwrap_or(true);
    app.agent_activity = Some(activity);
    if activity_changed {
        app.agent_activity_changed_at = Some(crate::tui::unix_epoch_now_ms());
    }
    app.idle_since = None;
    app.idle_status_changed_at = None;
    app.pending = Some(app.pending_str());
    app.mark_dirty_status();
    publish_runtime_snapshot(app);
}

pub(crate) fn clear_agent_activity(app: &mut App) {
    app.agent_activity = None;
    app.agent_activity_changed_at = None;
    app.idle_since = Some(std::time::Instant::now());
    app.idle_status_changed_at = Some(crate::tui::unix_epoch_now_ms());
    app.pending = None;
    app.mark_dirty_status();
    publish_runtime_snapshot(app);
}

pub(crate) fn on_tick(app: &mut App) {
    app.activity_counters.tick_count += 1;
    app.expire_ctrl_c_exit_if_needed(std::time::Instant::now());
    app.record_runtime_snapshot();
    publish_runtime_snapshot(app);
    let previous = app.pending.clone();
    app.anim_frame = app.anim_frame.wrapping_add(1);
    if app.agent_busy && app.pending.is_some() {
        app.pending = Some(app.pending_str());
    }
    if app.pending != previous {
        app.mark_dirty_status();
    }
}


pub(crate) fn handle_runtime_command(
    app: &mut App,
    command: crate::app_runtime::RuntimeCommand,
    frame_requester: &FrameRequester,
    app_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    match command {
        crate::app_runtime::RuntimeCommand::LoginCodex { profile_name } => {
            if app.agent_busy {
                app.push(Entry::Assistant {
                    agent_id: None,
                    text: "busy, please wait".to_string(),
                });
                app.push(Entry::Blank);
                app.mark_dirty_all();
                return;
            }
            app.agent_busy = true;
            set_agent_activity(app, AgentActivity::LoginStarting);
            let target_profile = profile_name.clone().unwrap_or_else(|| {
                if app
                    .session
                    .profiles
                    .get(&app.session.active_profile)
                    .is_some_and(|profile| profile.provider.as_deref() == Some("openai-codex"))
                {
                    app.session.active_profile.clone()
                } else {
                    "codex".to_string()
                }
            });
            let login_message = if profile_name.is_some() {
                format!("logging in to OpenAI Codex for profile '{}'…", target_profile)
            } else if target_profile == app.session.active_profile {
                format!("logging in to OpenAI Codex for current profile '{}'…", target_profile)
            } else {
                format!("logging in to OpenAI Codex for profile '{}'…", target_profile)
            };
            app.push(Entry::Assistant {
                agent_id: None,
                text: login_message,
            });
            let tx = app_tx.clone();
            app.background_domain().spawn(async move {
                match crate::login_codex::start_device_flow().await {
                    Err(e) => {
                        tx.send(AppEvent::LoginComplete { profile_name: target_profile.clone(), auth_result: Err(e) }).ok();
                    }
                    Ok((info, poll)) => {
                        tx.send(AppEvent::LoginPrompt {
                            user_code: info.user_code,
                            verification_uri: info.verification_uri,
                        })
                        .ok();
                        let result = poll.await;
                        tx.send(AppEvent::LoginComplete { profile_name: target_profile.clone(), auth_result: result }).ok();
                    }
                }
            });
            app.mark_dirty_all();
        }
        crate::app_runtime::RuntimeCommand::SemanticMemoryIndex { full } => {
            #[cfg(not(feature = "semantic-memory"))]
            {
                let mode = if full { "full reindex" } else { "index" };
                app.push(Entry::Assistant {
                    agent_id: None,
                    text: format!(
                        "semantic-memory {} is unavailable in this build; enable the semantic-memory feature",
                        mode
                    ),
                });
                app.push(Entry::Blank);
                app.mark_dirty_all();
            }
            #[cfg(feature = "semantic-memory")]
            {
                if app.agent_busy {
                    app.push(Entry::Assistant {
                        agent_id: None,
                        text: "busy, please wait".to_string(),
                    });
                    app.push(Entry::Blank);
                    app.mark_dirty_all();
                    return;
                }
                app.agent_busy = true;
                set_agent_activity(app, AgentActivity::RunningTool(if full {
                    "semantic-memory full reindex".to_string()
                } else {
                    "semantic-memory index pending".to_string()
                }));
                app.push(Entry::Assistant {
                    agent_id: None,
                    text: if full {
                        "rebuilding all stale or missing Project Memory semantic embeddings…".to_string()
                    } else {
                        "indexing missing or pending Project Memory semantic embeddings…".to_string()
                    },
                });
                let tx = app.runtime_tx.clone();
                let db = app.db.clone();
                app.background_domain().spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        db.memory_store().index_pending_embeddings(full)
                    })
                    .await;
                    let text = match result {
                        Ok(Ok(report)) => serde_json::to_string_pretty(&report)
                            .unwrap_or_else(|err| format!("indexing report serialization failed: {}", err)),
                        Ok(Err(err)) => {
                            if full {
                                format!("semantic-memory full reindex failed: {}", err)
                            } else {
                                format!("semantic-memory indexing failed: {}", err)
                            }
                        }
                        Err(err) => {
                            if full {
                                format!("semantic-memory full reindex task failed: {}", err)
                            } else {
                                format!("semantic-memory indexing task failed: {}", err)
                            }
                        }
                    };
                    let _ = tx.send(AppRuntimeEvent::ShellComplete {
                        output: text,
                        exit_code: Some(0),
                    });
                });
                app.mark_dirty_all();
            }
        }
        crate::app_runtime::RuntimeCommand::SessionProfileUse { .. }
        | crate::app_runtime::RuntimeCommand::SessionModelUse { .. }
        | crate::app_runtime::RuntimeCommand::SessionReset
        | crate::app_runtime::RuntimeCommand::ConfigProfileUse { .. }
        | crate::app_runtime::RuntimeCommand::ConfigProfileCreate { .. }
        | crate::app_runtime::RuntimeCommand::ConfigProfileSet { .. }
        | crate::app_runtime::RuntimeCommand::SetApiLogEnabled { .. }
        | crate::app_runtime::RuntimeCommand::ClearContext => {
            let outcome = crate::app_runtime::execute_runtime_command(
                command,
                crate::app_runtime::RuntimeCommandContext {
                    session: &mut app.session,
                    project_dir: &app.project_dir,
                    db: &app.db,
                    #[cfg(feature = "stylos")]
                    stylos_tool_bridge: app.stylos_tool_bridge.clone(),
                    #[cfg(feature = "stylos")]
                    local_stylos_instance: app.local_stylos_instance.as_deref(),
                    api_log_enabled: app.api_log_enabled,
                    local_agent_mgmt_tx: app.local_agent_mgmt_tx.clone(),
                },
            );
            let application = crate::app_runtime::apply_runtime_command_outcome_to_app_runtime(
                &mut app.agents,
                &mut app.status_model_info,
                &mut app.workflow_state,
                &mut app.api_log_enabled,
                &mut app.last_ctx_tokens,
                outcome,
            );
            if application.had_effect {
                publish_runtime_snapshot(app);
            }
            for line in application.output_lines {
                app.push(Entry::Assistant {
                    agent_id: None,
                    text: line,
                });
            }
            if application.had_effect {
                app.push(Entry::Blank);
                app.mark_dirty_all();
            }
        }
    }
    if app.dirty.any() {
        app.request_draw(frame_requester);
    }
}

pub(crate) fn publish_runtime_snapshot(app: &mut App) {
    let debug_runtime_lines = app.debug_runtime_lines();
    let activity_status = app.activity_status_value();
    #[cfg(feature = "stylos")]
    let primary_activity_label = app
        .agent_activity
        .as_ref()
        .map(|activity| activity.status_bar(app.stream_chunks, app.stream_chars));
    let recent_window_ms = app.recent_runtime_delta().map(|recent| recent.wall_elapsed_ms);
    let uptime_ms = app.process_started_at.elapsed().as_millis() as u64;
    #[cfg(feature = "stylos")]
    let stylos_status = app.stylos_status_value();
    #[cfg(feature = "stylos")]
    let stylos = app.stylos.as_ref().map(|_| crate::app_runtime::StylosRuntimeStatusPublishState {
        hub: &app.shared_status_hub,
        startup_project_dir: &app.startup_project_dir,
        fallback_project_dir: &app.project_dir,
        provider: &app.session.provider,
        model: &app.session.model,
        active_profile: &app.session.active_profile,
        rate_limits: app.status_rate_limits.as_ref(),
        idle_since: app.idle_since,
        idle_status_changed_at: app.idle_status_changed_at,
        primary_activity_label: primary_activity_label.clone(),
        primary_activity_changed_at_ms: app.agent_activity_changed_at,
        primary_workflow: &app.workflow_state,
    });

    app.runtime_observer_publisher.publish(crate::app_runtime::AppRuntimeObserverPublishState {
        agents: &mut app.agents,
        snapshot: {
            #[cfg(feature = "stylos")]
            {
                crate::app_runtime::AppRuntimeSnapshotPublishState {
                    agent_busy: app.agent_busy,
                    activity_status: activity_status.clone(),
                    stylos_status: Some(stylos_status),
                    watchdog_state: &app.watchdog_state,
                }
            }
            #[cfg(not(feature = "stylos"))]
            {
                crate::app_runtime::AppRuntimeSnapshotPublishState::new(
                    app.agent_busy,
                    activity_status.clone(),
                )
            }
        },
        system_inspection: crate::app_runtime::SystemInspectionRuntimeRefreshState {
            session: &app.session,
            project_dir: &app.project_dir,
            workflow_state: &app.workflow_state,
            rate_limits: app.status_rate_limits.as_ref(),
            activity: app.agent_activity.as_ref(),
            stream_chunks: app.stream_chunks,
            stream_chars: app.stream_chars,
            agent_busy: app.agent_busy,
            activity_status,
            activity_status_changed_at_ms: app
                .agent_activity_changed_at
                .or(app.idle_status_changed_at),
            process_started_at_ms: app.process_started_at_ms,
            uptime_ms,
            recent_window_ms,
            debug_runtime_lines,
        },
        #[cfg(feature = "stylos")]
        stylos,
    });
}


#[cfg(feature = "stylos")]
pub(crate) fn handle_incoming_prompt_event(
    app: &mut App,
    request: crate::stylos::IncomingPromptRequest,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    app.activity_counters.incoming_prompt_count += 1;
    process_incoming_prompt_request(app, request, app_tx);
}


pub(crate) fn process_agent_event(
    app: &mut App,
    sid: Uuid,
    ev: themion_core::agent::AgentEvent,
    #[cfg(feature = "stylos")] app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    match ev {
        themion_core::agent::AgentEvent::LlmStart => {
            #[cfg(feature = "stylos")]
            {
                let agent_index = app.agents.iter().position(|h| h.session_id == sid);
                if let (Some(agent_index), Some(handle)) = (agent_index, app.stylos.as_ref()) {
                    if let Some(remote) = app.agents[agent_index].active_incoming_prompt.as_ref() {
                        if let Some(task_id) = remote.task_id.clone() {
                            crate::app_runtime::publish_stylos_task_running(
                                &app.background_domain,
                                handle.query_context(),
                                task_id,
                            );
                        }
                    }
                }
            }
            app.reset_stream_counters();
            #[cfg(feature = "stylos")]
            {
                app.last_assistant_text = None;
            }
            set_agent_activity(app, AgentActivity::WaitingForModel);
            app.streaming_idx = None;
        }
        themion_core::agent::AgentEvent::AssistantChunk(chunk) => {
            #[cfg(feature = "stylos")]
            {
                let next = match app.last_assistant_text.take() {
                    Some(mut existing) => {
                        existing.push_str(&chunk);
                        existing
                    }
                    None => chunk.clone(),
                };
                app.last_assistant_text = Some(next);
            }
            app.stream_chunks += 1;
            app.stream_chars += chunk.chars().count() as u64;
            set_agent_activity(app, AgentActivity::StreamingResponse);
            let agent_id = crate::tui::agent_id_for_session(&app.agents, sid);
            match app.streaming_idx {
                Some(i) => {
                    if let Some(crate::tui::Entry::Assistant { text, .. }) = app.entries.get_mut(i) {
                        text.push_str(&chunk);
                    }
                }
                None => {
                    app.push(crate::tui::Entry::Assistant { agent_id, text: chunk });
                    app.streaming_idx = Some(app.entries.len() - 1);
                }
            }
        }
        themion_core::agent::AgentEvent::AssistantText(text) => {
            #[cfg(feature = "stylos")]
            {
                app.last_assistant_text = Some(text.clone());
            }
            app.streaming_idx = None;
            clear_agent_activity(app);
            app.push(crate::tui::Entry::Assistant {
                agent_id: crate::tui::agent_id_for_session(&app.agents, sid),
                text,
            });
        }
        themion_core::agent::AgentEvent::ToolStart { name, arguments_json, display_arguments_json } => {
            app.streaming_idx = None;
            let display_args_json = display_arguments_json.as_deref().unwrap_or(&arguments_json);
            let (detail, reason) = crate::tui::split_tool_call_detail(&name, display_args_json);
            let activity_detail = match &reason {
                Some(reason) => format!("{detail} — {reason}"),
                None => detail.clone(),
            };
            set_agent_activity(app, AgentActivity::RunningTool(activity_detail));
            #[cfg(feature = "stylos")]
            {
                app.last_sender_side_transport_event = app
                    .local_stylos_instance
                    .as_deref()
                    .and_then(|local_instance| {
                        crate::stylos::sender_side_transport_event_from_tool_detail(
                            &detail,
                            local_instance,
                            app.stylos_tool_bridge.is_some(),
                        )
                    });
            }
            app.push(crate::tui::Entry::ToolCall {
                agent_id: crate::tui::agent_id_for_session(&app.agents, sid),
                detail,
                reason,
            });
        }
        themion_core::agent::AgentEvent::ToolEnd => {
            app.push(crate::tui::Entry::ToolDone);
            #[cfg(feature = "stylos")]
            if let Some(event) = app.last_sender_side_transport_event.take() {
                app.push(crate::tui::Entry::RemoteEvent {
                    agent_id: event.agent_id,
                    source: Some(crate::tui::NonAgentSource::Stylos),
                    text: event.text,
                });
            }
            set_agent_activity(app, AgentActivity::WaitingAfterTool);
        }
        themion_core::agent::AgentEvent::Status(text) => {
            app.push(crate::tui::Entry::Status {
                agent_id: crate::tui::agent_id_for_session(&app.agents, sid),
                source: None,
                text,
            });
        }
        themion_core::agent::AgentEvent::WorkflowStateChanged(state) => {
            app.workflow_state = state;
            app.mark_dirty_status();
            publish_runtime_snapshot(app);
        }
        themion_core::agent::AgentEvent::Stats(text) => {
            if let Some(json) = text.strip_prefix("[rate-limit] ") {
                if let Ok(report) = serde_json::from_str::<themion_core::client_codex::ApiCallRateLimitReport>(json) {
                    app.status_rate_limits = Some(report);
                    app.mark_dirty_status();
                    publish_runtime_snapshot(app);
                }
                return;
            }
            app.push(crate::tui::Entry::Stats(text));
        }
        themion_core::agent::AgentEvent::TurnDone(stats) => {
            #[cfg(feature = "stylos")]
            {
                let agent_index = app.agents.iter().position(|h| h.session_id == sid);
                if let Some(agent_index) = agent_index {
                    maybe_emit_done_mention_for_completed_note(app, agent_index, app_tx);
                }
                if let (Some(agent_index), Some(handle)) = (agent_index, app.stylos.as_ref()) {
                    if let Some(remote) = app.agents[agent_index].active_incoming_prompt.take() {
                        if let Some(task_id) = remote.task_id {
                            let result_text = app.last_assistant_text.clone();
                            crate::app_runtime::publish_stylos_task_completed(
                                &app.background_domain,
                                handle.query_context(),
                                task_id,
                                result_text,
                            );
                        }
                    }
                }
            }
            app.streaming_idx = None;
            set_agent_activity(app, AgentActivity::Finishing);
            clear_agent_activity(app);
            let interrupted = app.workflow_state.status == themion_core::workflow::WorkflowStatus::Interrupted;
            let stats_text = crate::format_stats(&stats);
            let stats_text = stats_text.strip_prefix("[stats: ").and_then(|s| s.strip_suffix("]")).unwrap_or(&stats_text).to_string();
            app.push(crate::tui::Entry::TurnDone {
                agent_id: crate::tui::agent_id_for_session(&app.agents, sid),
                summary: if interrupted { "󰇺 Turn interrupted".to_string() } else { "󰇺 Turn end".to_string() },
                stats: stats_text,
            });
            app.push(crate::tui::Entry::Blank);
            app.activity_counters.agent_turn_completed_count += 1;
            app.agent_busy = app.any_agent_busy() || app.agent_activity.is_some();
            if let Some(last_api_call_tokens_in) = stats.last_api_call_tokens_in {
                app.last_ctx_tokens = last_api_call_tokens_in;
            }
            app.session_tokens.tokens_in += stats.tokens_in;
            app.session_tokens.tokens_out += stats.tokens_out;
            app.session_tokens.tokens_cached += stats.tokens_cached;
            app.session_tokens.llm_rounds += stats.llm_rounds;
            app.session_tokens.tool_calls += stats.tool_calls;
            app.session_tokens.elapsed_ms += stats.elapsed_ms;
            app.reset_stream_counters();
            #[cfg(feature = "stylos")]
            {
                app.last_assistant_text = None;
            }
        }
    }
}

pub(crate) fn handle_agent_ready_event(
    app: &mut App,
    agent: Box<themion_core::agent::Agent>,
    sid: Uuid,
    frame_requester: &crate::tui::FrameRequester,
) {
    let agent = *agent;
    crate::app_runtime::apply_agent_ready_update(
        &mut app.agents,
        &mut app.status_model_info,
        &mut app.workflow_state,
        sid,
        agent,
        #[cfg(feature = "stylos")]
        &app.watchdog_state,
    );
    app.agent_busy = app.any_agent_busy() || app.agent_activity.is_some();
    app.mark_dirty_status();
    app.request_draw(frame_requester);
}

pub(crate) fn handle_shell_complete_event(
    app: &mut App,
    output: String,
    exit_code: Option<i32>,
    frame_requester: &crate::tui::FrameRequester,
) {
    app.activity_counters.shell_complete_count += 1;
    clear_agent_activity(app);
    app.push(crate::tui::Entry::Assistant { agent_id: None, text: output });
    if let Some(code) = exit_code {
        if code != 0 {
            app.push(crate::tui::Entry::Assistant {
                agent_id: None,
                text: format!("exit code: {}", code),
            });
        }
    }
    app.push(crate::tui::Entry::Blank);
    app.mark_dirty_all();
    app.request_draw(frame_requester);
}

#[cfg(feature = "stylos")]
pub(crate) fn resolve_and_submit_text(
    app: &mut App,
    text: String,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    let active_request = app
        .agents
        .iter()
        .find_map(|h| h.active_incoming_prompt.as_ref());
    let resolution = crate::app_runtime::resolve_submit_target(
        &crate::app_runtime::build_local_agent_status_entries(&app.agents),
        active_request,
    );
    let agent_index = match resolution {
        crate::app_runtime::SubmitTargetResolution::Interactive { agent_index }
        | crate::app_runtime::SubmitTargetResolution::IncomingPromptTarget { agent_index } => {
            agent_index
        }
        crate::app_runtime::SubmitTargetResolution::MissingIncomingPromptTarget {
            active_agent_index,
            ..
        } => {
            let effect = crate::app_runtime::submit_target_failure_effect(&resolution)
                .expect("missing target resolution has failure effect");
            app.push(crate::tui::Entry::RemoteEvent {
                agent_id: None,
                source: Some(crate::tui::NonAgentSource::Board),
                text: effect.log_text,
            });
            if let (Some(handle), Some(task_id)) = (app.stylos.as_ref(), effect.failed_task_id) {
                crate::app_runtime::publish_stylos_task_failed(
                    &app.background_domain,
                    handle.query_context(),
                    task_id,
                    effect.failure_reason.to_string(),
                );
            }
            crate::app_runtime::clear_active_incoming_prompt(
                &mut app.agents,
                &app.watchdog_state,
                active_agent_index,
            );
            return;
        }
    };

    if app.agents[agent_index].active_incoming_prompt.is_none() {
        app.push(crate::tui::Entry::User(text.clone()));
    }

    app.submit_text_to_agent(agent_index, text, app_tx);
}

#[cfg(feature = "stylos")]
pub(crate) fn maybe_emit_done_mention_for_completed_note(
    app: &mut App,
    agent_index: usize,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) -> bool {
    let Some(remote) = app.agents[agent_index]
        .active_incoming_prompt
        .as_ref()
        .cloned()
    else {
        return false;
    };
    let apply_plan = crate::app_runtime::completed_note_follow_up_apply_plan(
        crate::app_runtime::plan_completed_note_follow_up(&app.db, &remote),
    );
    if let (Some(request), Some(prompt)) = (apply_plan.continue_request, apply_plan.continue_prompt)
    {
        crate::app_runtime::continue_current_note_follow_up(
            &mut app.agents,
            &app.watchdog_state,
            agent_index,
            request,
        );
        resolve_and_submit_text(app, prompt, app_tx);
        return true;
    }
    match apply_plan.emission {
        Some(crate::app_runtime::CompletedNoteFollowUpEmission::RemoteEvent { text }) => {
            app.push(crate::tui::Entry::RemoteEvent {
                agent_id: None,
                source: Some(crate::tui::NonAgentSource::Board),
                text,
            });
        }
        Some(crate::app_runtime::CompletedNoteFollowUpEmission::Status { text }) => {
            app.push(crate::tui::Entry::Status {
                agent_id: None,
                source: Some(crate::tui::NonAgentSource::Board),
                text,
            });
        }
        None => return false,
    }
    false
}

#[cfg(feature = "stylos")]
fn process_incoming_prompt_request(
    app: &mut App,
    request: crate::stylos::IncomingPromptRequest,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    let outcome = crate::app_runtime::plan_incoming_prompt(
        &crate::app_runtime::build_local_agent_status_entries(&app.agents),
        &app.board_claims,
        request,
    );
    app.push(crate::tui::Entry::RemoteEvent {
        agent_id: outcome.log_agent_id.clone(),
        source: if outcome.log_agent_id.is_some() {
            None
        } else {
            Some(crate::tui::NonAgentSource::Stylos)
        },
        text: outcome.log_text.clone(),
    });
    let apply_plan = match crate::app_runtime::incoming_prompt_apply_plan(outcome) {
        Ok(apply_plan) => apply_plan,
        Err(outcome) => {
            if let Some(task_failure) = outcome.task_failure {
                if let (Some(handle), Some((task_id, reason))) =
                    (app.stylos.as_ref(), task_failure.split_once(':'))
                {
                    crate::app_runtime::publish_stylos_task_failed(
                        &app.background_domain,
                        handle.query_context(),
                        task_id.to_string(),
                        reason.to_string(),
                    );
                }
            }
            return;
        }
    };
    crate::app_runtime::apply_active_incoming_prompt(
        &mut app.agents,
        &app.watchdog_state,
        apply_plan.accepted_agent_index,
        apply_plan.accepted_request,
        apply_plan.pending_watchdog_note,
    );
    app.submit_text_to_agent(
        apply_plan.accepted_agent_index,
        apply_plan.accepted_prompt,
        app_tx,
    );
}

#[cfg(feature = "stylos")]
fn handle_watchdog_dispatch_event(
    app: &mut App,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    let Some(local_instance) = app.local_stylos_instance.as_deref() else {
        return;
    };
    let agent_statuses = crate::app_runtime::build_local_agent_status_entries(&app.agents);
    let mut candidate_ids = agent_statuses
        .iter()
        .filter(|h| h.roles.iter().any(|r| r == "interactive"))
        .map(|h| h.agent_id.clone())
        .collect::<Vec<_>>();
    candidate_ids.extend(
        agent_statuses
            .iter()
            .filter(|h| {
                !h.roles.iter().any(|r| r == "interactive")
                    && !h.roles.iter().any(|r| r == "master")
            })
            .map(|h| h.agent_id.clone()),
    );
    candidate_ids.extend(
        agent_statuses
            .iter()
            .filter(|h| {
                h.roles.iter().any(|r| r == "master")
                    && !h.roles.iter().any(|r| r == "interactive")
            })
            .map(|h| h.agent_id.clone()),
    );

    for agent_id in candidate_ids {
        let Some(handle) = agent_statuses.iter().find(|h| h.agent_id == agent_id) else {
            continue;
        };
        if handle.busy || handle.has_active_incoming_prompt {
            continue;
        }
        let Some(request) = crate::board_runtime::resolve_pending_board_note_injection(
            &app.db,
            &app.board_claims,
            local_instance,
            &agent_id,
            crate::stylos::IncomingPromptSource::WatchdogBoardNote,
        ) else {
            continue;
        };
        app.activity_counters.incoming_prompt_count += 1;
        process_incoming_prompt_request(app, request, app_tx);
        return;
    }
}

#[cfg(feature = "stylos")]
pub(crate) fn handle_stylos_cmd_event(
    app: &mut App,
    cmd: crate::stylos::StylosCmdRequest,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    app.push(crate::tui::Entry::RemoteEvent {
        agent_id: app
            .agents
            .iter()
            .find(|h| crate::app_runtime::is_interactive_agent_handle(h))
            .map(|h| h.agent_id.clone()),
        source: None,
        text: format!(
            "Stylos cmd scope=local preview={}",
            cmd.prompt.lines().next().unwrap_or("")
        ),
    });
    if let Some(index) = app.agents.iter().position(crate::app_runtime::is_interactive_agent_handle) {
        crate::app_runtime::clear_active_incoming_prompt(&mut app.agents, &app.watchdog_state, index);
    }
    resolve_and_submit_text(app, cmd.prompt, app_tx);
}


pub(crate) async fn handle_runtime_event(
    app: &mut App,
    event: AppRuntimeEvent,
    frame_requester: &crate::tui::FrameRequester,
    _app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    match event {
        AppRuntimeEvent::AgentReady(agent, sid) => handle_agent_ready_event(app, agent, sid, frame_requester),
        AppRuntimeEvent::ShellComplete { output, exit_code } => {
            handle_shell_complete_event(app, output, exit_code, frame_requester)
        }
        #[cfg(feature = "stylos")]
        AppRuntimeEvent::StylosCmd(cmd) => handle_stylos_cmd_event(app, cmd, _app_tx),
        #[cfg(feature = "stylos")]
        AppRuntimeEvent::IncomingPrompt(request) => handle_incoming_prompt_event(app, request, _app_tx),
        #[cfg(feature = "stylos")]
        AppRuntimeEvent::StylosEvent(text) => app.push(crate::tui::Entry::RemoteEvent {
            agent_id: None,
            source: Some(crate::tui::NonAgentSource::Stylos),
            text,
        }),
        #[cfg(feature = "stylos")]
        AppRuntimeEvent::WatchdogDispatchLog { agent_id, text } => {
            if !text.is_empty() {
                app.push(crate::tui::Entry::RemoteEvent {
                    agent_id,
                    source: Some(crate::tui::NonAgentSource::Watchdog),
                    text,
                });
            }
            handle_watchdog_dispatch_event(app, _app_tx);
        },
        AppRuntimeEvent::Agent(sid, ev) => {
            app.activity_counters.agent_event_count += 1;
            process_agent_event(
                app,
                sid,
                ev,
                #[cfg(feature = "stylos")]
                _app_tx,
            );
            if app.dirty.any() {
                app.request_draw(frame_requester);
            }
        }
    }
}

pub fn resolve_project_dir(project_dir_override: Option<PathBuf>) -> PathBuf {
    project_dir_override
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn open_history_db(interactive: bool) -> Arc<DbHandle> {
    match dirs::data_dir() {
        Some(d) => themion_core::db::open_default_in_data_dir(&d).unwrap_or_else(|e| {
            if interactive {
                eprintln!("warning: history persistence disabled: {}", e);
            }
            DbHandle::open_in_memory().expect("in-memory db")
        }),
        None => {
            if interactive {
                eprintln!("warning: history persistence disabled (no data dir)");
            }
            DbHandle::open_in_memory().expect("in-memory db")
        }
    }
}

#[cfg(feature = "stylos")]
pub async fn start_stylos(
    app_state: &AppState,
    #[cfg(feature = "stylos")] shared_status_hub: Option<crate::app_runtime::SharedStylosStatusHub>,
) -> anyhow::Result<crate::stylos::StylosHandle> {
    match app_state
        .runtime_domains
        .network()
        .spawn({
            let stylos_cfg = app_state.stylos_config.clone();
            let session = app_state.session.clone();
            let project_dir = app_state.project_dir.clone();
            let db = app_state.db.clone();
            let network_domain = app_state.runtime_domains.network();
            #[cfg(feature = "stylos")]
            let shared_status_hub = shared_status_hub.clone();
            async move {
                crate::stylos::start(&stylos_cfg, &session, &project_dir, db, network_domain, shared_status_hub).await
            }
        })
        .await
    {
        Ok(handle) => Ok(handle),
        Err(err) => Err(anyhow::anyhow!("failed to start stylos runtime: {}", err)),
    }
}

#[cfg(feature = "stylos")]
#[cfg(feature = "stylos")]
#[cfg(feature = "stylos")]
pub fn create_done_mention_locally(
    db: &DbHandle,
    request: &DoneMentionRequest,
) -> anyhow::Result<String> {
    let body = format!(
        "Done: delegated note completed.\n\nOriginal note: {} ({})\nCompleted by: {} / {}\nResult:\n{}",
        request.note_id,
        request.note_slug,
        request.completed_by_instance,
        request.completed_by_agent_id,
        request.result_summary,
    );
    db.create_board_note(CreateNoteArgs {
        note_id: uuid::Uuid::new_v4().to_string(),
        note_kind: NoteKind::DoneMention,
        column: NoteColumn::Todo,
        origin_note_id: Some(request.note_id.clone()),
        from_instance: Some(request.completed_by_instance.clone()),
        from_agent_id: Some(request.completed_by_agent_id.clone()),
        to_instance: request.from_instance.clone(),
        to_agent_id: request.from_agent_id.clone(),
        body,
        meta_json: None,
    })
    .map(|done_note| {
        serde_json::json!({
            "accepted": true,
            "note_id": done_note.note_id,
            "note_slug": done_note.note_slug,
            "agent_id": done_note.to_agent_id,
        })
        .to_string()
    })
    .map_err(anyhow::Error::from)
}

pub fn resolve_codex_auth(session: &Session) -> anyhow::Result<Option<themion_core::CodexAuth>> {
    if let Some(auth) = crate::auth_store::load_for_profile(&session.active_profile)? {
        return Ok(Some(auth));
    }

    let mut obvious_targets = Vec::new();
    if session
        .profiles
        .get(&session.configured_profile)
        .is_some_and(|profile| profile.provider.as_deref() == Some("openai-codex"))
    {
        obvious_targets.push(session.configured_profile.clone());
    }
    if session
        .profiles
        .get("codex")
        .is_some_and(|profile| profile.provider.as_deref() == Some("openai-codex"))
        && !obvious_targets.iter().any(|name| name == "codex")
    {
        obvious_targets.push("codex".to_string());
    }
    let codex_profile_names: Vec<String> = session
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

    if obvious_targets.len() == 1 && obvious_targets[0] == session.active_profile {
        return crate::auth_store::migrate_legacy_to_profile(&session.active_profile);
    }

    Ok(None)
}

pub fn build_agent(
    session: &Session,
    session_id: Uuid,
    project_dir: PathBuf,
    db: Arc<DbHandle>,
    #[cfg(feature = "stylos")] stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
    #[cfg(feature = "stylos")] local_instance_id: Option<&str>,
    #[cfg(feature = "stylos")] local_agent_id: &str,
    local_agent_tool_invoker: Option<themion_core::tools::LocalAgentToolInvoker>,
    system_inspection: Option<SystemInspectionResult>,
    api_log_enabled: bool,
) -> anyhow::Result<Agent> {
    let client: Box<dyn ChatBackend + Send + Sync> = match session.provider.as_str() {
        "openai-codex" => {
            let profile_name = session.active_profile.clone();
            let auth = resolve_codex_auth(session)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "no Codex auth for profile '{}'; run /login codex {}",
                    session.active_profile,
                    session.active_profile
                )
            })?;
            Box::new(CodexClient::new(
                session.base_url.clone(),
                auth,
                Box::new(move |a: &themion_core::CodexAuth| {
                    crate::auth_store::save_for_profile(&profile_name, a)
                }),
            ))
        }
        _ => {
            let mut c = ChatClient::new(session.base_url.clone(), session.api_key.clone());
            if session.provider == "openrouter" {
                c = c.with_headers([
                    (
                        "HTTP-Referer".to_string(),
                        "https://github.com/tasanakorn".to_string(),
                    ),
                    ("X-Title".to_string(), "themion".to_string()),
                    ("X-OpenRouter-Title".to_string(), "themion".to_string()),
                    (
                        "X-OpenRouter-Categories".to_string(),
                        "developer-tools".to_string(),
                    ),
                ]);
            }
            Box::new(c)
        }
    };

    let mut agent = Agent::new_with_db(
        client,
        session.model.clone(),
        Some(session.provider.clone()),
        Some(session.active_profile.clone()),
        session.system_prompt.clone(),
        session_id,
        project_dir,
        db,
    );
    agent.set_api_log_enabled(api_log_enabled);
    agent.set_system_inspection(system_inspection);
    agent.set_local_agent_tool_invoker(local_agent_tool_invoker);

    #[cfg(feature = "stylos")]
    {
        agent.set_stylos_tool_invoker(crate::app_runtime::stylos_tool_invoker(stylos_tool_bridge));
        agent.set_local_instance_id(local_instance_id.map(str::to_string));
        agent.set_local_agent_id(Some(local_agent_id.to_string()));
    }

    Ok(agent)
}
