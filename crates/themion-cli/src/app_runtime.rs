use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "stylos")]
use std::time::Duration;

#[cfg(feature = "stylos")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use themion_core::agent::{Agent, TurnCancellation};
use themion_core::client_codex::ApiCallRateLimitReport;
use themion_core::db::DbHandle;
use themion_core::tools::{
    SystemInspectionProvider, SystemInspectionRateLimits, SystemInspectionResult,
    SystemInspectionRuntime, SystemInspectionTaskRuntime, SystemInspectionTools,
};
use themion_core::workflow::WorkflowState;
use themion_core::ModelInfo;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::tui::AppEvent;
#[cfg(feature = "stylos")]
use crate::board_runtime::{
    board_note_id_from_prompt, release_board_note_claim, LocalBoardClaimRegistry,
    WATCHDOG_IDLE_DELAY_MS_DEFAULT,
};
use crate::app_state::{AppRuntimeEvent, AppSnapshot, AppSnapshotAgent, AppSnapshotHub};
use crate::config::save_profiles;
use crate::runtime_domains::DomainHandle;
use crate::Session;

#[cfg(feature = "stylos")]
use crate::stylos::StylosStatusSnapshot;

#[cfg(feature = "stylos")]
pub(crate) type StylosSnapshotFuture = std::pin::Pin<Box<dyn std::future::Future<Output = StylosStatusSnapshot> + Send>>;

#[cfg(feature = "stylos")]
pub(crate) type StylosSnapshotProvider = std::sync::Arc<dyn Fn() -> StylosSnapshotFuture + Send + Sync>;

#[cfg(feature = "stylos")]
#[derive(Clone, Debug, Default)]
pub(crate) struct StylosActivitySnapshot {
    pub status_publish_count: u64,
    pub status_publish_total_us: u64,
    pub status_publish_max_us: u64,
    pub query_request_count: u64,
    pub query_request_total_us: u64,
    pub query_request_max_us: u64,
    pub cmd_event_count: u64,
    pub prompt_event_count: u64,
    pub event_message_count: u64,
}

#[cfg(feature = "stylos")]
#[derive(Clone, Debug)]
pub(crate) struct SharedStylosStatusHub {
    snapshot: std::sync::Arc<tokio::sync::RwLock<StylosStatusSnapshot>>,
}

#[cfg(feature = "stylos")]
impl SharedStylosStatusHub {
    pub(crate) fn new() -> Self {
        Self {
            snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(StylosStatusSnapshot {
                startup_project_dir: String::new(),
                agents: Vec::new(),
            })),
        }
    }

    pub(crate) fn provider(&self) -> StylosSnapshotProvider {
        let snapshot = self.snapshot.clone();
        std::sync::Arc::new(move || {
            let snapshot = snapshot.clone();
            Box::pin(async move { snapshot.read().await.clone() })
        })
    }

    // TODO(runtime-ownership): replace with runtime-owned publisher once the TUI rewrite is reconnected.
    #[allow(dead_code)]
    pub(crate) async fn replace_snapshot(&self, next: StylosStatusSnapshot) {
        *self.snapshot.write().await = next;
    }
}

#[cfg(feature = "stylos")]
impl Default for SharedStylosStatusHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "stylos")]
pub(crate) fn stylos_tool_invoker(
    bridge: Option<crate::stylos::StylosToolBridge>,
) -> Option<themion_core::tools::StylosToolInvoker> {
    bridge.map(|bridge| {
        std::sync::Arc::new(move |name: String, args: serde_json::Value| {
            let bridge = bridge.clone();
            let fut: std::pin::Pin<
                Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>,
            > = Box::pin(async move { bridge.invoke(None, &name, args).await });
            fut
        }) as themion_core::tools::StylosToolInvoker
    })
}

#[derive(Debug)]
pub(crate) struct LocalAgentManagementRequest {
    pub(crate) action: String,
    pub(crate) args: serde_json::Value,
    pub(crate) reply_tx: tokio::sync::oneshot::Sender<anyhow::Result<String>>,
}

pub(crate) fn build_local_agent_tool_invoker(
    app_tx: mpsc::UnboundedSender<AppEvent>,
) -> themion_core::tools::LocalAgentToolInvoker {
    std::sync::Arc::new(move |name: String, args: serde_json::Value| {
        let app_tx = app_tx.clone();
        let fut: std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>,
        > = Box::pin(async move {
            let (reply_tx, reply_rx) = oneshot::channel();
            app_tx
                .send(AppEvent::LocalAgentManagement(
                    LocalAgentManagementRequest {
                        action: name,
                        args,
                        reply_tx,
                    },
                ))
                .map_err(|_| anyhow::anyhow!("local agent management queue unavailable"))?;
            reply_rx
                .await
                .map_err(|_| anyhow::anyhow!("local agent management reply unavailable"))?
        });
        fut
    })
}

#[cfg(feature = "stylos")]
#[derive(Clone, Debug)]
pub(crate) struct LocalAgentStatusEntry {
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
    pub session_id: String,
    pub workflow: Option<WorkflowState>,
    pub project_dir: Option<PathBuf>,
    pub busy: bool,
    pub has_active_incoming_prompt: bool,
}

#[cfg(feature = "stylos")]
pub(crate) fn local_agent_status_entry(
    agent_id: impl Into<String>,
    label: impl Into<String>,
    roles: impl IntoIterator<Item = impl Into<String>>,
    session_id: impl Into<String>,
    workflow: Option<WorkflowState>,
    project_dir: Option<PathBuf>,
    busy: bool,
    has_active_incoming_prompt: bool,
) -> LocalAgentStatusEntry {
    LocalAgentStatusEntry {
        agent_id: agent_id.into(),
        label: label.into(),
        roles: roles.into_iter().map(Into::into).collect(),
        session_id: session_id.into(),
        workflow,
        project_dir,
        busy,
        has_active_incoming_prompt,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalAgentRosterEntry {
    pub agent_id: String,
    pub roles: Vec<String>,
}

pub(crate) fn roster_entry(agent_id: impl Into<String>, roles: impl IntoIterator<Item = impl Into<String>>) -> LocalAgentRosterEntry {
    LocalAgentRosterEntry {
        agent_id: agent_id.into(),
        roles: roles.into_iter().map(Into::into).collect(),
    }
}

pub(crate) struct AgentTurnRuntimeLaunch {
    pub submit_setup: AgentTurnSubmitSetup,
    pub event_rx: mpsc::UnboundedReceiver<themion_core::agent::AgentEvent>,
    pub agent: Agent,
}

pub(crate) struct AgentTurnSubmitSetup {
    pub cancellation: TurnCancellation,
    pub handle_session_id: Uuid,
}

pub(crate) fn prepare_agent_turn_runtime_launch(
    agents: &mut [crate::tui::AgentHandle],
    agent_index: usize,
) -> AgentTurnRuntimeLaunch {
    let submit_setup = prepare_agent_turn_submit(agents, agent_index);
    let (event_tx, event_rx) = mpsc::unbounded_channel::<themion_core::agent::AgentEvent>();
    let agent = prepare_agent_turn_execution(agents, agent_index, event_tx);
    AgentTurnRuntimeLaunch {
        submit_setup,
        event_rx,
        agent,
    }
}

pub(crate) fn prepare_agent_turn_submit(
    agents: &mut [crate::tui::AgentHandle],
    agent_index: usize,
) -> AgentTurnSubmitSetup {
    let cancellation = TurnCancellation::new();
    let handle = agents.get_mut(agent_index).expect("agent index valid");
    handle.busy = true;
    handle.turn_cancellation = Some(cancellation.clone());
    AgentTurnSubmitSetup {
        cancellation,
        handle_session_id: handle.session_id,
    }
}

pub(crate) fn prepare_agent_turn_execution(
    agents: &mut [crate::tui::AgentHandle],
    agent_index: usize,
    event_tx: mpsc::UnboundedSender<themion_core::agent::AgentEvent>,
) -> Agent {
    let handle = agents.get_mut(agent_index).expect("agent index valid");
    let mut agent = handle.agent.take().expect("agent available when not busy");
    agent.set_event_tx(event_tx);
    agent
}

pub(crate) fn launch_agent_turn_runtime(
    background_domain: &DomainHandle,
    core_domain: &DomainHandle,
    runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
    text: String,
    runtime_launch: AgentTurnRuntimeLaunch,
) {
    spawn_agent_event_relay(
        background_domain,
        runtime_tx.clone(),
        runtime_launch.submit_setup.handle_session_id,
        runtime_launch.event_rx,
    );
    spawn_agent_turn_core_loop(
        core_domain,
        runtime_tx,
        runtime_launch.submit_setup.handle_session_id,
        text,
        runtime_launch.submit_setup.cancellation,
        runtime_launch.agent,
    );
}

pub(crate) fn spawn_agent_turn_core_loop(
    core_domain: &DomainHandle,
    runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
    handle_session_id: Uuid,
    text: String,
    cancellation: TurnCancellation,
    mut agent: Agent,
) {
    core_domain.spawn(async move {
        if let Err(e) = agent
            .run_loop_with_cancellation(&text, Some(cancellation.clone()))
            .await
        {
            let _ = runtime_tx.send(AppRuntimeEvent::Agent(
                handle_session_id,
                themion_core::agent::AgentEvent::AssistantText(format!("error: {e}")),
            ));
        }
        let _ = runtime_tx.send(AppRuntimeEvent::AgentReady(Box::new(agent), handle_session_id));
    });
}

pub(crate) fn spawn_agent_event_relay(
    background_domain: &DomainHandle,
    runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
    handle_session_id: Uuid,
    event_rx: mpsc::UnboundedReceiver<themion_core::agent::AgentEvent>,
) {
    background_domain.spawn(async move {
        let mut rx = event_rx;
        while let Some(ev) = rx.recv().await {
            let _ = runtime_tx.send(AppRuntimeEvent::Agent(handle_session_id, ev));
        }
    });
}

pub(crate) struct AppSnapshotBuildState<'a> {
    pub agents: &'a [crate::tui::AgentHandle],
    pub agent_busy: bool,
    pub activity_status: String,
    #[cfg(feature = "stylos")]
    pub stylos_status: Option<String>,
    #[cfg(feature = "stylos")]
    pub watchdog_state: &'a crate::app_runtime::WatchdogRuntimeState,
}

pub(crate) fn build_app_snapshot(
    latest: &AppSnapshot,
    state: AppSnapshotBuildState<'_>,
) -> AppSnapshot {
    AppSnapshot {
        primary_session_id: state
            .agents
            .first()
            .map(|h| h.session_id)
            .or(latest.primary_session_id),
        primary_agent_id: state
            .agents
            .first()
            .map(|h| h.agent_id.clone())
            .or_else(|| latest.primary_agent_id.clone()),
        busy: state.agent_busy,
        activity_status: Some(state.activity_status),
        local_agents: state
            .agents
            .iter()
            .map(|handle| AppSnapshotAgent {
                agent_id: handle.agent_id.clone(),
                label: handle.label.clone(),
                roles: handle.roles.clone(),
                busy: handle.busy,
                #[cfg(feature = "stylos")]
                incoming: handle.active_incoming_prompt.is_some(),
                #[cfg(not(feature = "stylos"))]
                incoming: false,
            })
            .collect(),
        #[cfg(feature = "stylos")]
        stylos_status: state.stylos_status,
        #[cfg(feature = "stylos")]
        pending_watchdog_note: state.watchdog_state.pending_watchdog_note(),
        #[cfg(feature = "stylos")]
        active_incoming_prompt_count: state
            .agents
            .iter()
            .filter(|handle| handle.active_incoming_prompt.is_some())
            .count(),
        #[cfg(feature = "stylos")]
        aggregate_busy_agents: state.agents.iter().any(|handle| handle.busy),
    }
}

pub(crate) struct AppSnapshotPublisher {
    snapshot_hub: AppSnapshotHub,
    latest: AppSnapshot,
}

impl AppSnapshotPublisher {
    pub(crate) fn new(snapshot_hub: AppSnapshotHub) -> Self {
        let latest = snapshot_hub.current();
        Self {
            snapshot_hub,
            latest,
        }
    }

    pub(crate) fn publish(&mut self, state: AppSnapshotBuildState<'_>) {
        let snapshot = build_app_snapshot(&self.latest, state);
        self.latest = snapshot.clone();
        self.snapshot_hub.publish(snapshot);
    }
}

pub(crate) struct AppRuntimeObserverPublisher {
    snapshot_publisher: AppSnapshotPublisher,
}

impl AppRuntimeObserverPublisher {
    pub(crate) fn new(snapshot_publisher: AppSnapshotPublisher) -> Self {
        Self { snapshot_publisher }
    }

    pub(crate) fn publish(&mut self, state: AppRuntimeObserverPublishState<'_>) {
        let AppRuntimeObserverPublishState {
            agents,
            snapshot,
            system_inspection,
            #[cfg(feature = "stylos")]
            stylos,
        } = state;

        self.snapshot_publisher.publish(AppSnapshotBuildState {
            agents: &*agents,
            agent_busy: snapshot.agent_busy,
            activity_status: snapshot.activity_status,
            #[cfg(feature = "stylos")]
            stylos_status: snapshot.stylos_status,
            #[cfg(feature = "stylos")]
            watchdog_state: snapshot.watchdog_state,
        });

        refresh_interactive_agent_system_inspection_from_runtime(agents, system_inspection);

        #[cfg(feature = "stylos")]
        if let Some(stylos) = stylos {
            refresh_stylos_status_snapshot(
                stylos.hub,
                StylosAppStatusRefreshState {
                    startup_project_dir: stylos.startup_project_dir,
                    fallback_project_dir: stylos.fallback_project_dir,
                    provider: stylos.provider,
                    model: stylos.model,
                    active_profile: stylos.active_profile,
                    rate_limits: stylos.rate_limits,
                    idle_since: stylos.idle_since,
                    idle_status_changed_at: stylos.idle_status_changed_at,
                    primary_activity_label: stylos.primary_activity_label,
                    primary_activity_changed_at_ms: stylos.primary_activity_changed_at_ms,
                    primary_workflow: stylos.primary_workflow,
                    agents: &*agents,
                },
            );
        }
    }
}

pub(crate) struct AppRuntimeObserverPublishState<'a> {
    pub agents: &'a mut [crate::tui::AgentHandle],
    pub snapshot: AppRuntimeSnapshotPublishState<'a>,
    pub system_inspection: SystemInspectionRuntimeRefreshState<'a>,
    #[cfg(feature = "stylos")]
    pub stylos: Option<StylosRuntimeStatusPublishState<'a>>,
}

