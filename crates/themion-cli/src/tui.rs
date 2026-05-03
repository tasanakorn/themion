use crate::app_runtime::{LocalAgentManagementRequest, RuntimeCommand};
use crate::app_state::{activity_status_value, on_tick as app_state_on_tick, publish_runtime_snapshot as app_state_publish_runtime_snapshot, AppRuntimeState, set_agent_activity as app_state_set_agent_activity, AgentActivity, AppSnapshot};
use crate::chat_composer::{ChatComposer, InputAction};
use crate::runtime_domains::DomainHandle;
#[cfg(feature = "stylos")]
use crate::stylos::StylosRuntimeState;

use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
    Frame, Terminal,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use themion_core::agent::{Agent, TurnCancellation};
use themion_core::client_codex::ApiCallRateLimitReport;
use themion_core::ModelInfo;
use themion_core::{
    EstimateMode, PromptContextReport, PromptSectionKind, ReplayForm, TokenizerResolutionSource,
    ToolEstimateMode,
};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

pub(crate) enum AppEvent {
    Key(event::KeyEvent),
    Mouse(event::MouseEvent),
    Paste(String),
    Tick,
    SnapshotUpdated(AppSnapshot),
    RuntimeCommand(RuntimeCommand),
    LoginPrompt {
        user_code: String,
        verification_uri: String,
    },
    LoginComplete {
        profile_name: String,
        auth_result: anyhow::Result<themion_core::CodexAuth>,
    },
    LocalAgentManagement(LocalAgentManagementRequest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "stylos"), allow(dead_code))]
pub(crate) enum NonAgentSource {
    Board,
    Stylos,
    Runtime,
}

impl NonAgentSource {
    fn label(self) -> &'static str {
        match self {
            Self::Board => "BOARD",
            Self::Stylos => "STYLOS",
            Self::Runtime => "RUNTIME",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Board => Color::Yellow,
            Self::Stylos => Color::Cyan,
            Self::Runtime => Color::Magenta,
        }
    }
}

