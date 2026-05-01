use crate::app_runtime::{
    build_local_agent_tool_invoker, build_main_agent, build_replacement_main_agent,
    build_system_inspection_snapshot, handle_local_agent_management_request as runtime_handle_local_agent_management_request,
    AgentReplacementParams, LocalAgentManagementRequest, LocalAgentRuntimeContext,
};
#[cfg(feature = "stylos")]
use crate::board_runtime::{
    finalize_board_note_injection, resolve_completed_note_follow_up, BoardTurnFollowUp,
    LocalBoardClaimRegistry,
};
use crate::chat_composer::{ChatComposer, InputAction};
use crate::config::{save_profiles, ProfileConfig};
use crate::runtime_domains::DomainHandle;
#[cfg(feature = "stylos")]
use crate::stylos::{
    sender_side_transport_event_from_tool_detail, tool_bridge, IncomingPromptRequest, StylosHandle,
    StylosRuntimeState, StylosToolBridge,
};

#[cfg(feature = "stylos")]
use crate::app_runtime::{
    resolve_incoming_prompt_disposition, select_watchdog_dispatch, start_watchdog_task,
    IncomingPromptDisposition, WatchdogRuntimeState,
};
use crate::{format_stats, Session};
use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
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
use themion_core::{
    EstimateMode, PromptContextReport, PromptSectionKind, ReplayForm, TokenizerResolutionSource,
    ToolEstimateMode,
};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

pub(crate) enum AppEvent {
    Key(event::KeyEvent),
    Mouse(event::MouseEvent),
    Paste(String),
    Agent(Uuid, AgentEvent),
    AgentReady(Box<Agent>, Uuid),
    Tick,
    #[cfg(feature = "stylos")]
    StylosCmd(crate::stylos::StylosCmdRequest),
    #[cfg(feature = "stylos")]
    IncomingPrompt(IncomingPromptRequest),
    #[cfg(feature = "stylos")]
    WatchdogPoll,
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
    LocalAgentManagement(LocalAgentManagementRequest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NonAgentSource {
    Board,
    Stylos,
    Runtime,
    Watchdog,
}

impl NonAgentSource {
    fn label(self) -> &'static str {
        match self {
            Self::Board => "BOARD",
            Self::Stylos => "STYLOS",
            Self::Runtime => "RUNTIME",
            Self::Watchdog => "WATCHDOG",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Board => Color::Yellow,
            Self::Stylos => Color::Cyan,
            Self::Runtime => Color::Magenta,
            Self::Watchdog => Color::LightRed,
        }
    }
}

#[derive(Clone)]
enum Entry {
    User(String),
    Assistant {
        agent_id: Option<String>,
        text: String,
    },
    Banner(String),
    ToolCall {
        agent_id: Option<String>,
        detail: String,
        reason: Option<String>,
    },
    ToolDone,
    Status {
        agent_id: Option<String>,
        source: Option<NonAgentSource>,
        text: String,
    },
    #[cfg(feature = "stylos")]
    RemoteEvent {
        agent_id: Option<String>,
        source: Option<NonAgentSource>,
        text: String,
    },
    TurnDone {
        agent_id: Option<String>,
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
    Watchdog,
}

const TOOL_DETAIL_MAX_CHARS: usize = 60;
const TOOL_DETAIL_CENTER_TRIM_MARKER: &str = "󱑼";
const CTRL_C_EXIT_CONFIRM_WINDOW: Duration = Duration::from_secs(3);
const CONTEXT_HISTORY_TURN_DISPLAY_MAX_AGE: usize = 10;

fn agent_tag_color(index: usize) -> Color {
    const PALETTE: [Color; 6] = [
        Color::Cyan,
        Color::Yellow,
        Color::Green,
        Color::Magenta,
        Color::Blue,
        Color::LightRed,
    ];
    PALETTE[index % PALETTE.len()]
}

fn agent_tag_style(agent_id: &str, agents: &[AgentHandle]) -> Style {
    let index = agents
        .iter()
        .position(|handle| handle.agent_id == agent_id)
        .unwrap_or(0);
    Style::default()
        .fg(agent_tag_color(index))
        .add_modifier(Modifier::BOLD)
}

fn agent_tag_spans(agent_id: Option<&str>, agents: &[AgentHandle]) -> Vec<Span<'static>> {
    match agent_id {
        Some(agent_id) => vec![
            Span::raw("  "),
            Span::styled(format!("[{agent_id}] "), agent_tag_style(agent_id, agents)),
        ],
        None => vec![Span::raw("  ")],
    }
}

fn non_agent_source_spans(source: Option<NonAgentSource>) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw("  ")];
    if let Some(source) = source {
        spans.push(Span::styled(
            format!("[{}] ", source.label()),
            Style::default()
                .fg(source.color())
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

fn center_trim(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }

    let marker_chars: Vec<char> = TOOL_DETAIL_CENTER_TRIM_MARKER.chars().collect();
    if max <= marker_chars.len() {
        return marker_chars.into_iter().take(max).collect();
    }

    let remaining = max - marker_chars.len();
    let prefix_len = remaining / 2;
    let suffix_len = remaining - prefix_len;

    let prefix: String = chars[..prefix_len].iter().collect();
    let suffix: String = chars[chars.len() - suffix_len..].iter().collect();
    format!("{}{}{}", prefix, TOOL_DETAIL_CENTER_TRIM_MARKER, suffix)
}

fn format_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn format_context_report(report: &PromptContextReport) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!(
        "prompt estimate: {} chars ≈ {} tokens",
        format_count(report.total_chars),
        format_count(report.total_tokens_estimate)
    ));
    match report.estimate_mode {
        EstimateMode::Tokenizer => {
            out.push("estimate mode: tokenizer".to_string());
            let tokenizer_label = match (
                report.tokenizer_name.as_deref(),
                report.tokenizer_resolution_source,
            ) {
                (Some(name), Some(TokenizerResolutionSource::ExactModelMatch)) => {
                    format!("tokenizer: {} (exact model match)", name)
                }
                (Some(name), Some(TokenizerResolutionSource::TrustedFallbackMapping)) => {
                    format!("tokenizer: {} (trusted fallback mapping)", name)
                }
                (Some(name), None) => format!("tokenizer: {}", name),
                _ => "tokenizer: unavailable".to_string(),
            };
            out.push(tokenizer_label);
        }
        EstimateMode::RoughFallback => {
            out.push("estimate mode: rough fallback".to_string());
            out.push("tokenizer: unavailable".to_string());
        }
    }
    out.push(format!(
        "turns: total={} replayed={} reduced={} omitted={}",
        report.total_turns, report.replayed_turns, report.reduced_turns, report.omitted_turns
    ));
    if report.cap_omitted_turns > 0 {
        out.push(format!(
            "history replay cap: omitted {} turn(s) older than T-7",
            report.cap_omitted_turns
        ));
    }
    if report.t0_exceeds_spike_budget {
        out.push(
            "history mode: T0 alone exceeds spike budget; prior turns not replayed".to_string(),
        );
    } else if report.t0_exceeds_normal_budget {
        out.push(
            "history mode: recent prior turns reduced because T0 exceeds normal budget".to_string(),
        );
    }
    out.push("sections:".to_string());
    for section in &report.sections {
        if section.kind == PromptSectionKind::HistoryReplay && report.total_turns == 0 {
            out.push("  history replay: 0 chars ≈ 0 tokens".to_string());
            continue;
        }
        if section.kind == PromptSectionKind::ToolDefinitions {
            if let Some(tool_estimate) = &section.tool_estimate {
                match tool_estimate.mode {
                    ToolEstimateMode::RawPlusEffective => {
                        out.push(format!(
                            "  {}: raw {} tok; effective ~{} tok; mode=raw_plus_effective",
                            section.label,
                            format_count(tool_estimate.raw_tokens),
                            format_count(
                                tool_estimate
                                    .effective_tokens
                                    .unwrap_or(tool_estimate.raw_tokens)
                            )
                        ));
                        out.push(
                            "    effective estimate discounts schema structure overhead"
                                .to_string(),
                        );
                    }
                    ToolEstimateMode::RawOnly => {
                        out.push(format!(
                            "  {}: raw {} tok; mode=raw_only",
                            section.label,
                            format_count(tool_estimate.raw_tokens)
                        ));
                    }
                }
                continue;
            }
        }
        out.push(format!(
            "  {}: {} chars ≈ {} tokens",
            section.label,
            format_count(section.chars),
            format_count(section.tokens_estimate)
        ));
    }
    if !report.history_turns.is_empty() {
        out.push("history turns:".to_string());
        for turn in report.history_turns.iter().filter(|turn| {
            if turn.turn_label == "T0" {
                return true;
            }
            turn.turn_label
                .strip_prefix("T-")
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|age| age <= CONTEXT_HISTORY_TURN_DISPLAY_MAX_AGE)
        }) {
            if turn.omitted {
                out.push(format!(
                    "  {}: omitted{}",
                    turn.turn_label,
                    turn.note
                        .as_deref()
                        .map(|note| format!(" ({})", note))
                        .unwrap_or_default()
                ));
            } else {
                let mode = match turn.replay_form {
                    ReplayForm::Full => "full",
                    ReplayForm::PureMessage => "reduced",
                };
                let note = turn
                    .note
                    .as_deref()
                    .map(|note| format!(" ({})", note))
                    .unwrap_or_default();
                out.push(format!(
                    "  {}: {} {} chars ≈ {} tokens{}",
                    turn.turn_label,
                    mode,
                    format_count(turn.chars),
                    format_count(turn.tokens_estimate),
                    note
                ));
            }
        }
    }
    out
}