pub(crate) struct AppRuntimeSnapshotPublishState<'a> {
    pub agent_busy: bool,
    pub activity_status: String,
    #[cfg(feature = "stylos")]
    pub stylos_status: Option<String>,
    #[cfg(feature = "stylos")]
    pub watchdog_state: &'a crate::app_runtime::WatchdogRuntimeState,
    #[cfg(not(feature = "stylos"))]
    _marker: std::marker::PhantomData<&'a ()>,
}

#[cfg(not(feature = "stylos"))]
impl<'a> AppRuntimeSnapshotPublishState<'a> {
    pub(crate) fn new(agent_busy: bool, activity_status: String) -> Self {
        Self {
            agent_busy,
            activity_status,
            _marker: std::marker::PhantomData,
        }
    }
}

#[cfg(feature = "stylos")]
pub(crate) struct StylosRuntimeStatusPublishState<'a> {
    pub hub: &'a SharedStylosStatusHub,
    pub startup_project_dir: &'a std::path::Path,
    pub fallback_project_dir: &'a std::path::Path,
    pub provider: &'a str,
    pub model: &'a str,
    pub active_profile: &'a str,
    pub rate_limits: Option<&'a ApiCallRateLimitReport>,
    pub idle_since: Option<std::time::Instant>,
    pub idle_status_changed_at: Option<u64>,
    pub primary_activity_label: Option<String>,
    pub primary_activity_changed_at_ms: Option<u64>,
    pub primary_workflow: &'a WorkflowState,
}

pub(crate) fn build_main_agent(
    session: &Session,
    db: Arc<DbHandle>,
    session_id: Uuid,
    project_dir: PathBuf,
    local_agent_mgmt_tx: mpsc::UnboundedSender<AppEvent>,
    #[cfg(feature = "stylos")] stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
    #[cfg(feature = "stylos")] local_stylos_instance: Option<&str>,
    #[cfg(feature = "stylos")] local_agent_id: &str,
    system_inspection: Option<SystemInspectionResult>,
    api_log_enabled: bool,
) -> anyhow::Result<Agent> {
    crate::app_state::build_agent(
        session,
        session_id,
        project_dir,
        db,
        #[cfg(feature = "stylos")]
        stylos_tool_bridge,
        #[cfg(feature = "stylos")]
        local_stylos_instance,
        #[cfg(feature = "stylos")]
        local_agent_id,
        Some(build_local_agent_tool_invoker(local_agent_mgmt_tx)),
        system_inspection,
        api_log_enabled,
    )
}


#[derive(Clone)]
pub(crate) enum RuntimeCommand {
    LoginCodex { profile_name: Option<String> },
    SemanticMemoryIndex { full: bool },
    SessionProfileUse { name: String },
    SessionModelUse { model: String },
    SessionReset,
    ConfigProfileUse { name: String },
    ConfigProfileCreate { name: String },
    ConfigProfileSet { key: String, value: String },
    SetApiLogEnabled { enabled: bool },
    ClearContext,
}


pub(crate) enum RuntimeCommandOutcome {
    Noop,
    Lines(Vec<String>),
    ReplaceMasterAgent {
        new_agent: Agent,
        new_session_id: Uuid,
        output_lines: Vec<String>,
    },
    SetInteractiveApiLogEnabled { enabled: bool, output_lines: Vec<String> },
    ClearInteractiveContext { output_lines: Vec<String> },
}

pub(crate) struct RuntimeCommandContext<'a> {
    pub session: &'a mut Session,
    pub project_dir: &'a PathBuf,
    pub db: &'a Arc<DbHandle>,
    #[cfg(feature = "stylos")]
    pub stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
    #[cfg(feature = "stylos")]
    pub local_stylos_instance: Option<&'a str>,
    pub api_log_enabled: bool,
    pub local_agent_mgmt_tx: mpsc::UnboundedSender<AppEvent>,
}