#[derive(Clone)]
pub(crate) enum Entry {
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

pub(crate) fn format_context_report(report: &PromptContextReport) -> Vec<String> {
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

pub(crate) fn agent_id_for_session(agents: &[AgentHandle], sid: Uuid) -> Option<String> {
    agents
        .iter()
        .find(|handle| handle.session_id == sid)
        .map(|handle| handle.agent_id.clone())
}

pub(crate) fn split_tool_call_detail(name: &str, args_json: &str) -> (String, Option<String>) {
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
        "board_list_notes" => {
            let columns = args["columns"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_else(|| "*".to_string());
            (
                format!(
                    "board_list_notes columns={}",
                    center_trim(&columns, TOOL_DETAIL_MAX_CHARS)
                ),
                None,
            )
        }
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
}

#[derive(Clone, Copy, Default)]
pub(crate) struct UiDirty {
    conversation: bool,
    input: bool,
    status: bool,
    overlay: bool,
    full: bool,
}

impl UiDirty {
    pub(crate) fn any(&self) -> bool {
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


#[derive(Clone, Copy)]
struct ActivityCountersSnapshot {
    draw_count: u64,
    draw_request_count: u64,
    draw_skip_clean_count: u64,
    pub(crate) tick_count: u64,
    input_key_count: u64,
    input_mouse_count: u64,
    input_paste_count: u64,
    pub(crate) agent_event_count: u64,
    pub(crate) incoming_prompt_count: u64,
    pub(crate) shell_complete_count: u64,
    agent_turn_started_count: u64,
    pub(crate) agent_turn_completed_count: u64,
    draw_total_us: u64,
    draw_max_us: u64,
    command_count: u64,
}

#[derive(Default)]
pub(crate) struct ActivityCounters {
    draw_count: u64,
    draw_request_count: u64,
    draw_skip_clean_count: u64,
    pub(crate) tick_count: u64,
    input_key_count: u64,
    input_mouse_count: u64,
    input_paste_count: u64,
    pub(crate) agent_event_count: u64,
    pub(crate) incoming_prompt_count: u64,
    pub(crate) shell_complete_count: u64,
    agent_turn_started_count: u64,
    pub(crate) agent_turn_completed_count: u64,
    draw_total_us: u64,
    draw_max_us: u64,
    command_count: u64,
}

impl ActivityCounters {
    pub(crate) fn record_agent_turn_started(&mut self) {
        self.agent_turn_started_count += 1;
    }

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
pub(crate) struct RuntimeMetricsSnapshot {
    at_ms: u64,
    uptime_ms: u64,
    counters: ActivityCountersSnapshot,
}

#[derive(Clone, Copy)]
pub(crate) struct TimedRuntimeDelta {
    latest_at_ms: u64,
    latest_uptime_ms: u64,
    pub(crate) wall_elapsed_ms: u64,
    counter_delta: ActivityCountersSnapshot,
    lifetime_counters: ActivityCountersSnapshot,
}

pub struct App {
    pub(crate) entries: Vec<Entry>,
    composer: ChatComposer,
    scroll_offset: usize,
    navigation_mode: NavigationMode,
    review_mode: ReviewMode,
    review_scroll_offset: usize,
    pub(crate) anim_frame: u8,
    pub(crate) dirty: UiDirty,
    pub(crate) runtime: AppRuntimeState,
    recent_runtime_snapshots: std::collections::VecDeque<RuntimeMetricsSnapshot>,
    snapshot_hub: crate::app_state::AppSnapshotHub,
}

impl App {
    pub fn new(
        runtime: AppRuntimeState,
        initial_snapshot: AppSnapshot,
    ) -> Self {
        let art = concat!(
            "████████╗██╗  ██╗███████╗███╗   ███╗██╗ ██████╗ ███╗   ██╗\n",
            "╚══██╔══╝██║  ██║██╔════╝████╗ ████║██║██╔═══██╗████╗  ██║\n",
            "   ██║   ███████║█████╗  ██╔████╔██║██║██║   ██║██╔██╗ ██║\n",
            "   ██║   ██╔══██║██╔══╝  ██║╚██╔╝██║██║██║   ██║██║╚██╗██║\n",
            "   ██║   ██║  ██║███████╗██║ ╚═╝ ██║██║╚██████╔╝██║ ╚████║\n",
            "   ╚═╝   ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝╚═╝ ╚═════╝ ╚═╝  ╚═══╝",
        );
        let project_display = runtime.project_dir.display().to_string();
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
                    runtime.session.active_profile,
                    runtime.session.model
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

        let mut app = Self {
            entries: initial_entries,
            composer: ChatComposer::new(),
            scroll_offset: 0,
            navigation_mode: NavigationMode::FollowTail,
            review_mode: ReviewMode::Closed,
            review_scroll_offset: 0,
            anim_frame: 0,
            dirty: {
                let mut d = UiDirty::default();
                d.mark_all();
                d
            },
            runtime,
            recent_runtime_snapshots: std::collections::VecDeque::new(),
            snapshot_hub: crate::app_state::AppSnapshotHub::new(initial_snapshot),
        };
        app.record_runtime_snapshot();
        app_state_publish_runtime_snapshot(&mut app);
        app
    }



    pub(crate) fn any_agent_busy(&self) -> bool {
        crate::app_state::runtime_any_agent_busy(&self.runtime)
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

    fn set_agent_activity(&mut self, activity: AgentActivity) {
        app_state_set_agent_activity(self, activity);
    }



    fn request_interrupt(&mut self) {
        crate::app_state::runtime_request_interrupt(self);
    }

    fn arm_ctrl_c_exit(&mut self) {
        crate::app_state::runtime_arm_ctrl_c_exit(self);
    }

    fn ctrl_c_exit_is_armed(&self, now: Instant) -> bool {
        crate::app_state::runtime_ctrl_c_exit_is_armed(&self.runtime, now)
    }

    pub(crate) fn expire_ctrl_c_exit_if_needed(&mut self, now: Instant) -> bool {
        crate::app_state::runtime_expire_ctrl_c_exit_if_needed(&mut self.runtime, now)
    }

    fn on_tick(&mut self) {
        app_state_on_tick(self);
    }

    fn mark_dirty_conversation(&mut self) {
        self.dirty.conversation = true;
    }

    fn mark_dirty_input(&mut self) {
        self.dirty.input = true;
    }

    pub(crate) fn mark_dirty_status(&mut self) {
        self.dirty.status = true;
    }

    fn mark_dirty_overlay(&mut self) {
        self.dirty.overlay = true;
    }

    pub(crate) fn mark_dirty_all(&mut self) {
        self.dirty.mark_all();
    }

    pub(crate) fn request_draw(&mut self, frame_requester: &FrameRequester) {
        self.runtime.activity_counters.draw_request_count += 1;
        frame_requester.schedule_frame();
    }

    pub(crate) fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    pub(crate) fn is_running(&self) -> bool {
        self.runtime.running
    }

    pub(crate) fn finish_initial_draw(&mut self, frame_requester: &FrameRequester) {
        self.clear_dirty();
        self.request_draw(frame_requester);
    }



    pub(crate) fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        self.mark_dirty_conversation();
    }

    pub(crate) fn activity_status_value(&self) -> String {
        activity_status_value(
            self.runtime.agent_activity.as_ref(),
            self.runtime.idle_since,
            self.runtime.stream_chunks,
            self.runtime.stream_chars,
        )
    }


    fn current_runtime_snapshot(&self) -> RuntimeMetricsSnapshot {
        RuntimeMetricsSnapshot {
            at_ms: unix_epoch_now_ms(),
            uptime_ms: self.runtime.process_started_at.elapsed().as_millis() as u64,
            counters: self.runtime.activity_counters.snapshot(),
        }
    }

    pub(crate) fn record_runtime_snapshot(&mut self) {
        let snapshot = self.current_runtime_snapshot();
        self.recent_runtime_snapshots.push_back(snapshot);
        while self.recent_runtime_snapshots.len() > 16 {
            self.recent_runtime_snapshots.pop_front();
        }
    }

    pub(crate) fn recent_runtime_delta(&self) -> Option<TimedRuntimeDelta> {
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

    pub(crate) fn debug_runtime_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        let now_ms = unix_epoch_now_ms();
        let uptime_ms = self.runtime.process_started_at.elapsed().as_millis() as u64;
        out.push("debug runtime snapshot: themion process/thread/task activity".to_string());
        out.push(format!(
            "process pid={} uptime={} started_at_ms={}",
            std::process::id(),
            format_duration_ms(uptime_ms),
            self.runtime.process_started_at_ms,
        ));
        out.push(format!(
            "app busy={} activity={} session={} project={}",
            self.runtime.agent_busy,
            self.activity_status_value(),
            self.runtime.agents
                .first()
                .map(|h| h.session_id.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            self.runtime.project_dir.display()
        ));
        out.push(format!(
            "workflow flow={} phase={} status={}",
            self.runtime.workflow_state.workflow_name,
            self.runtime.workflow_state.phase_name,
            format!("{:?}", self.runtime.workflow_state.status)
        ));
        #[cfg(feature = "stylos")]
        {
            let stylos_state = match self.runtime.stylos.as_ref().map(|h| h.state()) {
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
                &self.runtime.activity_counters.snapshot(),
            ));
        }

        if let Some(changed_at) = self
            .runtime
            .agent_activity_changed_at
            .or(self.runtime.idle_status_changed_at)
        {
            out.push(format!(
                "activity_status_changed {} ago",
                format_duration_ms(now_ms.saturating_sub(changed_at))
            ));
        }
        #[cfg(feature = "stylos")]
        if let Some(handle) = self.runtime.stylos.as_ref() {
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
        self.runtime.activity_counters.command_count += 1;

        if let Some(rest) = input.strip_prefix("/login codex") {
            let profile_name = rest.trim();
            let target_profile = if profile_name.is_empty() {
                None
            } else if profile_name.split_whitespace().count() == 1 {
                Some(profile_name.to_string())
            } else {
                out.push("usage: /login codex [profile]".to_string());
                return out;
            };
            app_tx
                .send(AppEvent::RuntimeCommand(RuntimeCommand::LoginCodex {
                    profile_name: target_profile,
                }))
                .ok();
            return out;
        }

        if input == "/debug runtime" {
            return self.debug_runtime_lines();
        }

        if input == "/context" {
            return crate::app_state::context_report_lines(&self.runtime);
        }

        if input == "/debug api-log enable" {
            app_tx
                .send(AppEvent::RuntimeCommand(RuntimeCommand::SetApiLogEnabled {
                    enabled: true,
                }))
                .ok();
            return vec![];
        }

        if input == "/debug api-log disable" {
            app_tx
                .send(AppEvent::RuntimeCommand(RuntimeCommand::SetApiLogEnabled {
                    enabled: false,
                }))
                .ok();
            return vec![];
        }

        if let Some(rest) = input.strip_prefix("/debug api-log ") {
            return vec![format!(
                "usage: /debug api-log <enable|disable>  (got '{}')",
                rest.trim()
            )];
        }

        if input == "/semantic-memory index" || input == "/semantic-memory reindex" {
            app_tx
                .send(AppEvent::RuntimeCommand(RuntimeCommand::SemanticMemoryIndex {
                    full: false,
                }))
                .ok();
            return out;
        }

        if input == "/semantic-memory index full" || input == "/semantic-memory reindex full" {
            app_tx
                .send(AppEvent::RuntimeCommand(RuntimeCommand::SemanticMemoryIndex {
                    full: true,
                }))
                .ok();
            return out;
        }

        if input == "/clear" {
            app_tx
                .send(AppEvent::RuntimeCommand(RuntimeCommand::ClearContext))
                .ok();
            return out;
        }

        if input == "/config" {
            return crate::app_state::session_config_lines(&self.runtime.session);
        }

        if input == "/session show" {
            return crate::app_state::session_show_lines(&self.runtime.session);
        }

        if let Some(rest) = input.strip_prefix("/session ") {
            let parts: Vec<&str> = rest.splitn(3, ' ').collect();
            match parts.as_slice() {
                ["profile", "use", name] => {
                    app_tx
                        .send(AppEvent::RuntimeCommand(RuntimeCommand::SessionProfileUse {
                            name: (*name).to_string(),
                        }))
                        .ok();
                }
                ["model", "use", model] => {
                    app_tx
                        .send(AppEvent::RuntimeCommand(RuntimeCommand::SessionModelUse {
                            model: (*model).to_string(),
                        }))
                        .ok();
                }
                ["reset"] => {
                    app_tx
                        .send(AppEvent::RuntimeCommand(RuntimeCommand::SessionReset))
                        .ok();
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
                    out.extend(crate::app_state::config_profile_list_lines(&self.runtime.session));
                }
                ["profile", "show"] => {
                    out.extend(crate::app_state::session_config_lines(&self.runtime.session));
                }
                ["profile", "create", name] => {
                    app_tx
                        .send(AppEvent::RuntimeCommand(RuntimeCommand::ConfigProfileCreate {
                            name: (*name).to_string(),
                        }))
                        .ok();
                }
                ["profile", "use", name] => {
                    app_tx
                        .send(AppEvent::RuntimeCommand(RuntimeCommand::ConfigProfileUse {
                            name: (*name).to_string(),
                        }))
                        .ok();
                }
                ["profile", "set", kv] => {
                    if let Some((key, val)) = kv.split_once('=') {
                        app_tx
                            .send(AppEvent::RuntimeCommand(RuntimeCommand::ConfigProfileSet {
                                key: key.to_string(),
                                value: val.to_string(),
                            }))
                            .ok();
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



    pub(crate) fn submit_text(&mut self, text: String, app_tx: &mpsc::UnboundedSender<AppEvent>) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }

        self.return_to_latest();

        if text == "/exit" || text == "/quit" {
            crate::app_state::request_app_exit(self);
            return;
        }

        if let Some(command) = text.strip_prefix('!') {
            crate::app_state::submit_shell_command(self, command);
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
        {
            crate::app_state::resolve_and_submit_text(self, text, app_tx);
            return;
        }
        #[cfg(not(feature = "stylos"))]
        crate::app_state::submit_text_default(self, text);
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
        self.runtime.activity_counters.input_mouse_count += 1;
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
        self.runtime.activity_counters.input_paste_count += 1;
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
            AppEvent::SnapshotUpdated(snapshot) => {
                self.snapshot_hub.publish(snapshot);
                self.mark_dirty_all();
                self.request_draw(frame_requester);
            }
            AppEvent::RuntimeCommand(command) => {
                crate::app_state::handle_runtime_command(self, command, frame_requester, app_tx);
            }
            AppEvent::LoginPrompt {
                user_code,
                verification_uri,
            } => {
                self.handle_login_prompt_event(user_code, verification_uri, frame_requester);
            }
            AppEvent::LoginComplete { profile_name, auth_result } => {
                crate::app_state::handle_login_complete_event(self, profile_name, auth_result, frame_requester)
                    .await;
            }
            AppEvent::LocalAgentManagement(request) => {
                crate::app_state::handle_local_agent_management_request(self, request, frame_requester);
            }
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
            self.runtime.activity_counters.draw_count += 1;
            self.runtime.activity_counters.draw_total_us += draw_us;
            self.runtime.activity_counters.draw_max_us = self.runtime.activity_counters.draw_max_us.max(draw_us);
            self.dirty.clear();
        } else {
            self.runtime.activity_counters.draw_skip_clean_count += 1;
        }
        Ok(())
    }

    pub(crate) fn handle_key_event(
        &mut self,
        key: event::KeyEvent,
        frame_requester: &FrameRequester,
        app_tx: &mpsc::UnboundedSender<AppEvent>,
        terminal: &Terminal<CrosstermBackend<std::io::Stdout>>,
    ) {
        self.runtime.activity_counters.input_key_count += 1;

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
                    crate::app_state::confirm_ctrl_c_exit(self);
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

    let lines = build_lines(&app.entries, &app.runtime.pending, &app.runtime.agents);
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
        .runtime.project_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("/");
    let activity = app.activity_status_value();
    #[cfg(feature = "stylos")]
    let stylos_status = match app.runtime.stylos.as_ref().map(|h| h.state()) {
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
        app.runtime.session.active_profile,
        app.runtime.session.model,
        project_leaf,
        stylos_status,
        app.runtime.workflow_state.workflow_name,
        app.runtime.workflow_state.phase_name,
        activity,
        nav,
    );
    #[cfg(not(feature = "stylos"))]
    let bar_top = format!(
        " {} | {} | {} | flow: {} | phase: {} | agent: {} | nav: {}",
        app.runtime.session.active_profile,
        app.runtime.session.model,
        project_leaf,
        app.runtime.workflow_state.workflow_name,
        app.runtime.workflow_state.phase_name,
        activity,
        nav,
    );
    let bar_bottom = format!(
        " {} | in:{} out:{} cached:{} | ctx:{}",
        build_rate_limit_statusline(app.runtime.status_rate_limits.as_ref()),
        format_human_count(app.runtime.session_tokens.tokens_in),
        format_human_count(app.runtime.session_tokens.tokens_out),
        format_human_count(app.runtime.session_tokens.tokens_cached),
        build_context_statusline(app.runtime.last_ctx_tokens, app.runtime.status_model_info.as_ref()),
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
            ReviewMode::Transcript => (" Transcript review ", build_lines(&app.entries, &None, &app.runtime.agents)),
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
        ReviewMode::Transcript => build_lines(&app.entries, &None, &app.runtime.agents),
        ReviewMode::Watchdog => build_watchdog_review_lines(app),
        ReviewMode::Closed => build_lines(&app.entries, &app.runtime.pending, &app.runtime.agents),
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
    let snapshot = app.snapshot_hub.current();
    #[cfg(feature = "stylos")]
    {
        let snapshot = app.snapshot_hub.current();
        let stylos_state = snapshot
            .stylos_status
            .clone()
            .unwrap_or_else(|| "off".to_string());
        lines.push(Line::from(vec![Span::styled(
            "watchdog",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(format!("stylos: {}", stylos_state)));
        lines.push(Line::from(format!(
            "pending_watchdog_note: {}",
            snapshot.pending_watchdog_note
        )));
        lines.push(Line::from(format!(
            "active_incoming_prompts: {}",
            snapshot.active_incoming_prompt_count
        )));
        lines.push(Line::from(format!(
            "aggregate_busy: {}",
            snapshot.aggregate_busy_agents
        )));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(vec![Span::styled(
        "local agents",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )]));
    for agent in &snapshot.local_agents {
        let roles = if agent.roles.is_empty() {
            "-".to_string()
        } else {
            agent.roles.join(",")
        };
        lines.push(Line::from(format!(
            "{} | label={} | roles={} | busy={} | incoming={}",
            agent.agent_id, agent.label, roles, agent.busy, agent.incoming
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("Ctrl-t transcript review  Ctrl-w watchdog  Esc close"));
    lines
}

pub(crate) fn unix_epoch_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(all(test, feature = "stylos"))]
mod tests {
    use super::*;
    use crate::app_runtime::{
        allocate_default_local_agent_id, build_local_agent_roster, is_interactive_agent_handle,
        normalize_created_agent_roles, validate_agent_roles,
    };
    use crate::local_prompts::{IncomingPromptRequest, IncomingPromptSource};

    fn handle(agent_id: &str, roles: &[&str]) -> AgentHandle {
        AgentHandle {
            agent: None,
            session_id: Uuid::nil(),
            agent_id: agent_id.to_string(),
            label: agent_id.to_string(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
            busy: false,
            turn_cancellation: None,
        }
    }

    #[test]
    fn stylos_note_display_identifier_prefers_slug() {
        let prompt = "type=stylos_note note_id=123e4567-e89b-12d3-a456-426614174000 note_slug=fix-tests-123e4567 column=todo\n\nbody";
        assert_eq!(
            crate::app_runtime::stylos_note_display_identifier(prompt),
            "note_slug=fix-tests-123e4567"
        );
    }

    #[test]
    fn stylos_note_display_identifier_falls_back_to_note_id() {
        let prompt =
            "type=stylos_note note_id=123e4567-e89b-12d3-a456-426614174000 column=todo\n\nbody";
        assert_eq!(
            crate::app_runtime::stylos_note_display_identifier(prompt),
            "note_id=123e4567-e89b-12d3-a456-426614174000"
        );
    }

    #[test]
    fn validate_agent_roles_accepts_one_master_and_one_interactive() {
        let agents = vec![
            handle("master", &["master", "interactive"]),
            handle("worker", &["background"]),
        ];
        validate_agent_roles(&build_local_agent_roster(&agents)).unwrap();
    }

    #[test]
    fn validate_agent_roles_rejects_zero_master() {
        let agents = vec![handle("worker", &["background"])];
        assert!(validate_agent_roles(&build_local_agent_roster(&agents)).is_err());
    }

    #[test]
    fn validate_agent_roles_rejects_two_master() {
        let agents = vec![handle("a", &["master"]), handle("b", &["master"])];
        assert!(validate_agent_roles(&build_local_agent_roster(&agents)).is_err());
    }

    #[test]
    fn allocates_next_free_smith_id() {
        let agents = vec![
            handle("master", &["master", "interactive"]),
            handle("smith-1", &["worker"]),
            handle("smith-2", &["worker"]),
            handle("smith-4", &["worker"]),
        ];
        assert_eq!(
            allocate_default_local_agent_id(&build_local_agent_roster(&agents)),
            "smith-3"
        );
    }

    #[test]
    fn created_agent_roles_default_to_executor_when_omitted_or_empty() {
        assert_eq!(normalize_created_agent_roles(None), vec!["executor"]);
        assert_eq!(
            normalize_created_agent_roles(Some(&serde_json::json!([]))),
            vec!["executor"]
        );
    }

    #[test]
    fn created_agent_roles_preserve_explicit_roles_without_executor_default() {
        assert_eq!(
            normalize_created_agent_roles(Some(&serde_json::json!(["reviewer"]))),
            vec!["reviewer"]
        );
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
            agents.iter().position(is_interactive_agent_handle)
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
            agents.iter().position(is_interactive_agent_handle)
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
fn format_stylos_activity_lines(snapshot: crate::app_runtime::StylosActivitySnapshot) -> Vec<String> {
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