fn self_session_id_fallback() -> String {
    "session-bound".to_string()
}

fn agent_id_for_session(agents: &[AgentHandle], sid: Uuid) -> Option<String> {
    agents
        .iter()
        .find(|handle| handle.session_id == sid)
        .map(|handle| handle.agent_id.clone())
}

fn split_tool_call_detail(name: &str, args_json: &str) -> (String, Option<String>) {
    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
    let t = |key: &str| center_trim(args[key].as_str().unwrap_or("?"), TOOL_DETAIL_MAX_CHARS);
    let board_note_display = || {
        center_trim(
            args["note_slug"]
                .as_str()
                .or_else(|| args["note_id"].as_str())
                .unwrap_or("?"),
            TOOL_DETAIL_MAX_CHARS,
        )
    };
    let reason = args["reason"]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let with_reason_first = |detail: String| match &reason {
        Some(reason) => (reason.clone(), Some(detail)),
        None => (detail, None),
    };
    match name {
        "shell_run_command" | "bash" => with_reason_first(format!("shell: {}", t("command"))),
        "fs_read_file" | "read_file" => with_reason_first(format!("read: {}", t("path"))),
        "fs_write_file" | "write_file" => with_reason_first(format!("write: {}", t("path"))),
        "fs_list_directory" | "list_directory" => with_reason_first(format!("ls: {}", t("path"))),
        "history_recall" | "recall_history" => (
            format!(
                "history_recall: session={}",
                center_trim(
                    &args["session_id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| self_session_id_fallback()),
                    TOOL_DETAIL_MAX_CHARS,
                )
            ),
            None,
        ),
        "history_search" | "search_history" => (format!("history_search: {}", t("query")), None),
        "workflow_get_state" | "get_workflow_state" => ("workflow: inspect".to_string(), None),
        "workflow_set_active" | "set_workflow" => {
            (format!("workflow: set {}", t("workflow")), None)
        }
        "workflow_set_phase" | "set_workflow_phase" => {
            (format!("workflow: phase {}", t("phase")), None)
        }
        "workflow_complete" | "complete_workflow" => {
            (format!("workflow: complete {}", t("outcome")), None)
        }
        "stylos_request_talk" => (
            format!(
                "stylos_request_talk instance={} to_agent_id={}",
                t("instance"),
                center_trim(
                    args["to_agent_id"]
                        .as_str()
                        .or_else(|| args["agent_id"].as_str())
                        .unwrap_or("master"),
                    TOOL_DETAIL_MAX_CHARS,
                )
            ),
            None,
        ),
        "board_create_note" => {
            let raw_to_instance = args["to_instance"].as_str().unwrap_or("?").trim();
            let resolved_to_instance = match raw_to_instance {
                "SELF" => "self",
                "" => "?",
                value => value,
            };
            let raw_to_agent = args["to_agent_id"].as_str().unwrap_or("?").trim();
            let resolved_to_agent = match raw_to_agent {
                "SELF" => "self",
                "" => "?",
                value => value,
            };
            (
                format!(
                    "board_create_note {}:{}",
                    center_trim(resolved_to_instance, TOOL_DETAIL_MAX_CHARS),
                    center_trim(resolved_to_agent, TOOL_DETAIL_MAX_CHARS)
                ),
                None,
            )
        }
        "board_list_notes" => (
            format!(
                "board_list_notes column={}",
                center_trim(
                    args["column"].as_str().unwrap_or("?"),
                    TOOL_DETAIL_MAX_CHARS
                )
            ),
            None,
        ),
        "board_read_note" => (format!("board_read_note {}", board_note_display()), None),
        "board_move_note" => (
            format!(
                "board_move_note {} -> {}",
                board_note_display(),
                center_trim(
                    args["column"].as_str().unwrap_or("?"),
                    TOOL_DETAIL_MAX_CHARS
                )
            ),
            None,
        ),
        "board_update_note_result" => (
            format!("board_update_note_result {}", board_note_display()),
            None,
        ),
        "memory_create_node" => (format!("memory_create_node {}", t("title")), None),
        "memory_update_node" => (format!("memory_update_node {}", t("node_id")), None),
        "memory_link_nodes" => (
            format!(
                "memory_link_nodes {} -> {}",
                t("from_node_id"),
                t("to_node_id")
            ),
            None,
        ),
        "memory_unlink_nodes" => (
            format!(
                "memory_unlink_nodes {} -> {}",
                t("from_node_id"),
                t("to_node_id")
            ),
            None,
        ),
        "memory_get_node" => (format!("memory_get_node {}", t("node_id")), None),
        "memory_search" => with_reason_first(format!("memory_search {}", t("query"))),
        "memory_open_graph" => (format!("memory_open_graph {}", t("node_id")), None),
        "memory_delete_node" => (format!("memory_delete_node {}", t("node_id")), None),
        "memory_list_hashtags" => (format!("memory_list_hashtags {}", t("prefix")), None),
        "local_agent_create" => (format!("local_agent_create {}", t("agent_id")), None),
        "local_agent_delete" => (format!("local_agent_delete {}", t("agent_id")), None),
        "system_inspect_local" => ("system_inspect_local".to_string(), None),
        other => (other.to_string(), None),
    }
}

fn is_interactive_handle(handle: &AgentHandle) -> bool {
    handle.roles.iter().any(|r| r == "interactive")
}


#[cfg(feature = "stylos")]
fn normalize_primary_role(value: &str) -> &str {
    if value == "main" {
        "master"
    } else {
        value
    }
}

#[cfg(feature = "stylos")]
fn has_role(handle: &AgentHandle, role: &str) -> bool {
    let role = normalize_primary_role(role);
    handle
        .roles
        .iter()
        .any(|r| normalize_primary_role(r) == role)
}

pub struct AgentHandle {
    pub(crate) agent: Option<Agent>,
    pub(crate) session_id: Uuid,
    #[allow(dead_code)]
    pub agent_id: String,
    #[allow(dead_code)]
    pub label: String,
    pub(crate) roles: Vec<String>,
    pub(crate) busy: bool,
    pub(crate) turn_cancellation: Option<TurnCancellation>,
    #[cfg(feature = "stylos")]
    pub(crate) active_incoming_prompt: Option<IncomingPromptRequest>,
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

pub struct App {
    #[cfg(feature = "stylos")]
    pub(crate) stylos: Option<StylosHandle>,
    #[cfg(feature = "stylos")]
    local_stylos_instance: Option<String>,
    session: Session,
    entries: Vec<Entry>,
    pending: Option<String>,
    composer: ChatComposer,
    pub(crate) running: bool,
    ctrl_c_exit_armed_until: Option<Instant>,
    agent_busy: bool,
    scroll_offset: usize,
    navigation_mode: NavigationMode,
    review_mode: ReviewMode,
    review_scroll_offset: usize,
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
    api_log_enabled: bool,
    process_started_at: Instant,
    process_started_at_ms: u64,
    background_domain: DomainHandle,
    core_domain: DomainHandle,
    recent_runtime_snapshots: VecDeque<RuntimeMetricsSnapshot>,
    activity_counters: ActivityCounters,
    workflow_state: WorkflowState,
    #[cfg(feature = "stylos")]
    watchdog_state: Arc<WatchdogRuntimeState>,
    #[cfg(feature = "stylos")]
    board_claims: Arc<LocalBoardClaimRegistry>,
    #[cfg(feature = "stylos")]
    last_assistant_text: Option<String>,
    #[cfg(feature = "stylos")]
    stylos_tool_bridge: Option<StylosToolBridge>,
    #[cfg(feature = "stylos")]
    last_sender_side_transport_event: Option<crate::stylos::SenderSideTransportEvent>,
    local_agent_mgmt_tx: mpsc::UnboundedSender<AppEvent>,
}

impl App {
    pub fn new(
        session: Session,
        db: Arc<DbHandle>,
        session_id: Uuid,
        project_dir: PathBuf,
        background_domain: DomainHandle,
        core_domain: DomainHandle,
        app_tx: mpsc::UnboundedSender<AppEvent>,
        #[cfg(feature = "stylos")] stylos: Option<StylosHandle>,
    ) -> Self {
        #[cfg(feature = "stylos")]
        let stylos_tool_bridge = stylos.as_ref().and_then(tool_bridge);
        #[cfg(feature = "stylos")]
        let local_stylos_instance = stylos.as_ref().and_then(|handle| match handle.state() {
            StylosRuntimeState::Active { instance, .. } => Some(instance.clone()),
            _ => Some(crate::stylos::derive_local_instance_id()),
        });
        #[cfg(feature = "stylos")]
        let watchdog_state = Arc::new(WatchdogRuntimeState::default());
        #[cfg(feature = "stylos")]
        let board_claims = Arc::new(LocalBoardClaimRegistry::default());
        let agent = build_main_agent(
            &session,
            db.clone(),
            session_id,
            project_dir.clone(),
            app_tx.clone(),
            #[cfg(feature = "stylos")]
            stylos_tool_bridge.clone(),
            #[cfg(feature = "stylos")]
            local_stylos_instance.as_deref(),
            #[cfg(feature = "stylos")]
            "master",
            None,
            false,
        )
        .expect("failed to build agent");
        let initial_model_info = session.model_info.clone();
        let handle = AgentHandle {
            agent: Some(agent),
            session_id,
            agent_id: "master".to_string(),
            label: "master".to_string(),
            roles: vec!["master".to_string(), "interactive".to_string()],
            busy: false,
            turn_cancellation: None,
            #[cfg(feature = "stylos")]
            active_incoming_prompt: None,
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
            Entry::Assistant {
                agent_id: None,
                text: format!(
                    "version: {}  |  profile: {}  |  model: {}",
                    env!("CARGO_PKG_VERSION"),
                    session.active_profile,
                    session.model,
                ),
            },
            Entry::Assistant {
                agent_id: None,
                text: format!("project directory: {}", project_display),
            },
            Entry::Assistant {
                agent_id: None,
                text: "type /config to change settings, /exit to quit, Alt-t transcript review"
                    .to_string(),
            },
            Entry::Blank,
        ];

        #[cfg(feature = "stylos")]
        if let Some(handle) = stylos.as_ref() {
            match handle.state() {
                StylosRuntimeState::Off => initial_entries.push(Entry::Status {
                    agent_id: None,
                    source: Some(NonAgentSource::Stylos),
                    text: "stylos disabled".to_string(),
                }),
                StylosRuntimeState::Active {
                    mode,
                    realm,
                    instance,
                } => initial_entries.push(Entry::Status {
                    agent_id: None,
                    source: Some(NonAgentSource::Stylos),
                    text: format!(
                        "stylos ready: mode={} realm={} instance={}",
                        mode, realm, instance
                    ),
                }),
                StylosRuntimeState::Error(err) => initial_entries.push(Entry::Status {
                    agent_id: None,
                    source: Some(NonAgentSource::Stylos),
                    text: format!("stylos start failed: {}", err),
                }),
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
            composer: ChatComposer::new(),
            ctrl_c_exit_armed_until: None,
            running: true,
            agent_busy: false,
            scroll_offset: 0,
            navigation_mode: NavigationMode::FollowTail,
            review_mode: ReviewMode::Closed,
            review_scroll_offset: 0,
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
                last_api_call_tokens_in: None,
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
            api_log_enabled: false,
            process_started_at: Instant::now(),
            process_started_at_ms: unix_epoch_now_ms(),
            background_domain,
            core_domain,
            recent_runtime_snapshots: VecDeque::new(),
            activity_counters: ActivityCounters::default(),
            workflow_state: WorkflowState::default(),
            #[cfg(feature = "stylos")]
            watchdog_state,
            #[cfg(feature = "stylos")]
            board_claims,
            #[cfg(feature = "stylos")]
            last_assistant_text: None,
            #[cfg(feature = "stylos")]
            stylos_tool_bridge,
            #[cfg(feature = "stylos")]
            last_sender_side_transport_event: None,
            local_agent_mgmt_tx: app_tx.clone(),
        };
        #[cfg(feature = "stylos")]
        start_watchdog_task(&app.background_domain, app_tx, app.watchdog_state.clone());
        #[cfg(feature = "stylos")]
        app.watchdog_state.set_idle_started_now();
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
        self.agents.iter_mut().find(|h| has_role(h, "master"))
    }

    fn replace_master_agent(&mut self, new_agent: Agent, new_session_id: Uuid) {
        self.status_model_info = new_agent.model_info().cloned();
        self.workflow_state = new_agent.workflow_state().clone();
        let mut replacement = Some(AgentHandle {
            agent: Some(new_agent),
            session_id: new_session_id,
            agent_id: "master".to_string(),
            label: "master".to_string(),
            roles: vec!["master".to_string(), "interactive".to_string()],
            busy: false,
            turn_cancellation: None,
            #[cfg(feature = "stylos")]
            active_incoming_prompt: None,
        });
        let mut retained = self
            .agents
            .drain(..)
            .filter(|handle| !handle.roles.iter().any(|r| r == "master"))
            .collect::<Vec<_>>();
        let mut next_agents = Vec::with_capacity(retained.len() + 1);
        next_agents.push(replacement.take().expect("replacement present"));
        next_agents.append(&mut retained);
        self.agents = next_agents;
    }

    fn background_domain(&self) -> DomainHandle {
        self.background_domain.clone()
    }


    fn any_agent_busy(&self) -> bool {
        self.agents.iter().any(|h| h.busy)
    }

    fn handle_local_agent_management_request(
        &mut self,
        request: LocalAgentManagementRequest,
        frame_requester: &FrameRequester,
    ) {
        let local_agent_tool_invoker =
            build_local_agent_tool_invoker(self.local_agent_mgmt_tx.clone());
        let any_agent_busy = self.any_agent_busy();
        let result = runtime_handle_local_agent_management_request(
            LocalAgentRuntimeContext {
                session: &self.session,
                project_dir: &self.project_dir,
                db: &self.db,
                agents: &mut self.agents,
                agent_busy: any_agent_busy,
                #[cfg(feature = "stylos")]
                stylos_tool_bridge: self.stylos_tool_bridge.clone(),
                #[cfg(feature = "stylos")]
                local_stylos_instance: self.local_stylos_instance.as_deref(),
                local_agent_tool_invoker: Some(local_agent_tool_invoker),
                api_log_enabled: self.api_log_enabled,
            },
            &request.action,
            request.args,
        );
        self.refresh_main_agent_system_inspection();
        self.mark_dirty_all();
        self.request_draw(frame_requester);
        let _ = request.reply_tx.send(result);
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

    fn open_watchdog_review(&mut self) {
        self.review_mode = ReviewMode::Watchdog;
        self.review_scroll_offset = 0;
    }

    fn toggle_watchdog_review(&mut self) {
        if self.review_mode == ReviewMode::Watchdog {
            self.review_mode = ReviewMode::Closed;
        } else {
            self.open_watchdog_review();
        }
    }

    fn transcript_review_open(&self) -> bool {
        self.review_mode == ReviewMode::Transcript
    }

    fn close_review(&mut self) {
        self.review_mode = ReviewMode::Closed;
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
        #[cfg(feature = "stylos")]
        {
            self.watchdog_state.clear_idle_started();
            self.watchdog_state.set_pending_watchdog_note(false);
        }
        self.pending = Some(self.pending_str());
        self.mark_dirty_status();
    }

    fn clear_agent_activity(&mut self) {
        self.agent_activity = None;
        self.agent_activity_changed_at = None;
        self.idle_since = Some(Instant::now());
        self.idle_status_changed_at = Some(unix_epoch_now_ms());
        #[cfg(feature = "stylos")]
        {
            self.watchdog_state.set_idle_started_now();
            self.watchdog_state.set_active_incoming_prompt(
                self.agents
                    .iter()
                    .any(|handle| handle.active_incoming_prompt.is_some()),
            );
        }
        self.pending = None;
        self.mark_dirty_status();
    }

    fn reset_stream_counters(&mut self) {
        self.stream_chunks = 0;
        self.stream_chars = 0;
    }

    fn request_interrupt(&mut self) {
        let mut interrupted_any = false;
        for handle in &self.agents {
            if let Some(cancel) = &handle.turn_cancellation {
                if !cancel.is_interrupted() {
                    cancel.interrupt();
                    interrupted_any = true;
                }
            }
        }
        if interrupted_any {
            self.push(Entry::Status {
                agent_id: None,
                source: Some(NonAgentSource::Runtime),
                text: "interrupt requested".to_string(),
            });
        }
    }

    fn arm_ctrl_c_exit(&mut self) {
        self.ctrl_c_exit_armed_until = Some(Instant::now() + CTRL_C_EXIT_CONFIRM_WINDOW);
        self.push(Entry::Status {
            agent_id: None,
            source: Some(NonAgentSource::Runtime),
            text: "Press Ctrl+C again within 3s to exit".to_string(),
        });
        self.mark_dirty_status();
    }

    fn ctrl_c_exit_is_armed(&self, now: Instant) -> bool {
        matches!(self.ctrl_c_exit_armed_until, Some(deadline) if deadline > now)
    }

    fn expire_ctrl_c_exit_if_needed(&mut self, now: Instant) -> bool {
        if matches!(self.ctrl_c_exit_armed_until, Some(deadline) if deadline <= now) {
            self.ctrl_c_exit_armed_until = None;
            return true;
        }
        false
    }

    fn on_tick(&mut self) {
        self.activity_counters.tick_count += 1;
        self.expire_ctrl_c_exit_if_needed(Instant::now());
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
            _ => {
                #[cfg(feature = "stylos")]
                if self.watchdog_state.pending_watchdog_note() {
                    return "idle-watchdog".to_string();
                }
                "idle".to_string()
            }
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

    fn process_agent_event(
        &mut self,
        #[allow(unused_variables)] sid: Uuid,
        ev: AgentEvent,
        #[cfg(feature = "stylos")] app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        match ev {
            AgentEvent::LlmStart => {
                #[cfg(feature = "stylos")]
                {
                    let agent_index = self.agents.iter().position(|h| h.session_id == sid);
                    if let (Some(agent_index), Some(handle)) = (agent_index, self.stylos.as_ref()) {
                        if let Some(remote) =
                            self.agents[agent_index].active_incoming_prompt.as_ref()
                        {
                            if let Some(task_id) = remote.task_id.clone() {
                                let query_context = handle.query_context();
                                self.background_domain.spawn(async move {
                                    query_context.task_registry().set_running(&task_id).await;
                                });
                            }
                        }
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
                let agent_id = agent_id_for_session(&self.agents, sid);
                match self.streaming_idx {
                    Some(i) => {
                        if let Some(Entry::Assistant { text, .. }) = self.entries.get_mut(i) {
                            text.push_str(&chunk);
                        }
                    }
                    None => {
                        self.push(Entry::Assistant {
                            agent_id,
                            text: chunk,
                        });
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
                self.push(Entry::Assistant {
                    agent_id: agent_id_for_session(&self.agents, sid),
                    text,
                });
            }
            AgentEvent::ToolStart {
                name,
                arguments_json,
                display_arguments_json,
            } => {
                self.streaming_idx = None;
                let display_args_json =
                    display_arguments_json.as_deref().unwrap_or(&arguments_json);
                let (detail, reason) = split_tool_call_detail(&name, display_args_json);
                let activity_detail = match &reason {
                    Some(reason) => format!("{detail} — {reason}"),
                    None => detail.clone(),
                };
                self.set_agent_activity(AgentActivity::RunningTool(activity_detail));
                #[cfg(feature = "stylos")]
                {
                    self.last_sender_side_transport_event = self
                        .local_stylos_instance
                        .as_deref()
                        .and_then(|local_instance| {
                            sender_side_transport_event_from_tool_detail(
                                &detail,
                                local_instance,
                                self.stylos_tool_bridge.is_some(),
                            )
                        });
                }
                self.push(Entry::ToolCall {
                    agent_id: agent_id_for_session(&self.agents, sid),
                    detail,
                    reason,
                });
            }
            AgentEvent::ToolEnd => {
                self.push(Entry::ToolDone);
                #[cfg(feature = "stylos")]
                if let Some(event) = self.last_sender_side_transport_event.take() {
                    self.push(Entry::RemoteEvent {
                        agent_id: event.agent_id,
                        source: Some(NonAgentSource::Stylos),
                        text: event.text,
                    });
                }
                self.set_agent_activity(AgentActivity::WaitingAfterTool);
            }
            AgentEvent::Status(text) => {
                self.push(Entry::Status {
                    agent_id: agent_id_for_session(&self.agents, sid),
                    source: None,
                    text,
                });
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
                {
                    let agent_index = self.agents.iter().position(|h| h.session_id == sid);
                    if let Some(agent_index) = agent_index {
                        self.maybe_emit_done_mention_for_completed_note(agent_index, app_tx);
                    }
                    if let (Some(agent_index), Some(handle)) = (agent_index, self.stylos.as_ref()) {
                        if let Some(remote) = self.agents[agent_index].active_incoming_prompt.take()
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
                    }
                }
                #[cfg(feature = "stylos")]
                self.watchdog_state.set_active_incoming_prompt(false);
                #[cfg(feature = "stylos")]
                self.watchdog_state.set_pending_watchdog_note(false);
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
                    agent_id: agent_id_for_session(&self.agents, sid),
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
                if let Some(last_api_call_tokens_in) = stats.last_api_call_tokens_in {
                    self.last_ctx_tokens = last_api_call_tokens_in;
                }
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

    fn task_runtime_snapshot(&self) -> themion_core::tools::SystemInspectionTaskRuntime {
        let current_activity = self.agent_activity.as_ref().map(|activity| match activity {
            AgentActivity::PreparingRequest => "preparing_request".to_string(),
            AgentActivity::WaitingForModel => "waiting_for_model".to_string(),
            AgentActivity::StreamingResponse => "streaming_response".to_string(),
            AgentActivity::RunningTool(_) => "running_tool".to_string(),
            AgentActivity::WaitingAfterTool => "waiting_after_tool".to_string(),
            AgentActivity::LoginStarting => "login_starting".to_string(),
            AgentActivity::WaitingForLoginBrowser => "waiting_for_login_browser".to_string(),
            AgentActivity::RunningShellCommand => "running_shell_command".to_string(),
            AgentActivity::Finishing => "finishing".to_string(),
        });
        let current_activity_detail = self
            .agent_activity
            .as_ref()
            .map(|activity| activity.label(self.stream_chunks, self.stream_chars));
        let mut runtime_notes = vec![
            "task metrics are Themion activity counters and approximate handler timing, not exact Tokio task CPU percentages".to_string(),
        ];
        let recent_window_ms = self
            .recent_runtime_delta()
            .map(|recent| recent.wall_elapsed_ms);
        if recent_window_ms.is_none() {
            runtime_notes.push(
                "recent task runtime window unavailable until more than one snapshot is recorded"
                    .to_string(),
            );
        }
        themion_core::tools::SystemInspectionTaskRuntime {
            status: if recent_window_ms.is_some() {
                "ok"
            } else {
                "partial"
            }
            .to_string(),
            current_activity,
            current_activity_detail,
            busy: Some(self.agent_busy),
            activity_status: Some(self.activity_status_value()),
            activity_status_changed_at_ms: self
                .agent_activity_changed_at
                .or(self.idle_status_changed_at),
            process_started_at_ms: Some(self.process_started_at_ms),
            uptime_ms: Some(self.process_started_at.elapsed().as_millis() as u64),
            recent_window_ms,
            runtime_notes,
        }
    }

    fn system_inspection_snapshot(&self) -> themion_core::tools::SystemInspectionResult {
        build_system_inspection_snapshot(
            &self.session,
            self.session.id,
            self.agents.first().map(|h| h.session_id),
            &self.project_dir,
            &self.workflow_state,
            self.status_rate_limits.as_ref(),
            self.task_runtime_snapshot(),
            self.debug_runtime_lines(),
        )
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
            self.push(Entry::Assistant {
                agent_id: None,
                text: "logging in to OpenAI Codex…".to_string(),
            });
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

        if input == "/context" {
            if let Some(handle) = self.agents.iter().find(|h| is_interactive_handle(h)) {
                if let Some(agent) = handle.agent.as_ref() {
                    return format_context_report(&agent.prompt_context_report());
                }
            }
            return vec!["context report unavailable".to_string()];
        }

        if input == "/debug api-log enable" {
            self.api_log_enabled = true;
            if let Some(handle) = self.agents.iter_mut().find(|h| is_interactive_handle(h)) {
                if let Some(agent) = handle.agent.as_mut() {
                    agent.set_api_log_enabled(true);
                }
            }
            return vec!["API call logging enabled for this session".to_string()];
        }

        if input == "/debug api-log disable" {
            self.api_log_enabled = false;
            if let Some(handle) = self.agents.iter_mut().find(|h| is_interactive_handle(h)) {
                if let Some(agent) = handle.agent.as_mut() {
                    agent.set_api_log_enabled(false);
                }
            }
            return vec!["API call logging disabled for this session".to_string()];
        }

        if let Some(rest) = input.strip_prefix("/debug api-log ") {
            return vec![format!(
                "usage: /debug api-log <enable|disable>  (got '{}')",
                rest.trim()
            )];
        }

        if input == "/semantic-memory index" || input == "/semantic-memory reindex" {
            #[cfg(not(feature = "semantic-memory"))]
            {
                return vec![
                    "semantic-memory indexing is unavailable in this build; enable the semantic-memory feature"
                        .to_string(),
                ];
            }
            #[cfg(feature = "semantic-memory")]
            {
                if self.agent_busy {
                    return vec!["busy, please wait".to_string()];
                }
                self.agent_busy = true;
                self.set_agent_activity(AgentActivity::RunningTool(
                    "semantic-memory index pending".to_string(),
                ));
                self.push(Entry::Assistant {
                    agent_id: None,
                    text: "indexing missing or pending Project Memory semantic embeddings…"
                        .to_string(),
                });
                let tx = app_tx.clone();
                let db = self.db.clone();
                self.background_domain().spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        db.memory_store().index_pending_embeddings(false)
                    })
                    .await;
                    let text = match result {
                        Ok(Ok(report)) => {
                            serde_json::to_string_pretty(&report).unwrap_or_else(|err| {
                                format!("indexing report serialization failed: {}", err)
                            })
                        }
                        Ok(Err(err)) => format!("semantic-memory indexing failed: {}", err),
                        Err(err) => format!("semantic-memory indexing task failed: {}", err),
                    };
                    let _ = tx.send(AppEvent::ShellComplete {
                        output: text,
                        exit_code: Some(0),
                    });
                });
                return out;
            }
        }

        if input == "/semantic-memory index full" || input == "/semantic-memory reindex full" {
            #[cfg(not(feature = "semantic-memory"))]
            {
                return vec![
                    "semantic-memory indexing is unavailable in this build; enable the semantic-memory feature"
                        .to_string(),
                ];
            }
            #[cfg(feature = "semantic-memory")]
            {
                if self.agent_busy {
                    return vec!["busy, please wait".to_string()];
                }
                self.agent_busy = true;
                self.set_agent_activity(AgentActivity::RunningTool(
                    "semantic-memory full reindex".to_string(),
                ));
                self.push(Entry::Assistant {
                    agent_id: None,
                    text: "rebuilding all stale or missing Project Memory semantic embeddings…"
                        .to_string(),
                });
                let tx = app_tx.clone();
                let db = self.db.clone();
                self.background_domain().spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        db.memory_store().index_pending_embeddings(true)
                    })
                    .await;
                    let text = match result {
                        Ok(Ok(report)) => {
                            serde_json::to_string_pretty(&report).unwrap_or_else(|err| {
                                format!("indexing report serialization failed: {}", err)
                            })
                        }
                        Ok(Err(err)) => format!("semantic-memory full reindex failed: {}", err),
                        Err(err) => format!("semantic-memory full reindex task failed: {}", err),
                    };
                    let _ = tx.send(AppEvent::ShellComplete {
                        output: text,
                        exit_code: Some(0),
                    });
                });
                return out;
            }
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
            if self.session.temporary_profile_override.is_some()
                || self.session.temporary_model_override.is_some()
            {
                out.push(
                    "note     : temporary session-only override active; config on disk unchanged"
                        .to_string(),
                );
            }
            return out;
        }

        if input == "/session show" {
            out.push(format!(
                "configured profile : {}",
                self.session.configured_profile
            ));
            out.push(format!(
                "effective profile   : {}",
                self.session.active_profile
            ));
            out.push(format!("effective provider  : {}", self.session.provider));
            out.push(format!("effective model     : {}", self.session.model));
            out.push(format!(
                "temporary profile override : {}",
                self.session
                    .temporary_profile_override
                    .as_deref()
                    .unwrap_or("(none)")
            ));
            out.push(format!(
                "temporary model override   : {}",
                self.session
                    .temporary_model_override
                    .as_deref()
                    .unwrap_or("(none)")
            ));
            return out;
        }

        if let Some(rest) = input.strip_prefix("/session ") {
            let parts: Vec<&str> = rest.splitn(3, ' ').collect();
            match parts.as_slice() {
                ["profile", "use", name] => {
                    let cleared_model_override = self.session.temporary_model_override.is_some();
                    if self.session.switch_profile_temporarily(name) {
                        match build_replacement_main_agent(AgentReplacementParams {
                            session: &self.session,
                            project_dir: &self.project_dir,
                            db: &self.db,
                            #[cfg(feature = "stylos")]
                            stylos_tool_bridge: self.stylos_tool_bridge.clone(),
                            #[cfg(feature = "stylos")]
                            local_stylos_instance: self.local_stylos_instance.as_deref(),
                            api_log_enabled: self.api_log_enabled,
                            local_agent_mgmt_tx: self.local_agent_mgmt_tx.clone(),
                            insert_session: true,
                        }) {
                            Ok((new_agent, new_session_id)) => {
                                self.replace_master_agent(new_agent, new_session_id);
                                if cleared_model_override {
                                    out.push(format!(
                                        "temporarily switched to profile '{}' for this session only; cleared temporary model override and reset to profile model  provider={}  model={}",
                                        name, self.session.provider, self.session.model
                                    ));
                                } else {
                                    out.push(format!(
                                        "temporarily switched to profile '{}' for this session only  provider={}  model={}",
                                        name, self.session.provider, self.session.model
                                    ));
                                }
                            }
                            Err(e) => out.push(format!("error building agent: {}", e)),
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
                ["model", "use", model] => {
                    self.session.set_temporary_model_override(model);
                    match build_replacement_main_agent(AgentReplacementParams {
                        session: &self.session,
                        project_dir: &self.project_dir,
                        db: &self.db,
                        #[cfg(feature = "stylos")]
                        stylos_tool_bridge: self.stylos_tool_bridge.clone(),
                        #[cfg(feature = "stylos")]
                        local_stylos_instance: self.local_stylos_instance.as_deref(),
                        api_log_enabled: self.api_log_enabled,
                        local_agent_mgmt_tx: self.local_agent_mgmt_tx.clone(),
                        insert_session: true,
                    }) {
                        Ok((new_agent, new_session_id)) => {
                            self.replace_master_agent(new_agent, new_session_id);
                            out.push(format!(
                                "temporarily using model '{}' for this session only",
                                self.session.model
                            ));
                        }
                        Err(e) => out.push(format!("error building agent: {}", e)),
                    }
                }
                ["reset"] => {
                    if self.session.clear_temporary_overrides() {
                        match build_replacement_main_agent(AgentReplacementParams {
                            session: &self.session,
                            project_dir: &self.project_dir,
                            db: &self.db,
                            #[cfg(feature = "stylos")]
                            stylos_tool_bridge: self.stylos_tool_bridge.clone(),
                            #[cfg(feature = "stylos")]
                            local_stylos_instance: self.local_stylos_instance.as_deref(),
                            api_log_enabled: self.api_log_enabled,
                            local_agent_mgmt_tx: self.local_agent_mgmt_tx.clone(),
                            insert_session: true,
                        }) {
                            Ok((new_agent, new_session_id)) => {
                                self.replace_master_agent(new_agent, new_session_id);
                                out.push(format!(
                                    "cleared temporary session overrides; back to configured profile '{}'  provider={}  model={}",
                                    self.session.active_profile,
                                    self.session.provider,
                                    self.session.model
                                ));
                            }
                            Err(e) => out.push(format!("error building agent: {}", e)),
                        }
                    } else {
                        out.push("no temporary session override is active".to_string());
                    }
                }
                _ => {
                    out.push("commands:".to_string());
                    out.push("  /session show                 show configured vs effective session runtime state".to_string());
                    out.push("  /session profile use <name>   temporarily switch profile for this session only".to_string());
                    out.push("  /session model use <model>    temporarily override model for this session only".to_string());
                    out.push(
                        "  /session reset                clear temporary session-only overrides"
                            .to_string(),
                    );
                }
            }
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
                        match build_replacement_main_agent(AgentReplacementParams {
                            session: &self.session,
                            project_dir: &self.project_dir,
                            db: &self.db,
                            #[cfg(feature = "stylos")]
                            stylos_tool_bridge: self.stylos_tool_bridge.clone(),
                            #[cfg(feature = "stylos")]
                            local_stylos_instance: self.local_stylos_instance.as_deref(),
                            api_log_enabled: self.api_log_enabled,
                            local_agent_mgmt_tx: self.local_agent_mgmt_tx.clone(),
                            insert_session: true,
                        }) {
                            Ok((new_agent, new_session_id)) => {
                                self.replace_master_agent(new_agent, new_session_id);
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
                    out.push("  /context                         show prompt-budget and history replay breakdown".to_string());
                    out.push("  /debug api-log enable            enable per-round API call logging for this session".to_string());
                    out.push("  /debug api-log disable           disable per-round API call logging for this session".to_string());
                    out.push("  /semantic-memory index           build missing or pending semantic indexes".to_string());
                    out.push("  /semantic-memory index full      rebuild all stale or missing semantic indexes".to_string());
                    out.push(
                        "  /config                          show current settings".to_string(),
                    );
                    out.push("  /config profile [list]           list profiles".to_string());
                    out.push("  /config profile show             show active profile".to_string());
                    out.push(
                        "  /config profile create <name>    create from current settings"
                            .to_string(),
                    );
                    out.push(
                        "  /config profile use <name>       switch profile and save to config"
                            .to_string(),
                    );
                    out.push(
                        "  /config profile set key=value    set provider/model/endpoint/api_key"
                            .to_string(),
                    );
                }
            }
            return out;
        }

        out.push(format!(
            "unknown command '{}'.  try /context, /config, /session show, /debug runtime, /debug api-log enable, or /semantic-memory index",
            input
        ));
        out
    }

    fn scroll_up(&mut self) {
        match self.review_mode {
            ReviewMode::Transcript | ReviewMode::Watchdog => {
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
            ReviewMode::Transcript | ReviewMode::Watchdog => {
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
            ReviewMode::Transcript | ReviewMode::Watchdog => {
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
            ReviewMode::Transcript | ReviewMode::Watchdog => {
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
            ReviewMode::Transcript | ReviewMode::Watchdog => {
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
            self.push(Entry::Assistant {
                agent_id: None,
                text: "empty shell command".to_string(),
            });
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
        self.activity_counters.agent_turn_started_count += 1;
        self.agent_busy = true;
        self.reset_stream_counters();
        self.set_agent_activity(AgentActivity::PreparingRequest);

        let cancellation = TurnCancellation::new();

        let handle_session_id = self.agents[agent_index].session_id;
        let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();
        let app_tx_relay = app_tx.clone();
        self.background_domain().spawn(async move {
            let mut rx = event_rx;
            while let Some(ev) = rx.recv().await {
                let _ = app_tx_relay.send(AppEvent::Agent(handle_session_id, ev));
            }
        });

        let handle = self.agents.get_mut(agent_index).expect("agent index valid");
        handle.busy = true;
        handle.turn_cancellation = Some(cancellation.clone());
        let mut agent = handle.agent.take().expect("agent available when not busy");
        agent.set_event_tx(event_tx);

        let app_tx_done = app_tx.clone();
        self.core_domain.spawn(async move {
            if let Err(e) = agent
                .run_loop_with_cancellation(&text, Some(cancellation))
                .await
            {
                let _ = app_tx_done.send(AppEvent::Agent(
                    handle_session_id,
                    AgentEvent::AssistantText(format!("error: {e}")),
                ));
            }
            let _ = app_tx_done.send(AppEvent::AgentReady(Box::new(agent), handle_session_id));
        });
    }
    fn submit_text(&mut self, text: String, app_tx: &mpsc::UnboundedSender<AppEvent>) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }

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
                self.push(Entry::Assistant {
                    agent_id: None,
                    text: line,
                });
            }
            self.push(Entry::Blank);
            self.mark_dirty_input();
            return;
        }

        #[cfg(feature = "stylos")]
        let agent_index = if let Some(index) = self
            .agents
            .iter()
            .position(|h| h.active_incoming_prompt.is_some())
        {
            let request = self.agents[index]
                .active_incoming_prompt
                .as_ref()
                .cloned()
                .expect("incoming prompt present");
            if let Some(target_agent_id) = request.agent_id.as_deref() {
                match self
                    .agents
                    .iter()
                    .position(|h| h.agent_id == target_agent_id)
                {
                    Some(target_index) => target_index,
                    None => {
                        let sender = request.from.as_deref().unwrap_or("unknown sender");
                        let sender_agent = request.from_agent_id.as_deref().unwrap_or("unknown");
                        let target_instance = request.to.as_deref().unwrap_or("unknown target");
                        let target_agent =
                            request.to_agent_id.as_deref().unwrap_or(target_agent_id);
                        let message = if request.prompt.starts_with("type=stylos_note ") {
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
                        self.push(Entry::RemoteEvent {
                            agent_id: None,
                            source: Some(NonAgentSource::Board),
                            text: message,
                        });
                        if let (Some(handle), Some(task_id)) =
                            (self.stylos.as_ref(), request.task_id.clone())
                        {
                            let query_context = handle.query_context();
                            self.background_domain().spawn(async move {
                                query_context
                                    .task_registry()
                                    .set_failed(&task_id, "target_agent_missing".to_string())
                                    .await;
                            });
                        }
                        self.agents[index].active_incoming_prompt = None;
                        self.watchdog_state.set_active_incoming_prompt(
                            self.agents
                                .iter()
                                .any(|handle| handle.active_incoming_prompt.is_some()),
                        );
                        self.watchdog_state.set_pending_watchdog_note(false);
                        return;
                    }
                }
            } else {
                self.agents
                    .iter()
                    .position(is_interactive_handle)
                    .expect("interactive agent")
            }
        } else {
            self.agents
                .iter()
                .position(is_interactive_handle)
                .expect("interactive agent")
        };
        #[cfg(not(feature = "stylos"))]
        let agent_index = {
            self.agents
                .iter()
                .position(is_interactive_handle)
                .expect("interactive agent")
        };

        #[cfg(feature = "stylos")]
        if self.agents[agent_index].active_incoming_prompt.is_none() {
            self.push(Entry::User(text.clone()));
        }
        #[cfg(not(feature = "stylos"))]
        self.push(Entry::User(text.clone()));

        self.submit_text_to_agent(agent_index, text, app_tx);
    }

    #[cfg(feature = "stylos")]
    fn handle_watchdog_poll(&mut self, app_tx: &mpsc::UnboundedSender<AppEvent>) {
        let Some(selection) = select_watchdog_dispatch(
            &self.agents,
            &self.db,
            &self.board_claims,
            self.local_stylos_instance.as_deref(),
        ) else {
            self.watchdog_state.set_pending_watchdog_note(false);
            return;
        };
        self.watchdog_state.set_pending_watchdog_note(true);
        finalize_board_note_injection(&self.db, &self.board_claims, &selection.action.note_id);
        self.push(Entry::RemoteEvent {
            agent_id: selection.action.request.agent_id.clone(),
            source: if selection.action.request.agent_id.is_some() { None } else { Some(NonAgentSource::Watchdog) },
            text: selection.action.log_line,
        });
        let prompt = selection.action.request.prompt.clone();
        self.agents[selection.agent_index].active_incoming_prompt =
            Some(selection.action.request.clone());
        self.watchdog_state.set_active_incoming_prompt(true);
        self.submit_text_to_agent(selection.agent_index, prompt, app_tx);
    }

    #[cfg(feature = "stylos")]
    fn maybe_emit_done_mention_for_completed_note(
        &mut self,
        agent_index: usize,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) -> bool {
        let Some(remote) = self.agents[agent_index]
            .active_incoming_prompt
            .as_ref()
            .cloned()
        else {
            return false;
        };
        match resolve_completed_note_follow_up(&self.db, &remote) {
            BoardTurnFollowUp::None => false,
            BoardTurnFollowUp::ContinueCurrentNote { request, prompt } => {
                self.agents[agent_index].active_incoming_prompt = Some(request);
                self.submit_text(prompt, app_tx);
                true
            }
            BoardTurnFollowUp::EmitDoneMention { log_line } => {
                self.push(Entry::RemoteEvent {
                    agent_id: None,
                    source: Some(NonAgentSource::Board),
                    text: log_line,
                });
                false
            }
            BoardTurnFollowUp::EmitDoneMentionError { status_line } => {
                self.push(Entry::Status {
                    agent_id: None,
                    source: Some(NonAgentSource::Board),
                    text: status_line,
                });
                false
            }
        }
    }

    fn submit_input(&mut self, app_tx: &mpsc::UnboundedSender<AppEvent>) -> bool {
        let Some(text) = self.composer.submit_input_text() else {
            return false;
        };
        let was_dirty = self.dirty.any();
        self.return_to_latest();
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
        self.composer.handle_paste_event(text);
        self.mark_dirty_input();
        self.request_draw(frame_requester);
    }

    pub(crate) fn handle_tick_event(&mut self, frame_requester: &FrameRequester) {
        let was_dirty = self.dirty.any();
        self.on_tick();
        if self.dirty.any() && !was_dirty {
            self.request_draw(frame_requester);
        }
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
            h.busy = false;
            h.turn_cancellation = None;
        }
        self.agent_busy = self.any_agent_busy();
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
        self.push(Entry::Assistant {
            agent_id: None,
            text: format!("open {} and enter code {}", verification_uri, user_code),
        });
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
                self.handle_tick_event(frame_requester);
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
            AppEvent::Agent(sid, ev) => {
                self.handle_agent_event_for_run(sid, ev, frame_requester, app_tx)
            }
            #[cfg(feature = "stylos")]
            AppEvent::WatchdogPoll => self.handle_watchdog_poll(app_tx),
            #[cfg(not(feature = "stylos"))]
            AppEvent::Agent(sid, ev) => self.handle_agent_event_for_run(sid, ev, frame_requester),
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
            AppEvent::LocalAgentManagement(request) => {
                self.handle_local_agent_management_request(request, frame_requester);
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
                    self.push(Entry::Assistant {
                        agent_id: None,
                        text: format!("warning: failed to save auth: {}", e),
                    });
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
                    self.push(Entry::Assistant {
                        agent_id: None,
                        text: format!("warning: failed to save config: {}", e),
                    });
                }
                match build_replacement_main_agent(AgentReplacementParams {
                    session: &self.session,
                    project_dir: &self.project_dir,
                    db: &self.db,
                    #[cfg(feature = "stylos")]
                    stylos_tool_bridge: self.stylos_tool_bridge.clone(),
                    #[cfg(feature = "stylos")]
                    local_stylos_instance: self.local_stylos_instance.as_deref(),
                    api_log_enabled: self.api_log_enabled,
                    local_agent_mgmt_tx: self.local_agent_mgmt_tx.clone(),
                    insert_session: true,
                }) {
                    Ok((mut new_agent, new_session_id)) => {
                        new_agent.refresh_model_info().await;
                        self.replace_master_agent(new_agent, new_session_id);
                        self.push(Entry::Assistant {
                            agent_id: None,
                            text: format!(
                                "logged in as {} — switched to codex profile (gpt-5.4)",
                                auth.account_id
                            ),
                        });
                        self.push(Entry::Blank);
                        self.mark_dirty_all();
                        self.request_draw(frame_requester);
                    }
                    Err(e) => {
                        self.push(Entry::Assistant {
                            agent_id: None,
                            text: format!("login succeeded but agent build failed: {}", e),
                        });
                        self.mark_dirty_all();
                        self.request_draw(frame_requester);
                    }
                }
            }
            Err(e) => {
                self.clear_agent_activity();
                self.push(Entry::Assistant {
                    agent_id: None,
                    text: format!("login failed: {}", e),
                });
                self.mark_dirty_all();
                self.request_draw(frame_requester);
            }
        }
    }

    pub(crate) fn handle_agent_event_for_run(
        &mut self,
        sid: Uuid,
        ev: AgentEvent,
        frame_requester: &FrameRequester,
        #[cfg(feature = "stylos")] app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        self.activity_counters.agent_event_count += 1;
        self.process_agent_event(
            sid,
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
        #[cfg(feature = "stylos")]
        self.watchdog_state.set_active_incoming_prompt(false);
        #[cfg(feature = "stylos")]
        self.watchdog_state.set_pending_watchdog_note(false);
        self.clear_agent_activity();
        self.push(Entry::Assistant {
            agent_id: None,
            text: output,
        });
        if let Some(code) = exit_code {
            if code != 0 {
                self.push(Entry::Assistant {
                    agent_id: None,
                    text: format!("exit code: {}", code),
                });
            }
        }
        self.push(Entry::Blank);
        self.mark_dirty_all();
        self.request_draw(frame_requester);
    }

    #[cfg(feature = "stylos")]
    pub(crate) fn handle_stylos_cmd_event(
        &mut self,
        cmd: crate::stylos::StylosCmdRequest,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        self.push(Entry::RemoteEvent {
            agent_id: self
                .agents
                .iter()
                .find(|h| is_interactive_handle(h))
                .map(|h| h.agent_id.clone()),
            source: None,
            text: format!(
                "Stylos cmd scope=local preview={}",
                cmd.prompt.lines().next().unwrap_or("")
            ),
        });
        if let Some(index) = self.agents.iter().position(is_interactive_handle) {
            self.agents[index].active_incoming_prompt = None;
        }
        #[cfg(feature = "stylos")]
        self.watchdog_state.set_active_incoming_prompt(false);
        #[cfg(feature = "stylos")]
        self.watchdog_state.set_pending_watchdog_note(false);
        self.submit_text(cmd.prompt, app_tx);
    }

    #[cfg(feature = "stylos")]
    pub(crate) fn handle_stylos_event_text(&mut self, text: String) {
        self.push(Entry::RemoteEvent {
            agent_id: None,
            source: Some(NonAgentSource::Stylos),
            text,
        });
    }

    #[cfg(feature = "stylos")]
    pub(crate) fn handle_incoming_prompt_event(
        &mut self,
        request: IncomingPromptRequest,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        self.activity_counters.incoming_prompt_count += 1;
        match resolve_incoming_prompt_disposition(&self.agents, &self.board_claims, request) {
            IncomingPromptDisposition::MissingTarget {
                log_agent_id,
                log_text,
                failed_task_id,
                ..
            }
            | IncomingPromptDisposition::BusyTarget {
                log_agent_id,
                log_text,
                failed_task_id,
                ..
            } => {
                self.push(Entry::RemoteEvent {
                    agent_id: log_agent_id.clone(),
                    source: if log_agent_id.is_some() { None } else { Some(NonAgentSource::Stylos) },
                    text: log_text,
                });
                if let (Some(handle), Some(task_id)) = (self.stylos.as_ref(), failed_task_id) {
                    let query_context = handle.query_context();
                    self.background_domain().spawn(async move {
                        query_context
                            .task_registry()
                            .set_failed(&task_id, "agent_busy".to_string())
                            .await;
                    });
                }
            }
            IncomingPromptDisposition::Accepted {
                agent_index,
                log_agent_id,
                log_text,
                prompt,
                request,
                pending_watchdog_note,
            } => {
                self.push(Entry::RemoteEvent {
                    agent_id: log_agent_id.clone(),
                    source: if log_agent_id.is_some() { None } else { Some(NonAgentSource::Stylos) },
                    text: log_text,
                });
                self.agents[agent_index].active_incoming_prompt = Some(request);
                self.watchdog_state.set_active_incoming_prompt(true);
                self.watchdog_state
                    .set_pending_watchdog_note(pending_watchdog_note);
                self.submit_text_to_agent(agent_index, prompt, app_tx);
            }
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

        match self.composer.handle_key_event(
            key,
            self.transcript_review_open(),
            self.any_agent_busy(),
        ) {
            InputAction::None => {}
            InputAction::RequestDraw => {
                self.mark_dirty_input();
                self.request_draw(frame_requester);
            }
            InputAction::Submit => {
                let tx = app_tx.clone();
                if self.submit_input(&tx) {
                    self.request_draw(frame_requester);
                }
            }
            InputAction::Quit => {
                let now = Instant::now();
                if self.ctrl_c_exit_is_armed(now) {
                    self.ctrl_c_exit_armed_until = None;
                    self.running = false;
                } else {
                    self.expire_ctrl_c_exit_if_needed(now);
                    self.arm_ctrl_c_exit();
                    self.request_draw(frame_requester);
                }
            }
            InputAction::Interrupt => self.request_interrupt(),
            InputAction::OpenTranscriptReview => {
                self.open_transcript_review();
                self.mark_dirty_overlay();
                self.request_draw(frame_requester);
            }
            InputAction::OpenWatchdogReview => {
                self.toggle_watchdog_review();
                self.mark_dirty_overlay();
                self.request_draw(frame_requester);
            }
            InputAction::CloseReview => {
                self.close_review();
                self.mark_dirty_overlay();
                self.request_draw(frame_requester);
            }
            InputAction::ScrollUp => self.scroll_up(),
            InputAction::ScrollDown => self.scroll_down(),
            InputAction::ReturnToLatest => {
                self.return_to_latest();
                self.mark_dirty_conversation();
                self.request_draw(frame_requester);
            }
            InputAction::JumpToTop => {
                let (total, height) = current_total_and_height(terminal, self);
                self.jump_to_top(total, height);
            }
            InputAction::PageUp => {
                let page = area_page_height(terminal, self);
                self.page_up(page);
            }
            InputAction::PageDown => {
                let page = area_page_height(terminal, self);
                self.page_down(page);
            }
        }
    }
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

fn build_lines<'a>(
    entries: &'a [Entry],
    pending: &'a Option<String>,
    agents: &'a [AgentHandle],
) -> Vec<Line<'a>> {
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
            Entry::Assistant { agent_id, text } => {
                for part in text.lines() {
                    let mut spans = agent_tag_spans(agent_id.as_deref(), agents);
                    spans.push(Span::raw(part.to_string()));
                    lines.push(Line::from(spans));
                }
            }
            #[cfg(feature = "stylos")]
            Entry::RemoteEvent { agent_id, source, text } => {
                let mut spans = if let Some(agent_id) = agent_id.as_deref() {
                    agent_tag_spans(Some(agent_id), agents)
                } else {
                    non_agent_source_spans(*source)
                };
                spans.push(Span::styled(
                    format!("󰀂 {}", text),
                    Style::default().fg(source.unwrap_or(NonAgentSource::Stylos).color()),
                ));
                lines.push(Line::from(spans));
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
            Entry::ToolCall {
                agent_id,
                detail,
                reason,
            } => {
                let mut spans = agent_tag_spans(agent_id.as_deref(), agents);
                spans.push(Span::styled(" ", Style::default().fg(Color::Yellow)));
                spans.push(Span::styled(
                    detail.clone(),
                    Style::default().fg(Color::Yellow),
                ));
                lines.push(Line::from(spans));
                if let Some(reason) = reason {
                    let mut spans = agent_tag_spans(agent_id.as_deref(), agents);
                    spans.push(Span::styled(
                        reason.clone(),
                        Style::default().fg(Color::DarkGray),
                    ));
                    lines.push(Line::from(spans));
                }
            }
            Entry::Status { agent_id, source, text } => {
                let mut spans = if let Some(agent_id) = agent_id.as_deref() {
                    agent_tag_spans(Some(agent_id), agents)
                } else {
                    non_agent_source_spans(*source)
                };
                spans.push(Span::styled(
                    format!("󰇺 {}", text),
                    Style::default().fg(source.unwrap_or(NonAgentSource::Runtime).color()),
                ));
                lines.push(Line::from(spans));
            }
            Entry::ToolDone { .. } => {
                if let Some(last) = lines.last_mut() {
                    let mut spans = last.spans.clone();
                    spans.push(Span::styled("  ✓", Style::default().fg(Color::Green)));
                    *last = Line::from(spans);
                }
            }
            Entry::TurnDone {
                agent_id,
                summary,
                stats,
            } => {
                let mut spans = agent_tag_spans(agent_id.as_deref(), agents);
                spans.push(Span::styled(
                    summary.to_string(),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!(" [stats: {}]", stats),
                    Style::default().fg(Color::DarkGray),
                ));
                lines.push(Line::from(spans));
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

fn watchdog_review_area(area: Rect, line_count: usize) -> Rect {
    let width = area.width.saturating_mul(78).saturating_div(100).max(40);
    let desired_height = (line_count as u16).saturating_add(2);
    let min_height = 10u16;
    let max_height = area.height.saturating_mul(60).saturating_div(100).max(min_height);
    let height = desired_height.clamp(min_height, max_height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .padding(Padding::left(1));

    let input_inner = input_block.inner(area);
    let input_inner_width = input_inner.width.max(1);
    let input_height = (app.composer.input.desired_height(input_inner_width) + 2).clamp(3, 8);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(input_height),
            Constraint::Length(3),
        ])
        .split(area);

    let lines = build_lines(&app.entries, &app.pending, &app.agents);
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
    f.render_widget(input_block.clone(), chunks[1]);
    let input_inner = input_block.inner(chunks[1]);
    let mut input_state = app.composer.input_state;
    app.composer
        .input
        .render_with_state(input_inner, f.buffer_mut(), &mut input_state);

    let overflow = app.composer.input.overflow_state(
        input_inner.width.max(1),
        input_inner.height,
        input_state,
    );
    if overflow.hidden_above && input_inner.height > 0 {
        let x = chunks[1].right().saturating_sub(2);
        let y = chunks[1].y + 1;
        if x < chunks[1].right() && y < chunks[1].bottom() {
            f.buffer_mut()[(x, y)].set_char('↑');
        }
    }
    if overflow.hidden_below && input_inner.height > 0 {
        let x = chunks[1].right().saturating_sub(2);
        let y = chunks[1].bottom().saturating_sub(2);
        if x < chunks[1].right() && y < chunks[1].bottom() {
            f.buffer_mut()[(x, y)].set_char('↓');
        }
    }

    if app.review_mode != ReviewMode::Transcript {
        if let Some((cursor_x, cursor_y)) = app
            .composer
            .input
            .cursor_pos_with_state(input_inner, input_state)
        {
            if cursor_y < chunks[1].bottom() && cursor_x < chunks[1].right() {
                f.set_cursor_position((cursor_x, cursor_y));
            }
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
        ReviewMode::Transcript => "review:transcript",
        ReviewMode::Watchdog => "review:watchdog",
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

    if app.review_mode == ReviewMode::Transcript || app.review_mode == ReviewMode::Watchdog {
        let (title, review_lines) = match app.review_mode {
            ReviewMode::Transcript => (" Transcript review ", build_lines(&app.entries, &None, &app.agents)),
            ReviewMode::Watchdog => (" Watchdog & agents ", build_watchdog_review_lines(app)),
            ReviewMode::Closed => unreachable!(),
        };
        let review = match app.review_mode {
            ReviewMode::Transcript => review_area(area),
            ReviewMode::Watchdog => watchdog_review_area(area, review_lines.len()),
            ReviewMode::Closed => unreachable!(),
        };
        let review_block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let review_inner = review_block.inner(review);
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
    app: &App,
) -> usize {
    let area = terminal.size().map(|r| r.height as usize).unwrap_or(24);
    let reserved = 8usize + 3usize;
    let conv = area.saturating_sub(reserved).max(1);
    if app.review_mode == ReviewMode::Transcript || app.review_mode == ReviewMode::Watchdog {
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
    app: &App,
) -> (usize, usize) {
    let size = terminal
        .size()
        .map(|s| Rect::new(0, 0, s.width, s.height))
        .unwrap_or(Rect::new(0, 0, 80, 24));
    let lines = match app.review_mode {
        ReviewMode::Transcript => build_lines(&app.entries, &None, &app.agents),
        ReviewMode::Watchdog => build_watchdog_review_lines(app),
        ReviewMode::Closed => build_lines(&app.entries, &app.pending, &app.agents),
    };
    let area = match app.review_mode {
        ReviewMode::Transcript => review_area(size),
        ReviewMode::Watchdog => watchdog_review_area(size, lines.len()),
        ReviewMode::Closed => size,
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


fn build_watchdog_review_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    #[cfg(feature = "stylos")]
    {
        let stylos_state = match app.stylos.as_ref().map(|h| h.state()) {
            Some(StylosRuntimeState::Off) => "off".to_string(),
            Some(StylosRuntimeState::Active { mode, realm, instance }) => {
                format!("active mode={} realm={} instance={}", mode, realm, instance)
            }
            Some(StylosRuntimeState::Error(err)) => format!("error {}", err),
            None => "off".to_string(),
        };
        lines.push(Line::from(vec![Span::styled(
            "watchdog",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(format!("stylos: {}", stylos_state)));
        lines.push(Line::from(format!(
            "pending_watchdog_note: {}",
            app.watchdog_state.pending_watchdog_note()
        )));
        lines.push(Line::from(format!(
            "active_incoming_prompts: {}",
            app.agents
                .iter()
                .filter(|handle| handle.active_incoming_prompt.is_some())
                .count()
        )));
        lines.push(Line::from(format!(
            "aggregate_busy: {}",
            app.agents.iter().any(|handle| handle.busy)
        )));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(vec![Span::styled(
        "local agents",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )]));
    for handle in &app.agents {
        let roles = if handle.roles.is_empty() {
            "-".to_string()
        } else {
            handle.roles.join(",")
        };
        #[cfg(feature = "stylos")]
        let incoming = handle.active_incoming_prompt.is_some();
        #[cfg(not(feature = "stylos"))]
        let incoming = false;
        lines.push(Line::from(format!(
            "{} | label={} | roles={} | busy={} | incoming={}",
            handle.agent_id, handle.label, roles, handle.busy, incoming
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("Ctrl-t transcript review  Ctrl-w watchdog  Esc close"));
    lines
}

fn unix_epoch_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
    fn validate_agent_roles_accepts_one_master_and_one_interactive() {
        let agents = vec![
            handle("master", &["master", "interactive"]),
            handle("worker", &["background"]),
        ];
        validate_agent_roles(&agents).unwrap();
    }

    #[test]
    fn validate_agent_roles_rejects_zero_master() {
        let agents = vec![handle("worker", &["background"])];
        assert!(validate_agent_roles(&agents).is_err());
    }

    #[test]
    fn validate_agent_roles_rejects_two_master() {
        let agents = vec![handle("a", &["master"]), handle("b", &["master"])];
        assert!(validate_agent_roles(&agents).is_err());
    }

    #[test]
    fn allocates_next_free_smith_id() {
        let agents = vec![
            handle("master", &["master", "interactive"]),
            handle("smith-1", &["worker"]),
            handle("smith-2", &["worker"]),
            handle("smith-4", &["worker"]),
        ];
        assert_eq!(allocate_default_local_agent_id(&agents), "smith-3");
    }

    #[test]
    fn build_snapshot_preserves_multiple_agents_and_startup_dir() {
        let startup = PathBuf::from(".");
        let snapshot = build_stylos_status_snapshot(
            &startup,
            vec![
                AgentStatusSource {
                    agent_id: "master".to_string(),
                    label: "master".to_string(),
                    roles: vec!["master".to_string(), "interactive".to_string()],
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
        assert_eq!(snapshot.agents[0].roles, vec!["master", "interactive"]);
        assert_eq!(snapshot.agents[1].provider, "p2");
        assert_eq!(snapshot.startup_project_dir, startup.display().to_string());
    }

    #[test]
    fn targeted_remote_request_prefers_matching_agent_id() {
        let agents = vec![
            handle("master", &["master", "interactive"]),
            handle("worker", &["background"]),
        ];
        let request = IncomingPromptRequest {
            prompt: "hi".to_string(),
            source: IncomingPromptSource::RemoteStylos,
            agent_id: Some("worker".to_string()),
            task_id: None,
            request_id: None,
            from: Some("peer-1:1234".to_string()),
            from_agent_id: Some("master".to_string()),
            to: Some("peer-2:5678".to_string()),
            to_agent_id: Some("worker".to_string()),
        };
        let index = if let Some(target_agent_id) = request.agent_id.as_deref() {
            agents.iter().position(|h| h.agent_id == target_agent_id)
        } else {
            agents.iter().position(is_interactive_handle)
        }
        .unwrap();
        assert_eq!(agents[index].agent_id, "worker");
    }

    #[test]
    fn targeted_remote_request_does_not_fall_back_to_interactive_when_missing() {
        let agents = vec![handle("master", &["master", "interactive"])];
        let request = IncomingPromptRequest {
            prompt: "hi".to_string(),
            source: IncomingPromptSource::RemoteStylos,
            agent_id: Some("worker".to_string()),
            task_id: None,
            request_id: None,
            from: Some("peer-1:1234".to_string()),
            from_agent_id: Some("master".to_string()),
            to: Some("peer-2:5678".to_string()),
            to_agent_id: Some("worker".to_string()),
        };
        let index = if let Some(target_agent_id) = request.agent_id.as_deref() {
            agents.iter().position(|h| h.agent_id == target_agent_id)
        } else {
            agents.iter().position(is_interactive_handle)
        };
        assert!(index.is_none());
    }

    #[test]
    fn sender_side_stylos_talk_log_format_is_exact() {
        let event = crate::stylos::sender_side_transport_event_from_tool_detail(
            "stylos_request_talk instance=node-2:77 to_agent_id=master",
            "node-1:42",
            true,
        )
        .unwrap();
        assert_eq!(event.text, "Stylos talk to=node-2:77 from=node-1:42");
        assert!(!event.text.contains('/'));
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