pub(crate) fn execute_runtime_command(
    command: RuntimeCommand,
    context: RuntimeCommandContext<'_>,
) -> RuntimeCommandOutcome {
    match command {
        RuntimeCommand::LoginCodex { .. } | RuntimeCommand::SemanticMemoryIndex { .. } => {
            RuntimeCommandOutcome::Noop
        }
        RuntimeCommand::SessionProfileUse { name } => {
            let cleared_model_override = context.session.temporary_model_override.is_some();
            if context.session.switch_profile_temporarily(&name) {
                match build_replacement_main_agent(AgentReplacementParams {
                    session: context.session,
                    project_dir: context.project_dir,
                    db: context.db,
                    #[cfg(feature = "stylos")]
                    stylos_tool_bridge: context.stylos_tool_bridge,
                    #[cfg(feature = "stylos")]
                    local_stylos_instance: context.local_stylos_instance,
                    api_log_enabled: context.api_log_enabled,
                    local_agent_mgmt_tx: context.local_agent_mgmt_tx,
                    insert_session: true,
                }) {
                    Ok((new_agent, new_session_id)) => RuntimeCommandOutcome::ReplaceMasterAgent {
                        new_agent,
                        new_session_id,
                        output_lines: vec![if cleared_model_override {
                            format!(
                                "temporarily switched to profile '{}' for this session only; cleared temporary model override and reset to profile model  provider={}  model={}",
                                name, context.session.provider, context.session.model
                            )
                        } else {
                            format!(
                                "temporarily switched to profile '{}' for this session only  provider={}  model={}",
                                name, context.session.provider, context.session.model
                            )
                        }],
                    },
                    Err(e) => RuntimeCommandOutcome::Lines(vec![format!(
                        "error building agent: {}",
                        e
                    )]),
                }
            } else {
                let mut names: Vec<String> = context.session.profiles.keys().cloned().collect();
                names.sort();
                RuntimeCommandOutcome::Lines(vec![format!(
                    "unknown profile '{}'.  available: {}",
                    name,
                    names.join(", ")
                )])
            }
        }
        RuntimeCommand::SessionModelUse { model } => {
            context.session.set_temporary_model_override(&model);
            match build_replacement_main_agent(AgentReplacementParams {
                session: context.session,
                project_dir: context.project_dir,
                db: context.db,
                #[cfg(feature = "stylos")]
                stylos_tool_bridge: context.stylos_tool_bridge,
                #[cfg(feature = "stylos")]
                local_stylos_instance: context.local_stylos_instance,
                api_log_enabled: context.api_log_enabled,
                local_agent_mgmt_tx: context.local_agent_mgmt_tx,
                insert_session: true,
            }) {
                Ok((new_agent, new_session_id)) => RuntimeCommandOutcome::ReplaceMasterAgent {
                    new_agent,
                    new_session_id,
                    output_lines: vec![format!(
                        "temporarily using model '{}' for this session only",
                        context.session.model
                    )],
                },
                Err(e) => RuntimeCommandOutcome::Lines(vec![format!(
                    "error building agent: {}",
                    e
                )]),
            }
        }
        RuntimeCommand::SessionReset => {
            if context.session.clear_temporary_overrides() {
                match build_replacement_main_agent(AgentReplacementParams {
                    session: context.session,
                    project_dir: context.project_dir,
                    db: context.db,
                    #[cfg(feature = "stylos")]
                    stylos_tool_bridge: context.stylos_tool_bridge,
                    #[cfg(feature = "stylos")]
                    local_stylos_instance: context.local_stylos_instance,
                    api_log_enabled: context.api_log_enabled,
                    local_agent_mgmt_tx: context.local_agent_mgmt_tx,
                    insert_session: true,
                }) {
                    Ok((new_agent, new_session_id)) => RuntimeCommandOutcome::ReplaceMasterAgent {
                        new_agent,
                        new_session_id,
                        output_lines: vec![format!(
                            "cleared temporary session overrides; back to configured profile '{}'  provider={}  model={}",
                            context.session.active_profile,
                            context.session.provider,
                            context.session.model
                        )],
                    },
                    Err(e) => RuntimeCommandOutcome::Lines(vec![format!(
                        "error building agent: {}",
                        e
                    )]),
                }
            } else {
                RuntimeCommandOutcome::Lines(vec![
                    "no temporary session override is active".to_string(),
                ])
            }
        }
        RuntimeCommand::ConfigProfileCreate { name } => {
            let profile = crate::config::ProfileConfig {
                provider: Some(context.session.provider.clone()),
                base_url: Some(context.session.base_url.clone()),
                model: Some(context.session.model.clone()),
                api_key: context.session.api_key.clone(),
            };
            context.session.profiles.insert(name.clone(), profile);
            context.session.active_profile = name.clone();
            let mut lines = Vec::new();
            if let Err(e) = save_profiles(&context.session.active_profile, &context.session.profiles)
            {
                lines.push(format!("warning: {}", e));
            }
            lines.push(format!("profile '{}' created and saved", name));
            RuntimeCommandOutcome::Lines(lines)
        }
        RuntimeCommand::ConfigProfileSet { key, value } => {
            match key.as_str() {
                "provider" => context.session.provider = value.clone(),
                "model" => context.session.model = value.clone(),
                "endpoint" => context.session.base_url = value.clone(),
                "api_key" => context.session.api_key = Some(value.clone()),
                _ => {
                    return RuntimeCommandOutcome::Lines(vec![format!(
                        "unknown key '{}'.  valid: provider, model, endpoint, api_key",
                        key
                    )]);
                }
            }
            context.session.profiles.insert(
                context.session.active_profile.clone(),
                crate::config::ProfileConfig {
                    provider: Some(context.session.provider.clone()),
                    base_url: Some(context.session.base_url.clone()),
                    model: Some(context.session.model.clone()),
                    api_key: context.session.api_key.clone(),
                },
            );
            let mut lines = Vec::new();
            if let Err(e) = save_profiles(&context.session.active_profile, &context.session.profiles)
            {
                lines.push(format!("warning: {}", e));
            }
            lines.push(format!(
                "{}={} saved",
                key,
                if key == "api_key" { "(set)" } else { value.as_str() }
            ));
            RuntimeCommandOutcome::Lines(lines)
        }
        RuntimeCommand::SetApiLogEnabled { enabled } => {
            RuntimeCommandOutcome::SetInteractiveApiLogEnabled {
                enabled,
                output_lines: vec![if enabled {
                    "API call logging enabled for this session".to_string()
                } else {
                    "API call logging disabled for this session".to_string()
                }],
            }
        }
        RuntimeCommand::ClearContext => RuntimeCommandOutcome::ClearInteractiveContext {
            output_lines: vec![
                "ok, future messages in this session will not include chat history before this point"
                    .to_string(),
            ],
        },
        RuntimeCommand::ConfigProfileUse { name } => {
            if context.session.switch_profile(&name) {
                let mut lines = Vec::new();
                if let Err(e) = save_profiles(&context.session.active_profile, &context.session.profiles) {
                    lines.push(format!("warning: {}", e));
                }
                match build_replacement_main_agent(AgentReplacementParams {
                    session: context.session,
                    project_dir: context.project_dir,
                    db: context.db,
                    #[cfg(feature = "stylos")]
                    stylos_tool_bridge: context.stylos_tool_bridge,
                    #[cfg(feature = "stylos")]
                    local_stylos_instance: context.local_stylos_instance,
                    api_log_enabled: context.api_log_enabled,
                    local_agent_mgmt_tx: context.local_agent_mgmt_tx,
                    insert_session: true,
                }) {
                    Ok((new_agent, new_session_id)) => {
                        lines.push(format!(
                            "switched to profile '{}'  provider={}  model={}",
                            name, context.session.provider, context.session.model
                        ));
                        RuntimeCommandOutcome::ReplaceMasterAgent {
                            new_agent,
                            new_session_id,
                            output_lines: lines,
                        }
                    }
                    Err(e) => {
                        lines.push(format!("error building agent: {}", e));
                        RuntimeCommandOutcome::Lines(lines)
                    }
                }
            } else {
                let mut names: Vec<String> = context.session.profiles.keys().cloned().collect();
                names.sort();
                RuntimeCommandOutcome::Lines(vec![format!(
                    "unknown profile '{}'.  available: {}",
                    name,
                    names.join(", ")
                )])
            }
        }
    }
}


pub(crate) struct RuntimeCommandApplication {
    pub output_lines: Vec<String>,
    pub had_effect: bool,
}

pub(crate) fn apply_runtime_command_outcome_to_app_runtime(
    agents: &mut Vec<crate::tui::AgentHandle>,
    status_model_info: &mut Option<ModelInfo>,
    workflow_state: &mut WorkflowState,
    api_log_enabled: &mut bool,
    last_ctx_tokens: &mut u64,
    outcome: RuntimeCommandOutcome,
) -> RuntimeCommandApplication {
    match outcome {
        RuntimeCommandOutcome::Noop => RuntimeCommandApplication {
            output_lines: Vec::new(),
            had_effect: false,
        },
        RuntimeCommandOutcome::Lines(output_lines) => RuntimeCommandApplication {
            output_lines,
            had_effect: true,
        },
        RuntimeCommandOutcome::ReplaceMasterAgent {
            new_agent,
            new_session_id,
            output_lines,
        } => {
            apply_master_agent_replacement(
                agents,
                status_model_info,
                workflow_state,
                new_agent,
                new_session_id,
            );
            RuntimeCommandApplication {
                output_lines,
                had_effect: true,
            }
        }
        RuntimeCommandOutcome::SetInteractiveApiLogEnabled {
            enabled,
            output_lines,
        } => {
            *api_log_enabled = enabled;
            if let Some(handle) = agents
                .iter_mut()
                .find(|h| h.roles.iter().any(|role| role == "interactive"))
            {
                if let Some(agent) = handle.agent.as_mut() {
                    agent.set_api_log_enabled(enabled);
                }
            }
            RuntimeCommandApplication {
                output_lines,
                had_effect: true,
            }
        }
        RuntimeCommandOutcome::ClearInteractiveContext { output_lines } => {
            if let Some(handle) = agents
                .iter_mut()
                .find(|h| h.roles.iter().any(|role| role == "interactive"))
            {
                if let Some(agent) = handle.agent.as_mut() {
                    agent.clear_context();
                }
            }
            *last_ctx_tokens = 0;
            RuntimeCommandApplication {
                output_lines,
                had_effect: true,
            }
        }
    }
}


pub(crate) fn current_activity_label(activity: Option<&crate::app_state::AgentActivity>) -> Option<String> {
    activity.map(|activity| match activity {
        crate::app_state::AgentActivity::PreparingRequest => "preparing_request".to_string(),
        crate::app_state::AgentActivity::WaitingForModel => "waiting_for_model".to_string(),
        crate::app_state::AgentActivity::StreamingResponse => "streaming_response".to_string(),
        crate::app_state::AgentActivity::RunningTool(_) => "running_tool".to_string(),
        crate::app_state::AgentActivity::WaitingAfterTool => "waiting_after_tool".to_string(),
        crate::app_state::AgentActivity::LoginStarting => "login_starting".to_string(),
        crate::app_state::AgentActivity::WaitingForLoginBrowser => "waiting_for_login_browser".to_string(),
        crate::app_state::AgentActivity::RunningShellCommand => "running_shell_command".to_string(),
        crate::app_state::AgentActivity::Finishing => "finishing".to_string(),
    })
}

