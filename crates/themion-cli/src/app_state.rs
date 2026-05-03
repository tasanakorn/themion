use crate::config::{save_profiles, Config};
use crate::tui::{AppEvent, App, Entry, FrameRequester};
use crate::app_runtime::{start_watchdog_task, WatchdogRuntimeState};
use crate::runtime_domains::RuntimeDomains;
use crate::Session;
use std::path::PathBuf;
use std::sync::Arc;
use themion_core::agent::Agent;
use themion_core::client::ChatClient;
use themion_core::client_codex::CodexClient;
use themion_core::db::DbHandle;
use themion_core::db::{CreateNoteArgs, NoteColumn, NoteKind};
use themion_core::ModelInfo;
use crate::app_runtime::{
    apply_master_agent_replacement, build_local_agent_roster,
    build_local_agent_tool_invoker, handle_local_agent_management_request as runtime_handle_local_agent_management_request,
    LocalAgentManagementRequest, LocalAgentRuntimeContext,
};
use themion_core::tools::SystemInspectionResult;
use themion_core::ChatBackend;
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

pub(crate) enum AppRuntimeEvent {
    Agent(Uuid, themion_core::agent::AgentEvent),
    AgentReady(Box<themion_core::agent::Agent>, Uuid),
    #[cfg(feature = "stylos")]
    StylosCmd(crate::stylos::StylosCmdRequest),
    #[cfg(feature = "stylos")]
    IncomingPrompt(crate::local_prompts::IncomingPromptRequest),
    WatchdogTick,
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

#[derive(Default)]
pub(crate) struct SessionTokens {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cached: u64,
    pub llm_rounds: u64,
    pub tool_calls: u64,
    pub elapsed_ms: u64,
}

pub(crate) struct AppRuntimeState {
    pub session: Session,
    pub db: Arc<DbHandle>,
    pub project_dir: PathBuf,
    pub session_id: Uuid,
    pub background_domain: crate::runtime_domains::DomainHandle,
    pub core_domain: crate::runtime_domains::DomainHandle,
    #[cfg_attr(not(feature = "stylos"), allow(dead_code))]
    pub startup_project_dir: PathBuf,
    pub local_agent_mgmt_tx: mpsc::UnboundedSender<AppEvent>,
    pub runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
    pub runtime_observer_publisher: crate::app_runtime::AppRuntimeObserverPublisher,
    pub api_log_enabled: bool,
    pub status_model_info: Option<ModelInfo>,
    pub status_rate_limits: Option<themion_core::client_codex::ApiCallRateLimitReport>,
    pub last_ctx_tokens: u64,
    pub session_tokens: SessionTokens,
    pub agent_busy: bool,
    pub agents: Vec<crate::tui::AgentHandle>,
    pub workflow_state: themion_core::workflow::WorkflowState,
    pub pending: Option<String>,
    pub running: bool,
    pub ctrl_c_exit_armed_until: Option<std::time::Instant>,
    pub streaming_idx: Option<usize>,
    pub process_started_at: std::time::Instant,
    pub process_started_at_ms: u64,
    pub idle_since: Option<std::time::Instant>,
    pub watchdog_no_pending_since_by_agent: std::collections::HashMap<String, std::time::Instant>,
    pub idle_status_changed_at: Option<u64>,
    pub agent_activity: Option<AgentActivity>,
    pub agent_activity_changed_at: Option<u64>,
    pub stream_chunks: u64,
    pub stream_chars: u64,
    pub activity_counters: crate::tui::ActivityCounters,
    #[cfg(feature = "stylos")]
    pub stylos: Option<crate::stylos::StylosHandle>,
    #[cfg(feature = "stylos")]
    pub local_instance_id: Option<String>,
    #[cfg(feature = "stylos")]
    pub stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
        pub watchdog_state: Arc<WatchdogRuntimeState>,
        pub board_claims: Arc<crate::board_runtime::LocalBoardClaimRegistry>,
    #[cfg(feature = "stylos")]
    pub shared_status_hub: crate::app_runtime::SharedStylosStatusHub,
    #[cfg(feature = "stylos")]
    pub last_sender_side_transport_event: Option<crate::stylos::SenderSideTransportEvent>,
    #[cfg(feature = "stylos")]
    pub incoming_prompts: crate::app_runtime::IncomingPromptState,
    #[cfg(feature = "stylos")]
    pub last_assistant_text: Option<String>,
}

impl AppRuntimeState {
    pub(crate) fn background_domain(&self) -> crate::runtime_domains::DomainHandle {
        self.background_domain.clone()
    }

    pub(crate) fn core_domain(&self) -> crate::runtime_domains::DomainHandle {
        self.core_domain.clone()
    }
}

pub struct AppState {
    pub runtime: AppRuntimeState,
    pub runtime_domains: Arc<RuntimeDomains>,
    pub snapshot_hub: AppSnapshotHub,
    #[cfg(feature = "stylos")]
    pub stylos_config: crate::config::StylosConfig,
}

#[cfg_attr(not(feature = "stylos"), allow(dead_code))]
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

        let process_started_at = std::time::Instant::now();
        let process_started_at_ms = crate::tui::unix_epoch_now_ms();

