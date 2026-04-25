use crate::config::{save_profiles, ProfileConfig};
use crate::runtime_domains::DomainHandle;
#[cfg(feature = "stylos")]
use crate::stylos::{
    tool_bridge, IncomingPromptRequest, StylosHandle, StylosRuntimeState, StylosToolBridge,
};
use crate::{format_stats, Session};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
    Frame, Terminal,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use themion_core::agent::{Agent, AgentEvent, TurnCancellation, TurnStats};
use themion_core::client_codex::ApiCallRateLimitReport;
use themion_core::db::DbHandle;
use themion_core::workflow::WorkflowState;
use themion_core::ModelInfo;
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc};
use tui_textarea::CursorMove;
use tui_textarea::TextArea;
use unicode_width::UnicodeWidthChar;

use crate::paste_burst::{CharDecision, FlushResult, PasteBurst};
use uuid::Uuid;

pub(crate) enum AppEvent {
    Key(event::KeyEvent),
    Mouse(event::MouseEvent),
    Paste(String),
    Agent(AgentEvent),
    AgentReady(Box<Agent>, Uuid),
    Tick,
    #[cfg(feature = "stylos")]
    StylosCmd(crate::stylos::StylosCmdRequest),
    #[cfg(feature = "stylos")]
    IncomingPrompt(IncomingPromptRequest),
    #[cfg(feature = "stylos")]
    StylosEvent(String),
    LoginPrompt {
        user_code: String,
        verification_uri: String,
    },
    LoginComplete(anyhow::Result<themion_core::CodexAuth>),
    ShellComplete {
        output: String,
        exit_code: Option<i32>,
    },
}