pub(crate) fn current_activity_detail(
    activity: Option<&crate::app_state::AgentActivity>,
    stream_chunks: u64,
    stream_chars: u64,
) -> Option<String> {
    activity.map(|activity| activity.label(stream_chunks, stream_chars))
}

pub(crate) fn build_task_runtime_snapshot(
    activity: Option<&crate::app_state::AgentActivity>,
    stream_chunks: u64,
    stream_chars: u64,
    agent_busy: bool,
    activity_status: String,
    activity_status_changed_at_ms: Option<u64>,
    process_started_at_ms: u64,
    uptime_ms: u64,
    recent_window_ms: Option<u64>,
) -> SystemInspectionTaskRuntime {
    let mut runtime_notes = vec![
        "task metrics are Themion activity counters and approximate handler timing, not exact Tokio task CPU percentages".to_string(),
    ];
    if recent_window_ms.is_none() {
        runtime_notes.push(
            "recent task runtime window unavailable until more than one snapshot is recorded"
                .to_string(),
        );
    }
    SystemInspectionTaskRuntime {
        status: if recent_window_ms.is_some() { "ok" } else { "partial" }.to_string(),
        current_activity: current_activity_label(activity),
        current_activity_detail: current_activity_detail(activity, stream_chunks, stream_chars),
        busy: Some(agent_busy),
        activity_status: Some(activity_status),
        activity_status_changed_at_ms,
        process_started_at_ms: Some(process_started_at_ms),
        uptime_ms: Some(uptime_ms),
        recent_window_ms,
        runtime_notes,
    }
}


pub(crate) struct SystemInspectionRuntimeRefreshState<'a> {
    pub session: &'a Session,
    pub project_dir: &'a std::path::Path,
    pub workflow_state: &'a WorkflowState,
    pub rate_limits: Option<&'a ApiCallRateLimitReport>,
    pub activity: Option<&'a crate::app_state::AgentActivity>,
    pub stream_chunks: u64,
    pub stream_chars: u64,
    pub agent_busy: bool,
    pub activity_status: String,
    pub activity_status_changed_at_ms: Option<u64>,
    pub process_started_at_ms: u64,
    pub uptime_ms: u64,
    pub recent_window_ms: Option<u64>,
    pub debug_runtime_lines: Vec<String>,
}

pub(crate) fn refresh_interactive_agent_system_inspection_from_runtime(
    agents: &mut [crate::tui::AgentHandle],
    state: SystemInspectionRuntimeRefreshState<'_>,
) {
    let interactive_session_id = agents.first().map(|h| h.session_id);
    let input = build_system_inspection_refresh_input(SystemInspectionRefreshState {
        session: state.session,
        fallback_session_id: state.session.id,
        interactive_session_id,
        project_dir: state.project_dir,
        workflow_state: state.workflow_state,
        rate_limits: state.rate_limits,
        activity: state.activity,
        stream_chunks: state.stream_chunks,
        stream_chars: state.stream_chars,
        agent_busy: state.agent_busy,
        activity_status: state.activity_status,
        activity_status_changed_at_ms: state.activity_status_changed_at_ms,
        process_started_at_ms: state.process_started_at_ms,
        uptime_ms: state.uptime_ms,
        recent_window_ms: state.recent_window_ms,
        debug_runtime_lines: state.debug_runtime_lines,
    });
    refresh_interactive_agent_system_inspection(agents, input);
}

pub(crate) struct SystemInspectionRefreshState<'a> {
    pub session: &'a Session,
    pub fallback_session_id: Uuid,
    pub interactive_session_id: Option<Uuid>,
    pub project_dir: &'a std::path::Path,
    pub workflow_state: &'a WorkflowState,
    pub rate_limits: Option<&'a ApiCallRateLimitReport>,
    pub activity: Option<&'a crate::app_state::AgentActivity>,
    pub stream_chunks: u64,
    pub stream_chars: u64,
    pub agent_busy: bool,
    pub activity_status: String,
    pub activity_status_changed_at_ms: Option<u64>,
    pub process_started_at_ms: u64,
    pub uptime_ms: u64,
    pub recent_window_ms: Option<u64>,
    pub debug_runtime_lines: Vec<String>,
}

pub(crate) fn build_system_inspection_refresh_input(
    state: SystemInspectionRefreshState<'_>,
) -> SystemInspectionRefreshInput {
    SystemInspectionRefreshInput {
        session: state.session.clone(),
        fallback_session_id: state.fallback_session_id,
        interactive_session_id: state.interactive_session_id,
        project_dir: state.project_dir.to_path_buf(),
        workflow_state: state.workflow_state.clone(),
        rate_limits: state.rate_limits.cloned(),
        task_runtime: build_task_runtime_snapshot(
            state.activity,
            state.stream_chunks,
            state.stream_chars,
            state.agent_busy,
            state.activity_status,
            state.activity_status_changed_at_ms,
            state.process_started_at_ms,
            state.uptime_ms,
            state.recent_window_ms,
        ),
        debug_runtime_lines: state.debug_runtime_lines,
    }
}

pub(crate) struct SystemInspectionRefreshInput {
    pub session: Session,
    pub fallback_session_id: Uuid,
    pub interactive_session_id: Option<Uuid>,
    pub project_dir: PathBuf,
    pub workflow_state: WorkflowState,
    pub rate_limits: Option<ApiCallRateLimitReport>,
    pub task_runtime: SystemInspectionTaskRuntime,
    pub debug_runtime_lines: Vec<String>,
}


pub(crate) fn build_master_agent_handle(
    new_agent: Agent,
    new_session_id: Uuid,
) -> crate::tui::AgentHandle {
    crate::tui::AgentHandle {
        agent: Some(new_agent),
        session_id: new_session_id,
        agent_id: "master".to_string(),
        label: "master".to_string(),
        roles: vec!["master".to_string(), "interactive".to_string()],
        busy: false,
        turn_cancellation: None,
        #[cfg(feature = "stylos")]
        active_incoming_prompt: None,
    }
}

pub(crate) fn replace_master_agent_handle(
    agents: &mut Vec<crate::tui::AgentHandle>,
    replacement: crate::tui::AgentHandle,
) {
    let mut replacement = Some(replacement);
    let mut retained = agents
        .drain(..)
        .filter(|handle| !handle.roles.iter().any(|role| role == "master"))
        .collect::<Vec<_>>();
    let mut next_agents = Vec::with_capacity(retained.len() + 1);
    next_agents.push(replacement.take().expect("replacement present"));
    next_agents.append(&mut retained);
    *agents = next_agents;
}

pub(crate) fn apply_master_agent_replacement(
    agents: &mut Vec<crate::tui::AgentHandle>,
    status_model_info: &mut Option<ModelInfo>,
    workflow_state: &mut WorkflowState,
    new_agent: Agent,
    new_session_id: Uuid,
) {
    *status_model_info = new_agent.model_info().cloned();
    *workflow_state = new_agent.workflow_state().clone();
    replace_master_agent_handle(agents, build_master_agent_handle(new_agent, new_session_id));
}

pub(crate) fn apply_agent_ready_update(
    agents: &mut [crate::tui::AgentHandle],
    status_model_info: &mut Option<ModelInfo>,
    workflow_state: &mut WorkflowState,
    sid: Uuid,
    agent: Agent,
    #[cfg(feature = "stylos")] watchdog_state: &Arc<WatchdogRuntimeState>,
) {
    *status_model_info = agent.model_info().cloned();
    *workflow_state = agent.workflow_state().clone();
    if let Some(handle) = agents.iter_mut().find(|h| h.session_id == sid) {
        handle.agent = Some(agent);
        handle.busy = false;
        handle.turn_cancellation = None;
    }
    #[cfg(feature = "stylos")]
    sync_watchdog_runtime_state(watchdog_state, agents);
}

pub(crate) fn apply_system_inspection_to_interactive_agent(
    agents: &mut [crate::tui::AgentHandle],
    inspection: SystemInspectionResult,
) {
    if let Some(handle) = agents.iter_mut().find(|h| h.roles.iter().any(|r| r == "interactive")) {
        if let Some(agent) = handle.agent.as_mut() {
            agent.set_system_inspection(Some(inspection));
        }
    }
}

pub(crate) fn refresh_interactive_agent_system_inspection(
    agents: &mut [crate::tui::AgentHandle],
    input: SystemInspectionRefreshInput,
) {
    let inspection = build_system_inspection_snapshot(
        &input.session,
        input.fallback_session_id,
        input.interactive_session_id,
        &input.project_dir,
        &input.workflow_state,
        input.rate_limits.as_ref(),
        input.task_runtime,
        input.debug_runtime_lines,
    );
    apply_system_inspection_to_interactive_agent(agents, inspection);
}

pub(crate) fn build_system_inspection_snapshot(
    session: &Session,
    fallback_session_id: Uuid,
    interactive_session_id: Option<Uuid>,
    project_dir: &std::path::Path,
    workflow_state: &WorkflowState,
    rate_limits: Option<&ApiCallRateLimitReport>,
    task_runtime: SystemInspectionTaskRuntime,
    debug_runtime_lines: Vec<String>,
) -> SystemInspectionResult {
    let rate_limits = rate_limits.map(|report| SystemInspectionRateLimits {
        api_call: report.api_call.clone(),
        source: report.source.clone(),
        http_status: report.http_status,
        active_limit: report.active_limit.clone(),
        snapshot_count: report.snapshots.len(),
    });
    let mut provider = SystemInspectionProvider {
        status: "ok".to_string(),
        active_profile: Some(session.active_profile.clone()),
        provider: Some(session.provider.clone()),
        model: Some(session.model.clone()),
        auth_configured: Some(match session.provider.as_str() {
            "openai-codex" => crate::app_state::resolve_codex_auth(session).ok().flatten().is_some(),
            _ => session
                .api_key
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false),
        }),
        base_url_present: Some(!session.base_url.trim().is_empty()),
        rate_limits,
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
        now_ms: unix_epoch_now_ms(),
        session_id: interactive_session_id
            .unwrap_or(fallback_session_id)
            .to_string(),
        project_dir: project_dir.display().to_string(),
        workflow_name: Some(workflow_state.workflow_name.clone()),
        phase_name: Some(workflow_state.phase_name.clone()),
        workflow_status: Some(format!("{:?}", workflow_state.status)),
        debug_runtime_lines,
        task_runtime: Some(task_runtime),
        warnings: Vec::new(),
        issues: Vec::new(),
    };
    let mut warnings = Vec::new();
    let issues = provider.issues.clone();
    if provider.status != "ok" {
        warnings.push("provider readiness is degraded".to_string());
    }
    let overall_status = if issues.is_empty() { "ok" } else { "degraded" }.to_string();
    let summary = if overall_status == "ok" {
        "local inspection snapshot available, including /debug runtime coverage".to_string()
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

pub(crate) struct AgentReplacementParams<'a> {
    pub local_agent_mgmt_tx: mpsc::UnboundedSender<AppEvent>,
    pub session: &'a Session,
    pub project_dir: &'a PathBuf,
    pub db: &'a Arc<DbHandle>,
    #[cfg(feature = "stylos")]
    pub stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
    #[cfg(feature = "stylos")]
    pub local_stylos_instance: Option<&'a str>,
    pub api_log_enabled: bool,
    pub insert_session: bool,
}