        Ok(Self {
            runtime: AppRuntimeState {
                session: session.clone(),
                db: db.clone(),
                project_dir: project_dir.clone(),
                session_id,
                background_domain: runtime_domains.background().expect("force-red bg runtime in AppState"),
                core_domain: runtime_domains.core(),
                startup_project_dir: project_dir.clone(),
                local_agent_mgmt_tx: tokio::sync::mpsc::unbounded_channel().0,
                runtime_tx: tokio::sync::mpsc::unbounded_channel().0,
                runtime_observer_publisher: crate::app_runtime::AppRuntimeObserverPublisher::new(crate::app_runtime::AppSnapshotPublisher::new(snapshot_hub.clone())),
                api_log_enabled: false,
                status_model_info: session.model_info.clone(),
                status_rate_limits: None,
                last_ctx_tokens: 0,
                session_tokens: Default::default(),
                agent_busy: false,
                agents: Vec::new(),
                workflow_state: themion_core::workflow::WorkflowState::default(),
                pending: None,
                running: true,
                ctrl_c_exit_armed_until: None,
                streaming_idx: None,
                process_started_at,
                process_started_at_ms,
                idle_since: Some(process_started_at),
                watchdog_no_pending_since_by_agent: std::collections::HashMap::new(),
                idle_status_changed_at: Some(process_started_at_ms),
                agent_activity: None,
                agent_activity_changed_at: None,
                stream_chunks: 0,
                stream_chars: 0,
                activity_counters: Default::default(),
                #[cfg(feature = "stylos")]
                stylos: None,
                #[cfg(feature = "stylos")]
                local_instance_id: None,
                #[cfg(feature = "stylos")]
                stylos_tool_bridge: None,
                                watchdog_state: Arc::new(WatchdogRuntimeState::default()),
                                board_claims: Arc::new(crate::board_runtime::LocalBoardClaimRegistry::default()),
                #[cfg(feature = "stylos")]
                shared_status_hub: crate::app_runtime::SharedStylosStatusHub::default(),
                #[cfg(feature = "stylos")]
                last_sender_side_transport_event: None,
                #[cfg(feature = "stylos")]
                incoming_prompts: Default::default(),
                #[cfg(feature = "stylos")]
                last_assistant_text: None,
            },
            runtime_domains,
            snapshot_hub,
            #[cfg(feature = "stylos")]
            stylos_config,
        })
    }
}