enum Entry {
    User(String),
    Assistant(String),
    Banner(String),
    ToolCall(String),
    ToolDone,
    Status(String),
    #[cfg(feature = "stylos")]
    RemoteEvent(String),
    TurnDone {
        summary: String,
        stats: String,
    },
    Stats(String),
    Blank,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavigationMode {
    FollowTail,
    BrowsedHistory,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewMode {
    Closed,
    Transcript,
}

pub struct AgentHandle {
    pub(crate) agent: Option<Agent>,
    pub(crate) session_id: Uuid,
    #[allow(dead_code)]
    pub agent_id: String,
    #[allow(dead_code)]
    pub label: String,
    pub(crate) roles: Vec<String>,
}

#[cfg(feature = "stylos")]
#[derive(Clone, Debug)]
struct AgentStatusSource {
    agent_id: String,
    label: String,
    roles: Vec<String>,
    session_id: String,
    workflow: WorkflowState,
    activity_status: String,
    activity_status_changed_at_ms: u64,
    project_dir: PathBuf,
    provider: String,
    model: String,
    active_profile: String,
    rate_limits: Option<ApiCallRateLimitReport>,
}

#[cfg(feature = "stylos")]
fn has_role(handle: &AgentHandle, role: &str) -> bool {
    handle.roles.iter().any(|r| r == role)
}

fn is_interactive_handle(handle: &AgentHandle) -> bool {
    handle.roles.iter().any(|r| r == "interactive")
}

#[cfg(feature = "stylos")]
fn validate_agent_roles(agents: &[AgentHandle]) -> anyhow::Result<()> {
    let main_count = agents.iter().filter(|h| has_role(h, "main")).count();
    if main_count != 1 {
        anyhow::bail!(
            "invalid agent roles: expected exactly one main agent, found {}",
            main_count
        );
    }
    let interactive_count = agents.iter().filter(|h| has_role(h, "interactive")).count();
    if interactive_count > 1 {
        anyhow::bail!(
            "invalid agent roles: expected at most one interactive agent, found {}",
            interactive_count
        );
    }
    Ok(())
}

#[cfg(feature = "stylos")]
fn build_stylos_status_snapshot(
    startup_project_dir: &std::path::Path,
    agent_sources: Vec<AgentStatusSource>,
) -> anyhow::Result<crate::stylos::StylosStatusSnapshot> {
    let main_count = agent_sources
        .iter()
        .filter(|agent| agent.roles.iter().any(|r| r == "main"))
        .count();
    if main_count != 1 {
        anyhow::bail!(
            "invalid agent roles: expected exactly one main agent, found {}",
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
            let git_status =
                crate::stylos::GitStatusCache::new(agent.project_dir.clone()).snapshot();
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

#[derive(Clone, Copy, Default)]
struct UiDirty {
    conversation: bool,
    input: bool,
    status: bool,
    overlay: bool,
    full: bool,
}

impl UiDirty {
    fn any(&self) -> bool {
        self.full || self.conversation || self.input || self.status || self.overlay
    }

    fn mark_all(&mut self) {
        self.full = true;
        self.conversation = true;
        self.input = true;
        self.status = true;
        self.overlay = true;
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone)]
pub(crate) struct FrameRequester {
    tx: mpsc::UnboundedSender<Instant>,
}

impl FrameRequester {
    pub(crate) fn new(draw_tx: broadcast::Sender<()>, domain: &DomainHandle) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        domain.spawn(FrameScheduler::new(rx, draw_tx).run());
        Self { tx }
    }

    fn schedule_frame(&self) {
        let _ = self.tx.send(Instant::now());
    }
}

struct FrameScheduler {
    rx: mpsc::UnboundedReceiver<Instant>,
    draw_tx: broadcast::Sender<()>,
    last_emitted_at: Option<Instant>,
}

impl FrameScheduler {
    fn new(rx: mpsc::UnboundedReceiver<Instant>, draw_tx: broadcast::Sender<()>) -> Self {
        Self {
            rx,
            draw_tx,
            last_emitted_at: None,
        }
    }

    fn clamp_deadline(&self, requested: Instant) -> Instant {
        const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);
        match self.last_emitted_at {
            Some(last) => requested.max(last + MIN_FRAME_INTERVAL),
            None => requested,
        }
    }

    async fn run(mut self) {
        const ONE_YEAR: Duration = Duration::from_secs(60 * 60 * 24 * 365);
        let mut next_deadline: Option<Instant> = None;
        loop {
            let target = next_deadline.unwrap_or_else(|| Instant::now() + ONE_YEAR);
            let deadline = tokio::time::sleep_until(target.into());
            tokio::pin!(deadline);
            tokio::select! {
                requested = self.rx.recv() => {
                    let Some(requested) = requested else { break; };
                    let requested = self.clamp_deadline(requested);
                    next_deadline = Some(next_deadline.map_or(requested, |current| current.min(requested)));
                }
                _ = &mut deadline => {
                    if next_deadline.is_some() {
                        next_deadline = None;
                        self.last_emitted_at = Some(target);
                        let _ = self.draw_tx.send(());
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
enum AgentActivity {
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

#[derive(Clone, Copy)]
struct ActivityCountersSnapshot {
    draw_count: u64,
    draw_request_count: u64,
    draw_skip_clean_count: u64,
    tick_count: u64,
    input_key_count: u64,
    input_mouse_count: u64,
    input_paste_count: u64,
    agent_event_count: u64,
    incoming_prompt_count: u64,
    shell_complete_count: u64,
    agent_turn_started_count: u64,
    agent_turn_completed_count: u64,
    draw_total_us: u64,
    draw_max_us: u64,
    command_count: u64,
}

#[derive(Default)]
struct ActivityCounters {
    draw_count: u64,
    draw_request_count: u64,
    draw_skip_clean_count: u64,
    tick_count: u64,
    input_key_count: u64,
    input_mouse_count: u64,
    input_paste_count: u64,
    agent_event_count: u64,
    incoming_prompt_count: u64,
    shell_complete_count: u64,
    agent_turn_started_count: u64,
    agent_turn_completed_count: u64,
    draw_total_us: u64,
    draw_max_us: u64,
    command_count: u64,
}

impl ActivityCounters {
    fn snapshot(&self) -> ActivityCountersSnapshot {
        ActivityCountersSnapshot {
            draw_count: self.draw_count,
            draw_request_count: self.draw_request_count,
            draw_skip_clean_count: self.draw_skip_clean_count,
            tick_count: self.tick_count,
            input_key_count: self.input_key_count,
            input_mouse_count: self.input_mouse_count,
            input_paste_count: self.input_paste_count,
            agent_event_count: self.agent_event_count,
            incoming_prompt_count: self.incoming_prompt_count,
            shell_complete_count: self.shell_complete_count,
            agent_turn_started_count: self.agent_turn_started_count,
            agent_turn_completed_count: self.agent_turn_completed_count,
            draw_total_us: self.draw_total_us,
            draw_max_us: self.draw_max_us,
            command_count: self.command_count,
        }
    }
}

impl ActivityCountersSnapshot {
    fn saturating_sub(&self, earlier: &Self) -> Self {
        Self {
            draw_count: self.draw_count.saturating_sub(earlier.draw_count),
            draw_request_count: self
                .draw_request_count
                .saturating_sub(earlier.draw_request_count),
            draw_skip_clean_count: self
                .draw_skip_clean_count
                .saturating_sub(earlier.draw_skip_clean_count),
            tick_count: self.tick_count.saturating_sub(earlier.tick_count),
            input_key_count: self.input_key_count.saturating_sub(earlier.input_key_count),
            input_mouse_count: self
                .input_mouse_count
                .saturating_sub(earlier.input_mouse_count),
            input_paste_count: self
                .input_paste_count
                .saturating_sub(earlier.input_paste_count),
            agent_event_count: self
                .agent_event_count
                .saturating_sub(earlier.agent_event_count),
            incoming_prompt_count: self
                .incoming_prompt_count
                .saturating_sub(earlier.incoming_prompt_count),
            shell_complete_count: self
                .shell_complete_count
                .saturating_sub(earlier.shell_complete_count),
            agent_turn_started_count: self
                .agent_turn_started_count
                .saturating_sub(earlier.agent_turn_started_count),
            agent_turn_completed_count: self
                .agent_turn_completed_count
                .saturating_sub(earlier.agent_turn_completed_count),
            draw_total_us: self.draw_total_us.saturating_sub(earlier.draw_total_us),
            draw_max_us: self.draw_max_us.max(earlier.draw_max_us),
            command_count: self.command_count.saturating_sub(earlier.command_count),
        }
    }
}

#[derive(Clone, Copy)]
struct RuntimeMetricsSnapshot {
    at_ms: u64,
    uptime_ms: u64,
    counters: ActivityCountersSnapshot,
}

#[derive(Clone, Copy)]
struct TimedRuntimeDelta {
    latest_at_ms: u64,
    latest_uptime_ms: u64,
    wall_elapsed_ms: u64,
    counter_delta: ActivityCountersSnapshot,
    lifetime_counters: ActivityCountersSnapshot,
}

#[cfg(feature = "stylos")]
#[derive(Clone, Copy)]
pub(crate) struct StylosActivitySnapshot {
    pub(crate) status_publish_count: u64,
    pub(crate) status_publish_total_us: u64,
    pub(crate) status_publish_max_us: u64,
    pub(crate) query_request_count: u64,
    pub(crate) query_request_total_us: u64,
    pub(crate) query_request_max_us: u64,
    pub(crate) cmd_event_count: u64,
    pub(crate) prompt_event_count: u64,
    pub(crate) event_message_count: u64,
}

impl AgentActivity {
    fn label(&self, stream_chunks: u64, stream_chars: u64) -> String {
        match self {
            AgentActivity::PreparingRequest => "preparing request…".to_string(),
            AgentActivity::WaitingForModel => "waiting for model…".to_string(),
            AgentActivity::StreamingResponse => format!(
                "receiving response… chunks:{} chars:{}",
                stream_chunks, stream_chars
            ),
            AgentActivity::RunningTool(detail) => format!("running tool… {}", detail),
            AgentActivity::WaitingAfterTool => "tool finished, waiting for model…".to_string(),
            AgentActivity::LoginStarting => "starting login…".to_string(),
            AgentActivity::WaitingForLoginBrowser => "waiting for login confirmation…".to_string(),
            AgentActivity::RunningShellCommand => "running shell command…".to_string(),
            AgentActivity::Finishing => "finalizing…".to_string(),
        }
    }

    fn status_bar(&self, stream_chunks: u64, stream_chars: u64) -> String {
        match self {
            AgentActivity::PreparingRequest => "preparing".to_string(),
            AgentActivity::WaitingForModel => "waiting-model".to_string(),
            AgentActivity::StreamingResponse => {
                format!("streaming c:{} ch:{}", stream_chunks, stream_chars)
            }
            AgentActivity::RunningTool(_) => "running-tool".to_string(),
            AgentActivity::WaitingAfterTool => "waiting-after-tool".to_string(),
            AgentActivity::LoginStarting => "login-start".to_string(),
            AgentActivity::WaitingForLoginBrowser => "login-wait".to_string(),
            AgentActivity::RunningShellCommand => "shell".to_string(),
            AgentActivity::Finishing => "finalizing".to_string(),
        }
    }
}

pub struct App<'a> {
    #[cfg(feature = "stylos")]
    pub(crate) stylos: Option<StylosHandle>,
    #[cfg(feature = "stylos")]
    local_stylos_instance: Option<String>,
    session: Session,
    entries: Vec<Entry>,
    pending: Option<String>,
    input: TextArea<'a>,
    paste_burst: PasteBurst,
    pub(crate) running: bool,
    agent_busy: bool,
    scroll_offset: usize,
    navigation_mode: NavigationMode,
    review_mode: ReviewMode,
    review_scroll_offset: usize,
    history: Vec<String>,
    history_pos: Option<usize>,
    history_draft: String,
    streaming_idx: Option<usize>,
    anim_frame: u8,
    dirty: UiDirty,
    agents: Vec<AgentHandle>,
    db: Arc<DbHandle>,
    project_dir: PathBuf,
    #[allow(dead_code)]
    startup_project_dir: PathBuf,
    session_tokens: TurnStats,
    last_ctx_tokens: u64,
    agent_activity: Option<AgentActivity>,
    idle_since: Option<Instant>,
    idle_status_changed_at: Option<u64>,
    agent_activity_changed_at: Option<u64>,
    stream_chunks: u64,
    stream_chars: u64,
    status_rate_limits: Option<ApiCallRateLimitReport>,
    status_model_info: Option<ModelInfo>,
    process_started_at: Instant,
    process_started_at_ms: u64,
    background_domain: DomainHandle,
    core_domain: DomainHandle,
    recent_runtime_snapshots: VecDeque<RuntimeMetricsSnapshot>,
    activity_counters: ActivityCounters,
    workflow_state: WorkflowState,
    active_turn_cancellation: Option<TurnCancellation>,
    #[cfg(feature = "stylos")]
    active_incoming_prompt: Option<IncomingPromptRequest>,
    #[cfg(feature = "stylos")]
    last_assistant_text: Option<String>,
    #[cfg(feature = "stylos")]
    stylos_tool_bridge: Option<StylosToolBridge>,
}

impl<'a> App<'a> {
    pub fn new(
        session: Session,
        db: Arc<DbHandle>,
        session_id: Uuid,
        project_dir: PathBuf,
        background_domain: DomainHandle,
        core_domain: DomainHandle,
        #[cfg(feature = "stylos")] stylos: Option<StylosHandle>,
    ) -> Self {
        #[cfg(feature = "stylos")]
        let stylos_tool_bridge = stylos.as_ref().and_then(tool_bridge);
        #[cfg(feature = "stylos")]
        let local_stylos_instance = stylos.as_ref().and_then(|handle| match handle.state() {
            StylosRuntimeState::Active { instance, .. } => Some(instance.clone()),
            _ => Some(crate::stylos::derive_local_instance_id()),
        });
        let agent = crate::app_runtime::build_agent(
            &session,
            session_id,
            project_dir.clone(),
            db.clone(),
            #[cfg(feature = "stylos")]
            stylos_tool_bridge.clone(),
            #[cfg(feature = "stylos")]
            local_stylos_instance.as_deref(),
            #[cfg(feature = "stylos")]
            "main",
            None,
        )
        .expect("failed to build agent");
        let initial_model_info = session.model_info.clone();
        let handle = AgentHandle {
            agent: Some(agent),
            session_id,
            agent_id: "main".to_string(),
            label: "main".to_string(),
            roles: vec!["main".to_string(), "interactive".to_string()],
        };

        let art = concat!(
            "████████╗██╗  ██╗███████╗███╗   ███╗██╗ ██████╗ ███╗   ██╗\n",
            "╚══██╔══╝██║  ██║██╔════╝████╗ ████║██║██╔═══██╗████╗  ██║\n",
            "   ██║   ███████║█████╗  ██╔████╔██║██║██║   ██║██╔██╗ ██║\n",
            "   ██║   ██╔══██║██╔══╝  ██║╚██╔╝██║██║██║   ██║██║╚██╗██║\n",
            "   ██║   ██║  ██║███████╗██║ ╚═╝ ██║██║╚██████╔╝██║ ╚████║\n",
            "   ╚═╝   ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝╚═╝ ╚═════╝ ╚═╝  ╚═══╝",
        );
        let project_display = project_dir.display().to_string();
        #[allow(unused_mut)]
        let mut initial_entries = vec![
            Entry::Blank,
            Entry::Banner(art.to_string()),
            Entry::Blank,
            Entry::Assistant(format!(
                "version: {}  |  profile: {}  |  model: {}",
                env!("CARGO_PKG_VERSION"),
                session.active_profile,
                session.model,
            )),
            Entry::Assistant(format!("project directory: {}", project_display)),
            Entry::Assistant(
                "type /config to change settings, /exit to quit, Alt-t transcript review"
                    .to_string(),
            ),
            Entry::Blank,
        ];

        #[cfg(feature = "stylos")]
        if let Some(handle) = stylos.as_ref() {
            match handle.state() {
                StylosRuntimeState::Off => {
                    initial_entries.push(Entry::Status("stylos disabled".to_string()))
                }
                StylosRuntimeState::Active {
                    mode,
                    realm,
                    instance,
                } => initial_entries.push(Entry::Status(format!(
                    "stylos ready: mode={} realm={} instance={}",
                    mode, realm, instance
                ))),
                StylosRuntimeState::Error(err) => {
                    initial_entries.push(Entry::Status(format!("stylos start failed: {}", err)))
                }
            }
            initial_entries.push(Entry::Blank);
        }

        let mut app = Self {
            #[cfg(feature = "stylos")]
            stylos,
            #[cfg(feature = "stylos")]
            local_stylos_instance,
            session,
            entries: initial_entries,
            pending: None,
            input: make_input(),
            paste_burst: PasteBurst::default(),
            running: true,
            agent_busy: false,
            scroll_offset: 0,
            navigation_mode: NavigationMode::FollowTail,
            review_mode: ReviewMode::Closed,
            review_scroll_offset: 0,
            history: Vec::new(),
            history_pos: None,
            history_draft: String::new(),
            streaming_idx: None,
            anim_frame: 0,
            dirty: {
                let mut d = UiDirty::default();
                d.mark_all();
                d
            },
            agents: vec![handle],
            db,
            startup_project_dir: project_dir.clone(),
            project_dir,
            session_tokens: TurnStats {
                llm_rounds: 0,
                tool_calls: 0,
                tokens_in: 0,
                tokens_out: 0,
                tokens_cached: 0,
                elapsed_ms: 0,
            },
            last_ctx_tokens: 0,
            agent_activity: None,
            idle_since: Some(Instant::now()),
            idle_status_changed_at: Some(unix_epoch_now_ms()),
            agent_activity_changed_at: None,
            stream_chunks: 0,
            stream_chars: 0,
            status_rate_limits: None,
            status_model_info: initial_model_info,
            process_started_at: Instant::now(),
            process_started_at_ms: unix_epoch_now_ms(),
            background_domain,
            core_domain,
            recent_runtime_snapshots: VecDeque::new(),
            activity_counters: ActivityCounters::default(),
            workflow_state: WorkflowState::default(),
            active_turn_cancellation: None,
            #[cfg(feature = "stylos")]
            active_incoming_prompt: None,
            #[cfg(feature = "stylos")]
            last_assistant_text: None,
            #[cfg(feature = "stylos")]
            stylos_tool_bridge,
        };
        app.record_runtime_snapshot();
        app.refresh_main_agent_system_inspection();
        app
    }

    #[cfg(feature = "stylos")]
    #[allow(dead_code)]
    fn interactive_agent_mut(&mut self) -> Option<&mut AgentHandle> {
        self.agents.iter_mut().find(|h| has_role(h, "interactive"))
    }

    #[cfg(feature = "stylos")]
    #[allow(dead_code)]
    fn main_agent_mut(&mut self) -> Option<&mut AgentHandle> {
        self.agents.iter_mut().find(|h| has_role(h, "main"))
    }

    fn background_domain(&self) -> DomainHandle {
        self.background_domain.clone()
    }

    fn enter_browsed_history(&mut self) {
        self.navigation_mode = NavigationMode::BrowsedHistory;
    }

    fn return_to_latest(&mut self) {
        self.scroll_offset = 0;
        self.review_scroll_offset = 0;
        self.navigation_mode = NavigationMode::FollowTail;
        self.review_mode = ReviewMode::Closed;
    }

    fn open_transcript_review(&mut self) {
        self.review_mode = ReviewMode::Transcript;
        self.navigation_mode = NavigationMode::BrowsedHistory;
        self.review_scroll_offset = 0;
    }

    fn close_transcript_review(&mut self) {
        self.review_mode = ReviewMode::Closed;
    }

    fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let new_pos = match self.history_pos {
            None => {
                self.history_draft = self.input.lines().join("\n");
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_pos = Some(new_pos);
        set_input_text(&mut self.input, &self.history[new_pos].clone());
    }

    fn history_down(&mut self) {
        match self.history_pos {
            None => {}
            Some(i) if i + 1 < self.history.len() => {
                self.history_pos = Some(i + 1);
                let text = self.history[i + 1].clone();
                set_input_text(&mut self.input, &text);
            }
            Some(_) => {
                self.history_pos = None;
                let draft = self.history_draft.clone();
                set_input_text(&mut self.input, &draft);
            }
        }
    }

    fn pending_str(&self) -> String {
        const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let ch = SPINNER[self.anim_frame as usize % SPINNER.len()];
        let activity = self
            .agent_activity
            .as_ref()
            .map(|p| p.label(self.stream_chunks, self.stream_chars))
            .unwrap_or_else(|| "thinking…".to_string());
        format!("  {} {}", ch, activity)
    }

    fn set_agent_activity(&mut self, activity: AgentActivity) {
        let activity_changed = self
            .agent_activity
            .as_ref()
            .map(|current| std::mem::discriminant(current) != std::mem::discriminant(&activity))
            .unwrap_or(true);
        self.agent_activity = Some(activity);
        if activity_changed {
            self.agent_activity_changed_at = Some(unix_epoch_now_ms());
        }
        self.idle_since = None;
        self.idle_status_changed_at = None;
        self.pending = Some(self.pending_str());
        self.mark_dirty_status();
        self.refresh_stylos_status();
    }

    fn clear_agent_activity(&mut self) {
        self.agent_activity = None;
        self.agent_activity_changed_at = None;
        self.idle_since = Some(Instant::now());
        self.idle_status_changed_at = Some(unix_epoch_now_ms());
        self.pending = None;
        self.mark_dirty_status();
        self.refresh_stylos_status();
    }

    fn reset_stream_counters(&mut self) {
        self.stream_chunks = 0;
        self.stream_chars = 0;
    }

    fn request_interrupt(&mut self) {
        if let Some(cancel) = &self.active_turn_cancellation {
            if !cancel.is_interrupted() {
                cancel.interrupt();
                self.push(Entry::Status("interrupt requested".to_string()));
            }
        }
    }

    fn on_tick(&mut self) {
        self.activity_counters.tick_count += 1;
        self.record_runtime_snapshot();
        self.refresh_main_agent_system_inspection();
        let previous = self.pending.clone();
        self.anim_frame = self.anim_frame.wrapping_add(1);
        if self.agent_busy && self.pending.is_some() {
            self.pending = Some(self.pending_str());
        }
        if self.pending != previous {
            self.mark_dirty_status();
        }
    }

    fn mark_dirty_conversation(&mut self) {
        self.dirty.conversation = true;
    }

    fn mark_dirty_input(&mut self) {
        self.dirty.input = true;
    }

    fn mark_dirty_status(&mut self) {
        self.dirty.status = true;
    }

    fn mark_dirty_overlay(&mut self) {
        self.dirty.overlay = true;
    }

    fn mark_dirty_all(&mut self) {
        self.dirty.mark_all();
    }

    pub(crate) fn request_draw(&mut self, frame_requester: &FrameRequester) {
        self.activity_counters.draw_request_count += 1;
        frame_requester.schedule_frame();
    }

    pub(crate) fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running
    }

    pub(crate) fn finish_initial_draw(&mut self, frame_requester: &FrameRequester) {
        self.clear_dirty();
        self.request_draw(frame_requester);
    }

    #[cfg(feature = "stylos")]
    pub(crate) fn shutdown_stylos(&mut self) -> Option<StylosHandle> {
        self.stylos.take()
    }

    fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        self.mark_dirty_conversation();
    }

    fn activity_status_value(&self) -> String {
        const NAP_AFTER: Duration = Duration::from_secs(5 * 60);

        if let Some(activity) = self.agent_activity.as_ref() {
            return activity.status_bar(self.stream_chunks, self.stream_chars);
        }

        match self.idle_since {
            Some(idle_since) if idle_since.elapsed() > NAP_AFTER => "nap".to_string(),
            _ => "idle".to_string(),
        }
    }

    #[cfg(feature = "stylos")]
    pub(crate) fn wire_stylos_event_streams(
        &mut self,
        tui_domain: &DomainHandle,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        if let Some(handle) = self.stylos.as_mut() {
            if let Some(mut cmd_rx) = handle.take_cmd_rx() {
                let app_tx_cmd = app_tx.clone();
                tui_domain.spawn(async move {
                    while let Some(cmd) = cmd_rx.recv().await {
                        let _ = app_tx_cmd.send(AppEvent::StylosCmd(cmd));
                    }
                });
            }
            if let Some(mut prompt_rx) = handle.take_prompt_rx() {
                let app_tx_prompt = app_tx.clone();
                tui_domain.spawn(async move {
                    while let Some(request) = prompt_rx.recv().await {
                        let _ = app_tx_prompt.send(AppEvent::IncomingPrompt(request));
                    }
                });
            }
            if let Some(mut event_rx) = handle.take_event_rx() {
                let app_tx_event = app_tx.clone();
                tui_domain.spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        let _ = app_tx_event.send(AppEvent::StylosEvent(event));
                    }
                });
            }
        }
    }

    pub(crate) fn refresh_stylos_status(&self) {
        #[cfg(feature = "stylos")]
        if self.stylos.is_some() {
            if validate_agent_roles(&self.agents).is_err() {
                return;
            }
            let startup_project_dir = self.startup_project_dir.clone();
            let rate_limits = self.status_rate_limits.clone();
            let idle_since = self.idle_since;
            let idle_status_changed_at = self.idle_status_changed_at;
            let agent_activity = self.agent_activity.clone();
            let agent_activity_changed_at = self.agent_activity_changed_at;
            let stream_chunks = self.stream_chunks;
            let stream_chars = self.stream_chars;

            let agent_sources: Vec<AgentStatusSource> = self
                .agents
                .iter()
                .enumerate()
                .map(|(idx, h)| {
                    let (activity_status, activity_status_changed_at_ms) = if idx == 0 {
                        if let Some(activity) = agent_activity.as_ref() {
                            (
                                activity.status_bar(stream_chunks, stream_chars),
                                agent_activity_changed_at.unwrap_or_else(unix_epoch_now_ms),
                            )
                        } else {
                            const NAP_AFTER: Duration = Duration::from_secs(5 * 60);
                            match idle_since {
                                Some(idle_since) if idle_since.elapsed() > NAP_AFTER => (
                                    "nap".to_string(),
                                    idle_status_changed_at.unwrap_or_else(unix_epoch_now_ms)
                                        + NAP_AFTER.as_millis() as u64,
                                ),
                                _ => (
                                    "idle".to_string(),
                                    idle_status_changed_at.unwrap_or_else(unix_epoch_now_ms),
                                ),
                            }
                        }
                    } else {
                        ("idle".to_string(), unix_epoch_now_ms())
                    };

                    let workflow = h
                        .agent
                        .as_ref()
                        .map(|agent| agent.workflow_state().clone())
                        .unwrap_or_else(|| {
                            if idx == 0 {
                                self.workflow_state.clone()
                            } else {
                                WorkflowState::default()
                            }
                        });

                    AgentStatusSource {
                        agent_id: h.agent_id.clone(),
                        label: h.label.clone(),
                        roles: h.roles.clone(),
                        session_id: h.session_id.to_string(),
                        workflow,
                        activity_status,
                        activity_status_changed_at_ms,
                        project_dir: h
                            .agent
                            .as_ref()
                            .map(|agent| agent.project_dir.clone())
                            .unwrap_or_else(|| self.project_dir.clone()),
                        provider: self.session.provider.clone(),
                        model: self.session.model.clone(),
                        active_profile: self.session.active_profile.clone(),
                        rate_limits: if idx == 0 { rate_limits.clone() } else { None },
                    }
                })
                .collect();

            let provider = std::sync::Arc::new(move || {
                let startup_project_dir = startup_project_dir.clone();
                let agent_sources = agent_sources.clone();
                Box::pin(async move {
                    build_stylos_status_snapshot(&startup_project_dir, agent_sources)
                        .unwrap_or_else(|_| crate::stylos::StylosStatusSnapshot {
                            startup_project_dir: startup_project_dir.display().to_string(),
                            agents: Vec::new(),
                        })
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<Output = crate::stylos::StylosStatusSnapshot>
                                + Send,
                        >,
                    >
            });
            if let Some(handle) = self.stylos.as_ref() {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(handle.set_snapshot_provider(provider));
                });
            }
        }
    }

    fn process_agent_event(
        &mut self,
        ev: AgentEvent,
        #[cfg(feature = "stylos")] app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        match ev {
            AgentEvent::LlmStart => {
                #[cfg(feature = "stylos")]
                if let (Some(remote), Some(handle)) =
                    (self.active_incoming_prompt.as_ref(), self.stylos.as_ref())
                {
                    if let Some(task_id) = remote.task_id.clone() {
                        let query_context = handle.query_context();
                        self.background_domain.spawn(async move {
                            query_context.task_registry().set_running(&task_id).await;
                        });
                    }
                }
                self.reset_stream_counters();
                #[cfg(feature = "stylos")]
                {
                    self.last_assistant_text = None;
                }
                self.set_agent_activity(AgentActivity::WaitingForModel);
                self.streaming_idx = None;
            }
            AgentEvent::AssistantChunk(chunk) => {
                #[cfg(feature = "stylos")]
                {
                    let next = match self.last_assistant_text.take() {
                        Some(mut existing) => {
                            existing.push_str(&chunk);
                            existing
                        }
                        None => chunk.clone(),
                    };
                    self.last_assistant_text = Some(next);
                }
                self.stream_chunks += 1;
                self.stream_chars += chunk.chars().count() as u64;
                self.set_agent_activity(AgentActivity::StreamingResponse);
                match self.streaming_idx {
                    Some(i) => {
                        if let Some(Entry::Assistant(ref mut text)) = self.entries.get_mut(i) {
                            text.push_str(&chunk);
                        }
                    }
                    None => {
                        self.push(Entry::Assistant(chunk));
                        self.streaming_idx = Some(self.entries.len() - 1);
                    }
                }
            }
            AgentEvent::AssistantText(text) => {
                #[cfg(feature = "stylos")]
                {
                    self.last_assistant_text = Some(text.clone());
                }
                self.streaming_idx = None;
                self.clear_agent_activity();
                self.push(Entry::Assistant(text));
            }
            AgentEvent::ToolStart { detail } => {
                self.streaming_idx = None;
                self.set_agent_activity(AgentActivity::RunningTool(detail.clone()));
                self.push(Entry::ToolCall(detail));
            }
            AgentEvent::ToolEnd => {
                self.push(Entry::ToolDone);
                #[cfg(feature = "stylos")]
                self.maybe_log_sender_side_stylos_talk();
                self.set_agent_activity(AgentActivity::WaitingAfterTool);
            }
            AgentEvent::Status(text) => {
                self.push(Entry::Status(text));
            }
            AgentEvent::WorkflowStateChanged(state) => {
                self.workflow_state = state;
                self.mark_dirty_status();
            }
            AgentEvent::Stats(text) => {
                if let Some(json) = text.strip_prefix("[rate-limit] ") {
                    if let Ok(report) = serde_json::from_str::<ApiCallRateLimitReport>(json) {
                        self.status_rate_limits = Some(report);
                        self.mark_dirty_status();
                    }
                    return;
                }
                self.push(Entry::Stats(text));
            }
            AgentEvent::TurnDone(stats) => {
                #[cfg(feature = "stylos")]
                self.maybe_emit_done_mention_for_completed_note(app_tx);
                #[cfg(feature = "stylos")]
                if let (Some(remote), Some(handle)) =
                    (self.active_incoming_prompt.take(), self.stylos.as_ref())
                {
                    if let Some(task_id) = remote.task_id {
                        let result_text = self.last_assistant_text.clone();
                        let query_context = handle.query_context();
                        self.background_domain().spawn(async move {
                            query_context
                                .task_registry()
                                .set_completed(&task_id, result_text, None)
                                .await;
                        });
                    }
                }
                self.streaming_idx = None;
                self.set_agent_activity(AgentActivity::Finishing);
                self.clear_agent_activity();
                let interrupted = self.workflow_state.status
                    == themion_core::workflow::WorkflowStatus::Interrupted;
                let stats_text = format_stats(&stats);
                let stats_text = stats_text
                    .strip_prefix("[stats: ")
                    .and_then(|s| s.strip_suffix("]"))
                    .unwrap_or(&stats_text)
                    .to_string();
                self.push(Entry::TurnDone {
                    summary: if interrupted {
                        "󰇺 Turn interrupted".to_string()
                    } else {
                        "󰇺 Turn end".to_string()
                    },
                    stats: stats_text,
                });
                self.push(Entry::Blank);
                self.activity_counters.agent_turn_completed_count += 1;
                self.agent_busy = false;
                self.active_turn_cancellation = None;
                self.last_ctx_tokens = stats.tokens_in;
                self.session_tokens.tokens_in += stats.tokens_in;
                self.session_tokens.tokens_out += stats.tokens_out;
                self.session_tokens.tokens_cached += stats.tokens_cached;
                self.session_tokens.llm_rounds += stats.llm_rounds;
                self.session_tokens.tool_calls += stats.tool_calls;
                self.session_tokens.elapsed_ms += stats.elapsed_ms;
                self.reset_stream_counters();
                #[cfg(feature = "stylos")]
                {
                    self.last_assistant_text = None;
                }
            }
        }
    }

    #[cfg(feature = "stylos")]
    fn maybe_log_sender_side_stylos_talk(&mut self) {
        if self.active_incoming_prompt.is_some() {
            return;
        }
        let Some(Entry::ToolDone) = self.entries.last() else {
            return;
        };
        let Some(Entry::ToolCall(detail)) = self.entries.iter().rev().nth(1) else {
            return;
        };
        let Some(handle) = self.stylos.as_ref() else {
            return;
        };
        let local_instance = match handle.state() {
            StylosRuntimeState::Active { instance, .. } => instance.as_str(),
            _ => return,
        };

        if detail.starts_with("stylos_request_talk") {
            if let Some(target) = extract_stylos_talk_target_from_detail(detail) {
                self.push(Entry::RemoteEvent(format!(
                    "Stylos talk to={} from={}",
                    target, local_instance,
                )));
            }
            return;
        }

        if detail.starts_with("board_create_note") {
            let mode = if self.stylos_tool_bridge.is_some() {
                "send request via stylos"
            } else {
                "create local"
            };
            self.push(Entry::RemoteEvent(format!(
                "board_create_note {} from={} detail={}",
                mode, local_instance, detail
            )));
        }
    }

    fn current_runtime_snapshot(&self) -> RuntimeMetricsSnapshot {
        RuntimeMetricsSnapshot {
            at_ms: unix_epoch_now_ms(),
            uptime_ms: self.process_started_at.elapsed().as_millis() as u64,
            counters: self.activity_counters.snapshot(),
        }
    }

    fn record_runtime_snapshot(&mut self) {
        let snapshot = self.current_runtime_snapshot();
        self.recent_runtime_snapshots.push_back(snapshot);
        while self.recent_runtime_snapshots.len() > 16 {
            self.recent_runtime_snapshots.pop_front();
        }
    }

    fn recent_runtime_delta(&self) -> Option<TimedRuntimeDelta> {
        let latest = *self.recent_runtime_snapshots.back()?;
        let earliest = *self.recent_runtime_snapshots.front()?;
        if latest.at_ms <= earliest.at_ms {
            return None;
        }
        Some(TimedRuntimeDelta {
            latest_at_ms: latest.at_ms,
            latest_uptime_ms: latest.uptime_ms,
            wall_elapsed_ms: latest.at_ms.saturating_sub(earliest.at_ms),
            counter_delta: latest.counters.saturating_sub(&earliest.counters),
            lifetime_counters: latest.counters,
        })
    }

    fn system_inspection_snapshot(&self) -> themion_core::tools::SystemInspectionResult {
        let runtime_lines = self.debug_runtime_lines();
        let rate_limits = self.status_rate_limits.as_ref().map(|report| {
            themion_core::tools::SystemInspectionRateLimits {
                api_call: report.api_call.clone(),
                source: report.source.clone(),
                http_status: report.http_status,
                active_limit: report.active_limit.clone(),
                snapshot_count: report.snapshots.len(),
            }
        });
        let mut provider = themion_core::tools::SystemInspectionProvider {
            status: "ok".to_string(),
            active_profile: Some(self.session.active_profile.clone()),
            provider: Some(self.session.provider.clone()),
            model: Some(self.session.model.clone()),
            auth_configured: Some(match self.session.provider.as_str() {
                "openai-codex" => crate::auth_store::load().ok().flatten().is_some(),
                _ => self
                    .session
                    .api_key
                    .as_ref()
                    .map(|v| !v.is_empty())
                    .unwrap_or(false),
            }),
            base_url_present: Some(!self.session.base_url.trim().is_empty()),
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
        let tools = themion_core::tools::SystemInspectionTools {
            status: "ok".to_string(),
            tool_count: tool_names.len(),
            available_names: tool_names,
            warnings: Vec::new(),
            issues: Vec::new(),
        };
        let runtime = themion_core::tools::SystemInspectionRuntime {
            status: "ok".to_string(),
            pid: Some(std::process::id()),
            now_ms: unix_epoch_now_ms(),
            session_id: self
                .agents
                .first()
                .map(|h| h.session_id.to_string())
                .unwrap_or_else(|| self.session.id.to_string()),
            project_dir: self.project_dir.display().to_string(),
            workflow_name: Some(self.workflow_state.workflow_name.clone()),
            phase_name: Some(self.workflow_state.phase_name.clone()),
            workflow_status: Some(format!("{:?}", self.workflow_state.status)),
            debug_runtime_lines: runtime_lines,
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
        themion_core::tools::SystemInspectionResult {
            overall_status,
            summary,
            runtime,
            tools,
            provider,
            warnings,
            issues,
        }
    }

    fn refresh_main_agent_system_inspection(&mut self) {
        let inspection = self.system_inspection_snapshot();
        if let Some(handle) = self.agents.iter_mut().find(|h| is_interactive_handle(h)) {
            if let Some(agent) = handle.agent.as_mut() {
                agent.set_system_inspection(Some(inspection));
            }
        }
    }

    fn debug_runtime_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        let now_ms = unix_epoch_now_ms();
        let uptime_ms = self.process_started_at.elapsed().as_millis() as u64;
        out.push("debug runtime snapshot: themion process/thread/task activity".to_string());
        out.push(format!(
            "process pid={} uptime={} started_at_ms={}",
            std::process::id(),
            format_duration_ms(uptime_ms),
            self.process_started_at_ms,
        ));
        out.push(format!(
            "app busy={} activity={} session={} project={}",
            self.agent_busy,
            self.activity_status_value(),
            self.agents
                .first()
                .map(|h| h.session_id.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            self.project_dir.display()
        ));
        out.push(format!(
            "workflow flow={} phase={} status={}",
            self.workflow_state.workflow_name,
            self.workflow_state.phase_name,
            format!("{:?}", self.workflow_state.status)
        ));
        #[cfg(feature = "stylos")]
        {
            let stylos_state = match self.stylos.as_ref().map(|h| h.state()) {
                Some(StylosRuntimeState::Off) => "off".to_string(),
                Some(StylosRuntimeState::Active {
                    mode,
                    realm,
                    instance,
                }) => format!("active mode={} realm={} instance={}", mode, realm, instance),
                Some(StylosRuntimeState::Error(err)) => format!("error {}", err),
                None => "off".to_string(),
            };
            out.push(format!("stylos {}", stylos_state));
        }
        #[cfg(not(feature = "stylos"))]
        out.push("stylos feature disabled".to_string());

        out.push("threads:".to_string());
        out.extend(
            sample_thread_cpu_lines()
                .into_iter()
                .map(|line| format!("  {}", line)),
        );

        if let Some(recent) = self.recent_runtime_delta() {
            out.push(format!(
                "recent window={} ending_at_ms={} uptime={}",
                format_duration_ms(recent.wall_elapsed_ms),
                recent.latest_at_ms,
                format_duration_ms(recent.latest_uptime_ms),
            ));
            out.extend(format_runtime_activity_lines(
                &recent.counter_delta,
                recent.wall_elapsed_ms,
            ));
            out.extend(format_runtime_lifetime_lines(&recent.lifetime_counters));
        } else {
            out.push("recent window=unavailable (need more than one sample)".to_string());
            out.extend(format_runtime_lifetime_lines(
                &self.activity_counters.snapshot(),
            ));
        }

        if let Some(changed_at) = self
            .agent_activity_changed_at
            .or(self.idle_status_changed_at)
        {
            out.push(format!(
                "activity_status_changed {} ago",
                format_duration_ms(now_ms.saturating_sub(changed_at))
            ));
        }
        #[cfg(feature = "stylos")]
        if let Some(handle) = self.stylos.as_ref() {
            if let Some(snapshot) = handle.activity_snapshot() {
                out.push("stylos activity:".to_string());
                out.extend(format_stylos_activity_lines(snapshot));
            }
        }
        out
    }

    fn handle_command(
        &mut self,
        input: &str,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) -> Vec<String> {
        let mut out = Vec::new();
        self.activity_counters.command_count += 1;

        if input == "/login codex" {
            if self.agent_busy {
                return vec!["busy, please wait".to_string()];
            }
            self.agent_busy = true;
            self.set_agent_activity(AgentActivity::LoginStarting);
            self.push(Entry::Assistant("logging in to OpenAI Codex…".to_string()));
            let tx = app_tx.clone();
            self.background_domain().spawn(async move {
                match crate::login_codex::start_device_flow().await {
                    Err(e) => {
                        tx.send(AppEvent::LoginComplete(Err(e))).ok();
                    }
                    Ok((info, poll)) => {
                        tx.send(AppEvent::LoginPrompt {
                            user_code: info.user_code,
                            verification_uri: info.verification_uri,
                        })
                        .ok();
                        let result = poll.await;
                        tx.send(AppEvent::LoginComplete(result)).ok();
                    }
                }
            });
            return out;
        }

        if input == "/debug runtime" {
            return self.debug_runtime_lines();
        }

        if input == "/clear" {
            if let Some(handle) = self.agents.iter_mut().find(|h| is_interactive_handle(h)) {
                if let Some(agent) = handle.agent.as_mut() {
                    agent.clear_context();
                }
            }
            self.last_ctx_tokens = 0;
            out.push("ok, future messages in this session will not include chat history before this point".to_string());
            return out;
        }

        if input == "/config" {
            let key_display = match &self.session.api_key {
                Some(k) if k.len() > 8 => format!("{}…", &k[..8]),
                Some(_) => "(set)".to_string(),
                None => "(none)".to_string(),
            };
            out.push(format!("profile  : {}", self.session.active_profile));
            out.push(format!("provider : {}", self.session.provider));
            out.push(format!("model    : {}", self.session.model));
            out.push(format!("endpoint : {}", self.session.base_url));
            out.push(format!("api_key  : {}", key_display));
            return out;
        }

        if let Some(rest) = input.strip_prefix("/config ") {
            let parts: Vec<&str> = rest.splitn(3, ' ').collect();
            match parts.as_slice() {
                ["profile"] | ["profile", "list"] => {
                    let mut names: Vec<String> = self.session.profiles.keys().cloned().collect();
                    names.sort();
                    for name in names {
                        let marker = if name == self.session.active_profile {
                            "* "
                        } else {
                            "  "
                        };
                        out.push(format!("{}{}", marker, name));
                    }
                }
                ["profile", "show"] => {
                    let key_display = match &self.session.api_key {
                        Some(k) if k.len() > 8 => format!("{}…", &k[..8]),
                        Some(_) => "(set)".to_string(),
                        None => "(none)".to_string(),
                    };
                    out.push(format!("profile  : {}", self.session.active_profile));
                    out.push(format!("provider : {}", self.session.provider));
                    out.push(format!("model    : {}", self.session.model));
                    out.push(format!("endpoint : {}", self.session.base_url));
                    out.push(format!("api_key  : {}", key_display));
                }
                ["profile", "create", name] => {
                    let p = ProfileConfig {
                        provider: Some(self.session.provider.clone()),
                        base_url: Some(self.session.base_url.clone()),
                        model: Some(self.session.model.clone()),
                        api_key: self.session.api_key.clone(),
                    };
                    self.session.profiles.insert(name.to_string(), p);
                    self.session.active_profile = name.to_string();
                    if let Err(e) =
                        save_profiles(&self.session.active_profile, &self.session.profiles)
                    {
                        out.push(format!("warning: {}", e));
                    }
                    out.push(format!("profile '{}' created and saved", name));
                }
                ["profile", "use", name] => {
                    if self.session.switch_profile(name) {
                        if let Err(e) =
                            save_profiles(&self.session.active_profile, &self.session.profiles)
                        {
                            out.push(format!("warning: {}", e));
                        }
                        let new_session_id = Uuid::new_v4();
                        match crate::app_runtime::build_agent(
                            &self.session,
                            new_session_id,
                            self.project_dir.clone(),
                            self.db.clone(),
                            #[cfg(feature = "stylos")]
                            self.stylos_tool_bridge.clone(),
                            #[cfg(feature = "stylos")]
                            self.local_stylos_instance.as_deref(),
                            #[cfg(feature = "stylos")]
                            "main",
                            None,
                        ) {
                            Ok(new_agent) => {
                                let db = self.db.clone();
                                let pdir = self.project_dir.clone();
                                let _ = db.insert_session(new_session_id, &pdir, true);
                                self.status_model_info = new_agent.model_info().cloned();
                                self.agents = vec![AgentHandle {
                                    agent: Some(new_agent),
                                    session_id: new_session_id,
                                    agent_id: "main".to_string(),
                                    label: "main".to_string(),
                                    roles: vec!["main".to_string(), "interactive".to_string()],
                                }];
                                out.push(format!(
                                    "switched to profile '{}'  provider={}  model={}",
                                    name, self.session.provider, self.session.model
                                ));
                            }
                            Err(e) => {
                                out.push(format!("error building agent: {}", e));
                            }
                        }
                    } else {
                        let mut names: Vec<String> =
                            self.session.profiles.keys().cloned().collect();
                        names.sort();
                        out.push(format!(
                            "unknown profile '{}'.  available: {}",
                            name,
                            names.join(", ")
                        ));
                    }
                }
                ["profile", "set", kv] => {
                    if let Some((key, val)) = kv.split_once('=') {
                        match key {
                            "provider" => self.session.provider = val.to_string(),
                            "model" => self.session.model = val.to_string(),
                            "endpoint" => self.session.base_url = val.to_string(),
                            "api_key" => self.session.api_key = Some(val.to_string()),
                            _ => {
                                out.push(format!(
                                    "unknown key '{}'.  valid: provider, model, endpoint, api_key",
                                    key
                                ));
                                return out;
                            }
                        }
                        self.session.profiles.insert(
                            self.session.active_profile.clone(),
                            ProfileConfig {
                                provider: Some(self.session.provider.clone()),
                                base_url: Some(self.session.base_url.clone()),
                                model: Some(self.session.model.clone()),
                                api_key: self.session.api_key.clone(),
                            },
                        );
                        if let Err(e) =
                            save_profiles(&self.session.active_profile, &self.session.profiles)
                        {
                            out.push(format!("warning: {}", e));
                        }
                        out.push(format!(
                            "{}={} saved",
                            key,
                            if key == "api_key" { "(set)" } else { val }
                        ));
                    } else {
                        out.push("usage: /config profile set key=value".to_string());
                    }
                }
                _ => {
                    out.push("commands:".to_string());
                    out.push("  /debug runtime                   show Themion process/thread/task activity".to_string());
                    out.push(
                        "  /config                          show current settings".to_string(),
                    );
                    out.push("  /config profile [list]           list profiles".to_string());
                    out.push("  /config profile show             show active profile".to_string());
                    out.push(
                        "  /config profile create <name>    create from current settings"
                            .to_string(),
                    );
                    out.push("  /config profile use <name>       switch profile".to_string());
                    out.push(
                        "  /config profile set key=value    set provider/model/endpoint/api_key"
                            .to_string(),
                    );
                }
            }
            return out;
        }

        out.push(format!(
            "unknown command '{}'.  try /config or /debug runtime",
            input
        ));
        out
    }

    fn scroll_up(&mut self) {
        match self.review_mode {
            ReviewMode::Transcript => {
                self.review_scroll_offset += 3;
            }
            ReviewMode::Closed => {
                self.scroll_offset += 3;
                self.enter_browsed_history();
            }
        }
    }

    fn scroll_down(&mut self) {
        match self.review_mode {
            ReviewMode::Transcript => {
                self.review_scroll_offset = self.review_scroll_offset.saturating_sub(3);
            }
            ReviewMode::Closed => {
                self.scroll_offset = self.scroll_offset.saturating_sub(3);
                if self.scroll_offset == 0 {
                    self.navigation_mode = NavigationMode::FollowTail;
                }
            }
        }
    }

    fn page_up(&mut self, amount: usize) {
        match self.review_mode {
            ReviewMode::Transcript => {
                self.review_scroll_offset = self.review_scroll_offset.saturating_add(amount.max(1));
            }
            ReviewMode::Closed => {
                self.scroll_offset = self.scroll_offset.saturating_add(amount.max(1));
                self.enter_browsed_history();
            }
        }
    }

    fn page_down(&mut self, amount: usize) {
        match self.review_mode {
            ReviewMode::Transcript => {
                self.review_scroll_offset = self.review_scroll_offset.saturating_sub(amount.max(1));
            }
            ReviewMode::Closed => {
                self.scroll_offset = self.scroll_offset.saturating_sub(amount.max(1));
                if self.scroll_offset == 0 {
                    self.navigation_mode = NavigationMode::FollowTail;
                }
            }
        }
    }

    fn jump_to_top(&mut self, total_visual: usize, height: usize) {
        let max_scroll = total_visual.saturating_sub(height);
        match self.review_mode {
            ReviewMode::Transcript => {
                self.review_scroll_offset = max_scroll;
            }
            ReviewMode::Closed => {
                self.scroll_offset = max_scroll;
                self.enter_browsed_history();
            }
        }
    }

    fn submit_shell_command(&mut self, command: &str, app_tx: &mpsc::UnboundedSender<AppEvent>) {
        let command = command.trim_start().to_string();
        self.push(Entry::User(format!("!{}", command)));

        if command.is_empty() {
            self.push(Entry::Assistant("empty shell command".to_string()));
            self.push(Entry::Blank);
            return;
        }

        self.agent_busy = true;
        self.set_agent_activity(AgentActivity::RunningShellCommand);

        let tx = app_tx.clone();
        let project_dir = self.project_dir.clone();
        self.background_domain().spawn(async move {
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

            let _ = tx.send(AppEvent::ShellComplete { output, exit_code });
        });
    }

    fn submit_text_to_agent(
        &mut self,
        agent_index: usize,
        text: String,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        self.agent_busy = true;
        self.activity_counters.agent_turn_started_count += 1;
        self.reset_stream_counters();
        self.set_agent_activity(AgentActivity::PreparingRequest);

        let cancellation = TurnCancellation::new();
        self.active_turn_cancellation = Some(cancellation.clone());

        let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();
        let app_tx_relay = app_tx.clone();
        self.background_domain().spawn(async move {
            let mut rx = event_rx;
            while let Some(ev) = rx.recv().await {
                let _ = app_tx_relay.send(AppEvent::Agent(ev));
            }
        });

        let handle = self.agents.get_mut(agent_index).expect("agent index valid");
        let mut agent = handle.agent.take().expect("agent available when not busy");
        let handle_session_id = handle.session_id;
        agent.set_event_tx(event_tx);

        let app_tx_done = app_tx.clone();
        self.core_domain.spawn(async move {
            if let Err(e) = agent
                .run_loop_with_cancellation(&text, Some(cancellation))
                .await
            {
                let _ = app_tx_done.send(AppEvent::Agent(AgentEvent::AssistantText(format!(
                    "error: {e}"
                ))));
            }
            let _ = app_tx_done.send(AppEvent::AgentReady(Box::new(agent), handle_session_id));
        });
    }

    fn submit_text(&mut self, text: String, app_tx: &mpsc::UnboundedSender<AppEvent>) {
        let text = text.trim().to_string();
        if text.is_empty() || self.agent_busy {
            return;
        }

        if self.history.last() != Some(&text) {
            self.history.push(text.clone());
        }
        self.history_pos = None;
        self.history_draft = String::new();
        self.input = make_input();
        self.return_to_latest();

        if text == "/exit" || text == "/quit" {
            self.running = false;
            return;
        }

        if let Some(command) = text.strip_prefix('!') {
            self.submit_shell_command(command, app_tx);
            return;
        }

        if text.starts_with('/') {
            let output = self.handle_command(&text, app_tx);
            self.push(Entry::User(text));
            for line in output {
                self.push(Entry::Assistant(line));
            }
            self.push(Entry::Blank);
            self.mark_dirty_input();
            return;
        }

        #[cfg(feature = "stylos")]
        let target_agent_id = self
            .active_incoming_prompt
            .as_ref()
            .and_then(|request| request.agent_id.as_deref());
        #[cfg(feature = "stylos")]
        let agent_index = target_agent_id
            .and_then(|agent_id| self.agents.iter().position(|h| h.agent_id == agent_id))
            .or_else(|| self.agents.iter().position(is_interactive_handle))
            .expect("interactive or targeted agent");
        #[cfg(not(feature = "stylos"))]
        let agent_index = self
            .agents
            .iter()
            .position(is_interactive_handle)
            .expect("interactive agent");

        #[cfg(feature = "stylos")]
        if self.active_incoming_prompt.is_none() {
            self.push(Entry::User(text.clone()));
        }
        #[cfg(not(feature = "stylos"))]
        self.push(Entry::User(text.clone()));

        self.submit_text_to_agent(agent_index, text, app_tx);
    }

    #[cfg(feature = "stylos")]
    fn maybe_inject_pending_board_note(&mut self, app_tx: &mpsc::UnboundedSender<AppEvent>) {
        if self.agent_busy {
            return;
        }
        let Some(instance) = self.local_stylos_instance.clone() else {
            return;
        };
        let interactive_agent_id = self
            .agents
            .iter()
            .find(|h| is_interactive_handle(h))
            .map(|h| h.agent_id.clone());
        let main_agent_id = self
            .agents
            .iter()
            .find(|h| h.roles.iter().any(|r| r == "main"))
            .map(|h| h.agent_id.clone());
        let Some(agent_id) = interactive_agent_id.or(main_agent_id) else {
            return;
        };
        let Ok(Some(note)) = self.db.next_board_note_for_injection(&instance, &agent_id) else {
            return;
        };
        let _ = self.db.mark_board_note_injected(&note.note_id);
        let prompt = crate::stylos::build_board_note_prompt(
            &note.note_id,
            &note.note_slug,
            note.note_kind,
            note.origin_note_id.as_deref(),
            note.from_instance.as_deref(),
            note.from_agent_id.as_deref(),
            &note.to_instance,
            &note.to_agent_id,
            note.column,
            &note.body,
        );
        self.push(Entry::RemoteEvent(format!(
            "Board note injection note_slug={} to={} to_agent_id={} column={}",
            note.note_slug,
            note.to_instance,
            note.to_agent_id,
            note.column.as_str()
        )));
        self.active_incoming_prompt = Some(IncomingPromptRequest {
            prompt: prompt.clone(),
            agent_id: Some(note.to_agent_id.clone()),
            task_id: None,
            request_id: None,
            from: note.from_instance.clone(),
            from_agent_id: note.from_agent_id.clone(),
            to: Some(note.to_instance.clone()),
            to_agent_id: Some(note.to_agent_id.clone()),
        });
        self.submit_text(prompt, app_tx);
    }

    #[cfg(feature = "stylos")]
    fn maybe_emit_done_mention_for_completed_note(
        &mut self,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        let Some(remote) = self.active_incoming_prompt.as_ref().cloned() else {
            return;
        };
        if !remote.prompt.starts_with("type=stylos_note ") {
            return;
        }
        let header = remote.prompt.lines().next().unwrap_or_default();
        let note_id = header
            .split_whitespace()
            .find_map(|part| part.strip_prefix("note_id="));
        let Some(note_id) = note_id else {
            return;
        };
        let Ok(Some(note)) = self.db.get_board_note(note_id) else {
            return;
        };
        if note.column != themion_core::db::NoteColumn::Done {
            let prompt = format!(
                "This turn ended but note {} is still in {}. You still have a pending board task. Continue handling this note now. Decide from the note context whether any real action remains. If no further action is needed, move the note to done in this turn. Otherwise keep progressing it through the board workflow and do not end the turn while it is still pending.",
                note.note_slug,
                note.column.as_str(),
            );
            self.active_incoming_prompt = Some(remote);
            self.submit_text(prompt, app_tx);
            return;
        }
        if note.note_kind != themion_core::db::NoteKind::WorkRequest {
            return;
        }
        if note.completion_notified_at_ms.is_some() {
            return;
        }
        let (Some(to_instance), Some(to_agent_id)) =
            (note.from_instance.clone(), note.from_agent_id.clone())
        else {
            return;
        };
        let result_summary = note
            .result_text
            .clone()
            .unwrap_or_else(|| "completed with no explicit stored result".to_string());
        let request = crate::app_runtime::DoneMentionRequest {
            note_id: note.note_id.clone(),
            note_slug: note.note_slug.clone(),
            from_instance: to_instance.clone(),
            from_agent_id: to_agent_id.clone(),
            completed_by_instance: note.to_instance.clone(),
            completed_by_agent_id: note.to_agent_id.clone(),
            result_summary,
        };

        let create_reply = if let Some(bridge) = self.stylos_tool_bridge.as_ref().cloned() {
            let app_tx_done = app_tx.clone();
            let from_agent_id = note.to_agent_id.clone();
            self.background_domain().spawn(async move {
                let result = crate::app_runtime::create_done_mention_via_bridge(
                    &bridge,
                    &from_agent_id,
                    &request,
                )
                .await;
                let output = match result {
                    Ok(output) => output,
                    Err(err) => format!("error: {}", err),
                };
                let _ = app_tx_done.send(AppEvent::Agent(AgentEvent::Status(output)));
            });
            return;
        } else {
            crate::app_runtime::create_done_mention_locally(&self.db, &request)
        };

        match create_reply {
            Ok(reply) => {
                let created_note_slug = serde_json::from_str::<serde_json::Value>(&reply)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("note_slug")
                            .or_else(|| value.get("note_id"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                let _ = self.db.mark_board_note_completion_notified(&note.note_id);
                self.push(Entry::RemoteEvent(format!(
                    "Board done mention note_slug={} origin_note_slug={} to={} to_agent_id={}",
                    created_note_slug, note.note_slug, to_instance, to_agent_id,
                )));
            }
            Err(err) => {
                self.push(Entry::Status(format!(
                    "done mention create failed for note_id={}: {}",
                    note.note_id, err
                )));
            }
        }
    }

    fn submit_input(&mut self, app_tx: &mpsc::UnboundedSender<AppEvent>) -> bool {
        let text: String = self.input.lines().join("\n");
        let was_dirty = self.dirty.any();
        self.submit_text(text, app_tx);
        self.dirty.any() && !was_dirty
    }

    pub(crate) fn handle_mouse_event(
        &mut self,
        mouse: event::MouseEvent,
        frame_requester: &FrameRequester,
    ) {
        self.activity_counters.input_mouse_count += 1;
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_up();
                self.mark_dirty_conversation();
                self.request_draw(frame_requester);
            }
            MouseEventKind::ScrollDown => {
                self.scroll_down();
                self.mark_dirty_conversation();
                self.request_draw(frame_requester);
            }
            _ => {}
        }
    }

    pub(crate) fn handle_paste_event(&mut self, text: String, frame_requester: &FrameRequester) {
        self.activity_counters.input_paste_count += 1;
        handle_paste(self, text);
        self.mark_dirty_input();
        self.request_draw(frame_requester);
    }

    pub(crate) fn handle_tick_event(
        &mut self,
        frame_requester: &FrameRequester,
        #[cfg(feature = "stylos")] app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        let was_dirty = self.dirty.any();
        self.on_tick();
        if self.dirty.any() && !was_dirty {
            self.request_draw(frame_requester);
        }
        #[cfg(feature = "stylos")]
        self.maybe_inject_pending_board_note(app_tx);
    }

    pub(crate) fn handle_agent_ready_event(
        &mut self,
        agent: Box<Agent>,
        sid: Uuid,
        frame_requester: &FrameRequester,
    ) {
        let agent = *agent;
        self.status_model_info = agent.model_info().cloned();
        self.workflow_state = agent.workflow_state().clone();
        if let Some(h) = self.agents.iter_mut().find(|h| h.session_id == sid) {
            h.agent = Some(agent);
        }
        self.agent_busy = false;
        self.active_turn_cancellation = None;
        self.mark_dirty_status();
        self.request_draw(frame_requester);
    }

    pub(crate) fn handle_login_prompt_event(
        &mut self,
        user_code: String,
        verification_uri: String,
        frame_requester: &FrameRequester,
    ) {
        self.set_agent_activity(AgentActivity::WaitingForLoginBrowser);
        self.push(Entry::Assistant(format!(
            "open {} and enter code {}",
            verification_uri, user_code
        )));
        self.request_draw(frame_requester);
    }

    pub(crate) async fn handle_app_event(
        &mut self,
        event: AppEvent,
        frame_requester: &FrameRequester,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
        terminal: &Terminal<CrosstermBackend<std::io::Stdout>>,
    ) {
        match event {
            AppEvent::Mouse(m) => self.handle_mouse_event(m, frame_requester),
            AppEvent::Paste(text) => self.handle_paste_event(text, frame_requester),
            AppEvent::Key(key) => self.handle_key_event(key, frame_requester, app_tx, terminal),
            AppEvent::Tick => {
                #[cfg(feature = "stylos")]
                self.handle_tick_event(frame_requester, app_tx);
                #[cfg(not(feature = "stylos"))]
                self.handle_tick_event(frame_requester);
            }
            #[cfg(feature = "stylos")]
            AppEvent::StylosCmd(cmd) => self.handle_stylos_cmd_event(cmd, app_tx),
            #[cfg(feature = "stylos")]
            AppEvent::StylosEvent(text) => self.handle_stylos_event_text(text),
            #[cfg(feature = "stylos")]
            AppEvent::IncomingPrompt(request) => self.handle_incoming_prompt_event(request, app_tx),
            #[cfg(feature = "stylos")]
            AppEvent::Agent(ev) => self.handle_agent_event_for_run(ev, frame_requester, app_tx),
            #[cfg(not(feature = "stylos"))]
            AppEvent::Agent(ev) => self.handle_agent_event_for_run(ev, frame_requester),
            AppEvent::AgentReady(agent, sid) => {
                self.handle_agent_ready_event(agent, sid, frame_requester);
            }
            AppEvent::LoginPrompt {
                user_code,
                verification_uri,
            } => {
                self.handle_login_prompt_event(user_code, verification_uri, frame_requester);
            }
            AppEvent::LoginComplete(auth_result) => {
                self.handle_login_complete_event(auth_result, frame_requester)
                    .await;
            }
            AppEvent::ShellComplete { output, exit_code } => {
                self.handle_shell_complete_event(output, exit_code, frame_requester);
            }
        }
    }

    pub(crate) async fn handle_login_complete_event(
        &mut self,
        auth_result: anyhow::Result<themion_core::CodexAuth>,
        frame_requester: &FrameRequester,
    ) {
        match auth_result {
            Ok(auth) => {
                self.clear_agent_activity();
                if let Err(e) = crate::auth_store::save(&auth) {
                    self.push(Entry::Assistant(format!(
                        "warning: failed to save auth: {}",
                        e
                    )));
                }
                self.session.profiles.insert(
                    "codex".to_string(),
                    ProfileConfig {
                        provider: Some("openai-codex".to_string()),
                        model: Some("gpt-5.4".to_string()),
                        base_url: None,
                        api_key: None,
                    },
                );
                self.session.switch_profile("codex");
                if let Err(e) = save_profiles(&self.session.active_profile, &self.session.profiles)
                {
                    self.push(Entry::Assistant(format!(
                        "warning: failed to save config: {}",
                        e
                    )));
                }
                let new_session_id = Uuid::new_v4();
                match crate::app_runtime::build_agent(
                    &self.session,
                    new_session_id,
                    self.project_dir.clone(),
                    self.db.clone(),
                    #[cfg(feature = "stylos")]
                    self.stylos_tool_bridge.clone(),
                    #[cfg(feature = "stylos")]
                    self.local_stylos_instance.as_deref(),
                    #[cfg(feature = "stylos")]
                    "main",
                    None,
                ) {
                    Ok(mut new_agent) => {
                        new_agent.refresh_model_info().await;
                        let _ = self
                            .db
                            .insert_session(new_session_id, &self.project_dir, true);
                        self.status_model_info = new_agent.model_info().cloned();
                        self.workflow_state = new_agent.workflow_state().clone();
                        self.agents = vec![AgentHandle {
                            agent: Some(new_agent),
                            session_id: new_session_id,
                            agent_id: "main".to_string(),
                            label: "main".to_string(),
                            roles: vec!["main".to_string(), "interactive".to_string()],
                        }];
                        self.push(Entry::Assistant(format!(
                            "logged in as {} — switched to codex profile (gpt-5.4)",
                            auth.account_id
                        )));
                        self.push(Entry::Blank);
                        self.agent_busy = false;
                        self.mark_dirty_all();
                        self.request_draw(frame_requester);
                    }
                    Err(e) => {
                        self.push(Entry::Assistant(format!(
                            "login succeeded but agent build failed: {}",
                            e
                        )));
                        self.agent_busy = false;
                        self.mark_dirty_all();
                        self.request_draw(frame_requester);
                    }
                }
            }
            Err(e) => {
                self.clear_agent_activity();
                self.push(Entry::Assistant(format!("login failed: {}", e)));
                self.agent_busy = false;
                self.mark_dirty_all();
                self.request_draw(frame_requester);
            }
        }
    }

    pub(crate) fn handle_agent_event_for_run(
        &mut self,
        ev: AgentEvent,
        frame_requester: &FrameRequester,
        #[cfg(feature = "stylos")] app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        self.activity_counters.agent_event_count += 1;
        self.process_agent_event(
            ev,
            #[cfg(feature = "stylos")]
            app_tx,
        );
        if self.dirty.any() {
            self.request_draw(frame_requester);
        }
    }

    pub(crate) fn handle_draw_event(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<()> {
        if self.dirty.any() {
            let draw_started = Instant::now();
            terminal.draw(|f| draw(f, self))?;
            let draw_us = draw_started.elapsed().as_micros() as u64;
            self.activity_counters.draw_count += 1;
            self.activity_counters.draw_total_us += draw_us;
            self.activity_counters.draw_max_us = self.activity_counters.draw_max_us.max(draw_us);
            self.dirty.clear();
        } else {
            self.activity_counters.draw_skip_clean_count += 1;
        }
        Ok(())
    }

    pub(crate) fn handle_shell_complete_event(
        &mut self,
        output: String,
        exit_code: Option<i32>,
        frame_requester: &FrameRequester,
    ) {
        self.activity_counters.shell_complete_count += 1;
        self.clear_agent_activity();
        self.push(Entry::Assistant(output));
        if let Some(code) = exit_code {
            if code != 0 {
                self.push(Entry::Assistant(format!("exit code: {}", code)));
            }
        }
        self.push(Entry::Blank);
        self.agent_busy = false;
        self.mark_dirty_all();
        self.request_draw(frame_requester);
    }

    #[cfg(feature = "stylos")]
    pub(crate) fn handle_stylos_cmd_event(
        &mut self,
        cmd: crate::stylos::StylosCmdRequest,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        self.push(Entry::RemoteEvent(format!(
            "Stylos cmd scope=local preview={}",
            cmd.prompt.lines().next().unwrap_or("")
        )));
        self.active_incoming_prompt = None;
        self.submit_text(cmd.prompt, app_tx);
    }

    #[cfg(feature = "stylos")]
    pub(crate) fn handle_stylos_event_text(&mut self, text: String) {
        self.push(Entry::RemoteEvent(text));
    }

    #[cfg(feature = "stylos")]
    pub(crate) fn handle_incoming_prompt_event(
        &mut self,
        request: IncomingPromptRequest,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        self.activity_counters.incoming_prompt_count += 1;
        let target = request
            .agent_id
            .clone()
            .unwrap_or_else(|| "interactive".to_string());
        if self.agent_busy {
            let sender = request.from.as_deref().unwrap_or("unknown sender");
            let sender_agent = request.from_agent_id.as_deref().unwrap_or("unknown");
            let target_instance = request.to.as_deref().unwrap_or("unknown target");
            let target_agent = request.to_agent_id.as_deref().unwrap_or(target.as_str());
            if request.prompt.starts_with("type=stylos_note ") {
                let note_identifier = stylos_note_display_identifier(&request.prompt);
                self.push(Entry::RemoteEvent(format!(
                    "Board note intake {} from={} from_agent_id={} to={} to_agent_id={} deferred: local agent busy",
                    note_identifier, sender, sender_agent, target_instance, target_agent
                )));
            } else {
                self.push(Entry::RemoteEvent(format!(
                    "Stylos hear from={} from_agent_id={} to={} to_agent_id={} rejected: local agent busy",
                    sender, sender_agent, target_instance, target_agent
                )));
            }
            if let (Some(handle), Some(task_id)) = (self.stylos.as_ref(), request.task_id.clone()) {
                let query_context = handle.query_context();
                self.background_domain().spawn(async move {
                    query_context
                        .task_registry()
                        .set_failed(&task_id, "agent_busy".to_string())
                        .await;
                });
            }
        } else {
            let sender = request.from.as_deref().unwrap_or("unknown sender");
            let sender_agent = request.from_agent_id.as_deref().unwrap_or("unknown");
            let target_instance = request.to.as_deref().unwrap_or("unknown target");
            let target_agent = request.to_agent_id.as_deref().unwrap_or(target.as_str());
            if request.prompt.starts_with("type=stylos_note ") {
                let note_identifier = stylos_note_display_identifier(&request.prompt);
                let column =
                    stylos_note_header_value(&request.prompt, "column").unwrap_or("unknown");
                self.push(Entry::RemoteEvent(format!(
                    "Board note intake {} from={} from_agent_id={} to={} to_agent_id={} column={}",
                    note_identifier, sender, sender_agent, target_instance, target_agent, column
                )));
            } else {
                self.push(Entry::RemoteEvent(format!(
                    "Stylos hear from={} from_agent_id={} to={} to_agent_id={}",
                    sender, sender_agent, target_instance, target_agent
                )));
            }
            self.active_incoming_prompt = Some(request.clone());
            self.submit_text(request.prompt, app_tx);
        }
    }

    pub(crate) fn handle_key_event(
        &mut self,
        key: event::KeyEvent,
        frame_requester: &FrameRequester,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
        terminal: &Terminal<CrosstermBackend<std::io::Stdout>>,
    ) {
        self.activity_counters.input_key_count += 1;

        let now = Instant::now();
        match self.paste_burst.flush_if_due(now) {
            FlushResult::Paste(text) => handle_paste(self, text),
            FlushResult::Typed(ch) => self.input.insert_char(ch),
            FlushResult::None => {}
        }

        if matches!(key.code, KeyCode::Enter)
            && self.paste_burst.is_active()
            && self.paste_burst.append_newline_if_active(now)
        {
            return;
        }

        if let KeyCode::Char(ch) = key.code {
            let has_ctrl_or_alt = key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::ALT);
            if !has_ctrl_or_alt {
                if !ch.is_ascii() {
                    let _ = handle_non_ascii_char(self, key, now);
                    return;
                }

                if let Some(decision) = self.paste_burst.on_plain_char_no_hold(now) {
                    match decision {
                        CharDecision::BufferAppend => {
                            self.paste_burst.append_char_to_buffer(ch, now);
                            return;
                        }
                        CharDecision::BeginBuffer { retro_chars } => {
                            let (text, byte_pos) = input_text_and_cursor_byte(&self.input);
                            let safe_cursor = clamp_to_char_boundary(&text, byte_pos);
                            let before = &text[..safe_cursor];
                            if let Some(grab) = self.paste_burst.decide_begin_buffer(
                                now,
                                before,
                                retro_chars as usize,
                            ) {
                                let kept =
                                    format!("{}{}", &text[..grab.start_byte], &text[safe_cursor..]);
                                set_input_text_and_cursor(&mut self.input, &kept, grab.start_byte);
                                self.paste_burst.append_char_to_buffer(ch, now);
                                return;
                            }
                        }
                    }
                }
            }

            if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                handle_paste(self, pasted);
            }
        }

        if !matches!(key.code, KeyCode::Char(_) | KeyCode::Enter) {
            if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                handle_paste(self, pasted);
            }
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => self.running = false,
            (KeyCode::Esc, _) if self.review_mode == ReviewMode::Transcript => {
                self.close_transcript_review();
                self.mark_dirty_overlay();
                self.request_draw(frame_requester);
            }
            (KeyCode::Esc, _) if self.agent_busy => self.request_interrupt(),
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                let tx = app_tx.clone();
                if self.submit_input(&tx) {
                    self.request_draw(frame_requester);
                }
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.review_mode != ReviewMode::Closed {
                    self.close_transcript_review();
                } else if self
                    .paste_burst
                    .newline_should_insert_instead_of_submit(now)
                {
                    self.input.insert_newline();
                    self.mark_dirty_input();
                    self.request_draw(frame_requester);
                    self.paste_burst.extend_window(now);
                } else {
                    let tx = app_tx.clone();
                    if self.submit_input(&tx) {
                        self.request_draw(frame_requester);
                    }
                }
            }
            (KeyCode::Enter, KeyModifiers::SHIFT) | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                    handle_paste(self, pasted);
                }
                self.input.insert_newline();
                self.mark_dirty_input();
                self.request_draw(frame_requester);
            }
            (KeyCode::PageUp, _) => {
                let page = area_page_height(terminal, self);
                self.page_up(page);
            }
            (KeyCode::PageDown, _) => {
                let page = area_page_height(terminal, self);
                self.page_down(page);
            }
            (KeyCode::Up, KeyModifiers::ALT) => self.scroll_up(),
            (KeyCode::Down, KeyModifiers::ALT) => self.scroll_down(),
            (KeyCode::Char('g'), KeyModifiers::ALT) => {
                self.return_to_latest();
                self.mark_dirty_conversation();
                self.request_draw(frame_requester);
            }
            (KeyCode::Char('t'), KeyModifiers::ALT) => {
                if self.review_mode == ReviewMode::Transcript {
                    self.close_transcript_review();
                } else {
                    self.open_transcript_review();
                    self.mark_dirty_overlay();
                    self.request_draw(frame_requester);
                }
            }
            (KeyCode::Home, KeyModifiers::ALT) => {
                let (total, height) = current_total_and_height(terminal, self);
                self.jump_to_top(total, height);
            }
            (KeyCode::Up, KeyModifiers::NONE) if self.review_mode == ReviewMode::Closed => {
                self.history_up();
                self.mark_dirty_input();
                self.request_draw(frame_requester)
            }
            (KeyCode::Down, KeyModifiers::NONE) if self.review_mode == ReviewMode::Closed => {
                self.history_down();
                self.mark_dirty_input();
                self.request_draw(frame_requester)
            }
            _ => {
                if self.review_mode == ReviewMode::Closed {
                    self.input.input(key);
                    self.mark_dirty_input();
                    self.request_draw(frame_requester);
                    match key.code {
                        KeyCode::Char(_) => {
                            let has_ctrl_or_alt = key.modifiers.contains(KeyModifiers::CONTROL)
                                || key.modifiers.contains(KeyModifiers::ALT);
                            if has_ctrl_or_alt {
                                self.paste_burst.clear_window_after_non_char();
                            }
                        }
                        KeyCode::Enter => {}
                        _ => self.paste_burst.clear_window_after_non_char(),
                    }
                }
            }
        }
    }
}

#[cfg(feature = "stylos")]
fn extract_stylos_talk_target_from_detail(detail: &str) -> Option<&str> {
    let prefix = "stylos_request_talk ";
    let rest = detail.strip_prefix(prefix)?;
    for field in rest.split_whitespace() {
        if let Some(value) = field.strip_prefix("instance=") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

fn format_human_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{}m", n / 1_000_000)
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

fn build_context_statusline(last_ctx_tokens: u64, info: Option<&ModelInfo>) -> String {
    let max_part = info
        .and_then(|info| info.max_context_window.or(info.context_window))
        .map(format_human_count)
        .unwrap_or_else(|| "?".to_string());
    format!("{}/{}", format_human_count(last_ctx_tokens), max_part)
}

fn build_rate_limit_statusline(report: Option<&ApiCallRateLimitReport>) -> String {
    let Some(report) = report else {
        return "--".to_string();
    };
    let Some(snapshot) = report
        .snapshots
        .iter()
        .find(|s| {
            s.limit_id
                .as_deref()
                .map(|id| id.eq_ignore_ascii_case("codex"))
                .unwrap_or(false)
        })
        .or_else(|| report.snapshots.first())
    else {
        return "--".to_string();
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let fmt = |key: &str, fallback: &str| -> Option<String> {
        let limit = snapshot
            .limits
            .iter()
            .find(|l| l.status_line_key.as_deref() == Some(key))
            .or_else(|| snapshot.limits.iter().find(|l| l.kind == fallback))?;

        let elapsed_percent = match (limit.window_minutes, limit.resets_at) {
            (Some(window_minutes), Some(resets_at)) if window_minutes > 0 => {
                let window_secs = window_minutes.saturating_mul(60);
                let remaining_secs = (resets_at - now).clamp(0, window_secs);
                let elapsed_secs = window_secs.saturating_sub(remaining_secs);
                (elapsed_secs as f64 / window_secs as f64 * 100.0).clamp(0.0, 100.0)
            }
            _ => 0.0,
        };

        Some(format!(
            "{}:{:.0}%/{:.0}%",
            limit.label, limit.used_percent, elapsed_percent
        ))
    };

    let mut parts = Vec::new();
    if let Some(s) = fmt("five-hour-limit", "primary") {
        parts.push(s);
    }
    if let Some(s) = fmt("weekly-limit", "secondary") {
        parts.push(s);
    }

    if parts.is_empty() {
        "--".to_string()
    } else {
        parts.join(" | ")
    }
}

#[cfg(feature = "stylos")]
pub(crate) fn stylos_tool_invoker(
    bridge: Option<StylosToolBridge>,
) -> Option<themion_core::tools::StylosToolInvoker> {
    bridge.map(|bridge| {
        std::sync::Arc::new(move |name: String, args: serde_json::Value| {
            let bridge = bridge.clone();
            let local_agent_id = args
                .get("_local_agent_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let fut: std::pin::Pin<
                Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>,
            > = Box::pin(
                async move { bridge.invoke(local_agent_id.as_deref(), &name, args).await },
            );
            fut
        }) as themion_core::tools::StylosToolInvoker
    })
}

fn set_input_text(input: &mut TextArea, text: &str) {
    *input = make_input();
    if !text.is_empty() {
        input.insert_str(text);
    }
}

fn set_input_text_and_cursor(input: &mut TextArea, text: &str, cursor_byte: usize) {
    set_input_text(input, text);
    let cursor_byte = clamp_to_char_boundary(text, cursor_byte);
    let mut row = 0usize;
    let mut col = 0usize;
    let mut remaining = cursor_byte;
    for line in text.split('\n') {
        if remaining <= line.len() {
            col = line[..remaining].chars().count();
            break;
        }
        remaining = remaining.saturating_sub(line.len() + 1);
        row += 1;
    }
    input.move_cursor(CursorMove::Jump(row as u16, col as u16));
}

fn input_text_and_cursor_byte(input: &TextArea) -> (String, usize) {
    let lines = input.lines();
    let text = lines.join("\n");
    let (row, col) = input.cursor();
    let mut byte_pos = 0usize;
    for (idx, line) in lines.iter().enumerate() {
        if idx == row {
            let safe_col = col.min(line.chars().count());
            byte_pos += line
                .char_indices()
                .nth(safe_col)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            break;
        }
        byte_pos += line.len() + 1;
    }
    (text, byte_pos)
}

fn handle_paste(app: &mut App<'_>, pasted: String) {
    insert_pasted_text(&mut app.input, &pasted);
    app.paste_burst.clear_after_explicit_paste();
}

pub(crate) fn dispatch_terminal_event(
    app_tx: &mpsc::UnboundedSender<AppEvent>,
    event: Event,
) -> bool {
    let app_event = match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => AppEvent::Key(key),
        Event::Key(_) => return true,
        Event::Mouse(mouse) => AppEvent::Mouse(mouse),
        Event::Paste(text) => AppEvent::Paste(text),
        _ => return true,
    };
    app_tx.send(app_event).is_ok()
}

fn handle_non_ascii_char(app: &mut App<'_>, key: event::KeyEvent, _now: Instant) -> bool {
    if let Some(pasted) = app.paste_burst.flush_before_modified_input() {
        handle_paste(app, pasted);
    }
    app.input.input(key);
    true
}

fn insert_pasted_text(input: &mut TextArea, text: &str) {
    if text.is_empty() {
        return;
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    input.insert_str(normalized);
}

fn clamp_to_char_boundary(text: &str, pos: usize) -> usize {
    let mut p = pos.min(text.len());
    if p < text.len() && !text.is_char_boundary(p) {
        p = text
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= p)
            .last()
            .unwrap_or(0);
    }
    p
}

fn make_input<'a>() -> TextArea<'a> {
    let mut ta = TextArea::default();
    ta.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .padding(Padding::left(1)),
    );
    ta.set_cursor_line_style(Style::default());
    ta.set_placeholder_text(
        "message…  (Enter/Ctrl-S send | Shift-Enter/Ctrl-J newline | Esc interrupt | Ctrl-C quit)",
    );
    ta
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputLayoutMetrics {
    visual_lines: u16,
    cursor_row: u16,
    cursor_col: u16,
}

fn char_display_width(ch: char) -> u16 {
    UnicodeWidthChar::width(ch).unwrap_or(0).max(1) as u16
}

fn input_layout_metrics(text: &str, cursor_byte: usize, width: u16) -> InputLayoutMetrics {
    let width = width.max(1);
    let cursor_byte = clamp_to_char_boundary(text, cursor_byte);
    let mut visual_lines = 1u16;
    let mut cursor_row = 0u16;
    let mut cursor_col = 0u16;
    let mut row = 0u16;
    let mut col = 0u16;

    for (byte_idx, ch) in text.char_indices() {
        if byte_idx == cursor_byte {
            cursor_row = row;
            cursor_col = col;
        }

        if ch == '\n' {
            row = row.saturating_add(1);
            visual_lines = visual_lines.max(row.saturating_add(1));
            col = 0;
            continue;
        }

        let ch_width = char_display_width(ch);
        if col.saturating_add(ch_width) > width {
            row = row.saturating_add(1);
            visual_lines = visual_lines.max(row.saturating_add(1));
            col = 0;
        }
        col = col.saturating_add(ch_width);
        visual_lines = visual_lines.max(row.saturating_add(1));
    }

    if cursor_byte == text.len() {
        cursor_row = row;
        cursor_col = col;
    }

    InputLayoutMetrics {
        visual_lines: visual_lines.max(1),
        cursor_row,
        cursor_col,
    }
}

fn build_lines<'a>(entries: &'a [Entry], pending: &'a Option<String>) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();

    for entry in entries {
        match entry {
            Entry::User(text) => {
                lines.push(Line::default());
                for (i, part) in text.lines().enumerate() {
                    let prefix = if i == 0 {
                        Span::styled(
                            "❯ ",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::raw("  ")
                    };
                    lines.push(Line::from(vec![
                        prefix,
                        Span::styled(
                            part.to_string(),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                    ]));
                }
            }
            Entry::Assistant(text) => {
                for part in text.lines() {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::raw(part.to_string()),
                    ]));
                }
            }
            #[cfg(feature = "stylos")]
            Entry::RemoteEvent(text) => {
                lines.push(Line::from(vec![Span::styled(
                    format!("  󰀂 {}", text),
                    Style::default().fg(Color::Magenta),
                )]));
            }
            Entry::Banner(text) => {
                for part in text.lines() {
                    lines.push(Line::from(vec![Span::styled(
                        format!("  {}", part),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )]));
                }
            }
            Entry::ToolCall(detail) => {
                lines.push(Line::from(vec![Span::styled(
                    format!("   {}", detail),
                    Style::default().fg(Color::Yellow),
                )]));
            }
            Entry::Status(text) => {
                lines.push(Line::from(vec![Span::styled(
                    format!("  󰇺 {}", text),
                    Style::default().fg(Color::DarkGray),
                )]));
            }
            Entry::ToolDone => {
                if let Some(last) = lines.last_mut() {
                    let mut spans = last.spans.clone();
                    spans.push(Span::styled("  ✓", Style::default().fg(Color::Green)));
                    *last = Line::from(spans);
                }
            }
            Entry::TurnDone { summary, stats } => {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("", Style::default().fg(Color::Green)),
                    Span::styled(
                        summary.to_string(),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" [stats: {}]", stats),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
            Entry::Stats(s) => {
                lines.push(Line::from(vec![Span::styled(
                    format!("  {}", s),
                    Style::default().fg(Color::DarkGray),
                )]));
            }
            Entry::Blank => {
                lines.push(Line::default());
            }
        }
    }

    if let Some(p) = pending {
        lines.push(Line::from(vec![Span::styled(
            p.as_str(),
            Style::default().fg(Color::Yellow),
        )]));
    }

    lines
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
}

fn scroll_from_bottom(offset_from_bottom: usize, total_visual: usize, height: usize) -> u16 {
    let max_scroll = total_visual.saturating_sub(height);
    max_scroll.saturating_sub(offset_from_bottom) as u16
}

fn review_area(area: Rect) -> Rect {
    let width = area.width.saturating_mul(85).saturating_div(100).max(20);
    let height = area.height.saturating_mul(85).saturating_div(100).max(10);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let input_text = app.input.lines().join("\n");

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .padding(Padding::left(1));

    let input_inner = input_block.inner(area);
    let input_inner_width = input_inner.width.max(1);
    let (_, cursor_byte) = input_text_and_cursor_byte(&app.input);
    let input_layout = input_layout_metrics(&input_text, cursor_byte, input_inner_width);

    let input_height = (input_layout.visual_lines + 2).clamp(3, 8);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(input_height),
            Constraint::Length(3),
        ])
        .split(area);

    let lines = build_lines(&app.entries, &app.pending);
    let height = chunks[0].height as usize;
    let width = chunks[0].width;

    let conv_base = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .block(Block::default());
    let total_visual = conv_base.line_count(width);
    let scroll = if app.navigation_mode == NavigationMode::FollowTail {
        scroll_from_bottom(0, total_visual, height)
    } else {
        scroll_from_bottom(app.scroll_offset, total_visual, height)
    };

    f.render_widget(Clear, chunks[0]);
    f.render_widget(conv_base.scroll((scroll, 0)), chunks[0]);

    f.render_widget(Clear, chunks[1]);
    let display_input = input_text.clone();
    let input_para = Paragraph::new(display_input)
        .wrap(Wrap { trim: false })
        .block(input_block);
    f.render_widget(input_para, chunks[1]);

    if app.review_mode == ReviewMode::Closed {
        let cursor_x = chunks[1].x + 2 + input_layout.cursor_col;
        let cursor_y = chunks[1].y + 1 + input_layout.cursor_row;
        if cursor_y < chunks[1].bottom() && cursor_x < chunks[1].right() {
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }

    let project_leaf = app
        .project_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("/");
    let activity = app.activity_status_value();
    #[cfg(feature = "stylos")]
    let stylos_status = match app.stylos.as_ref().map(|h| h.state()) {
        Some(StylosRuntimeState::Off) => "stylos: off".to_string(),
        Some(StylosRuntimeState::Active { mode, .. }) => format!("stylos: {}", mode),
        Some(StylosRuntimeState::Error(_)) => "stylos: error".to_string(),
        None => "stylos: off".to_string(),
    };
    let nav = match app.review_mode {
        ReviewMode::Transcript => "review",
        ReviewMode::Closed => match app.navigation_mode {
            NavigationMode::FollowTail => "tail",
            NavigationMode::BrowsedHistory => "browse",
        },
    };
    #[cfg(feature = "stylos")]
    let bar_top = format!(
        " {} | {} | {} | {} | flow: {} | phase: {} | agent: {} | nav: {}",
        app.session.active_profile,
        app.session.model,
        project_leaf,
        stylos_status,
        app.workflow_state.workflow_name,
        app.workflow_state.phase_name,
        activity,
        nav,
    );
    #[cfg(not(feature = "stylos"))]
    let bar_top = format!(
        " {} | {} | {} | flow: {} | phase: {} | agent: {} | nav: {}",
        app.session.active_profile,
        app.session.model,
        project_leaf,
        app.workflow_state.workflow_name,
        app.workflow_state.phase_name,
        activity,
        nav,
    );
    let bar_bottom = format!(
        " {} | in:{} out:{} cached:{} | ctx:{}",
        build_rate_limit_statusline(app.status_rate_limits.as_ref()),
        format_human_count(app.session_tokens.tokens_in),
        format_human_count(app.session_tokens.tokens_out),
        format_human_count(app.session_tokens.tokens_cached),
        build_context_statusline(app.last_ctx_tokens, app.status_model_info.as_ref()),
    );
    f.render_widget(Clear, chunks[2]);
    f.render_widget(
        Paragraph::new(format!("{}\n{}", bar_top, bar_bottom))
            .style(Style::default().bg(Color::Black).fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
        chunks[2],
    );

    if app.review_mode == ReviewMode::Transcript {
        let review = review_area(area);
        let review_block = Block::default()
            .title(" Transcript review ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let review_inner = review_block.inner(review);
        let review_lines = build_lines(&app.entries, &None);
        let review_para = Paragraph::new(review_lines)
            .wrap(Wrap { trim: false })
            .block(review_block);
        let review_total = review_para.line_count(review_inner.width.max(1));
        let review_scroll = scroll_from_bottom(
            app.review_scroll_offset,
            review_total,
            review_inner.height as usize,
        );
        f.render_widget(Clear, review);
        f.render_widget(review_para.scroll((review_scroll, 0)), review);
    }
}

pub(crate) fn area_page_height(
    terminal: &Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &App<'_>,
) -> usize {
    let area = terminal.size().map(|r| r.height as usize).unwrap_or(24);
    let reserved = 8usize + 3usize;
    let conv = area.saturating_sub(reserved).max(1);
    if app.review_mode == ReviewMode::Transcript {
        conv.saturating_mul(85)
            .saturating_div(100)
            .saturating_sub(2)
            .max(1)
    } else {
        conv.saturating_sub(1).max(1)
    }
}

pub(crate) fn current_total_and_height(
    terminal: &Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &App<'_>,
) -> (usize, usize) {
    let size = terminal
        .size()
        .map(|s| Rect::new(0, 0, s.width, s.height))
        .unwrap_or(Rect::new(0, 0, 80, 24));
    let lines = match app.review_mode {
        ReviewMode::Transcript => build_lines(&app.entries, &None),
        ReviewMode::Closed => build_lines(&app.entries, &app.pending),
    };
    let area = if app.review_mode == ReviewMode::Transcript {
        review_area(size)
    } else {
        size
    };
    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(Block::default());
    let height = if app.review_mode == ReviewMode::Transcript {
        area.height.saturating_sub(2) as usize
    } else {
        area.height.saturating_sub(11) as usize
    }
    .max(1);
    (para.line_count(area.width.max(1)), height)
}

fn unix_epoch_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod input_layout_tests {
    use super::*;

    #[test]
    fn input_layout_metrics_moves_cursor_after_explicit_newline() {
        let text = "hello\nworld";
        let metrics = input_layout_metrics(text, "hello\n".len(), 20);
        assert_eq!(
            metrics,
            InputLayoutMetrics {
                visual_lines: 2,
                cursor_row: 1,
                cursor_col: 0,
            }
        );
    }

    #[test]
    fn input_layout_metrics_wraps_long_lines_for_cursor_tracking() {
        let text = "abcdef";
        let metrics = input_layout_metrics(text, text.len(), 4);
        assert_eq!(
            metrics,
            InputLayoutMetrics {
                visual_lines: 2,
                cursor_row: 1,
                cursor_col: 2,
            }
        );
    }

    #[test]
    fn input_layout_metrics_handles_wide_chars_across_wraps() {
        let text = "ab界c";
        let metrics = input_layout_metrics(text, text.len(), 4);
        assert_eq!(
            metrics,
            InputLayoutMetrics {
                visual_lines: 2,
                cursor_row: 1,
                cursor_col: 1,
            }
        );
    }
}

#[cfg(all(test, feature = "stylos"))]
mod tests {
    use super::*;

    fn handle(agent_id: &str, roles: &[&str]) -> AgentHandle {
        AgentHandle {
            agent: None,
            session_id: Uuid::nil(),
            agent_id: agent_id.to_string(),
            label: agent_id.to_string(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn stylos_note_display_identifier_prefers_slug() {
        let prompt = "type=stylos_note note_id=123e4567-e89b-12d3-a456-426614174000 note_slug=fix-tests-123e4567 column=todo\n\nbody";
        assert_eq!(
            stylos_note_display_identifier(prompt),
            "note_slug=fix-tests-123e4567"
        );
    }

    #[test]
    fn stylos_note_display_identifier_falls_back_to_note_id() {
        let prompt =
            "type=stylos_note note_id=123e4567-e89b-12d3-a456-426614174000 column=todo\n\nbody";
        assert_eq!(
            stylos_note_display_identifier(prompt),
            "note_id=123e4567-e89b-12d3-a456-426614174000"
        );
    }

    #[test]
    fn validate_agent_roles_accepts_one_main_and_one_interactive() {
        let agents = vec![
            handle("main", &["main", "interactive"]),
            handle("worker", &["background"]),
        ];
        validate_agent_roles(&agents).unwrap();
    }

    #[test]
    fn validate_agent_roles_rejects_zero_main() {
        let agents = vec![handle("worker", &["background"])];
        assert!(validate_agent_roles(&agents).is_err());
    }

    #[test]
    fn validate_agent_roles_rejects_two_main() {
        let agents = vec![handle("a", &["main"]), handle("b", &["main"])];
        assert!(validate_agent_roles(&agents).is_err());
    }

    #[test]
    fn build_snapshot_preserves_multiple_agents_and_startup_dir() {
        let startup = PathBuf::from(".");
        let snapshot = build_stylos_status_snapshot(
            &startup,
            vec![
                AgentStatusSource {
                    agent_id: "main".to_string(),
                    label: "main".to_string(),
                    roles: vec!["main".to_string(), "interactive".to_string()],
                    session_id: "s1".to_string(),
                    workflow: WorkflowState::default(),
                    activity_status: "idle".to_string(),
                    activity_status_changed_at_ms: 1,
                    project_dir: PathBuf::from("."),
                    provider: "p1".to_string(),
                    model: "m1".to_string(),
                    active_profile: "prof1".to_string(),
                    rate_limits: None,
                },
                AgentStatusSource {
                    agent_id: "worker".to_string(),
                    label: "worker".to_string(),
                    roles: vec!["background".to_string()],
                    session_id: "s2".to_string(),
                    workflow: WorkflowState::default(),
                    activity_status: "idle".to_string(),
                    activity_status_changed_at_ms: 2,
                    project_dir: PathBuf::from("."),
                    provider: "p2".to_string(),
                    model: "m2".to_string(),
                    active_profile: "prof2".to_string(),
                    rate_limits: None,
                },
            ],
        )
        .unwrap();

        assert_eq!(snapshot.agents.len(), 2);
        assert_eq!(snapshot.agents[0].roles, vec!["main", "interactive"]);
        assert_eq!(snapshot.agents[1].provider, "p2");
        assert_eq!(snapshot.startup_project_dir, startup.display().to_string());
    }

    #[test]
    fn targeted_remote_request_prefers_matching_agent_id() {
        let agents = vec![
            handle("main", &["main", "interactive"]),
            handle("worker", &["background"]),
        ];
        let request = IncomingPromptRequest {
            prompt: "hi".to_string(),
            agent_id: Some("worker".to_string()),
            task_id: None,
            request_id: None,
            from: Some("peer-1:1234".to_string()),
            from_agent_id: Some("main".to_string()),
            to: Some("peer-2:5678".to_string()),
            to_agent_id: Some("worker".to_string()),
        };
        let target = request.agent_id.as_deref();
        let index = target
            .and_then(|agent_id| agents.iter().position(|h| h.agent_id == agent_id))
            .or_else(|| agents.iter().position(is_interactive_handle))
            .unwrap();
        assert_eq!(agents[index].agent_id, "worker");
    }

    #[test]
    fn extract_stylos_talk_target_from_detail_reads_exact_instance() {
        let detail = "stylos_request_talk instance=node-2:77, to_agent_id=main, message=hello";
        assert_eq!(
            extract_stylos_talk_target_from_detail(detail),
            Some("node-2:77,")
        );
    }

    #[test]
    fn sender_side_stylos_talk_log_format_is_exact() {
        let target = extract_stylos_talk_target_from_detail(
            "stylos_request_talk instance=node-2:77 to_agent_id=main",
        )
        .unwrap();
        let text = format!("Stylos talk to={} from={}", target, "node-1:42");
        assert_eq!(text, "Stylos talk to=node-2:77 from=node-1:42");
        assert!(!text.contains('/'));
    }
}

fn format_duration_ms(ms: u64) -> String {
    if ms >= 60_000 {
        format!("{:.1}m", ms as f64 / 60_000.0)
    } else if ms >= 1_000 {
        format!("{:.2}s", ms as f64 / 1_000.0)
    } else {
        format!("{}ms", ms)
    }
}

fn per_second(count: u64, window_ms: u64) -> f64 {
    if window_ms == 0 {
        0.0
    } else {
        count as f64 / (window_ms as f64 / 1_000.0)
    }
}

fn avg_us(total_us: u64, count: u64) -> u64 {
    if count == 0 {
        0
    } else {
        total_us / count
    }
}

fn format_runtime_activity_lines(
    counters: &ActivityCountersSnapshot,
    window_ms: u64,
) -> Vec<String> {
    vec![
        format!(
            "recent activity counts: draws={} draw_requests={} draw_skipped_clean={} ticks={} keys={} mouse={} paste={} commands={}",
            counters.draw_count,
            counters.draw_request_count,
            counters.draw_skip_clean_count,
            counters.tick_count,
            counters.input_key_count,
            counters.input_mouse_count,
            counters.input_paste_count,
            counters.command_count,
        ),
        format!(
            "recent activity rates: draw={:.2}/s tick={:.2}/s input={:.2}/s agent_events={:.2}/s incoming_prompts={:.2}/s",
            per_second(counters.draw_count, window_ms),
            per_second(counters.tick_count, window_ms),
            per_second(counters.input_key_count + counters.input_mouse_count + counters.input_paste_count, window_ms),
            per_second(counters.agent_event_count, window_ms),
            per_second(counters.incoming_prompt_count, window_ms),
        ),
        format!(
            "recent task activity: agent_turns started={} completed={} shell_completions={} draw_avg={} draw_max=lifetime:{}",
            counters.agent_turn_started_count,
            counters.agent_turn_completed_count,
            counters.shell_complete_count,
            format_duration_ms(avg_us(counters.draw_total_us, counters.draw_count) / 1_000),
            format_duration_ms(counters.draw_max_us / 1_000),
        ),
        "task metrics are Themion activity counters and approximate handler timing, not exact Tokio task CPU percentages".to_string(),
    ]
}

fn format_runtime_lifetime_lines(counters: &ActivityCountersSnapshot) -> Vec<String> {
    vec![
        format!(
            "lifetime activity counts: draws={} draw_requests={} draw_skipped_clean={} ticks={} keys={} mouse={} paste={} commands={}",
            counters.draw_count,
            counters.draw_request_count,
            counters.draw_skip_clean_count,
            counters.tick_count,
            counters.input_key_count,
            counters.input_mouse_count,
            counters.input_paste_count,
            counters.command_count,
        ),
        format!(
            "lifetime task activity: agent_turns started={} completed={} shell_completions={} draw_avg={} draw_max={}",
            counters.agent_turn_started_count,
            counters.agent_turn_completed_count,
            counters.shell_complete_count,
            format_duration_ms(avg_us(counters.draw_total_us, counters.draw_count) / 1_000),
            format_duration_ms(counters.draw_max_us / 1_000),
        ),
    ]
}

#[cfg(feature = "stylos")]
fn format_stylos_activity_lines(snapshot: StylosActivitySnapshot) -> Vec<String> {
    vec![
        format!(
            "  status_publish count={} avg={} max={}",
            snapshot.status_publish_count,
            format_duration_ms(
                avg_us(
                    snapshot.status_publish_total_us,
                    snapshot.status_publish_count
                ) / 1_000
            ),
            format_duration_ms(snapshot.status_publish_max_us / 1_000),
        ),
        format!(
            "  query_request count={} avg={} max={}",
            snapshot.query_request_count,
            format_duration_ms(
                avg_us(
                    snapshot.query_request_total_us,
                    snapshot.query_request_count
                ) / 1_000
            ),
            format_duration_ms(snapshot.query_request_max_us / 1_000),
        ),
        format!(
            "  bridges cmd_events={} prompt_events={} event_messages={}",
            snapshot.cmd_event_count, snapshot.prompt_event_count, snapshot.event_message_count,
        ),
    ]
}

fn sample_thread_cpu_lines() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        let Ok(entries) = fs::read_dir("/proc/self/task") else {
            return vec!["linux thread snapshot unavailable".to_string()];
        };
        let mut out = Vec::new();
        for entry in entries.flatten().take(8) {
            let tid = entry.file_name().to_string_lossy().to_string();
            let stat_path = entry.path().join("stat");
            let Ok(stat) = fs::read_to_string(stat_path) else {
                continue;
            };
            if let Some(line) = parse_linux_thread_stat_line(&tid, &stat) {
                out.push(line);
            }
        }
        if out.is_empty() {
            vec!["linux thread snapshot unavailable".to_string()]
        } else {
            out
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        vec!["thread cpu snapshot unavailable on this platform".to_string()]
    }
}

#[cfg(target_os = "linux")]
fn parse_linux_thread_stat_line(tid: &str, stat: &str) -> Option<String> {
    let close = stat.rfind(')')?;
    let after = stat.get(close + 2..)?;
    let fields: Vec<&str> = after.split_whitespace().collect();
    if fields.len() < 15 {
        return None;
    }
    let state = fields[0];
    let utime = fields.get(11)?;
    let stime = fields.get(12)?;
    let comm_start = stat.find('(')? + 1;
    let comm = &stat[comm_start..close];
    Some(format!(
        "tid={} name={} state={} cpu_ticks=user:{} system:{} (sampled total, not percent)",
        tid, comm, state, utime, stime
    ))
}