pub(crate) fn build_replacement_main_agent(
    params: AgentReplacementParams<'_>,
) -> anyhow::Result<(Agent, Uuid)> {
    let new_session_id = Uuid::new_v4();
    let new_agent = build_main_agent(
        params.session,
        params.db.clone(),
        new_session_id,
        params.project_dir.clone(),
        params.local_agent_mgmt_tx,
        #[cfg(feature = "stylos")]
        params.stylos_tool_bridge,
        #[cfg(feature = "stylos")]
        params.local_stylos_instance,
        #[cfg(feature = "stylos")]
        "master",
        None,
        params.api_log_enabled,
    )?;
    if params.insert_session {
        let _ = params
            .db
            .insert_session(new_session_id, params.project_dir, true);
    }
    Ok((new_agent, new_session_id))
}

fn unix_epoch_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}


pub(crate) fn agent_has_role(handle: &crate::tui::AgentHandle, role: &str) -> bool {
    #[cfg(feature = "stylos")]
    let role = if role == "main" { "master" } else { role };
    handle.roles.iter().any(|candidate| {
        #[cfg(feature = "stylos")]
        {
            let candidate = if candidate == "main" { "master" } else { candidate.as_str() };
            candidate == role
        }
        #[cfg(not(feature = "stylos"))]
        {
            candidate == role
        }
    })
}

pub(crate) fn is_interactive_agent_handle(handle: &crate::tui::AgentHandle) -> bool {
    agent_has_role(handle, "interactive")
}

pub(crate) fn build_local_agent_roster(
    agents: &[crate::tui::AgentHandle],
) -> Vec<LocalAgentRosterEntry> {
    agents
        .iter()
        .map(|handle| roster_entry(handle.agent_id.clone(), handle.roles.clone()))
        .collect()
}

#[cfg(feature = "stylos")]
pub(crate) fn build_local_agent_status_entries(
    agents: &[crate::tui::AgentHandle],
) -> Vec<LocalAgentStatusEntry> {
    agents
        .iter()
        .map(|handle| {
            local_agent_status_entry(
                handle.agent_id.clone(),
                handle.label.clone(),
                handle.roles.clone(),
                handle.session_id.to_string(),
                handle.agent.as_ref().map(|agent| agent.workflow_state().clone()),
                handle.agent.as_ref().map(|agent| agent.project_dir.clone()),
                handle.busy,
                handle.active_incoming_prompt.is_some(),
            )
        })
        .collect()
}

pub(crate) struct NewLocalAgentHandleParts {
    pub agent: Agent,
    pub session_id: Uuid,
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
}

pub(crate) fn build_local_agent_handle(parts: NewLocalAgentHandleParts) -> crate::tui::AgentHandle {
    crate::tui::AgentHandle {
        agent: Some(parts.agent),
        session_id: parts.session_id,
        agent_id: parts.agent_id,
        label: parts.label,
        roles: parts.roles,
        busy: false,
        turn_cancellation: None,
        #[cfg(feature = "stylos")]
        active_incoming_prompt: None,
    }
}

pub(crate) struct LocalAgentRuntimeContext<'a> {
    pub session: &'a Session,
    pub project_dir: &'a PathBuf,
    pub db: &'a Arc<DbHandle>,
    pub agents: &'a mut Vec<crate::tui::AgentHandle>,
    pub roster: &'a [LocalAgentRosterEntry],
    pub agent_busy: bool,
    #[cfg(feature = "stylos")]
    pub stylos_tool_bridge: Option<crate::stylos::StylosToolBridge>,
    #[cfg(feature = "stylos")]
    pub local_stylos_instance: Option<&'a str>,
    pub local_agent_tool_invoker: Option<themion_core::tools::LocalAgentToolInvoker>,
    pub api_log_enabled: bool,
}

fn normalize_primary_role(value: &str) -> &str {
    if value == "main" {
        "master"
    } else {
        value
    }
}

fn normalize_role_list(value: Option<&serde_json::Value>) -> Vec<String> {
    let mut roles = value
        .and_then(|v| v.as_array())
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    roles.sort();
    roles.dedup();
    roles
}

pub(crate) fn allocate_default_local_agent_id(agents: &[LocalAgentRosterEntry]) -> String {
    let mut n = 1usize;
    loop {
        let candidate = format!("smith-{n}");
        if !agents.iter().any(|h| h.agent_id == candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
pub(crate) fn validate_agent_roles(agents: &[LocalAgentRosterEntry]) -> anyhow::Result<()> {
    let master_count = agents
        .iter()
        .filter(|handle| handle.roles.iter().any(|role| normalize_primary_role(role) == "master"))
        .count();
    if master_count != 1 {
        anyhow::bail!("invalid agent roles: expected exactly one master agent");
    }
    let interactive_count = agents
        .iter()
        .filter(|handle| handle.roles.iter().any(|role| role == "interactive"))
        .count();
    if interactive_count > 1 {
        anyhow::bail!("invalid agent roles: expected at most one interactive agent");
    }
    Ok(())
}

pub(crate) fn handle_local_agent_management_request(
    ctx: LocalAgentRuntimeContext<'_>,
    action: &str,
    args: serde_json::Value,
) -> anyhow::Result<String> {
    match action {
        "local_agent_create" => create_local_agent(ctx, args),
        "local_agent_delete" => delete_local_agent(ctx, args),
        other => Err(anyhow::anyhow!(
            "unknown local agent management action: {other}"
        )),
    }
}

fn create_local_agent(
    ctx: LocalAgentRuntimeContext<'_>,
    args: serde_json::Value,
) -> anyhow::Result<String> {
    let requested_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let agent_id = requested_id
        .map(str::to_string)
        .unwrap_or_else(|| allocate_default_local_agent_id(ctx.roster));
    if agent_id == "master" {
        anyhow::bail!("agent_id 'master' is reserved for the predefined leader");
    }
    if ctx.agents.iter().any(|h| h.agent_id == agent_id) {
        anyhow::bail!("duplicate agent_id: {agent_id}");
    }
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(agent_id.as_str())
        .to_string();
    let roles = normalize_role_list(args.get("roles"));
    if roles.iter().any(|r| normalize_primary_role(r) == "master") {
        anyhow::bail!("cannot create another master agent");
    }
    if roles.iter().any(|r| r == "interactive")
        && ctx.roster.iter().any(|entry| entry.roles.iter().any(|r| r == "interactive"))
    {
        anyhow::bail!("invalid agent roles: expected at most one interactive agent");
    }
    let session_id = Uuid::new_v4();
    let _ = ctx.db.insert_session(session_id, ctx.project_dir, true);
    let agent = crate::app_state::build_agent(
        ctx.session,
        session_id,
        ctx.project_dir.clone(),
        ctx.db.clone(),
        #[cfg(feature = "stylos")]
        ctx.stylos_tool_bridge.clone(),
        #[cfg(feature = "stylos")]
        ctx.local_stylos_instance,
        #[cfg(feature = "stylos")]
        &agent_id,
        ctx.local_agent_tool_invoker,
        None,
        ctx.api_log_enabled,
    )?;
    ctx.agents.push(build_local_agent_handle(NewLocalAgentHandleParts {
        agent,
        session_id,
        agent_id: agent_id.clone(),
        label: label.clone(),
        roles: roles.clone(),
    }));
    Ok(serde_json::json!({
        "ok": true,
        "entity": "local_agent",
        "operation": "create",
        "agent_id": agent_id,
        "label": label,
        "roles": roles,
        "session_id": session_id.to_string(),
    })
    .to_string())
}

pub(crate) struct RemovedLocalAgentSummary {
    pub agent_id: String,
    pub label: String,
    pub session_id: Uuid,
}

pub(crate) fn remove_local_agent_handle(
    agents: &mut Vec<crate::tui::AgentHandle>,
    agent_id: &str,
) -> anyhow::Result<RemovedLocalAgentSummary> {
    let index = agents
        .iter()
        .position(|h| h.agent_id == agent_id)
        .ok_or_else(|| anyhow::anyhow!("unknown agent_id: {agent_id}"))?;
    let removed = agents.remove(index);
    Ok(RemovedLocalAgentSummary {
        agent_id: removed.agent_id,
        label: removed.label,
        session_id: removed.session_id,
    })
}

fn delete_local_agent(
    ctx: LocalAgentRuntimeContext<'_>,
    args: serde_json::Value,
) -> anyhow::Result<String> {
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing agent_id"))?;
    if agent_id == "master" {
        anyhow::bail!("cannot delete the predefined leader agent");
    }
    if ctx.agent_busy {
        anyhow::bail!("cannot delete local agents while the local runtime is busy");
    }
    let removed = remove_local_agent_handle(ctx.agents, agent_id)?;
    Ok(serde_json::json!({
        "ok": true,
        "entity": "local_agent",
        "operation": "delete",
        "agent_id": removed.agent_id,
        "label": removed.label,
        "session_id": removed.session_id.to_string(),
    })
    .to_string())
}

#[cfg(feature = "stylos")]
pub(crate) fn publish_stylos_task_running(
    background_domain: &DomainHandle,
    query_context: crate::stylos::StylosQueryContext,
    task_id: String,
) {
    background_domain.spawn(async move {
        query_context.task_registry().set_running(&task_id).await;
    });
}

#[cfg(feature = "stylos")]
pub(crate) fn publish_stylos_task_completed(
    background_domain: &DomainHandle,
    query_context: crate::stylos::StylosQueryContext,
    task_id: String,
    result_text: Option<String>,
) {
    background_domain.spawn(async move {
        query_context
            .task_registry()
            .set_completed(&task_id, result_text, None)
            .await;
    });
}

#[cfg(feature = "stylos")]
pub(crate) fn publish_stylos_task_failed(
    background_domain: &DomainHandle,
    query_context: crate::stylos::StylosQueryContext,
    task_id: String,
    reason: String,
) {
    background_domain.spawn(async move {
        query_context.task_registry().set_failed(&task_id, reason).await;
    });
}

#[cfg(feature = "stylos")]
#[derive(Default)]
pub(crate) struct WatchdogRuntimeState {
    idle_started_at_ms: AtomicU64,
    active_incoming_prompt: AtomicBool,
    pending_watchdog_note: AtomicBool,
}

#[cfg(feature = "stylos")]
impl WatchdogRuntimeState {
    pub(crate) fn sync_from_runtime_state(
        &self,
        agent_busy: bool,
        has_active_incoming_prompt: bool,
        pending_watchdog_note: bool,
    ) {
        if agent_busy {
            self.idle_started_at_ms.store(0, Ordering::Relaxed);
        } else {
            self.idle_started_at_ms
                .store(unix_epoch_now_ms(), Ordering::Relaxed);
        }
        self.active_incoming_prompt
            .store(has_active_incoming_prompt, Ordering::Relaxed);
        self.pending_watchdog_note
            .store(pending_watchdog_note, Ordering::Relaxed);
    }

    fn idle_started_at_ms(&self) -> Option<u64> {
        let value = self.idle_started_at_ms.load(Ordering::Relaxed);
        (value != 0).then_some(value)
    }

    pub(crate) fn pending_watchdog_note(&self) -> bool {
        self.pending_watchdog_note.load(Ordering::Relaxed)
    }

    fn should_trigger_watchdog_note(&self, now_ms: u64, idle_delay_ms: u64) -> bool {
        if self.active_incoming_prompt.load(Ordering::Relaxed) {
            return false;
        }
        let Some(idle_started_at_ms) = self.idle_started_at_ms() else {
            return false;
        };
        now_ms.saturating_sub(idle_started_at_ms) >= idle_delay_ms
    }
}

#[cfg(feature = "stylos")]
pub(crate) fn start_watchdog_task(
    background_domain: &DomainHandle,
    runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
    runtime_state: Arc<WatchdogRuntimeState>,
) {
    background_domain.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            let now_ms = unix_epoch_now_ms();
            if !runtime_state.should_trigger_watchdog_note(now_ms, WATCHDOG_IDLE_DELAY_MS_DEFAULT)
            {
                continue;
            }
            if runtime_tx.send(AppRuntimeEvent::WatchdogDispatchLog { agent_id: None, text: String::new() }).is_err() {
                break;
            }
        }
    });
}