pub(crate) fn finalize_tui_runtime_state(
    runtime: &mut AppRuntimeState,
    app_tx: mpsc::UnboundedSender<AppEvent>,
    runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
    runtime_observer_publisher: crate::app_runtime::AppRuntimeObserverPublisher,
) {
    runtime.local_agent_mgmt_tx = app_tx.clone();
    runtime.runtime_tx = runtime_tx;
    runtime.runtime_observer_publisher = runtime_observer_publisher;

    let agent = crate::app_runtime::build_main_agent(
        &runtime.session,
        runtime.db.clone(),
        runtime.session_id,
        runtime.project_dir.clone(),
        app_tx,
        #[cfg(feature = "stylos")]
        runtime.stylos_tool_bridge.clone(),
        #[cfg(feature = "stylos")]
        runtime.local_instance_id.as_deref(),
        #[cfg(feature = "stylos")]
        "master",
        None,
        runtime.api_log_enabled,
    )
    .expect("failed to build agent");

    runtime.agents = vec![crate::tui::AgentHandle {
        agent: Some(agent),
        session_id: runtime.session_id,
        agent_id: "master".to_string(),
        label: "master".to_string(),
        roles: vec!["master".to_string(), "interactive".to_string()],
        busy: false,
        turn_cancellation: None,
    }];
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


pub fn start_tui_watchdog_loop(
    app_state: &AppState,
    runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
) {
    let tui_domain = app_state
        .runtime_domains
        .tui()
        .expect("tui runtime available in TUI mode");
    start_watchdog_task(&tui_domain, runtime_tx, app_state.runtime.watchdog_state.clone());
}



pub(crate) fn context_report_lines(runtime: &AppRuntimeState) -> Vec<String> {
    if let Some(handle) = runtime
        .agents
        .iter()
        .find(|h| crate::app_runtime::is_interactive_agent_handle(h))
    {
        if let Some(agent) = handle.agent.as_ref() {
            return crate::tui::format_context_report(&agent.prompt_context_report());
        }
    }
    vec!["context report unavailable".to_string()]
}

pub(crate) fn runtime_any_agent_busy(runtime: &AppRuntimeState) -> bool {
    runtime.agents.iter().any(|h| h.busy)
}

pub(crate) fn runtime_reset_stream_counters(runtime: &mut AppRuntimeState) {
    runtime.stream_chunks = 0;
    runtime.stream_chars = 0;
}

pub(crate) fn runtime_pending_str(runtime: &AppRuntimeState, anim_frame: u8) -> String {
    const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let ch = SPINNER[anim_frame as usize % SPINNER.len()];
    let activity = runtime
        .agent_activity
        .as_ref()
        .map(|p| p.label(runtime.stream_chunks, runtime.stream_chars))
        .unwrap_or_else(|| "thinking…".to_string());
    format!("  {} {}", ch, activity)
}

pub(crate) fn runtime_replace_master_agent(
    runtime: &mut AppRuntimeState,
    new_agent: themion_core::agent::Agent,
    new_session_id: Uuid,
) {
    apply_master_agent_replacement(
        &mut runtime.agents,
        &mut runtime.status_model_info,
        &mut runtime.workflow_state,
        new_agent,
        new_session_id,
    );
}

pub(crate) fn handle_local_agent_management_request(
    app: &mut App,
    request: LocalAgentManagementRequest,
    frame_requester: &FrameRequester,
) {
    let local_agent_tool_invoker =
        build_local_agent_tool_invoker(app.runtime.local_agent_mgmt_tx.clone());
    let any_agent_busy = runtime_any_agent_busy(&app.runtime);
    let roster = build_local_agent_roster(&app.runtime.agents);
    let result = runtime_handle_local_agent_management_request(
        LocalAgentRuntimeContext {
            session: &app.runtime.session,
            project_dir: &app.runtime.project_dir,
            db: &app.runtime.db,
            agents: &mut app.runtime.agents,
            roster: &roster,
            agent_busy: any_agent_busy,
            #[cfg(feature = "stylos")]
            stylos_tool_bridge: app.runtime.stylos_tool_bridge.clone(),
            #[cfg(feature = "stylos")]
            local_instance_id: app.runtime.local_instance_id.as_deref(),
            local_agent_tool_invoker: Some(local_agent_tool_invoker),
            api_log_enabled: app.runtime.api_log_enabled,
        },
        &request.action,
        request.args,
    );
    publish_runtime_snapshot(app);
    app.mark_dirty_all();
    app.request_draw(frame_requester);
    let _ = request.reply_tx.send(result);
}

pub(crate) fn runtime_request_interrupt(app: &mut App) {
    let mut interrupted_any = false;
    for handle in &app.runtime.agents {
        if let Some(cancel) = &handle.turn_cancellation {
            if !cancel.is_interrupted() {
                cancel.interrupt();
                interrupted_any = true;
            }
        }
    }
    if interrupted_any {
        app.push(Entry::Status {
            agent_id: None,
            source: Some(crate::tui::NonAgentSource::Runtime),
            text: "interrupt requested".to_string(),
        });
    }
}

pub(crate) fn runtime_arm_ctrl_c_exit(app: &mut App) {
    app.runtime.ctrl_c_exit_armed_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
    app.push(Entry::Status {
        agent_id: None,
        source: Some(crate::tui::NonAgentSource::Runtime),
        text: "Press Ctrl+C again within 3s to exit".to_string(),
    });
    app.mark_dirty_status();
}

pub(crate) fn runtime_ctrl_c_exit_is_armed(runtime: &AppRuntimeState, now: std::time::Instant) -> bool {
    matches!(runtime.ctrl_c_exit_armed_until, Some(deadline) if deadline > now)
}

pub(crate) fn runtime_expire_ctrl_c_exit_if_needed(runtime: &mut AppRuntimeState, now: std::time::Instant) -> bool {
    if matches!(runtime.ctrl_c_exit_armed_until, Some(deadline) if deadline <= now) {
        runtime.ctrl_c_exit_armed_until = None;
        return true;
    }
    false
}

pub(crate) fn set_agent_activity(app: &mut App, activity: AgentActivity) {
    let activity_changed = app
        .runtime
        .agent_activity
        .as_ref()
        .map(|current| std::mem::discriminant(current) != std::mem::discriminant(&activity))
        .unwrap_or(true);
    app.runtime.agent_activity = Some(activity);
    if activity_changed {
        app.runtime.agent_activity_changed_at = Some(crate::tui::unix_epoch_now_ms());
    }
    app.runtime.idle_since = None;
    app.runtime.watchdog_no_pending_since_by_agent.clear();
    app.runtime.idle_status_changed_at = None;
    app.runtime.pending = Some(runtime_pending_str(&app.runtime, app.anim_frame));
    app.mark_dirty_status();
    publish_runtime_snapshot(app);
}

pub(crate) fn clear_agent_activity(app: &mut App) {
    app.runtime.agent_activity = None;
    app.runtime.agent_activity_changed_at = None;
    app.runtime.idle_since = Some(std::time::Instant::now());
    app.runtime.watchdog_no_pending_since_by_agent.clear();
    app.runtime.idle_status_changed_at = Some(crate::tui::unix_epoch_now_ms());
    app.runtime.pending = None;
    app.mark_dirty_status();
    publish_runtime_snapshot(app);
}

pub(crate) fn on_tick(app: &mut App) {
    app.runtime.activity_counters.tick_count += 1;
    runtime_expire_ctrl_c_exit_if_needed(&mut app.runtime, std::time::Instant::now());
    app.record_runtime_snapshot();
    publish_runtime_snapshot(app);
    let previous = app.runtime.pending.clone();
    app.anim_frame = app.anim_frame.wrapping_add(1);
    if app.runtime.agent_busy && app.runtime.pending.is_some() {
        app.runtime.pending = Some(runtime_pending_str(&app.runtime, app.anim_frame));
    }
    if app.runtime.pending != previous {
        app.mark_dirty_status();
    }
}




pub(crate) fn session_config_lines(session: &Session) -> Vec<String> {
    let key_display = match &session.api_key {
        Some(k) if k.len() > 8 => format!("{}…", &k[..8]),
        Some(_) => "(set)".to_string(),
        None => "(none)".to_string(),
    };
    let mut out = vec![
        format!("profile  : {}", session.active_profile),
        format!("provider : {}", session.provider),
        format!("model    : {}", session.model),
        format!("endpoint : {}", session.base_url),
        format!("api_key  : {}", key_display),
    ];
    if session.temporary_profile_override.is_some() || session.temporary_model_override.is_some() {
        out.push(
            "note     : temporary session-only override active; config on disk unchanged"
                .to_string(),
        );
    }
    out
}

pub(crate) fn session_show_lines(session: &Session) -> Vec<String> {
    vec![
        format!("configured profile : {}", session.configured_profile),
        format!("effective profile   : {}", session.active_profile),
        format!("effective provider  : {}", session.provider),
        format!("effective model     : {}", session.model),
        format!(
            "temporary profile override : {}",
            session
                .temporary_profile_override
                .as_deref()
                .unwrap_or("(none)")
        ),
        format!(
            "temporary model override   : {}",
            session
                .temporary_model_override
                .as_deref()
                .unwrap_or("(none)")
        ),
    ]
}

pub(crate) fn config_profile_list_lines(session: &Session) -> Vec<String> {
    let mut names: Vec<String> = session.profiles.keys().cloned().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let marker = if name == session.active_profile { "* " } else { "  " };
            format!("{}{}", marker, name)
        })
        .collect()
}