#[cfg(feature = "stylos")]
pub(crate) enum IncomingPromptDisposition {
    MissingTarget {
        log_agent_id: Option<String>,
        log_text: String,
        failed_task_id: Option<String>,
    },
    BusyTarget {
        log_agent_id: Option<String>,
        log_text: String,
        failed_task_id: Option<String>,
    },
    Accepted {
        agent_index: usize,
        log_agent_id: Option<String>,
        log_text: String,
        prompt: String,
        request: crate::stylos::IncomingPromptRequest,
        pending_watchdog_note: bool,
    },
}

#[cfg(feature = "stylos")]
pub(crate) fn resolve_incoming_prompt_disposition(
    agents: &[LocalAgentStatusEntry],
    board_claims: &Arc<LocalBoardClaimRegistry>,
    request: crate::stylos::IncomingPromptRequest,
) -> IncomingPromptDisposition {
    let target = request
        .agent_id
        .clone()
        .unwrap_or_else(|| "interactive".to_string());
    let sender = request.from.as_deref().unwrap_or("unknown sender");
    let sender_agent = request.from_agent_id.as_deref().unwrap_or("unknown");
    let target_instance = request.to.as_deref().unwrap_or("unknown target");
    let target_agent = request.to_agent_id.as_deref().unwrap_or(target.as_str());
    let is_note = request.prompt.starts_with("type=stylos_note ");

    let Some(agent_index) = agents.iter().position(|h| h.agent_id == target) else {
        return IncomingPromptDisposition::MissingTarget {
            log_agent_id: Some(target.clone()),
            log_text: format!(
                "Stylos hear from={} from_agent_id={} to={} to_agent_id={} rejected: target agent missing locally",
                sender, sender_agent, target_instance, target_agent
            ),
            failed_task_id: request.task_id.clone(),
        };
    };

    if agents[agent_index].busy || agents[agent_index].has_active_incoming_prompt {
        if let Some(note_id) = board_note_id_from_prompt(&request.prompt) {
            release_board_note_claim(board_claims, note_id);
        }
        let log_text = if is_note {
            let note_identifier = stylos_note_display_identifier(&request.prompt);
            format!(
                "Board note intake {} from={} from_agent_id={} to={} to_agent_id={} deferred: local agent busy",
                note_identifier, sender, sender_agent, target_instance, target_agent
            )
        } else {
            format!(
                "Stylos hear from={} from_agent_id={} to={} to_agent_id={} rejected: local agent busy",
                sender, sender_agent, target_instance, target_agent
            )
        };
        return IncomingPromptDisposition::BusyTarget {
            log_agent_id: Some(target),
            log_text,
            failed_task_id: request.task_id.clone(),
        };
    }

    let log_text = if is_note {
        let note_identifier = stylos_note_display_identifier(&request.prompt);
        let column = stylos_note_header_value(&request.prompt, "column")
            .unwrap_or("unknown");
        format!(
            "Board note intake {} from={} from_agent_id={} to={} to_agent_id={} column={}",
            note_identifier, sender, sender_agent, target_instance, target_agent, column
        )
    } else {
        format!(
            "Stylos hear from={} from_agent_id={} to={} to_agent_id={}",
            sender, sender_agent, target_instance, target_agent
        )
    };

    IncomingPromptDisposition::Accepted {
        agent_index,
        log_agent_id: Some(target),
        log_text,
        prompt: request.prompt.clone(),
        pending_watchdog_note: matches!(
            request.source,
            crate::stylos::IncomingPromptSource::WatchdogBoardNote
        ),
        request,
    }
}

#[cfg(feature = "stylos")]
pub(crate) struct SubmitTargetFailureEffect {
    pub log_text: String,
    pub failed_task_id: Option<String>,
    pub failure_reason: &'static str,
}

#[cfg(feature = "stylos")]
pub(crate) fn submit_target_failure_effect(
    resolution: &SubmitTargetResolution,
) -> Option<SubmitTargetFailureEffect> {
    match resolution {
        SubmitTargetResolution::MissingIncomingPromptTarget {
            failed_task_id,
            log_text,
            ..
        } => Some(SubmitTargetFailureEffect {
            log_text: log_text.clone(),
            failed_task_id: failed_task_id.clone(),
            failure_reason: "target_agent_missing",
        }),
        _ => None,
    }
}

#[cfg(feature = "stylos")]
pub(crate) enum SubmitTargetResolution {
    Interactive { agent_index: usize },
    IncomingPromptTarget { agent_index: usize },
    MissingIncomingPromptTarget {
        active_agent_index: usize,
        failed_task_id: Option<String>,
        log_text: String,
    },
}

#[cfg(feature = "stylos")]
pub(crate) fn resolve_submit_target(
    agents: &[LocalAgentStatusEntry],
    active_request: Option<&crate::stylos::IncomingPromptRequest>,
) -> SubmitTargetResolution {
    let interactive_index = agents
        .iter()
        .position(|h| h.roles.iter().any(|r| r == "interactive"))
        .expect("interactive agent");
    let Some(active_agent_index) = agents
        .iter()
        .position(|h| h.has_active_incoming_prompt)
    else {
        return SubmitTargetResolution::Interactive {
            agent_index: interactive_index,
        };
    };
    let Some(request) = active_request else {
        return SubmitTargetResolution::Interactive {
            agent_index: interactive_index,
        };
    };
    let Some(target_agent_id) = request.agent_id.as_deref() else {
        return SubmitTargetResolution::Interactive {
            agent_index: interactive_index,
        };
    };
    match agents.iter().position(|h| h.agent_id == target_agent_id) {
        Some(agent_index) => SubmitTargetResolution::IncomingPromptTarget { agent_index },
        None => {
            let sender = request.from.as_deref().unwrap_or("unknown sender");
            let sender_agent = request.from_agent_id.as_deref().unwrap_or("unknown");
            let target_instance = request.to.as_deref().unwrap_or("unknown target");
            let target_agent = request.to_agent_id.as_deref().unwrap_or(target_agent_id);
            let log_text = if request.prompt.starts_with("type=stylos_note ") {
                let note_identifier = stylos_note_display_identifier(&request.prompt);
                format!(
                    "Board note intake {} from={} from_agent_id={} to={} to_agent_id={} rejected: target agent missing locally",
                    note_identifier, sender, sender_agent, target_instance, target_agent
                )
            } else {
                format!(
                    "Stylos hear from={} from_agent_id={} to={} to_agent_id={} rejected: target agent missing locally",
                    sender, sender_agent, target_instance, target_agent
                )
            };
            SubmitTargetResolution::MissingIncomingPromptTarget {
                active_agent_index,
                failed_task_id: request.task_id.clone(),
                log_text,
            }
        }
    }
}