pub(crate) async fn handle_login_complete_event(
    app: &mut App,
    profile_name: String,
    auth_result: anyhow::Result<themion_core::CodexAuth>,
    frame_requester: &FrameRequester,
) {
    match auth_result {
        Ok(auth) => {
            clear_agent_activity(app);
            if let Err(e) = crate::auth_store::save_for_profile(&profile_name, &auth) {
                app.push(Entry::Assistant {
                    agent_id: None,
                    text: format!("warning: failed to save auth: {}", e),
                });
            }
            app.runtime
                .session
                .profiles
                .insert(profile_name.clone(), crate::config::codex_profile_defaults());
            app.runtime.session.configured_profile = profile_name.clone();
            app.runtime.session.switch_profile(&profile_name);
            if let Err(e) = save_profiles(&app.runtime.session.active_profile, &app.runtime.session.profiles)
            {
                app.push(Entry::Assistant {
                    agent_id: None,
                    text: format!("warning: failed to save config: {}", e),
                });
            }
            match crate::app_runtime::build_replacement_main_agent(crate::app_runtime::AgentReplacementParams {
                session: &app.runtime.session,
                project_dir: &app.runtime.project_dir,
                db: &app.runtime.db,
                #[cfg(feature = "stylos")]
                stylos_tool_bridge: app.runtime.stylos_tool_bridge.clone(),
                #[cfg(feature = "stylos")]
                local_instance_id: app.runtime.local_instance_id.as_deref(),
                api_log_enabled: app.runtime.api_log_enabled,
                local_agent_mgmt_tx: app.runtime.local_agent_mgmt_tx.clone(),
                insert_session: true,
            }) {
                Ok((mut new_agent, new_session_id)) => {
                    new_agent.refresh_model_info().await;
                    runtime_replace_master_agent(&mut app.runtime, new_agent, new_session_id);
                    publish_runtime_snapshot(app);
                    app.push(Entry::Assistant {
                        agent_id: None,
                        text: format!(
                            "logged in as {} — switched to Codex profile '{}' ({})",
                            auth.account_id,
                            profile_name,
                            app.runtime.session.model
                        ),
                    });
                    app.push(Entry::Blank);
                    app.mark_dirty_all();
                    app.request_draw(frame_requester);
                }
                Err(e) => {
                    app.push(Entry::Assistant {
                        agent_id: None,
                        text: format!("login succeeded but agent build failed: {}", e),
                    });
                    app.mark_dirty_all();
                    app.request_draw(frame_requester);
                }
            }
        }
        Err(e) => {
            clear_agent_activity(app);
            app.push(Entry::Assistant {
                agent_id: None,
                text: format!("login failed: {}", e),
            });
            app.mark_dirty_all();
            app.request_draw(frame_requester);
        }
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
            if app.runtime.agent_busy {
                app.push(Entry::Assistant {
                    agent_id: None,
                    text: "busy, please wait".to_string(),
                });
                app.push(Entry::Blank);
                app.mark_dirty_all();
                return;
            }
            app.runtime.agent_busy = true;
            set_agent_activity(app, AgentActivity::LoginStarting);
            let target_profile = profile_name.clone().unwrap_or_else(|| {
                if app
                    .runtime.session
                    .profiles
                    .get(&app.runtime.session.active_profile)
                    .is_some_and(|profile| profile.provider.as_deref() == Some("openai-codex"))
                {
                    app.runtime.session.active_profile.clone()
                } else {
                    "codex".to_string()
                }
            });
            let login_message = if profile_name.is_some() {
                format!("logging in to OpenAI Codex for profile '{}'…", target_profile)
            } else if target_profile == app.runtime.session.active_profile {
                format!("logging in to OpenAI Codex for current profile '{}'…", target_profile)
            } else {
                format!("logging in to OpenAI Codex for profile '{}'…", target_profile)
            };
            app.push(Entry::Assistant {
                agent_id: None,
                text: login_message,
            });
            let tx = app_tx.clone();
            app.runtime.background_domain().spawn(async move {
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
        crate::app_runtime::RuntimeCommand::UnifiedSearchIndex { full, source_kind } => {
            #[cfg(not(feature = "semantic-memory"))]
            {
                let _ = &source_kind;
                let mode = if full { "full reindex" } else { "index" };
                app.push(Entry::Assistant {
                    agent_id: None,
                    text: format!(
                        "unified-search {} is unavailable in this build; enable the semantic-memory feature",
                        mode
                    ),
                });
                app.push(Entry::Blank);
                app.mark_dirty_all();
            }
            #[cfg(feature = "semantic-memory")]
            {
                if app.runtime.agent_busy {
                    app.push(Entry::Assistant {
                        agent_id: None,
                        text: "busy, please wait".to_string(),
                    });
                    app.push(Entry::Blank);
                    app.mark_dirty_all();
                    return;
                }
                app.runtime.agent_busy = true;
                let scope_suffix = source_kind.as_deref().map(|kind| format!(" ({})", kind)).unwrap_or_default();
                set_agent_activity(app, AgentActivity::RunningTool(if full {
                    format!("unified-search full reindex{}", scope_suffix)
                } else {
                    format!("unified-search index pending{}", scope_suffix)
                }));
                app.push(Entry::Assistant {
                    agent_id: None,
                    text: match (full, source_kind.as_deref()) {
                        (true, Some(kind)) => format!("rebuilding generalized unified-search index for source kind '{}' in this project…", kind),
                        (false, Some(kind)) => format!("refreshing generalized unified-search index for source kind '{}' in this project…", kind),
                        (true, None) => "rebuilding generalized unified-search index for this project…".to_string(),
                        (false, None) => "refreshing generalized unified-search index for this project…".to_string(),
                    },
                });
                let tx = app.runtime.runtime_tx.clone();
                let db = app.runtime.db.clone();
                let project_dir = app.runtime.project_dir.display().to_string();
                app.runtime.background_domain().spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        db.memory_store().rebuild_unified_search_index(Some(&project_dir), source_kind.as_deref(), full)
                    })
                    .await;
                    let text = match result {
                        Ok(Ok(report)) => serde_json::to_string_pretty(&report)
                            .unwrap_or_else(|err| format!("indexing report serialization failed: {}", err)),
                        Ok(Err(err)) => {
                            if full {
                                format!("unified-search full reindex failed: {}", err)
                            } else {
                                format!("unified-search indexing failed: {}", err)
                            }
                        }
                        Err(err) => {
                            if full {
                                format!("unified-search full reindex task failed: {}", err)
                            } else {
                                format!("unified-search indexing task failed: {}", err)
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
                    session: &mut app.runtime.session,
                    project_dir: &app.runtime.project_dir,
                    db: &app.runtime.db,
                    #[cfg(feature = "stylos")]
                    stylos_tool_bridge: app.runtime.stylos_tool_bridge.clone(),
                    #[cfg(feature = "stylos")]
                    local_instance_id: app.runtime.local_instance_id.as_deref(),
                    api_log_enabled: app.runtime.api_log_enabled,
                    local_agent_mgmt_tx: app.runtime.local_agent_mgmt_tx.clone(),
                },
            );
            let application = crate::app_runtime::apply_runtime_command_outcome_to_app_runtime(
                &mut app.runtime.agents,
                &mut app.runtime.status_model_info,
                &mut app.runtime.workflow_state,
                &mut app.runtime.api_log_enabled,
                &mut app.runtime.last_ctx_tokens,
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
        .runtime
        .agent_activity
        .as_ref()
        .map(|activity| activity.status_bar(app.runtime.stream_chunks, app.runtime.stream_chars));
    let recent_window_ms = app.recent_runtime_delta().map(|recent| recent.wall_elapsed_ms);
    let uptime_ms = app.runtime.process_started_at.elapsed().as_millis() as u64;
    #[cfg(feature = "stylos")]
    let stylos_status = app
        .runtime
        .stylos
        .as_ref()
        .map(|handle| match handle.state() {
            crate::stylos::StylosRuntimeState::Off => "off".to_string(),
            crate::stylos::StylosRuntimeState::Active { .. } => "active".to_string(),
            crate::stylos::StylosRuntimeState::Error(_) => "error".to_string(),
        })
        .unwrap_or_else(|| "off".to_string());
    #[cfg(feature = "stylos")]
    let stylos = app.runtime.stylos.as_ref().map(|_| {
        let agent_status_entries = crate::app_runtime::build_local_agent_status_entries(
            &app.runtime.agents,
            &app.runtime.incoming_prompts,
        );
        crate::app_runtime::StylosRuntimeStatusPublishState {
            hub: &app.runtime.shared_status_hub,
            startup_project_dir: &app.runtime.startup_project_dir,
            fallback_project_dir: &app.runtime.project_dir,
            provider: &app.runtime.session.provider,
            model: &app.runtime.session.model,
            active_profile: &app.runtime.session.active_profile,
            rate_limits: app.runtime.status_rate_limits.as_ref(),
            idle_since: app.runtime.idle_since,
            idle_status_changed_at: app.runtime.idle_status_changed_at,
            primary_activity_label: primary_activity_label.clone(),
            primary_activity_changed_at_ms: app.runtime.agent_activity_changed_at,
            primary_workflow: &app.runtime.workflow_state,
            agent_status_entries,
        }
    });

    app.runtime.runtime_observer_publisher.publish(crate::app_runtime::AppRuntimeObserverPublishState {
        agents: &mut app.runtime.agents,
        snapshot: {
            #[cfg(feature = "stylos")]
            {
                crate::app_runtime::AppRuntimeSnapshotPublishState {
                    agent_busy: app.runtime.agent_busy,
                    activity_status: activity_status.clone(),
                    stylos_status: Some(stylos_status),
                    watchdog_state: &app.runtime.watchdog_state,
                    incoming_prompts: &app.runtime.incoming_prompts,
                }
            }
            #[cfg(not(feature = "stylos"))]
            {
                crate::app_runtime::AppRuntimeSnapshotPublishState::new(
                    app.runtime.agent_busy,
                    activity_status.clone(),
                )
            }
        },
        system_inspection: crate::app_runtime::SystemInspectionRuntimeRefreshState {
            session: &app.runtime.session,
            project_dir: &app.runtime.project_dir,
            workflow_state: &app.runtime.workflow_state,
            rate_limits: app.runtime.status_rate_limits.as_ref(),
            activity: app.runtime.agent_activity.as_ref(),
            stream_chunks: app.runtime.stream_chunks,
            stream_chars: app.runtime.stream_chars,
            agent_busy: app.runtime.agent_busy,
            activity_status,
            activity_status_changed_at_ms: app
                .runtime
                .agent_activity_changed_at
                .or(app.runtime.idle_status_changed_at),
            process_started_at_ms: app.runtime.process_started_at_ms,
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
    request: crate::local_prompts::IncomingPromptRequest,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    app.runtime.activity_counters.incoming_prompt_count += 1;
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
                let agent_index = app.runtime.agents.iter().position(|h| h.session_id == sid);
                if let (Some(agent_index), Some(handle)) = (agent_index, app.runtime.stylos.as_ref()) {
                    if let Some(remote) = crate::app_runtime::incoming_prompt_request(&app.runtime.incoming_prompts, &app.runtime.agents[agent_index].agent_id) {
                        if let Some(task_id) = remote.task_id.clone() {
                            crate::app_runtime::publish_stylos_task_running(
                                &app.runtime.background_domain,
                                handle.query_context(),
                                task_id,
                            );
                        }
                    }
                }
            }
            runtime_reset_stream_counters(&mut app.runtime);
            #[cfg(feature = "stylos")]
            {
                app.runtime.last_assistant_text = None;
            }
            set_agent_activity(app, AgentActivity::WaitingForModel);
            app.runtime.streaming_idx = None;
        }
        themion_core::agent::AgentEvent::AssistantChunk(chunk) => {
            #[cfg(feature = "stylos")]
            {
                let next = match app.runtime.last_assistant_text.take() {
                    Some(mut existing) => {
                        existing.push_str(&chunk);
                        existing
                    }
                    None => chunk.clone(),
                };
                app.runtime.last_assistant_text = Some(next);
            }
            app.runtime.stream_chunks += 1;
            app.runtime.stream_chars += chunk.chars().count() as u64;
            set_agent_activity(app, AgentActivity::StreamingResponse);
            let agent_id = crate::tui::agent_id_for_session(&app.runtime.agents, sid);
            match app.runtime.streaming_idx {
                Some(i) => {
                    if let Some(crate::tui::Entry::Assistant { text, .. }) = app.entries.get_mut(i) {
                        text.push_str(&chunk);
                    }
                }
                None => {
                    app.push(crate::tui::Entry::Assistant { agent_id, text: chunk });
                    app.runtime.streaming_idx = Some(app.entries.len() - 1);
                }
            }
        }
        themion_core::agent::AgentEvent::AssistantText(text) => {
            #[cfg(feature = "stylos")]
            {
                app.runtime.last_assistant_text = Some(text.clone());
            }
            app.runtime.streaming_idx = None;
            clear_agent_activity(app);
            app.push(crate::tui::Entry::Assistant {
                agent_id: crate::tui::agent_id_for_session(&app.runtime.agents, sid),
                text,
            });
        }
        themion_core::agent::AgentEvent::ToolStart { name, arguments_json, display_arguments_json } => {
            app.runtime.streaming_idx = None;
            let display_args_json = display_arguments_json.as_deref().unwrap_or(&arguments_json);
            let (detail, reason) = crate::tui::split_tool_call_detail(&name, display_args_json);
            let activity_detail = match &reason {
                Some(reason) => format!("{detail} — {reason}"),
                None => detail.clone(),
            };
            set_agent_activity(app, AgentActivity::RunningTool(activity_detail));
            #[cfg(feature = "stylos")]
            {
                app.runtime.last_sender_side_transport_event = app
                    .runtime
                    .local_instance_id
                    .as_deref()
                    .and_then(|local_instance| {
                        crate::stylos::sender_side_transport_event_from_tool_detail(
                            &detail,
                            local_instance,
                            app.runtime.stylos_tool_bridge.is_some(),
                        )
                    });
            }
            app.push(crate::tui::Entry::ToolCall {
                agent_id: crate::tui::agent_id_for_session(&app.runtime.agents, sid),
                detail,
                reason,
            });
        }
        themion_core::agent::AgentEvent::ToolEnd => {
            app.push(crate::tui::Entry::ToolDone);
            #[cfg(feature = "stylos")]
            if let Some(event) = app.runtime.last_sender_side_transport_event.take() {
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
                agent_id: crate::tui::agent_id_for_session(&app.runtime.agents, sid),
                source: None,
                text,
            });
        }
        themion_core::agent::AgentEvent::WorkflowStateChanged(state) => {
            app.runtime.workflow_state = state;
            app.mark_dirty_status();
            publish_runtime_snapshot(app);
        }
        themion_core::agent::AgentEvent::Stats(text) => {
            if let Some(json) = text.strip_prefix("[rate-limit] ") {
                if let Ok(report) = serde_json::from_str::<themion_core::client_codex::ApiCallRateLimitReport>(json) {
                    app.runtime.status_rate_limits = Some(report);
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
                let agent_index = app.runtime.agents.iter().position(|h| h.session_id == sid);
                if let Some(agent_index) = agent_index {
                    maybe_emit_done_mention_for_completed_note(app, agent_index, app_tx);
                }
                if let (Some(agent_index), Some(handle)) = (agent_index, app.runtime.stylos.as_ref()) {
                    if let Some(remote) = crate::app_runtime::take_incoming_prompt_request(&mut app.runtime.incoming_prompts, &app.runtime.agents[agent_index].agent_id) {
                        if let Some(task_id) = remote.task_id {
                            let result_text = app.runtime.last_assistant_text.clone();
                            crate::app_runtime::publish_stylos_task_completed(
                                &app.runtime.background_domain,
                                handle.query_context(),
                                task_id,
                                result_text,
                            );
                        }
                    }
                }
            }
            app.runtime.streaming_idx = None;
            set_agent_activity(app, AgentActivity::Finishing);
            clear_agent_activity(app);
            let interrupted = app.runtime.workflow_state.status == themion_core::workflow::WorkflowStatus::Interrupted;
            let stats_text = crate::format_stats(&stats);
            let stats_text = stats_text.strip_prefix("[stats: ").and_then(|s| s.strip_suffix("]")).unwrap_or(&stats_text).to_string();
            app.push(crate::tui::Entry::TurnDone {
                agent_id: crate::tui::agent_id_for_session(&app.runtime.agents, sid),
                summary: if interrupted { "󰇺 Turn interrupted".to_string() } else { "󰇺 Turn end".to_string() },
                stats: stats_text,
            });
            app.push(crate::tui::Entry::Blank);
            app.runtime.activity_counters.agent_turn_completed_count += 1;
            app.runtime.agent_busy = runtime_any_agent_busy(&app.runtime) || app.runtime.agent_activity.is_some();
            if let Some(last_api_call_tokens_in) = stats.last_api_call_tokens_in {
                app.runtime.last_ctx_tokens = last_api_call_tokens_in;
            }
            app.runtime.session_tokens.tokens_in += stats.tokens_in as u64;
            app.runtime.session_tokens.tokens_out += stats.tokens_out as u64;
            app.runtime.session_tokens.tokens_cached += stats.tokens_cached as u64;
            app.runtime.session_tokens.llm_rounds += stats.llm_rounds as u64;
            app.runtime.session_tokens.tool_calls += stats.tool_calls as u64;
            app.runtime.session_tokens.elapsed_ms += stats.elapsed_ms as u64;
            runtime_reset_stream_counters(&mut app.runtime);
            #[cfg(feature = "stylos")]
            {
                app.runtime.last_assistant_text = None;
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
        &mut app.runtime.agents,
        &mut app.runtime.status_model_info,
        &mut app.runtime.workflow_state,
        sid,
        agent,
        #[cfg(feature = "stylos")]
        &app.runtime.watchdog_state,
        #[cfg(feature = "stylos")]
        &app.runtime.incoming_prompts,
    );
    app.runtime.agent_busy = runtime_any_agent_busy(&app.runtime) || app.runtime.agent_activity.is_some();
    app.mark_dirty_status();
    app.request_draw(frame_requester);
}

pub(crate) fn handle_shell_complete_event(
    app: &mut App,
    output: String,
    exit_code: Option<i32>,
    frame_requester: &crate::tui::FrameRequester,
) {
    app.runtime.activity_counters.shell_complete_count += 1;
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


pub(crate) fn submit_shell_command(app: &mut App, command: &str) {
    let command = command.trim_start().to_string();
    app.push(Entry::User(format!("!{}", command)));

    if command.is_empty() {
        app.push(Entry::Assistant {
            agent_id: None,
            text: "empty shell command".to_string(),
        });
        app.push(Entry::Blank);
        return;
    }

    app.runtime.agent_busy = true;
    set_agent_activity(app, AgentActivity::RunningShellCommand);

    let tx = app.runtime.runtime_tx.clone();
    let project_dir = app.runtime.project_dir.clone();
    app.runtime.background_domain().spawn(async move {
        let result = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .current_dir(project_dir)
            .output()
            .await;

        let (output, exit_code) = match result {
            Ok(output) => {
                let mut text = String::new();
                text.push_str(&String::from_utf8_lossy(&output.stdout));
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                let trimmed = text.trim_end_matches(['\n', '\r']);
                let display = if trimmed.is_empty() {
                    "(no output)".to_string()
                } else {
                    trimmed.to_string()
                };
                (display, output.status.code())
            }
            Err(e) => (format!("failed to run shell command: {}", e), None),
        };

        let _ = tx.send(AppRuntimeEvent::ShellComplete { output, exit_code });
    });
}

pub(crate) fn request_app_exit(app: &mut App) {
    app.runtime.running = false;
}

pub(crate) fn confirm_ctrl_c_exit(app: &mut App) {
    app.runtime.ctrl_c_exit_armed_until = None;
    app.runtime.running = false;
}

#[cfg(not(feature = "stylos"))]
pub(crate) fn submit_text_default(app: &mut App, text: String) {
    let agent_index = app
        .runtime
        .agents
        .iter()
        .position(crate::app_runtime::is_interactive_agent_handle)
        .expect("interactive agent");
    app.push(crate::tui::Entry::User(text.clone()));
    submit_text_to_agent(app, agent_index, text);
}

pub(crate) fn submit_text_to_agent(
    app: &mut App,
    agent_index: usize,
    text: String,
) {
    app.runtime.activity_counters.record_agent_turn_started();
    app.runtime.agent_busy = true;
    runtime_reset_stream_counters(&mut app.runtime);
    set_agent_activity(app, AgentActivity::PreparingRequest);

    let runtime_launch = crate::app_runtime::prepare_agent_turn_runtime_launch(&mut app.runtime.agents, agent_index);

    crate::app_runtime::launch_agent_turn_runtime(
        &app.runtime.background_domain(),
        &app.runtime.core_domain(),
        app.runtime.runtime_tx.clone(),
        text,
        runtime_launch,
    );
}

#[cfg(feature = "stylos")]
pub(crate) fn resolve_and_submit_text(
    app: &mut App,
    text: String,
    _app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    let active_request = app
        .runtime.agents
        .iter()
        .find_map(|h| crate::app_runtime::incoming_prompt_request(&app.runtime.incoming_prompts, &h.agent_id));
    let resolution = crate::app_runtime::resolve_submit_target(
        &crate::app_runtime::build_local_agent_status_entries(&app.runtime.agents, &app.runtime.incoming_prompts),
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
            if let (Some(handle), Some(task_id)) = (app.runtime.stylos.as_ref(), effect.failed_task_id) {
                crate::app_runtime::publish_stylos_task_failed(
                    &app.runtime.background_domain,
                    handle.query_context(),
                    task_id,
                    effect.failure_reason.to_string(),
                );
            }
            crate::app_runtime::clear_active_incoming_prompt(
                &app.runtime.agents,
                &mut app.runtime.incoming_prompts,
                &app.runtime.watchdog_state,
                active_agent_index,
            );
            return;
        }
    };

    if crate::app_runtime::incoming_prompt_request(&app.runtime.incoming_prompts, &app.runtime.agents[agent_index].agent_id).is_none() {
        app.push(crate::tui::Entry::User(text.clone()));
    }

    submit_text_to_agent(app, agent_index, text);
}

#[cfg(feature = "stylos")]
fn process_incoming_prompt_request(
    app: &mut App,
    request: crate::local_prompts::IncomingPromptRequest,
    _app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    let outcome = crate::app_runtime::plan_incoming_prompt(
        &crate::app_runtime::build_local_agent_status_entries(&app.runtime.agents, &app.runtime.incoming_prompts),
        &app.runtime.board_claims,
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
                    (app.runtime.stylos.as_ref(), task_failure.split_once(':'))
                {
                    crate::app_runtime::publish_stylos_task_failed(
                        &app.runtime.background_domain,
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
        &app.runtime.agents,
        &mut app.runtime.incoming_prompts,
        &app.runtime.watchdog_state,
        apply_plan.accepted_agent_index,
        apply_plan.accepted_request,
        apply_plan.pending_watchdog_note,
    );
    submit_text_to_agent(
        app,
        apply_plan.accepted_agent_index,
        apply_plan.accepted_prompt,
    );
}

#[cfg(feature = "stylos")]
pub(crate) fn handle_stylos_cmd_event(
    app: &mut App,
    cmd: crate::stylos::StylosCmdRequest,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    app.push(crate::tui::Entry::RemoteEvent {
        agent_id: app
            .runtime.agents
            .iter()
            .find(|h| crate::app_runtime::is_interactive_agent_handle(h))
            .map(|h| h.agent_id.clone()),
        source: None,
        text: format!(
            "Stylos cmd scope=local preview={}",
            cmd.prompt.lines().next().unwrap_or("")
        ),
    });
    if let Some(index) = app.runtime.agents.iter().position(crate::app_runtime::is_interactive_agent_handle) {
        crate::app_runtime::clear_active_incoming_prompt(&app.runtime.agents, &mut app.runtime.incoming_prompts, &app.runtime.watchdog_state, index);
    }
    resolve_and_submit_text(app, cmd.prompt, app_tx);
}

#[cfg(feature = "stylos")]
pub(crate) fn maybe_emit_done_mention_for_completed_note(
    app: &mut App,
    agent_index: usize,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) -> bool {
    let Some(remote) = crate::app_runtime::incoming_prompt_request(
        &app.runtime.incoming_prompts,
        &app.runtime.agents[agent_index].agent_id,
    )
    .cloned()
    else {
        return false;
    };
    let apply_plan = crate::app_runtime::completed_note_follow_up_apply_plan(
        crate::app_runtime::plan_completed_note_follow_up(&app.runtime.db, &remote),
    );
    if let (Some(request), Some(prompt)) = (apply_plan.continue_request, apply_plan.continue_prompt)
    {
        crate::app_runtime::continue_current_note_follow_up(
            &app.runtime.agents,
            &mut app.runtime.incoming_prompts,
            &app.runtime.watchdog_state,
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
fn current_local_instance_id(app: &App) -> String {
    app.runtime
        .local_instance_id
        .clone()
        .unwrap_or_else(crate::instance_id::derive_local_instance_id)
}

#[cfg(not(feature = "stylos"))]
#[allow(dead_code)]
fn current_local_instance_id(_app: &App) -> String {
    crate::instance_id::derive_local_instance_id()
}

fn handle_watchdog_tick_local_event(app: &mut App) {
    let agent_statuses = crate::app_runtime::build_local_agent_status_entries_local(&app.runtime.agents);
    let mut candidate_ids = agent_statuses
        .iter()
        .filter(|h| h.roles.iter().any(|r| r == "interactive"))
        .map(|h| h.agent_id.clone())
        .collect::<Vec<_>>();
    candidate_ids.extend(
        agent_statuses
            .iter()
            .filter(|h| !h.roles.iter().any(|r| r == "interactive") && !h.roles.iter().any(|r| r == "master"))
            .map(|h| h.agent_id.clone()),
    );
    candidate_ids.extend(
        agent_statuses
            .iter()
            .filter(|h| h.roles.iter().any(|r| r == "master") && !h.roles.iter().any(|r| r == "interactive"))
            .map(|h| h.agent_id.clone()),
    );

    app.runtime.watchdog_no_pending_since_by_agent.retain(|agent_id, _| {
        agent_statuses.iter().any(|h| h.agent_id == *agent_id)
    });

    let local_instance = current_local_instance_id(app);
    for agent_id in candidate_ids {
        let Some(handle) = agent_statuses.iter().find(|h| h.agent_id == agent_id) else {
            continue;
        };
        if handle.busy || handle.has_active_incoming_prompt {
            continue;
        }
        if let Some(no_pending_since) = app.runtime.watchdog_no_pending_since_by_agent.get(&agent_id) {
            if (no_pending_since.elapsed().as_millis() as u64)
                < crate::board_runtime::WATCHDOG_NO_PENDING_COOLDOWN_MS_DEFAULT
            {
                continue;
            }
        }
        let Some(request) = crate::board_runtime::resolve_pending_board_note_injection(
            &app.runtime.db,
            &app.runtime.board_claims,
            &local_instance,
            &agent_id,
            crate::local_prompts::IncomingPromptSource::WatchdogBoardNote,
        ) else {
            app.runtime
                .watchdog_no_pending_since_by_agent
                .insert(agent_id.clone(), std::time::Instant::now());
            continue;
        };
        let Some(agent_index) = app.runtime.agents.iter().position(|h| h.agent_id == agent_id) else {
            if let Some(note_id) = crate::board_runtime::board_note_id_from_prompt(&request.prompt) {
                crate::board_runtime::release_board_note_claim(&app.runtime.board_claims, note_id);
            }
            app.runtime
                .watchdog_no_pending_since_by_agent
                .insert(agent_id.clone(), std::time::Instant::now());
            continue;
        };
        app.runtime.watchdog_no_pending_since_by_agent.remove(&agent_id);
        app.push(crate::tui::Entry::Status {
            agent_id: Some(agent_id.clone()),
            source: Some(crate::tui::NonAgentSource::Runtime),
            text: format!(
                "watchdog asked agent {} to handle {}",
                agent_id,
                crate::app_runtime::stylos_note_display_identifier(&request.prompt)
            ),
        });
        submit_text_to_agent(app, agent_index, request.prompt.clone());
        return;
    }
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
        AppRuntimeEvent::WatchdogTick => handle_watchdog_tick_local_event(app),
        AppRuntimeEvent::Agent(sid, ev) => {
            app.runtime.activity_counters.agent_event_count += 1;
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

#[cfg(feature = "stylos")]
pub(crate) async fn start_tui_runtime_services(
    app_state: &mut AppState,
    runtime_tx: &mpsc::UnboundedSender<AppRuntimeEvent>,
) -> anyhow::Result<()> {
    let shared_status_hub = crate::app_runtime::SharedStylosStatusHub::new();
    let mut stylos = start_stylos(app_state, Some(shared_status_hub.clone())).await?;
    crate::app_runtime::wire_stylos_event_streams(&app_state.runtime_domains, &mut stylos, runtime_tx);
    app_state.runtime.shared_status_hub = shared_status_hub;
    app_state.runtime.stylos_tool_bridge = crate::stylos::tool_bridge(&stylos);
    app_state.runtime.local_instance_id = match stylos.state() {
        crate::stylos::StylosRuntimeState::Active { instance, .. } => Some(instance.clone()),
        _ => Some(crate::instance_id::derive_local_instance_id()),
    };
    if let Some(local_instance_id) = app_state.runtime.local_instance_id.clone() {
        for handle in &mut app_state.runtime.agents {
            if let Some(agent) = handle.agent.as_mut() {
                agent.set_local_instance_id(Some(local_instance_id.clone()));
            }
        }
    }
    app_state.runtime.stylos = Some(stylos);
    Ok(())
}


#[cfg(feature = "stylos")]
pub async fn start_stylos(
    app_state: &AppState,
    shared_status_hub: Option<crate::app_runtime::SharedStylosStatusHub>,
) -> anyhow::Result<crate::stylos::StylosHandle> {
    match app_state
        .runtime_domains
        .network()
        .spawn({
            let stylos_cfg = app_state.stylos_config.clone();
            let session = app_state.runtime.session.clone();
            let project_dir = app_state.runtime.project_dir.clone();
            let db = app_state.runtime.db.clone();
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

#[cfg_attr(not(feature = "stylos"), allow(dead_code))]
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
    local_role_agent_id: &str,
    local_role_label: &str,
    local_role_roles: Vec<String>,
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
    agent.set_local_agent_role_context(local_role_agent_id, local_role_label, local_role_roles);

    #[cfg(feature = "stylos")]
    {
        agent.set_stylos_tool_invoker(crate::app_runtime::stylos_tool_invoker(stylos_tool_bridge, local_agent_id));
        agent.set_local_instance_id(local_instance_id.map(str::to_string));
        agent.set_local_agent_id(Some(local_agent_id.to_string()));
    }

    Ok(agent)
}