#[cfg(feature = "stylos")]
pub(crate) struct IncomingPromptApplyPlan {
    pub accepted_agent_index: usize,
    pub accepted_prompt: String,
    pub accepted_request: crate::stylos::IncomingPromptRequest,
    pub pending_watchdog_note: bool,
}

#[cfg(feature = "stylos")]
pub(crate) fn recompute_watchdog_state_from_app(
    agents: &[crate::tui::AgentHandle],
) -> (bool, bool, bool) {
    let agent_busy = agents.iter().any(|handle| handle.busy);
    let has_active_incoming_prompt = agents
        .iter()
        .any(|handle| handle.active_incoming_prompt.is_some());
    let pending_watchdog_note = has_active_incoming_prompt;
    (
        agent_busy,
        has_active_incoming_prompt,
        pending_watchdog_note,
    )
}

#[cfg(feature = "stylos")]
pub(crate) fn sync_watchdog_runtime_state(
    watchdog_state: &Arc<WatchdogRuntimeState>,
    agents: &[crate::tui::AgentHandle],
) {
    let (agent_busy, has_active_incoming_prompt, pending_watchdog_note) =
        recompute_watchdog_state_from_app(agents);
    watchdog_state.sync_from_runtime_state(
        agent_busy,
        has_active_incoming_prompt,
        pending_watchdog_note,
    );
}

#[cfg(feature = "stylos")]
pub(crate) fn set_active_incoming_prompt(
    agents: &mut [crate::tui::AgentHandle],
    watchdog_state: &Arc<WatchdogRuntimeState>,
    agent_index: usize,
    request: Option<crate::stylos::IncomingPromptRequest>,
    _pending_watchdog_note: bool,
) {
    agents[agent_index].active_incoming_prompt = request;
    sync_watchdog_runtime_state(watchdog_state, agents);
}

#[cfg(feature = "stylos")]
pub(crate) fn clear_active_incoming_prompt(
    agents: &mut [crate::tui::AgentHandle],
    watchdog_state: &Arc<WatchdogRuntimeState>,
    agent_index: usize,
) {
    set_active_incoming_prompt(agents, watchdog_state, agent_index, None, false);
}

#[cfg(feature = "stylos")]
pub(crate) fn continue_current_note_follow_up(
    agents: &mut [crate::tui::AgentHandle],
    watchdog_state: &Arc<WatchdogRuntimeState>,
    agent_index: usize,
    request: crate::stylos::IncomingPromptRequest,
) {
    let pending_watchdog_note = watchdog_state.pending_watchdog_note();
    set_active_incoming_prompt(
        agents,
        watchdog_state,
        agent_index,
        Some(request),
        pending_watchdog_note,
    );
}

#[cfg(feature = "stylos")]
pub(crate) fn apply_active_incoming_prompt(
    agents: &mut [crate::tui::AgentHandle],
    watchdog_state: &Arc<WatchdogRuntimeState>,
    agent_index: usize,
    request: crate::stylos::IncomingPromptRequest,
    pending_watchdog_note: bool,
) {
    set_active_incoming_prompt(
        agents,
        watchdog_state,
        agent_index,
        Some(request),
        pending_watchdog_note,
    );
}

#[cfg(feature = "stylos")]
pub(crate) fn incoming_prompt_apply_plan(
    outcome: IncomingPromptOutcome,
) -> Result<IncomingPromptApplyPlan, IncomingPromptOutcome> {
    match outcome {
        IncomingPromptOutcome {
            accepted_agent_index: Some(accepted_agent_index),
            accepted_prompt: Some(accepted_prompt),
            accepted_request: Some(accepted_request),
            pending_watchdog_note,
            ..
        } => Ok(IncomingPromptApplyPlan {
            accepted_agent_index,
            accepted_prompt,
            accepted_request,
            pending_watchdog_note,
        }),
        other => Err(other),
    }
}

#[cfg(feature = "stylos")]
pub(crate) struct IncomingPromptOutcome {
    pub log_agent_id: Option<String>,
    pub log_text: String,
    pub task_failure: Option<String>,
    pub accepted_agent_index: Option<usize>,
    pub accepted_prompt: Option<String>,
    pub accepted_request: Option<crate::stylos::IncomingPromptRequest>,
    pub pending_watchdog_note: bool,
}

#[cfg(feature = "stylos")]
pub(crate) fn plan_incoming_prompt(
    agents: &[LocalAgentStatusEntry],
    board_claims: &Arc<LocalBoardClaimRegistry>,
    request: crate::stylos::IncomingPromptRequest,
) -> IncomingPromptOutcome {
    match resolve_incoming_prompt_disposition(agents, board_claims, request) {
        IncomingPromptDisposition::MissingTarget {
            log_agent_id,
            log_text,
            failed_task_id,
        } => IncomingPromptOutcome {
            log_agent_id,
            log_text,
            task_failure: failed_task_id.map(|task_id| format!("{task_id}:target_agent_missing")),
            accepted_agent_index: None,
            accepted_prompt: None,
            accepted_request: None,
            pending_watchdog_note: false,
        },
        IncomingPromptDisposition::BusyTarget {
            log_agent_id,
            log_text,
            failed_task_id,
        } => IncomingPromptOutcome {
            log_agent_id,
            log_text,
            task_failure: failed_task_id.map(|task_id| format!("{task_id}:agent_busy")),
            accepted_agent_index: None,
            accepted_prompt: None,
            accepted_request: None,
            pending_watchdog_note: false,
        },
        IncomingPromptDisposition::Accepted {
            agent_index,
            log_agent_id,
            log_text,
            prompt,
            request,
            pending_watchdog_note,
        } => IncomingPromptOutcome {
            log_agent_id,
            log_text,
            task_failure: None,
            accepted_agent_index: Some(agent_index),
            accepted_prompt: Some(prompt),
            accepted_request: Some(request),
            pending_watchdog_note,
        },
    }
}

#[cfg(feature = "stylos")]
pub(crate) enum CompletedNoteFollowUpEmission {
    RemoteEvent { text: String },
    Status { text: String },
}

#[cfg(feature = "stylos")]
pub(crate) struct CompletedNoteFollowUpApplyPlan {
    pub continue_request: Option<crate::stylos::IncomingPromptRequest>,
    pub continue_prompt: Option<String>,
    pub emission: Option<CompletedNoteFollowUpEmission>,
}

#[cfg(feature = "stylos")]
pub(crate) fn completed_note_follow_up_apply_plan(
    plan: CompletedNoteFollowUpPlan,
) -> CompletedNoteFollowUpApplyPlan {
    match plan {
        CompletedNoteFollowUpPlan::None => CompletedNoteFollowUpApplyPlan {
            continue_request: None,
            continue_prompt: None,
            emission: None,
        },
        CompletedNoteFollowUpPlan::ContinueCurrentNote { request, prompt } => {
            CompletedNoteFollowUpApplyPlan {
                continue_request: Some(request),
                continue_prompt: Some(prompt),
                emission: None,
            }
        }
        CompletedNoteFollowUpPlan::EmitDoneMentionLog { log_line } => {
            CompletedNoteFollowUpApplyPlan {
                continue_request: None,
                continue_prompt: None,
                emission: Some(CompletedNoteFollowUpEmission::RemoteEvent { text: log_line }),
            }
        }
        CompletedNoteFollowUpPlan::EmitDoneMentionStatus { status_line } => {
            CompletedNoteFollowUpApplyPlan {
                continue_request: None,
                continue_prompt: None,
                emission: Some(CompletedNoteFollowUpEmission::Status { text: status_line }),
            }
        }
    }
}

#[cfg(feature = "stylos")]
pub(crate) enum CompletedNoteFollowUpPlan {
    None,
    ContinueCurrentNote {
        request: crate::stylos::IncomingPromptRequest,
        prompt: String,
    },
    EmitDoneMentionLog {
        log_line: String,
    },
    EmitDoneMentionStatus {
        status_line: String,
    },
}

#[cfg(feature = "stylos")]
pub(crate) fn plan_completed_note_follow_up(
    db: &Arc<DbHandle>,
    remote: &crate::stylos::IncomingPromptRequest,
) -> CompletedNoteFollowUpPlan {
    match crate::board_runtime::resolve_completed_note_follow_up(db, remote) {
        crate::board_runtime::BoardTurnFollowUp::None => CompletedNoteFollowUpPlan::None,
        crate::board_runtime::BoardTurnFollowUp::ContinueCurrentNote { request, prompt } => {
            CompletedNoteFollowUpPlan::ContinueCurrentNote { request, prompt }
        }
        crate::board_runtime::BoardTurnFollowUp::EmitDoneMention { log_line } => {
            CompletedNoteFollowUpPlan::EmitDoneMentionLog { log_line }
        }
        crate::board_runtime::BoardTurnFollowUp::EmitDoneMentionError { status_line } => {
            CompletedNoteFollowUpPlan::EmitDoneMentionStatus { status_line }
        }
    }
}

#[cfg(feature = "stylos")]
fn stylos_note_header_value<'a>(prompt: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    prompt
        .lines()
        .next()?
        .split_whitespace()
        .find_map(|part| part.strip_prefix(&prefix))
}

#[cfg(feature = "stylos")]
fn stylos_note_display_identifier(prompt: &str) -> String {
    if let Some(note_slug) = stylos_note_header_value(prompt, "note_slug") {
        format!("note_slug={note_slug}")
    } else if let Some(note_id) = stylos_note_header_value(prompt, "note_id") {
        format!("note_id={note_id}")
    } else {
        "note_id=unknown".to_string()
    }
}#[cfg(feature = "stylos")]
#[derive(Clone, Debug)]
pub(crate) struct AgentStatusSource {
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
    pub session_id: String,
    pub workflow: WorkflowState,
    pub activity_status: String,
    pub activity_status_changed_at_ms: u64,
    pub project_dir: PathBuf,
    pub provider: String,
    pub model: String,
    pub active_profile: String,
    pub rate_limits: Option<ApiCallRateLimitReport>,
}

#[cfg(feature = "stylos")]
pub(crate) fn build_stylos_status_snapshot(
    startup_project_dir: &std::path::Path,
    agent_sources: Vec<AgentStatusSource>,
) -> anyhow::Result<crate::stylos::StylosStatusSnapshot> {
    let main_count = agent_sources
        .iter()
        .filter(|agent| agent.roles.iter().any(|r| normalize_primary_role(r) == "master"))
        .count();
    if main_count != 1 {
        anyhow::bail!(
            "invalid agent roles: expected exactly one master agent, found {}",
            main_count
        );
    }
    let interactive_count = agent_sources
        .iter()
        .filter(|agent| agent.roles.iter().any(|r| r == "interactive"))
        .count();
    if interactive_count > 1 {
        anyhow::bail!(
            "invalid agent roles: expected at most one interactive agent, found {}",
            interactive_count
        );
    }

    let agents = agent_sources
        .into_iter()
        .map(|agent| {
            let git_status = crate::stylos::GitStatusCache::new(agent.project_dir.clone()).snapshot();
            crate::stylos::StylosAgentStatusSnapshot {
                agent_id: agent.agent_id,
                label: agent.label,
                roles: agent.roles,
                session_id: agent.session_id,
                workflow: agent.workflow,
                activity_status: agent.activity_status,
                activity_status_changed_at_ms: agent.activity_status_changed_at_ms,
                project_dir: agent.project_dir.display().to_string(),
                project_dir_is_git_repo: git_status.is_repo,
                git_remotes: git_status.remotes,
                provider: agent.provider,
                model: agent.model,
                active_profile: agent.active_profile,
                rate_limits: agent.rate_limits,
            }
        })
        .collect();

    Ok(crate::stylos::StylosStatusSnapshot {
        startup_project_dir: startup_project_dir.display().to_string(),
        agents,
    })
}

#[cfg(feature = "stylos")]
#[derive(Clone, Debug)]
pub(crate) struct AgentSnapshotInput {
    pub agent_id: String,
    pub label: String,
    pub roles: Vec<String>,
    pub session_id: String,
    pub workflow: Option<WorkflowState>,
    pub project_dir: Option<PathBuf>,
}

#[cfg(feature = "stylos")]
#[derive(Clone, Debug)]
pub(crate) struct StylosStatusRefreshInput {
    pub startup_project_dir: PathBuf,
    pub fallback_project_dir: PathBuf,
    pub provider: String,
    pub model: String,
    pub active_profile: String,
    pub rate_limits: Option<ApiCallRateLimitReport>,
    pub idle_since: Option<std::time::Instant>,
    pub idle_status_changed_at: Option<u64>,
    pub primary_activity_label: Option<String>,
    pub primary_activity_changed_at_ms: Option<u64>,
    pub primary_workflow: WorkflowState,
    pub agents: Vec<AgentSnapshotInput>,
}

#[cfg(feature = "stylos")]
pub(crate) fn build_stylos_status_snapshot_from_runtime(
    input: StylosStatusRefreshInput,
) -> crate::stylos::StylosStatusSnapshot {
    let agent_sources: Vec<AgentStatusSource> = input
        .agents
        .into_iter()
        .enumerate()
        .map(|(idx, agent)| {
            let (activity_status, activity_status_changed_at_ms) = if idx == 0 {
                if let Some(activity_label) = input.primary_activity_label.as_ref() {
                    (
                        activity_label.clone(),
                        input
                            .primary_activity_changed_at_ms
                            .unwrap_or_else(unix_epoch_now_ms),
                    )
                } else {
                    const NAP_AFTER: Duration = Duration::from_secs(5 * 60);
                    match input.idle_since {
                        Some(idle_since) if idle_since.elapsed() > NAP_AFTER => (
                            "nap".to_string(),
                            input
                                .idle_status_changed_at
                                .unwrap_or_else(unix_epoch_now_ms)
                                + NAP_AFTER.as_millis() as u64,
                        ),
                        _ => (
                            "idle".to_string(),
                            input
                                .idle_status_changed_at
                                .unwrap_or_else(unix_epoch_now_ms),
                        ),
                    }
                }
            } else {
                ("idle".to_string(), unix_epoch_now_ms())
            };

            AgentStatusSource {
                agent_id: agent.agent_id,
                label: agent.label,
                roles: agent.roles,
                session_id: agent.session_id,
                workflow: agent.workflow.unwrap_or_else(|| {
                    if idx == 0 {
                        input.primary_workflow.clone()
                    } else {
                        WorkflowState::default()
                    }
                }),
                activity_status,
                activity_status_changed_at_ms,
                project_dir: agent
                    .project_dir
                    .unwrap_or_else(|| input.fallback_project_dir.clone()),
                provider: input.provider.clone(),
                model: input.model.clone(),
                active_profile: input.active_profile.clone(),
                rate_limits: if idx == 0 {
                    input.rate_limits.clone()
                } else {
                    None
                },
            }
        })
        .collect();

    build_stylos_status_snapshot(&input.startup_project_dir, agent_sources).unwrap_or_else(|_| {
        crate::stylos::StylosStatusSnapshot {
            startup_project_dir: input.startup_project_dir.display().to_string(),
            agents: Vec::new(),
        }
    })
}

#[cfg(feature = "stylos")]
pub(crate) async fn publish_stylos_status_snapshot(
    hub: &SharedStylosStatusHub,
    snapshot: crate::stylos::StylosStatusSnapshot,
) {
    hub.replace_snapshot(snapshot).await;
}

#[cfg(feature = "stylos")]
pub(crate) struct StylosAppStatusRefreshState<'a> {
    pub startup_project_dir: &'a std::path::Path,
    pub fallback_project_dir: &'a std::path::Path,
    pub provider: &'a str,
    pub model: &'a str,
    pub active_profile: &'a str,
    pub rate_limits: Option<&'a ApiCallRateLimitReport>,
    pub idle_since: Option<std::time::Instant>,
    pub idle_status_changed_at: Option<u64>,
    pub primary_activity_label: Option<String>,
    pub primary_activity_changed_at_ms: Option<u64>,
    pub primary_workflow: &'a WorkflowState,
    pub agents: &'a [crate::tui::AgentHandle],
}

#[cfg(feature = "stylos")]
pub(crate) fn refresh_stylos_status_snapshot(
    hub: &SharedStylosStatusHub,
    state: StylosAppStatusRefreshState<'_>,
) {
    let agent_status_entries = build_local_agent_status_entries(state.agents);
    build_and_publish_stylos_status_snapshot(
        hub,
        StylosAppStatusSnapshotInput {
            startup_project_dir: state.startup_project_dir.to_path_buf(),
            fallback_project_dir: state.fallback_project_dir.to_path_buf(),
            provider: state.provider.to_string(),
            model: state.model.to_string(),
            active_profile: state.active_profile.to_string(),
            rate_limits: state.rate_limits.cloned(),
            idle_since: state.idle_since,
            idle_status_changed_at: state.idle_status_changed_at,
            primary_activity_label: state.primary_activity_label,
            primary_activity_changed_at_ms: state.primary_activity_changed_at_ms,
            primary_workflow: state.primary_workflow.clone(),
            agents: &agent_status_entries,
        },
    );
}

#[cfg(feature = "stylos")]
#[derive(Clone)]
pub(crate) struct StylosAppStatusSnapshotInput<'a> {
    pub startup_project_dir: PathBuf,
    pub fallback_project_dir: PathBuf,
    pub provider: String,
    pub model: String,
    pub active_profile: String,
    pub rate_limits: Option<ApiCallRateLimitReport>,
    pub idle_since: Option<std::time::Instant>,
    pub idle_status_changed_at: Option<u64>,
    pub primary_activity_label: Option<String>,
    pub primary_activity_changed_at_ms: Option<u64>,
    pub primary_workflow: WorkflowState,
    pub agents: &'a [LocalAgentStatusEntry],
}

#[cfg(feature = "stylos")]
pub(crate) fn build_and_publish_stylos_status_snapshot(
    hub: &SharedStylosStatusHub,
    input: StylosAppStatusSnapshotInput<'_>,
) {
    let snapshot = build_stylos_status_snapshot_from_runtime(StylosStatusRefreshInput {
        startup_project_dir: input.startup_project_dir,
        fallback_project_dir: input.fallback_project_dir,
        provider: input.provider,
        model: input.model,
        active_profile: input.active_profile,
        rate_limits: input.rate_limits,
        idle_since: input.idle_since,
        idle_status_changed_at: input.idle_status_changed_at,
        primary_activity_label: input.primary_activity_label,
        primary_activity_changed_at_ms: input.primary_activity_changed_at_ms,
        primary_workflow: input.primary_workflow,
        agents: input
            .agents
            .iter()
            .map(|h| AgentSnapshotInput {
                agent_id: h.agent_id.clone(),
                label: h.label.clone(),
                roles: h.roles.clone(),
                session_id: h.session_id.clone(),
                workflow: h.workflow.clone(),
                project_dir: h.project_dir.clone(),
            })
            .collect(),
    });
    let hub = hub.clone();
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async move {
            publish_stylos_status_snapshot(&hub, snapshot).await;
        });
    });
}



#[cfg(test)]
mod tests {
    use super::stylos_note_display_identifier;

    #[test]
    fn stylos_note_display_identifier_prefers_slug() {
        let prompt = "type=stylos_note note_id=123e4567-e89b-12d3-a456-426614174000 note_slug=fix-tests-123e4567 column=todo

body";
        assert_eq!(
            stylos_note_display_identifier(prompt),
            "note_slug=fix-tests-123e4567"
        );
    }

    #[test]
    fn stylos_note_display_identifier_falls_back_to_note_id() {
        let prompt =
            "type=stylos_note note_id=123e4567-e89b-12d3-a456-426614174000 column=todo

body";
        assert_eq!(
            stylos_note_display_identifier(prompt),
            "note_id=123e4567-e89b-12d3-a456-426614174000"
        );
    }
}
