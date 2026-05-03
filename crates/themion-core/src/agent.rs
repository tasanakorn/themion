use crate::agents_md;
use crate::client::{ChatBackend, ChatRoundTrace, Message, ModelInfo};
use crate::codex_cli_instruction::CODEX_CLI_WEB_SEARCH_INSTRUCTION;
use crate::context_report::{
    estimate_messages_chars, estimate_messages_tokens, estimate_text_tokens, EstimateMode,
    HistoryTurnReport, PromptContextReport, PromptSectionKind, PromptSectionReport, ReplayForm,
    TokenizerResolutionSource, ToolEstimateMode, ToolEstimateReport,
};
use crate::db::DbHandle;
use crate::predefined_guardrails::PREDEFINED_GUARDRAILS;
use crate::tools;
use crate::workflow::{
    activation_marker, allowed_transitions, can_transition, normalize_workflow_name,
    phase_instructions, previous_phase, start_phase_for_workflow, PhaseEntryKind, PhaseResult,
    PhaseRetryState, WorkflowState, WorkflowStatus, WorkflowTransitionKind, DEFAULT_AGENT,
    DEFAULT_PHASE, DEFAULT_WORKFLOW, LITE_WORKFLOW,
};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;
use tokio::sync::mpsc;
use uuid::Uuid;

const EFFECTIVE_PROMPT_BUDGET_TOKENS: usize = 170_000;
const EFFECTIVE_PROMPT_SPIKE_TOKENS: usize = 250_000;
const RECENT_PRIOR_TURN_BAND: usize = 5;

#[derive(Clone)]
struct TokenEstimateContext {
    estimate_mode: EstimateMode,
    tokenizer_name: Option<String>,
    tokenizer_resolution_source: Option<TokenizerResolutionSource>,
}

impl TokenEstimateContext {
    fn rough_fallback() -> Self {
        Self {
            estimate_mode: EstimateMode::RoughFallback,
            tokenizer_name: None,
            tokenizer_resolution_source: None,
        }
    }

    fn estimate_text(&self, text: &str) -> usize {
        match self.tokenizer_name.as_deref() {
            Some("o200k_base") => tiktoken_rs::o200k_base_singleton()
                .encode_with_special_tokens(text)
                .len(),
            Some("cl100k_base") => tiktoken_rs::cl100k_base_singleton()
                .encode_with_special_tokens(text)
                .len(),
            Some("p50k_base") => tiktoken_rs::p50k_base_singleton()
                .encode_with_special_tokens(text)
                .len(),
            Some("p50k_edit") => tiktoken_rs::p50k_edit_singleton()
                .encode_with_special_tokens(text)
                .len(),
            Some("r50k_base") => tiktoken_rs::r50k_base_singleton()
                .encode_with_special_tokens(text)
                .len(),
            Some("o200k_harmony") => tiktoken_rs::o200k_harmony_singleton()
                .encode_with_special_tokens(text)
                .len(),
            _ => estimate_text_tokens(text),
        }
    }

    fn estimate_messages(&self, messages: &[Message]) -> usize {
        match self.estimate_mode {
            EstimateMode::Tokenizer => messages
                .iter()
                .map(|msg| {
                    let mut total = self.estimate_text(&msg.role);
                    if let Some(content) = &msg.content {
                        total += self.estimate_text(content);
                    }
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        total += self.estimate_text(tool_call_id);
                    }
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            total += self.estimate_text(&tc.id);
                            total += self.estimate_text(&tc.function.name);
                            total += self.estimate_text(&tc.function.arguments);
                        }
                    }
                    total
                })
                .sum(),
            EstimateMode::RoughFallback => estimate_messages_tokens(messages),
        }
    }
}

fn codex_responses_effective_tool_estimate(
    provider: Option<&str>,
    backend: &str,
    tool_defs: &Value,
    token_ctx: &TokenEstimateContext,
) -> ToolEstimateReport {
    let tool_defs_text = serde_json::to_string(tool_defs).unwrap_or_default();
    let raw_tokens = token_ctx.estimate_text(&tool_defs_text);

    if provider != Some("openai-codex") || backend != "responses" {
        return ToolEstimateReport {
            raw_tokens,
            effective_tokens: None,
            mode: ToolEstimateMode::RawOnly,
            backend_scope: None,
        };
    }

    let mut description_tokens = 0usize;
    let mut structural_tokens = raw_tokens;
    if let Some(tools) = tool_defs.as_array() {
        for tool in tools {
            if let Some(desc) = tool.get("description").and_then(Value::as_str) {
                let tokens = token_ctx.estimate_text(desc);
                description_tokens += tokens;
                structural_tokens = structural_tokens.saturating_sub(tokens);
            }
            if let Some(props) = tool
                .get("parameters")
                .and_then(|v| v.get("properties"))
                .and_then(Value::as_object)
            {
                for spec in props.values() {
                    if let Some(desc) = spec.get("description").and_then(Value::as_str) {
                        let tokens = token_ctx.estimate_text(desc);
                        description_tokens += tokens;
                        structural_tokens = structural_tokens.saturating_sub(tokens);
                    }
                }
            }
        }
    }

    let effective_tokens = description_tokens + ((structural_tokens * 3) / 4);
    ToolEstimateReport {
        raw_tokens,
        effective_tokens: Some(effective_tokens),
        mode: ToolEstimateMode::RawPlusEffective,
        backend_scope: Some("openai-codex/responses".to_string()),
    }
}

fn trusted_tokenizer_name_for_model(
    model: &str,
) -> Option<(&'static str, TokenizerResolutionSource)> {
    if let Some(tokenizer) = tiktoken_rs::tokenizer::get_tokenizer(model) {
        return Some((
            match tokenizer {
                tiktoken_rs::tokenizer::Tokenizer::O200kBase => "o200k_base",
                tiktoken_rs::tokenizer::Tokenizer::O200kHarmony => "o200k_harmony",
                tiktoken_rs::tokenizer::Tokenizer::Cl100kBase => "cl100k_base",
                tiktoken_rs::tokenizer::Tokenizer::P50kBase => "p50k_base",
                tiktoken_rs::tokenizer::Tokenizer::P50kEdit => "p50k_edit",
                tiktoken_rs::tokenizer::Tokenizer::R50kBase => "r50k_base",
                tiktoken_rs::tokenizer::Tokenizer::Gpt2 => "r50k_base",
            },
            TokenizerResolutionSource::ExactModelMatch,
        ));
    }

    let trusted = [
        ("gpt-4o", "o200k_base"),
        ("gpt-4.1", "o200k_base"),
        ("gpt-5", "o200k_base"),
        ("o1", "o200k_base"),
        ("o3", "o200k_base"),
        ("o4", "o200k_base"),
        ("gpt-4", "cl100k_base"),
        ("gpt-3.5-turbo", "cl100k_base"),
    ];
    for (prefix, tokenizer) in trusted {
        if model == prefix || model.starts_with(&format!("{prefix}-")) {
            return Some((tokenizer, TokenizerResolutionSource::TrustedFallbackMapping));
        }
    }
    None
}

fn summarize_tool_calls(tool_calls: &[crate::client::ToolCall]) -> String {
    let mut parts = Vec::new();
    for tc in tool_calls {
        let mut line = format!("tool call: {}", tc.function.name);
        if let Ok(value) = serde_json::from_str::<Value>(&tc.function.arguments) {
            if let Some(reason) = value.get("reason").and_then(Value::as_str) {
                let reason = reason.trim();
                if !reason.is_empty() {
                    line.push_str(&format!("\nreason: {}", reason));
                }
            }
        }
        parts.push(line);
    }
    parts.join("\n")
}

fn role_instruction(role: &str) -> Option<&'static str> {
    match role {
        "master" => Some("- master: Lead the team; for non-trivial work, consider creating or delegating to another local agent instead of handling everything yourself. Use board notes or local-agent tools when useful. Simple direct Q&A may be answered directly."),
        "interactive" => Some("- interactive: Own human-facing conversation; respond directly to the user when active/targeted."),
        "executor" => Some("- executor: Do general implementation, investigation, and task execution; report concise results to the task originator."),
        "reviewer" => Some("- reviewer: Review, audit, and validate; do not change files unless explicitly asked."),
        "architect" => Some("- architect: Cover system design; explore, clarify, refine requirements, plan, and identify tradeoffs."),
        _ => None,
    }
}

fn build_local_agent_role_context_text(agent_id: &str, label: &str, roles: &[String]) -> String {
    let roles_text = if roles.is_empty() {
        "executor".to_string()
    } else {
        roles.join(", ")
    };
    let resolved_roles = if roles.is_empty() {
        vec!["executor".to_string()]
    } else {
        roles.to_vec()
    };
    let identity = if label == agent_id {
        format!("- You are agent `{agent_id}`.")
    } else {
        format!("- You are agent `{agent_id}` (alias: `{label}`).")
    };
    let mut lines = vec![
        "Local agent role context:".to_string(),
        identity,
        format!("- Your roles are: {roles_text}."),
        "- Known roles: master=team leader; interactive=human responder; executor=general worker; reviewer=review/validation; architect=design/planning.".to_string(),
        "- Act only from your listed roles; do not assume unlisted roles.".to_string(),
        String::new(),
        "Your role instructions:".to_string(),
    ];
    for role in &resolved_roles {
        if let Some(instruction) = role_instruction(role) {
            lines.push(instruction.to_string());
        }
    }
    if !resolved_roles.iter().any(|role| role == "interactive") {
        lines.push(String::new());
        lines.push("Keep direct chat very short and activity-oriented; report final results to the requester, board note, or coordinating master/interactive agent.".to_string());
    }
    lines.join("\n")
}

fn build_pure_message_turn(turn_messages: &[Message]) -> Vec<Message> {
    let mut out = Vec::new();
    for msg in turn_messages {
        match msg.role.as_str() {
            "assistant" => {
                let mut parts = Vec::new();
                if let Some(content) = &msg.content {
                    let content = content.trim();
                    if !content.is_empty() {
                        parts.push(content.to_string());
                    }
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    let summary = summarize_tool_calls(tool_calls);
                    if !summary.is_empty() {
                        parts.push(summary);
                    }
                }
                if !parts.is_empty() {
                    out.push(Message {
                        role: "assistant".to_string(),
                        content: Some(parts.join("\n")),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
            "tool" => {}
            _ => out.push(Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
                tool_calls: None,
                tool_call_id: None,
            }),
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct TurnStats {
    pub llm_rounds: u32,
    pub tool_calls: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cached: u64,
    pub last_api_call_tokens_in: Option<u64>,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    LlmStart,
    ToolStart {
        name: String,
        arguments_json: String,
        display_arguments_json: Option<String>,
    },
    ToolEnd,
    Status(String),
    AssistantChunk(String),
    AssistantText(String),
    Stats(String),
    WorkflowStateChanged(WorkflowState),
    TurnDone(TurnStats),
}

#[derive(Clone, Default)]
pub struct TurnCancellation {
    interrupted: Arc<AtomicBool>,
}

impl TurnCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }
}

const MAX_HISTORY_REPLAY_AGE: usize = 7;

const MEMORY_KB_GUIDANCE: &str = "Project Memory guidance: memory_* tools are for intentional durable Project Memory knowledge that should outlive the current session, not routine transcript logging or disposable task tracking. Project Memory stores durable knowledge for the current project by default. Use project_dir=\"[GLOBAL]\" only for Global Knowledge: reusable cross-project facts, preferences, conventions, provider/tool behavior, or troubleshooting patterns. Global Knowledge is an explicitly selected context inside Project Memory, not a separate system. When unsure, keep knowledge project-local and promote later only when cross-project usefulness is clear. Prefer knowledge-base shaped entries: concepts, components, files, tasks, decisions, facts, observations, troubleshooting records, and typed links between them. Use node_type values such as concept, component, file, task, decision, fact, observation, troubleshooting, or person. Use node_type=memory only for genuinely narrative long-term capture when a more specific knowledge-base type is not yet known. Add hashtags for retrieval, and link related nodes when the relationship is useful. Keep ordinary conversation history in session history and coordination work in board notes rather than duplicating it into Project Memory.";

const TOOL_START_BOARD_NOTE_DISPLAY_TOOLS: &[&str] = &[
    "board_create_note",
    "board_read_note",
    "board_move_note",
    "board_update_note_result",
];

fn build_tool_start_display_arguments_json(
    db: &DbHandle,
    tool_name: &str,
    arguments_json: &str,
) -> Option<String> {
    if !TOOL_START_BOARD_NOTE_DISPLAY_TOOLS.contains(&tool_name) {
        return None;
    }
    let mut args: Value = serde_json::from_str(arguments_json).ok()?;
    let args_obj = args.as_object_mut()?;

    if tool_name == "board_create_note" {
        if let Some(to_instance) = args_obj.get_mut("to_instance") {
            if to_instance.as_str() == Some("SELF") {
                *to_instance = Value::String("local".to_string());
            }
        }
        if let Some(to_agent_id) = args_obj.get_mut("to_agent_id") {
            if to_agent_id.as_str() == Some("SELF") {
                *to_agent_id = Value::String("master".to_string());
            }
        }
        return serde_json::to_string(&args).ok();
    }

    let needs_note_slug = args_obj
        .get("note_slug")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_none_or(str::is_empty);
    if !needs_note_slug {
        return None;
    }
    let note_id = args_obj.get("note_id")?.as_str()?.trim();
    if note_id.is_empty() {
        return None;
    }
    let note = db.get_board_note(note_id).ok().flatten()?;
    args_obj.insert("note_slug".to_string(), Value::String(note.note_slug));
    serde_json::to_string(&args).ok()
}

pub struct Agent {
    client: Box<dyn ChatBackend + Send + Sync>,
    model: String,
    provider: Option<String>,
    active_profile: Option<String>,
    system_prompt: String,
    messages: Vec<Message>,
    event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
    pub session_id: Uuid,
    pub project_dir: PathBuf,
    pub db: Arc<DbHandle>,
    pub window_turns: usize,
    turn_boundaries: Vec<usize>,
    turn_seq_counter: u32,
    model_info: Option<ModelInfo>,
    workflow_state: WorkflowState,
    local_role_agent_id: Option<String>,
    local_role_label: Option<String>,
    local_role_roles: Vec<String>,
    #[cfg(feature = "stylos")]
    local_agent_id: Option<String>,
    #[cfg(feature = "stylos")]
    local_instance_id: Option<String>,
    #[cfg(feature = "stylos")]
    stylos_tool_invoker: Option<crate::tools::StylosToolInvoker>,
    local_agent_tool_invoker: Option<crate::tools::LocalAgentToolInvoker>,
    system_inspection: Option<crate::tools::SystemInspectionResult>,
    api_log_enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApiRoundLogArtifact {
    pub session_id: String,
    pub project_dir: String,
    pub turn: u32,
    pub round: u32,
    pub provider: Option<String>,
    pub backend: String,
    pub model: String,
    pub request: Value,
    pub response: Option<Value>,
    pub meta: Value,
}

impl Agent {
    pub fn new(
        client: Box<dyn ChatBackend + Send + Sync>,
        model: String,
        system_prompt: String,
    ) -> Self {
        Self {
            client,
            model,
            provider: None,
            active_profile: None,
            system_prompt,
            messages: Vec::new(),
            event_tx: None,
            session_id: Uuid::new_v4(),
            project_dir: PathBuf::new(),
            db: DbHandle::open_in_memory().expect("in-memory db"),
            window_turns: 5,
            turn_boundaries: Vec::new(),
            turn_seq_counter: 0,
            model_info: None,
            workflow_state: WorkflowState::default(),
            local_role_agent_id: None,
            local_role_label: None,
            local_role_roles: Vec::new(),
            #[cfg(feature = "stylos")]
            local_agent_id: None,
            #[cfg(feature = "stylos")]
            local_instance_id: None,
            #[cfg(feature = "stylos")]
            stylos_tool_invoker: None,
            local_agent_tool_invoker: None,
            system_inspection: None,
            api_log_enabled: false,
        }
    }

    pub fn new_verbose(
        client: Box<dyn ChatBackend + Send + Sync>,
        model: String,
        system_prompt: String,
    ) -> Self {
        Self::new(client, model, system_prompt)
    }

    pub fn new_with_events(
        client: Box<dyn ChatBackend + Send + Sync>,
        model: String,
        system_prompt: String,
        tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Self {
        let mut agent = Self::new(client, model, system_prompt);
        agent.event_tx = Some(tx);
        agent
    }

    pub fn new_with_db(
        client: Box<dyn ChatBackend + Send + Sync>,
        model: String,
        provider: Option<String>,
        active_profile: Option<String>,
        system_prompt: String,
        session_id: Uuid,
        project_dir: PathBuf,
        db: Arc<DbHandle>,
    ) -> Self {
        let workflow_state = db
            .get_session_workflow_state(session_id)
            .ok()
            .flatten()
            .unwrap_or_default();
        Self {
            client,
            model,
            provider,
            active_profile,
            system_prompt,
            messages: Vec::new(),
            event_tx: None,
            session_id,
            project_dir,
            db,
            window_turns: 5,
            turn_boundaries: Vec::new(),
            turn_seq_counter: 0,
            model_info: None,
            workflow_state,
            local_role_agent_id: None,
            local_role_label: None,
            local_role_roles: Vec::new(),
            #[cfg(feature = "stylos")]
            local_agent_id: None,
            #[cfg(feature = "stylos")]
            local_instance_id: None,
            #[cfg(feature = "stylos")]
            stylos_tool_invoker: None,
            local_agent_tool_invoker: None,
            system_inspection: None,
            api_log_enabled: false,
        }
    }

    pub fn set_event_tx(&mut self, tx: mpsc::UnboundedSender<AgentEvent>) {
        self.event_tx = Some(tx);
    }

    pub fn set_system_inspection(
        &mut self,
        inspection: Option<crate::tools::SystemInspectionResult>,
    ) {
        self.system_inspection = inspection;
    }

    pub fn set_local_agent_tool_invoker(
        &mut self,
        invoker: Option<crate::tools::LocalAgentToolInvoker>,
    ) {
        self.local_agent_tool_invoker = invoker;
    }

    pub fn set_api_log_enabled(&mut self, enabled: bool) {
        self.api_log_enabled = enabled;
    }

    fn build_api_round_log_artifact(
        &self,
        turn_seq: u32,
        round: u32,
        started_at_ms: u64,
        finished_at_ms: u64,
        trace: ChatRoundTrace,
    ) -> ApiRoundLogArtifact {
        let outcome = if trace.error.is_some() {
            "failed"
        } else {
            "ok"
        };
        ApiRoundLogArtifact {
            session_id: self.session_id.to_string(),
            project_dir: self.project_dir.display().to_string(),
            turn: turn_seq,
            round,
            provider: self.provider.clone(),
            backend: trace.backend.clone(),
            model: self.model.clone(),
            request: trace.request,
            response: trace.response,
            meta: json!({
                "started_at_ms": started_at_ms,
                "finished_at_ms": finished_at_ms,
                "duration_ms": finished_at_ms.saturating_sub(started_at_ms),
                "http_status": trace.http_status,
                "outcome": outcome,
                "error": trace.error,
                "usage": trace.usage,
                "rate_limits": trace.rate_limits,
            }),
        }
    }

    fn write_api_round_log(&self, artifact: &ApiRoundLogArtifact) -> Result<()> {
        let turn_dir = std::env::temp_dir()
            .join("themion")
            .join(&artifact.session_id)
            .join(artifact.turn.to_string());
        std::fs::create_dir_all(&turn_dir)?;
        let path = turn_dir.join(format!("round_{}.json", artifact.round));
        let body = serde_json::to_vec_pretty(artifact)?;
        std::fs::write(path, body)?;
        Ok(())
    }

    fn write_api_round_log_if_enabled(&self, artifact: ApiRoundLogArtifact) {
        if let Err(err) = self.write_api_round_log(&artifact) {
            self.emit_status(format!(
                "warning: failed to write api log round {}: {}",
                artifact.round, err
            ));
        }
    }

    fn build_failed_api_round_log_artifact(
        &self,
        turn_seq: u32,
        round: u32,
        started_at_ms: u64,
        finished_at_ms: u64,
        backend: &str,
        request: Value,
        error: &anyhow::Error,
    ) -> ApiRoundLogArtifact {
        self.build_api_round_log_artifact(
            turn_seq,
            round,
            started_at_ms,
            finished_at_ms,
            ChatRoundTrace {
                backend: backend.to_string(),
                request,
                response: None,
                error: Some(error.to_string()),
                http_status: None,
                usage: None,
                rate_limits: None,
            },
        )
    }

    pub fn set_local_agent_role_context(
        &mut self,
        agent_id: impl Into<String>,
        label: impl Into<String>,
        roles: Vec<String>,
    ) {
        self.local_role_agent_id = Some(agent_id.into());
        self.local_role_label = Some(label.into());
        self.local_role_roles = roles;
    }

    #[cfg(feature = "stylos")]
    pub fn set_stylos_tool_invoker(&mut self, invoker: Option<crate::tools::StylosToolInvoker>) {
        self.stylos_tool_invoker = invoker;
    }

    #[cfg(feature = "stylos")]
    pub fn set_local_agent_id(&mut self, agent_id: Option<String>) {
        self.local_agent_id = agent_id;
    }

    #[cfg(feature = "stylos")]
    pub fn set_local_instance_id(&mut self, instance_id: Option<String>) {
        self.local_instance_id = instance_id;
    }

    fn build_prompt_context_report(&self, activation_source: &str) -> PromptContextReport {
        let token_ctx = trusted_tokenizer_name_for_model(&self.model)
            .map(|(tokenizer_name, resolution_source)| TokenEstimateContext {
                estimate_mode: EstimateMode::Tokenizer,
                tokenizer_name: Some(tokenizer_name.to_string()),
                tokenizer_resolution_source: Some(resolution_source),
            })
            .unwrap_or_else(TokenEstimateContext::rough_fallback);
        let mut sections = Vec::new();

        let system_prompt = vec![Message {
            role: "system".to_string(),
            content: Some(self.system_prompt.clone()),
            tool_calls: None,
            tool_call_id: None,
        }];
        sections.push(PromptSectionReport {
            kind: PromptSectionKind::SystemPrompt,
            label: "system prompt".to_string(),
            chars: estimate_messages_chars(&system_prompt),
            tokens_estimate: token_ctx.estimate_messages(&system_prompt),
            messages: system_prompt,
            extra_text: None,
            tool_estimate: None,
        });

        let coding_guardrails = vec![Message {
            role: "system".to_string(),
            content: Some(PREDEFINED_GUARDRAILS.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }];
        sections.push(PromptSectionReport {
            kind: PromptSectionKind::CodingGuardrails,
            label: "coding guardrails".to_string(),
            chars: estimate_messages_chars(&coding_guardrails),
            tokens_estimate: token_ctx.estimate_messages(&coding_guardrails),
            messages: coding_guardrails,
            extra_text: None,
            tool_estimate: None,
        });

        if let Some(agent_id) = self.local_role_agent_id.as_deref() {
            let label = self.local_role_label.as_deref().unwrap_or(agent_id);
            let role_context = vec![Message {
                role: "system".to_string(),
                content: Some(build_local_agent_role_context_text(
                    agent_id,
                    label,
                    &self.local_role_roles,
                )),
                tool_calls: None,
                tool_call_id: None,
            }];
            sections.push(PromptSectionReport {
                kind: PromptSectionKind::RoleContext,
                label: "local agent role context".to_string(),
                chars: estimate_messages_chars(&role_context),
                tokens_estimate: token_ctx.estimate_messages(&role_context),
                messages: role_context,
                extra_text: None,
                tool_estimate: None,
            });
        }

        #[cfg(feature = "stylos")]
        let board_guidance_text = {
            let self_instance = self.local_instance_id.as_deref().unwrap_or("local");
            let self_agent_id = self.local_agent_id.as_deref().unwrap_or("master");
            format!(
                "Board guidance: simple direct Q&A without tools usually should not create a self-note. If the task needs tools, edits, validation, or durable follow-up tracking, consider creating a durable board note for yourself to help keep track of the work. For self-notes, prefer the magic local target to_instance=local to_agent_id={self_agent_id}. In this session, the exact current self target is to_instance={self_instance} to_agent_id={self_agent_id}. Use local for ordinary self-notes unless you specifically need the exact instance id. Multi-agent collaboration guidance: prefer durable board notes over stylos_request_talk when delegating asynchronous or non-urgent work to another agent. Treat stylos_request_talk as an interrupting realtime path for urgent coordination or brief clarification. When you receive a done-mention note, treat it as an informational completion notification rather than a fresh work request."
            )
        };
        #[cfg(not(feature = "stylos"))]
        let board_guidance_text = "Board guidance: simple direct Q&A without tools usually should not create a self-note. If the task needs tools, edits, validation, or durable follow-up tracking, consider creating a durable board note for yourself to help keep track of the work. For self-notes, prefer the magic local target to_instance=local to_agent_id=master. Do not invent remote identifiers for local-only board work. Multi-agent collaboration guidance: prefer durable board notes over stylos_request_talk when delegating asynchronous or non-urgent work to another agent. Treat stylos_request_talk as an interrupting realtime path for urgent coordination or brief clarification. When you receive a done-mention note, treat it as an informational completion notification rather than a fresh work request.".to_string();
        let board_guidance = vec![Message {
            role: "system".to_string(),
            content: Some(board_guidance_text),
            tool_calls: None,
            tool_call_id: None,
        }];
        sections.push(PromptSectionReport {
            kind: PromptSectionKind::BoardGuidance,
            label: "board guidance".to_string(),
            chars: estimate_messages_chars(&board_guidance),
            tokens_estimate: token_ctx.estimate_messages(&board_guidance),
            messages: board_guidance,
            extra_text: None,
            tool_estimate: None,
        });

        let memory_guidance = vec![Message {
            role: "system".to_string(),
            content: Some(MEMORY_KB_GUIDANCE.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }];
        sections.push(PromptSectionReport {
            kind: PromptSectionKind::MemoryGuidance,
            label: "memory guidance".to_string(),
            chars: estimate_messages_chars(&memory_guidance),
            tokens_estimate: token_ctx.estimate_messages(&memory_guidance),
            messages: memory_guidance,
            extra_text: None,
            tool_estimate: None,
        });

        let codex_instruction = vec![Message {
            role: "system".to_string(),
            content: Some(CODEX_CLI_WEB_SEARCH_INSTRUCTION.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }];
        sections.push(PromptSectionReport {
            kind: PromptSectionKind::CodexCliWebSearch,
            label: "codex cli web-search instruction".to_string(),
            chars: estimate_messages_chars(&codex_instruction),
            tokens_estimate: token_ctx.estimate_messages(&codex_instruction),
            messages: codex_instruction,
            extra_text: None,
            tool_estimate: None,
        });

        if let Some(agents_md_message) = agents_md::build_agents_md_message(&self.project_dir) {
            let agents_md = vec![Message {
                role: "user".to_string(),
                content: Some(agents_md_message),
                tool_calls: None,
                tool_call_id: None,
            }];
            sections.push(PromptSectionReport {
                kind: PromptSectionKind::AgentsMd,
                label: "AGENTS.md instructions".to_string(),
                chars: estimate_messages_chars(&agents_md),
                tokens_estimate: token_ctx.estimate_messages(&agents_md),
                messages: agents_md,
                extra_text: None,
                tool_estimate: None,
            });
        }

        let workflow_messages = self.build_workflow_context_messages(activation_source);
        sections.push(PromptSectionReport {
            kind: PromptSectionKind::WorkflowContext,
            label: "workflow context".to_string(),
            chars: estimate_messages_chars(&workflow_messages),
            tokens_estimate: token_ctx.estimate_messages(&workflow_messages),
            messages: workflow_messages,
            extra_text: None,
            tool_estimate: None,
        });

        let tool_defs = tools::tool_definitions();
        let tool_defs_text = serde_json::to_string(&tool_defs).unwrap_or_default();
        let tool_estimate = codex_responses_effective_tool_estimate(
            self.provider.as_deref(),
            self.client.backend_name(),
            &tool_defs,
            &token_ctx,
        );
        sections.push(PromptSectionReport {
            kind: PromptSectionKind::ToolDefinitions,
            label: "tool definitions".to_string(),
            messages: Vec::new(),
            extra_text: Some(tool_defs_text.clone()),
            chars: tool_defs_text.len(),
            tokens_estimate: tool_estimate
                .effective_tokens
                .unwrap_or(tool_estimate.raw_tokens),
            tool_estimate: Some(tool_estimate),
        });

        let prompt_overhead_tokens = sections.iter().map(|s| s.tokens_estimate).sum::<usize>();
        let recent_turn_count = self.turn_boundaries.len();
        let mut included_history = Vec::new();
        let mut history_turns = Vec::new();
        let mut omitted_turns = 0usize;
        let mut cap_omitted_turns = 0usize;
        let mut t0_exceeds_normal_budget = false;
        let mut t0_exceeds_spike_budget = false;

        if recent_turn_count > 0 {
            let t0_index = recent_turn_count - 1;
            let t0_start = self.turn_boundaries[t0_index];
            let t0_messages = &self.messages[t0_start..];
            let t0_tokens = token_ctx.estimate_messages(t0_messages);
            t0_exceeds_normal_budget = t0_tokens > EFFECTIVE_PROMPT_BUDGET_TOKENS;
            t0_exceeds_spike_budget = t0_tokens > EFFECTIVE_PROMPT_SPIKE_TOKENS;
            included_history.extend_from_slice(t0_messages);
            history_turns.push(HistoryTurnReport {
                turn_label: "T0".to_string(),
                replay_form: ReplayForm::Full,
                omitted: false,
                chars: estimate_messages_chars(t0_messages),
                tokens_estimate: t0_tokens,
                messages: t0_messages.to_vec(),
                note: if t0_exceeds_spike_budget {
                    Some("T0 alone exceeds spike budget; prior turns are not replayed".to_string())
                } else {
                    None
                },
            });
            let mut used_tokens = prompt_overhead_tokens + t0_tokens;

            for age_from_t0 in (MAX_HISTORY_REPLAY_AGE + 1)..=t0_index {
                history_turns.push(HistoryTurnReport {
                    turn_label: format!("T-{}", age_from_t0),
                    replay_form: ReplayForm::Full,
                    omitted: true,
                    chars: 0,
                    tokens_estimate: 0,
                    messages: Vec::new(),
                    note: Some("omitted by T-7 replay cap".to_string()),
                });
                omitted_turns += 1;
                cap_omitted_turns += 1;
            }

            if !t0_exceeds_spike_budget {
                let oldest_allowed_idx = t0_index.saturating_sub(MAX_HISTORY_REPLAY_AGE);
                for older_idx in (oldest_allowed_idx..t0_index).rev() {
                    let start = self.turn_boundaries[older_idx];
                    let end = self
                        .turn_boundaries
                        .get(older_idx + 1)
                        .copied()
                        .unwrap_or(self.messages.len());
                    let age_from_t0 = t0_index - older_idx;
                    let replay_form =
                        if t0_exceeds_normal_budget && age_from_t0 <= RECENT_PRIOR_TURN_BAND {
                            ReplayForm::PureMessage
                        } else {
                            ReplayForm::Full
                        };
                    let candidate = match replay_form {
                        ReplayForm::Full => self.messages[start..end].to_vec(),
                        ReplayForm::PureMessage => {
                            build_pure_message_turn(&self.messages[start..end])
                        }
                    };
                    let candidate_tokens = token_ctx.estimate_messages(&candidate);
                    let candidate_chars = estimate_messages_chars(&candidate);
                    let turn_label = format!("T-{}", age_from_t0);
                    if used_tokens + candidate_tokens > EFFECTIVE_PROMPT_SPIKE_TOKENS {
                        history_turns.push(HistoryTurnReport {
                            turn_label,
                            replay_form,
                            omitted: true,
                            chars: 0,
                            tokens_estimate: 0,
                            messages: Vec::new(),
                            note: Some("omitted to stay within spike budget".to_string()),
                        });
                        omitted_turns += 1;
                        for skipped_idx in (oldest_allowed_idx..older_idx).rev() {
                            history_turns.push(HistoryTurnReport {
                                turn_label: format!("T-{}", t0_index - skipped_idx),
                                replay_form: ReplayForm::Full,
                                omitted: true,
                                chars: 0,
                                tokens_estimate: 0,
                                messages: Vec::new(),
                                note: Some("omitted because a newer allowed turn already hit the spike budget".to_string()),
                            });
                            omitted_turns += 1;
                        }
                        break;
                    }
                    used_tokens += candidate_tokens;
                    included_history.splice(0..0, candidate.clone());
                    history_turns.push(HistoryTurnReport {
                        turn_label,
                        replay_form,
                        omitted: false,
                        chars: candidate_chars,
                        tokens_estimate: candidate_tokens,
                        messages: candidate,
                        note: if replay_form == ReplayForm::PureMessage {
                            Some(
                                "reduced pure-message replay; raw tool payloads omitted"
                                    .to_string(),
                            )
                        } else {
                            None
                        },
                    });
                }
            } else {
                omitted_turns += t0_index.min(MAX_HISTORY_REPLAY_AGE);
                let highest_age_in_band = t0_index.min(MAX_HISTORY_REPLAY_AGE);
                for age_from_t0 in 1..=highest_age_in_band {
                    history_turns.push(HistoryTurnReport {
                        turn_label: format!("T-{}", age_from_t0),
                        replay_form: ReplayForm::Full,
                        omitted: true,
                        chars: 0,
                        tokens_estimate: 0,
                        messages: Vec::new(),
                        note: Some("omitted because T0 alone exceeds spike budget".to_string()),
                    });
                }
            }
        }

        history_turns.sort_by_key(|turn| {
            if turn.turn_label == "T0" {
                0usize
            } else {
                turn.turn_label
                    .strip_prefix("T-")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(usize::MAX)
            }
        });

        if omitted_turns > 0 {
            let recall_hint = vec![Message {
                role: "system".to_string(),
                content: Some(format!(
                    "Note: {} earlier turn(s) are stored in history. Use history_recall or history_search without session_id for the current session, or pass session_id=\"*\" to search across all sessions in the current project.",
                    omitted_turns
                )),
                tool_calls: None,
                tool_call_id: None,
            }];
            sections.push(PromptSectionReport {
                kind: PromptSectionKind::HistoryRecallHint,
                label: "history recall hint".to_string(),
                chars: estimate_messages_chars(&recall_hint),
                tokens_estimate: token_ctx.estimate_messages(&recall_hint),
                messages: recall_hint,
                extra_text: None,
                tool_estimate: None,
            });
        }

        sections.push(PromptSectionReport {
            kind: PromptSectionKind::HistoryReplay,
            label: "history replay".to_string(),
            chars: estimate_messages_chars(&included_history),
            tokens_estimate: token_ctx.estimate_messages(&included_history),
            messages: included_history,
            extra_text: None,
            tool_estimate: None,
        });

        let total_chars = sections.iter().map(|s| s.chars).sum();
        let total_tokens_estimate = sections.iter().map(|s| s.tokens_estimate).sum();
        let replayed_turns = history_turns.iter().filter(|t| !t.omitted).count();
        let reduced_turns = history_turns
            .iter()
            .filter(|t| !t.omitted && t.replay_form == ReplayForm::PureMessage)
            .count();

        PromptContextReport {
            sections,
            history_turns,
            total_turns: recent_turn_count,
            replayed_turns,
            omitted_turns,
            cap_omitted_turns,
            reduced_turns,
            total_chars,
            total_tokens_estimate,
            t0_exceeds_normal_budget,
            t0_exceeds_spike_budget,
            estimate_mode: token_ctx.estimate_mode,
            tokenizer_name: token_ctx.tokenizer_name.clone(),
            tokenizer_resolution_source: token_ctx.tokenizer_resolution_source,
        }
    }

    pub fn prompt_context_report(&self) -> PromptContextReport {
        self.build_prompt_context_report("session_state")
    }

    fn current_turn_meta(&self) -> crate::db::TurnMeta {
        crate::db::TurnMeta {
            app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            profile: self.active_profile.clone(),
            provider: self.provider.clone(),
            model: Some(self.model.clone()),
        }
    }

    pub fn clear_context(&mut self) {
        self.messages.clear();
        self.turn_boundaries.clear();
    }

    pub async fn refresh_model_info(&mut self) {
        self.model_info = self
            .client
            .fetch_model_info(&self.model)
            .await
            .ok()
            .flatten();
    }

    pub fn model_info(&self) -> Option<&ModelInfo> {
        self.model_info.as_ref()
    }

    pub fn workflow_state(&self) -> &WorkflowState {
        &self.workflow_state
    }

    fn emit(&self, event: AgentEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event);
        }
    }

    fn emit_status<S: Into<String>>(&self, text: S) {
        self.emit(AgentEvent::Status(text.into()));
    }

    fn emit_phase_result_update(&self, old: PhaseResult, new: PhaseResult) {
        if old != new {
            self.emit_status(format!(
                "phase result: {} -> {}",
                old.as_str(),
                new.as_str()
            ));
        }
    }

    async fn persist_workflow_state(&self) -> Result<()> {
        let db = self.db.clone();
        let sid = self.session_id;
        let state = self.workflow_state.clone();
        tokio::task::spawn_blocking(move || db.update_session_workflow_state(sid, &state))
            .await??;
        Ok(())
    }

    async fn record_transition(
        &self,
        turn_id: Option<i64>,
        turn_seq: Option<u32>,
        from_phase: Option<String>,
        to_phase: String,
        kind: WorkflowTransitionKind,
        trigger: Option<&str>,
    ) -> Result<()> {
        let db = self.db.clone();
        let sid = self.session_id;
        let workflow = self.workflow_state.clone();
        let trigger = trigger.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            db.record_workflow_transition(
                sid,
                turn_id,
                turn_seq,
                &workflow.workflow_name,
                from_phase.as_deref(),
                &to_phase,
                workflow.status.as_str(),
                kind,
                trigger.as_deref(),
                None,
                Some(workflow.retry_state.current_phase_retries),
                Some(workflow.retry_state.previous_phase_retries),
                Some(workflow.retry_state.entered_via.as_str()),
            )
        })
        .await??;
        Ok(())
    }

    fn reset_retry_state(&mut self) {
        self.workflow_state.retry_state = PhaseRetryState::default();
    }

    async fn reset_to_default_workflow(
        &mut self,
        turn_id: Option<i64>,
        turn_seq: Option<u32>,
        trigger: Option<&str>,
    ) -> Result<()> {
        let old_workflow = self.workflow_state.workflow_name.clone();
        let old_phase = self.workflow_state.phase_name.clone();
        let from_phase = Some(old_phase.clone());
        self.workflow_state = WorkflowState::default();
        self.workflow_state.last_updated_turn_seq = turn_seq;
        if old_workflow != self.workflow_state.workflow_name {
            self.emit_status(format!(
                "workflow: {} -> {}",
                old_workflow, self.workflow_state.workflow_name
            ));
        }
        if old_phase != self.workflow_state.phase_name {
            self.emit_status(format!(
                "phase: {} -> {}",
                old_phase, self.workflow_state.phase_name
            ));
        }
        self.emit_phase_result_update(PhaseResult::Pending, self.workflow_state.phase_result);
        self.emit(AgentEvent::WorkflowStateChanged(
            self.workflow_state.clone(),
        ));
        self.persist_workflow_state().await?;
        self.record_transition(
            turn_id,
            turn_seq,
            from_phase,
            self.workflow_state.phase_name.clone(),
            WorkflowTransitionKind::WorkflowStarted,
            trigger,
        )
        .await?;
        Ok(())
    }

    async fn set_workflow(
        &mut self,
        workflow_name: &str,
        turn_id: Option<i64>,
        turn_seq: Option<u32>,
        trigger: Option<&str>,
    ) -> Result<()> {
        let workflow_name = normalize_workflow_name(workflow_name)
            .ok_or_else(|| anyhow::anyhow!("unknown workflow: {workflow_name}"))?;
        let start_phase = start_phase_for_workflow(workflow_name)
            .ok_or_else(|| anyhow::anyhow!("workflow missing start phase: {workflow_name}"))?;
        let old_workflow = self.workflow_state.workflow_name.clone();
        let old_phase = self.workflow_state.phase_name.clone();
        let old_phase_result = self.workflow_state.phase_result;
        let from_phase = Some(old_phase.clone());
        self.workflow_state.workflow_name = workflow_name.to_string();
        self.workflow_state.phase_name = start_phase.to_string();
        self.workflow_state.status = WorkflowStatus::Running;
        self.workflow_state.phase_result = PhaseResult::Pending;
        self.workflow_state.agent_name = DEFAULT_AGENT.to_string();
        self.workflow_state.last_updated_turn_seq = turn_seq;
        self.reset_retry_state();
        if old_workflow != self.workflow_state.workflow_name {
            self.emit_status(format!(
                "workflow: {} -> {}",
                old_workflow, self.workflow_state.workflow_name
            ));
        }
        if old_phase != self.workflow_state.phase_name {
            self.emit_status(format!(
                "phase: {} -> {}",
                old_phase, self.workflow_state.phase_name
            ));
        }
        self.emit_phase_result_update(old_phase_result, self.workflow_state.phase_result);
        self.emit(AgentEvent::WorkflowStateChanged(
            self.workflow_state.clone(),
        ));
        self.persist_workflow_state().await?;
        self.record_transition(
            turn_id,
            turn_seq,
            from_phase,
            self.workflow_state.phase_name.clone(),
            WorkflowTransitionKind::WorkflowStarted,
            trigger,
        )
        .await?;
        Ok(())
    }

    async fn set_phase(
        &mut self,
        next_phase: &str,
        turn_id: Option<i64>,
        turn_seq: Option<u32>,
        trigger: Option<&str>,
    ) -> Result<()> {
        if !can_transition(
            &self.workflow_state.workflow_name,
            &self.workflow_state.phase_name,
            next_phase,
        ) {
            anyhow::bail!(
                "invalid phase transition: {}:{} -> {}",
                self.workflow_state.workflow_name,
                self.workflow_state.phase_name,
                next_phase
            );
        }
        let from_phase = self.workflow_state.phase_name.clone();
        let old_phase_result = self.workflow_state.phase_result;
        self.workflow_state.phase_name = next_phase.to_string();
        self.workflow_state.status = WorkflowStatus::Running;
        self.workflow_state.phase_result = PhaseResult::Pending;
        self.workflow_state.last_updated_turn_seq = turn_seq;
        self.reset_retry_state();
        self.emit_status(format!(
            "phase: {} -> {}",
            from_phase, self.workflow_state.phase_name
        ));
        self.emit_phase_result_update(old_phase_result, self.workflow_state.phase_result);
        self.emit(AgentEvent::WorkflowStateChanged(
            self.workflow_state.clone(),
        ));
        self.persist_workflow_state().await?;
        self.record_transition(
            turn_id,
            turn_seq,
            Some(from_phase),
            self.workflow_state.phase_name.clone(),
            WorkflowTransitionKind::PhaseStarted,
            trigger,
        )
        .await?;
        Ok(())
    }

    async fn retry_or_fail_phase(
        &mut self,
        turn_id: i64,
        turn_seq: u32,
        phase: &str,
        trigger: &str,
    ) -> Result<bool> {
        if self.workflow_state.retry_state.current_phase_retries
            < self.workflow_state.retry_state.current_phase_retry_limit
        {
            self.workflow_state.retry_state.current_phase_retries += 1;
            self.workflow_state.retry_state.entered_via = PhaseEntryKind::RetryCurrentPhase;
            self.workflow_state.status = WorkflowStatus::Running;
            self.workflow_state.phase_result = PhaseResult::Pending;
            self.workflow_state.last_updated_turn_seq = Some(turn_seq);
            self.emit_status(format!("phase: {} -> {}", phase, phase));
            self.emit_phase_result_update(PhaseResult::Failed, self.workflow_state.phase_result);
            self.emit(AgentEvent::WorkflowStateChanged(
                self.workflow_state.clone(),
            ));
            self.persist_workflow_state().await?;
            self.record_transition(
                Some(turn_id),
                Some(turn_seq),
                Some(phase.to_string()),
                phase.to_string(),
                WorkflowTransitionKind::PhaseRetryCurrent,
                Some(trigger),
            )
            .await?;
            return Ok(true);
        }

        if self.workflow_state.retry_state.previous_phase_retries
            < self.workflow_state.retry_state.previous_phase_retry_limit
        {
            if let Some(prev) = previous_phase(&self.workflow_state.workflow_name, phase) {
                self.workflow_state.retry_state.previous_phase_retries += 1;
                self.workflow_state.retry_state.current_phase_retries = 0;
                self.workflow_state.retry_state.entered_via = PhaseEntryKind::RetryPreviousPhase;
                let from_phase = self.workflow_state.phase_name.clone();
                self.workflow_state.phase_name = prev.to_string();
                self.workflow_state.status = WorkflowStatus::Running;
                self.workflow_state.phase_result = PhaseResult::Pending;
                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                self.emit_status(format!(
                    "phase: {} -> {}",
                    from_phase, self.workflow_state.phase_name
                ));
                self.emit_phase_result_update(
                    PhaseResult::Failed,
                    self.workflow_state.phase_result,
                );
                self.emit(AgentEvent::WorkflowStateChanged(
                    self.workflow_state.clone(),
                ));
                self.persist_workflow_state().await?;
                self.record_transition(
                    Some(turn_id),
                    Some(turn_seq),
                    Some(from_phase),
                    prev.to_string(),
                    WorkflowTransitionKind::PhaseRetryPrevious,
                    Some(trigger),
                )
                .await?;
                return Ok(true);
            }
        }

        self.workflow_state.status = WorkflowStatus::Failed;
        self.workflow_state.last_updated_turn_seq = Some(turn_seq);
        self.emit(AgentEvent::WorkflowStateChanged(
            self.workflow_state.clone(),
        ));
        self.persist_workflow_state().await?;
        self.record_transition(
            Some(turn_id),
            Some(turn_seq),
            Some(phase.to_string()),
            phase.to_string(),
            WorkflowTransitionKind::PhaseRetryExhausted,
            Some(trigger),
        )
        .await?;
        Ok(false)
    }

    fn build_workflow_context_messages(&self, activation_source: &str) -> Vec<Message> {
        let mut out = vec![Message {
            role: "system".to_string(),
            content: Some(format!(
                "Workflow context: flow={} phase={} status={} phase_result={} agent={} activation_source={} allowed_next={} retry_current={}/{} retry_previous={}/{} entered_via={}",
                self.workflow_state.workflow_name,
                self.workflow_state.phase_name,
                self.workflow_state.status.as_str(),
                self.workflow_state.phase_result.as_str(),
                self.workflow_state.agent_name,
                activation_source,
                allowed_transitions(&self.workflow_state.workflow_name, &self.workflow_state.phase_name).join(","),
                self.workflow_state.retry_state.current_phase_retries,
                self.workflow_state.retry_state.current_phase_retry_limit,
                self.workflow_state.retry_state.previous_phase_retries,
                self.workflow_state.retry_state.previous_phase_retry_limit,
                self.workflow_state.retry_state.entered_via.as_str(),
            )),
            tool_calls: None,
            tool_call_id: None,
        }];

        let instructions = phase_instructions(
            &self.workflow_state.workflow_name,
            &self.workflow_state.phase_name,
        );
        if !instructions.is_empty() {
            out.push(Message {
                role: "system".to_string(),
                content: Some(format!(
                    "Phase instructions:\n- {}",
                    instructions.join("\n- ")
                )),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        out.push(Message {
            role: "system".to_string(),
            content: Some(
                "Workflow tools: use workflow_get_state to inspect state, workflow_set_active to activate a built-in workflow (which always resets phase to that workflow's start phase), workflow_set_phase for valid transitions within the current workflow, and workflow_complete to mark completed or failed. Workflow tools are internal runtime control actions, not user-facing output. Before ending the turn, always provide a normal assistant response to the user that clearly states the result, progress, or next question. Do not rely on workflow_set_phase_result or workflow_complete as a substitute for a user-facing message."
                    .to_string(),
            ),
            tool_calls: None,
            tool_call_id: None,
        });
        out
    }

    fn apply_workflow_tool_result(
        &mut self,
        tool_name: &str,
        result: &str,
        turn_seq: u32,
    ) -> Result<bool> {
        if result.starts_with("Error:") {
            return Ok(false);
        }
        let parsed: Value = match serde_json::from_str(result) {
            Ok(v) => v,
            Err(_) => return Ok(false),
        };
        match tool_name {
            "workflow_set_active" | "set_workflow" => {
                let workflow = parsed["workflow"].as_str().unwrap_or(DEFAULT_WORKFLOW);
                let phase = parsed["phase"].as_str().unwrap_or(DEFAULT_PHASE);
                let old_workflow = self.workflow_state.workflow_name.clone();
                let old_phase = self.workflow_state.phase_name.clone();
                let old_phase_result = self.workflow_state.phase_result;
                self.workflow_state.workflow_name = workflow.to_string();
                self.workflow_state.phase_name = phase.to_string();
                self.workflow_state.status = WorkflowStatus::Running;
                self.workflow_state.phase_result = PhaseResult::Pending;
                self.workflow_state.agent_name = parsed["agent"]
                    .as_str()
                    .unwrap_or(DEFAULT_AGENT)
                    .to_string();
                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                self.reset_retry_state();
                if old_workflow != self.workflow_state.workflow_name {
                    self.emit_status(format!(
                        "workflow: {} -> {}",
                        old_workflow, self.workflow_state.workflow_name
                    ));
                }
                if old_phase != self.workflow_state.phase_name {
                    self.emit_status(format!(
                        "phase: {} -> {}",
                        old_phase, self.workflow_state.phase_name
                    ));
                }
                self.emit_phase_result_update(old_phase_result, self.workflow_state.phase_result);
                Ok(true)
            }
            "workflow_set_phase" | "set_workflow_phase" => {
                let phase = parsed["phase"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing phase"))?;
                let old_phase = self.workflow_state.phase_name.clone();
                let old_phase_result = self.workflow_state.phase_result;
                self.workflow_state.phase_name = phase.to_string();
                self.workflow_state.status = WorkflowStatus::Running;
                self.workflow_state.phase_result = PhaseResult::Pending;
                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                self.reset_retry_state();
                if old_phase != self.workflow_state.phase_name {
                    self.emit_status(format!(
                        "phase: {} -> {}",
                        old_phase, self.workflow_state.phase_name
                    ));
                }
                self.emit_phase_result_update(old_phase_result, self.workflow_state.phase_result);
                Ok(true)
            }
            "workflow_set_phase_result" | "set_phase_result" => {
                let old_phase_result = self.workflow_state.phase_result;
                self.workflow_state.phase_result =
                    match parsed["phase_result"].as_str().unwrap_or("pending") {
                        "passed" => PhaseResult::Passed,
                        "failed" => PhaseResult::Failed,
                        "user_feedback_required" => PhaseResult::UserFeedbackRequired,
                        _ => PhaseResult::Pending,
                    };
                self.workflow_state.status = match parsed["status"]
                    .as_str()
                    .unwrap_or(self.workflow_state.status.as_str())
                {
                    "waiting_user" => WorkflowStatus::WaitingUser,
                    "completed" => WorkflowStatus::Completed,
                    "failed" => WorkflowStatus::Failed,
                    "interrupted" => WorkflowStatus::Interrupted,
                    _ => WorkflowStatus::Running,
                };
                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                self.emit_phase_result_update(old_phase_result, self.workflow_state.phase_result);
                Ok(true)
            }
            "workflow_complete" | "complete_workflow" => {
                self.workflow_state.status = match parsed["status"].as_str().unwrap_or("running") {
                    "completed" => WorkflowStatus::Completed,
                    "failed" => WorkflowStatus::Failed,
                    _ => WorkflowStatus::Running,
                };
                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn interrupt_turn(&mut self, turn_id: i64, turn_seq: u32) -> Result<()> {
        if self.workflow_state.status == WorkflowStatus::Interrupted {
            return Ok(());
        }
        self.workflow_state.status = WorkflowStatus::Interrupted;
        self.workflow_state.last_updated_turn_seq = Some(turn_seq);
        self.emit_status("turn interrupted");
        self.emit(AgentEvent::WorkflowStateChanged(
            self.workflow_state.clone(),
        ));
        self.persist_workflow_state().await?;
        self.record_transition(
            Some(turn_id),
            Some(turn_seq),
            Some(self.workflow_state.phase_name.clone()),
            self.workflow_state.phase_name.clone(),
            WorkflowTransitionKind::WorkflowInterrupted,
            Some("user_interrupt"),
        )
        .await?;
        Ok(())
    }

    pub async fn run_loop(&mut self, user_input: &str) -> Result<(String, TurnStats)> {
        self.run_loop_with_cancellation(user_input, None).await
    }

    pub async fn run_loop_with_cancellation(
        &mut self,
        user_input: &str,
        cancellation: Option<TurnCancellation>,
    ) -> Result<(String, TurnStats)> {
        if self.model_info.is_none() {
            self.refresh_model_info().await;
        }

        self.turn_seq_counter += 1;
        let turn_seq = self.turn_seq_counter;
        self.emit_status(format!("turn {} started", turn_seq));
        self.turn_boundaries.push(self.messages.len());

        let requested_workflow = activation_marker(user_input);
        let cleaned_user_input = crate::workflow::strip_activation_markers(user_input);
        let effective_user_input = if cleaned_user_input.is_empty() {
            user_input.trim().to_string()
        } else {
            cleaned_user_input
        };

        if self.workflow_state.status == WorkflowStatus::Interrupted {
            self.workflow_state.status = WorkflowStatus::Running;
            self.workflow_state.phase_result = PhaseResult::Pending;
            self.workflow_state.last_updated_turn_seq = Some(turn_seq);
            self.workflow_state.retry_state.entered_via = PhaseEntryKind::Normal;
            if self.workflow_state.workflow_name == DEFAULT_WORKFLOW {
                self.workflow_state.phase_name = "EXECUTE".to_string();
            }
            self.emit_status("workflow: interrupted -> running");
            self.emit(AgentEvent::WorkflowStateChanged(
                self.workflow_state.clone(),
            ));
            self.persist_workflow_state().await?;
        }

        if let Some(workflow) = requested_workflow {
            self.set_workflow(workflow, None, Some(turn_seq), Some("user_input"))
                .await?;
        } else if self.workflow_state.status == WorkflowStatus::WaitingUser
            && self.workflow_state.phase_result == PhaseResult::UserFeedbackRequired
        {
            let old_phase_result = self.workflow_state.phase_result;
            self.workflow_state.status = WorkflowStatus::Running;
            self.workflow_state.phase_result = PhaseResult::Pending;
            self.workflow_state.last_updated_turn_seq = Some(turn_seq);
            self.workflow_state.retry_state.entered_via = PhaseEntryKind::Normal;
            self.emit_phase_result_update(old_phase_result, self.workflow_state.phase_result);
            self.emit(AgentEvent::WorkflowStateChanged(
                self.workflow_state.clone(),
            ));
            self.persist_workflow_state().await?;
        } else if self.workflow_state.workflow_name == DEFAULT_WORKFLOW
            && self.workflow_state.phase_name == DEFAULT_PHASE
        {
            self.workflow_state.phase_name = "EXECUTE".to_string();
            self.workflow_state.status = WorkflowStatus::Running;
            self.workflow_state.phase_result = PhaseResult::Pending;
            self.workflow_state.agent_name = DEFAULT_AGENT.to_string();
            self.workflow_state.last_updated_turn_seq = Some(turn_seq);
            self.reset_retry_state();
            self.emit_status("workflow: NORMAL");
            self.emit_status("phase: IDLE -> EXECUTE");
            self.emit(AgentEvent::WorkflowStateChanged(
                self.workflow_state.clone(),
            ));
            self.persist_workflow_state().await?;
        }

        let turn_id = {
            let db = self.db.clone();
            let sid = self.session_id;
            let workflow = self.workflow_state.clone();
            let turn_meta = self.current_turn_meta();
            tokio::task::spawn_blocking(move || {
                db.begin_turn(sid, turn_seq, &workflow, Some(&turn_meta))
            })
            .await??
        };

        if requested_workflow.is_none() {
            self.record_transition(
                Some(turn_id),
                Some(turn_seq),
                None,
                self.workflow_state.phase_name.clone(),
                WorkflowTransitionKind::WorkflowStarted,
                Some("user_input"),
            )
            .await?;
        }

        self.messages.push(Message {
            role: "user".to_string(),
            content: Some(effective_user_input.clone()),
            tool_calls: None,
            tool_call_id: None,
        });

        {
            let db = self.db.clone();
            let sid = self.session_id;
            let msg = self.messages.last().unwrap().clone();
            let seq = self.messages.len() as u32;
            let workflow = self.workflow_state.clone();
            tokio::task::spawn_blocking(move || {
                db.append_message(turn_id, sid, seq, &msg, &workflow)
            })
            .await??;
        }

        let turn_start = Instant::now();
        let tool_defs = tools::tool_definitions();
        let mut final_response = String::new();

        let mut llm_rounds = 0u32;
        let mut round_index = 0u32;
        let mut tool_calls = 0u32;
        let mut tokens_in = 0u64;
        let mut tokens_out = 0u64;
        let mut tokens_cached = 0u64;
        let mut last_api_call_tokens_in: Option<u64> = None;
        let mut turn_end_reason = "turn_end".to_string();
        let activation_source = if requested_workflow.is_some() {
            "user_input"
        } else {
            "session_state"
        };

        loop {
            if cancellation.as_ref().is_some_and(|c| c.is_interrupted()) {
                self.interrupt_turn(turn_id, turn_seq).await?;
                turn_end_reason = "interrupted".to_string();
                break;
            }

            let prompt_report = self.build_prompt_context_report(activation_source);
            let msgs_with_system = prompt_report
                .sections
                .iter()
                .flat_map(|section| section.messages.clone())
                .collect::<Vec<_>>();

            self.emit(AgentEvent::LlmStart);
            round_index += 1;
            let round_started_at_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let event_tx = self.event_tx.clone();
            let cancellation_for_stream = cancellation.clone();
            let backend_name = self.client.backend_name();
            let request_payload =
                self.client
                    .build_round_request_payload(&self.model, &msgs_with_system, &tool_defs);
            let response_result = self
                .client
                .chat_completion_stream(
                    &self.model,
                    &msgs_with_system,
                    &tool_defs,
                    Box::new(move |chunk| {
                        if cancellation_for_stream
                            .as_ref()
                            .is_some_and(|c| c.is_interrupted())
                        {
                            return;
                        }
                        if let Some(ref tx) = event_tx {
                            let _ = tx.send(AgentEvent::AssistantChunk(chunk));
                        }
                    }),
                    cancellation.clone().map(|c| {
                        Box::new(move || c.is_interrupted())
                            as Box<dyn Fn() -> bool + Send + Sync + 'static>
                    }),
                )
                .await;

            let (response, usage, rate_limit_report, round_trace) = match response_result {
                Ok(v) => v,
                Err(err)
                    if cancellation.as_ref().is_some_and(|c| c.is_interrupted())
                        || err.to_string().contains("interrupted") =>
                {
                    self.interrupt_turn(turn_id, turn_seq).await?;
                    turn_end_reason = "interrupted".to_string();
                    break;
                }
                Err(err) => {
                    let round_finished_at_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(round_started_at_ms);
                    if self.api_log_enabled {
                        let artifact = self.build_failed_api_round_log_artifact(
                            turn_seq,
                            round_index,
                            round_started_at_ms,
                            round_finished_at_ms,
                            &backend_name,
                            request_payload.clone(),
                            &err,
                        );
                        self.write_api_round_log_if_enabled(artifact);
                    }
                    return Err(err);
                }
            };

            let round_finished_at_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(round_started_at_ms);
            if self.api_log_enabled {
                let artifact = self.build_api_round_log_artifact(
                    turn_seq,
                    round_index,
                    round_started_at_ms,
                    round_finished_at_ms,
                    round_trace,
                );
                self.write_api_round_log_if_enabled(artifact);
            }

            if let Some(report) = rate_limit_report {
                if let Ok(text) = serde_json::to_string(&report) {
                    self.emit(AgentEvent::Stats(format!("[rate-limit] {}", text)));
                }
            }

            llm_rounds += 1;
            if let Some(u) = usage {
                if let Some(pt) = u.prompt_tokens {
                    tokens_in += pt;
                    last_api_call_tokens_in = Some(pt);
                }
                if let Some(ct) = u.completion_tokens {
                    tokens_out += ct;
                }
                if let Some(details) = u.prompt_tokens_details {
                    if let Some(cached) = details.cached_tokens {
                        tokens_cached += cached;
                    }
                }
            }

            self.messages.push(Message {
                role: response.role.clone(),
                content: response.content.clone(),
                tool_calls: response.tool_calls.clone(),
                tool_call_id: None,
            });

            {
                let db = self.db.clone();
                let sid = self.session_id;
                let msg = self.messages.last().unwrap().clone();
                let seq = self.messages.len() as u32;
                let workflow = self.workflow_state.clone();
                tokio::task::spawn_blocking(move || {
                    db.append_message(turn_id, sid, seq, &msg, &workflow)
                })
                .await??;
            }

            if let Some(ref content) = response.content {
                final_response = content.clone();
            }

            let tool_calls_vec = match response.tool_calls {
                Some(calls) if !calls.is_empty() => calls,
                _ => {
                    if self.workflow_state.workflow_name == LITE_WORKFLOW {
                        match self.workflow_state.phase_name.as_str() {
                            "CLARIFY" => {
                                let content = response
                                    .content
                                    .as_deref()
                                    .unwrap_or_default()
                                    .to_ascii_lowercase();
                                if content.contains("?")
                                    || content.contains("need clarification")
                                    || content.contains("unclear")
                                {
                                    if self
                                        .retry_or_fail_phase(
                                            turn_id,
                                            turn_seq,
                                            "CLARIFY",
                                            "engine_rule",
                                        )
                                        .await?
                                    {
                                        if self.workflow_state.phase_name == "CLARIFY"
                                            && self.workflow_state.retry_state.entered_via
                                                == PhaseEntryKind::RetryCurrentPhase
                                        {
                                            continue;
                                        }
                                    }
                                    if self.workflow_state.status == WorkflowStatus::Failed {
                                        turn_end_reason = "workflow_failed".to_string();
                                        break;
                                    }
                                    self.workflow_state.status = WorkflowStatus::WaitingUser;
                                    self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                                    self.emit(AgentEvent::WorkflowStateChanged(
                                        self.workflow_state.clone(),
                                    ));
                                    self.persist_workflow_state().await?;
                                    self.record_transition(
                                        Some(turn_id),
                                        Some(turn_seq),
                                        Some("CLARIFY".to_string()),
                                        "CLARIFY".to_string(),
                                        WorkflowTransitionKind::WaitingUser,
                                        Some("engine_rule"),
                                    )
                                    .await?;
                                    turn_end_reason = "phase_waiting_user".to_string();
                                    break;
                                }
                                let old_phase_result = self.workflow_state.phase_result;
                                self.workflow_state.phase_result = PhaseResult::Passed;
                                self.emit_phase_result_update(
                                    old_phase_result,
                                    self.workflow_state.phase_result,
                                );
                                self.persist_workflow_state().await?;
                                self.set_phase(
                                    "EXECUTE",
                                    Some(turn_id),
                                    Some(turn_seq),
                                    Some("engine_rule"),
                                )
                                .await?;
                                continue;
                            }
                            "EXECUTE" => {
                                let content = response
                                    .content
                                    .as_deref()
                                    .unwrap_or_default()
                                    .to_ascii_lowercase();
                                if content.contains("cannot continue")
                                    || content.contains("blocked")
                                    || content.contains("failed")
                                {
                                    if self
                                        .retry_or_fail_phase(
                                            turn_id,
                                            turn_seq,
                                            "EXECUTE",
                                            "model_completion",
                                        )
                                        .await?
                                    {
                                        continue;
                                    }
                                    turn_end_reason = "workflow_failed".to_string();
                                    break;
                                }
                                let old_phase_result = self.workflow_state.phase_result;
                                self.workflow_state.phase_result = PhaseResult::Passed;
                                self.emit_phase_result_update(
                                    old_phase_result,
                                    self.workflow_state.phase_result,
                                );
                                self.persist_workflow_state().await?;
                                self.set_phase(
                                    "VALIDATE",
                                    Some(turn_id),
                                    Some(turn_seq),
                                    Some("engine_rule"),
                                )
                                .await?;
                                continue;
                            }
                            "VALIDATE" => {
                                let content = response
                                    .content
                                    .as_deref()
                                    .unwrap_or_default()
                                    .to_ascii_lowercase();
                                if content.contains("fail") {
                                    let old_phase_result = self.workflow_state.phase_result;
                                    self.workflow_state.phase_result = PhaseResult::Failed;
                                    self.emit_phase_result_update(
                                        old_phase_result,
                                        self.workflow_state.phase_result,
                                    );
                                    self.persist_workflow_state().await?;
                                    if self
                                        .retry_or_fail_phase(
                                            turn_id,
                                            turn_seq,
                                            "VALIDATE",
                                            "model_completion",
                                        )
                                        .await?
                                    {
                                        continue;
                                    }
                                    turn_end_reason = "workflow_failed".to_string();
                                    break;
                                }
                                let old_phase_result = self.workflow_state.phase_result;
                                self.workflow_state.phase_result = PhaseResult::Passed;
                                self.emit_phase_result_update(
                                    old_phase_result,
                                    self.workflow_state.phase_result,
                                );
                                self.workflow_state.status = WorkflowStatus::Completed;
                                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                                self.emit(AgentEvent::WorkflowStateChanged(
                                    self.workflow_state.clone(),
                                ));
                                self.persist_workflow_state().await?;
                                self.record_transition(
                                    Some(turn_id),
                                    Some(turn_seq),
                                    Some("VALIDATE".to_string()),
                                    "VALIDATE".to_string(),
                                    WorkflowTransitionKind::WorkflowCompleted,
                                    Some("model_completion"),
                                )
                                .await?;
                                turn_end_reason = "workflow_completed".to_string();
                                break;
                            }
                            _ => break,
                        }
                    }
                    break;
                }
            };

            for tc in &tool_calls_vec {
                if cancellation.as_ref().is_some_and(|c| c.is_interrupted()) {
                    self.interrupt_turn(turn_id, turn_seq).await?;
                    turn_end_reason = "interrupted".to_string();
                    break;
                }

                let display_arguments_json = build_tool_start_display_arguments_json(
                    self.db.as_ref(),
                    &tc.function.name,
                    &tc.function.arguments,
                );
                self.emit(AgentEvent::ToolStart {
                    name: tc.function.name.clone(),
                    arguments_json: tc.function.arguments.clone(),
                    display_arguments_json,
                });
                let tool_ctx = crate::tools::ToolCtx {
                    db: self.db.clone(),
                    session_id: self.session_id,
                    project_dir: self.project_dir.clone(),
                    workflow_state: Some(self.workflow_state.clone()),
                    turn_seq: Some(turn_seq),
                    #[cfg(feature = "stylos")]
                    local_agent_id: self.local_agent_id.clone(),
                    #[cfg(feature = "stylos")]
                    local_instance_id: self.local_instance_id.clone(),
                    #[cfg(feature = "stylos")]
                    stylos_tool_invoker: self.stylos_tool_invoker.clone(),
                    #[cfg(feature = "stylos")]
                    stylos_enabled: self.stylos_tool_invoker.is_some(),
                    local_agent_tool_invoker: self.local_agent_tool_invoker.clone(),
                    system_inspection: self.system_inspection.clone(),
                };
                let result =
                    tools::call_tool(&tc.function.name, &tc.function.arguments, &tool_ctx).await;
                self.emit(AgentEvent::ToolEnd);
                tool_calls += 1;

                if cancellation.as_ref().is_some_and(|c| c.is_interrupted()) {
                    self.interrupt_turn(turn_id, turn_seq).await?;
                    turn_end_reason = "interrupted".to_string();
                    break;
                }

                let old_phase = self.workflow_state.phase_name.clone();
                if self.apply_workflow_tool_result(&tc.function.name, &result, turn_seq)? {
                    let parsed: Value = serde_json::from_str(&result).unwrap_or_default();
                    if matches!(
                        tc.function.name.as_str(),
                        "workflow_set_active" | "set_workflow"
                    ) {
                        self.persist_workflow_state().await?;
                        self.record_transition(
                            Some(turn_id),
                            Some(turn_seq),
                            Some(old_phase),
                            parsed["phase"]
                                .as_str()
                                .unwrap_or(DEFAULT_PHASE)
                                .to_string(),
                            WorkflowTransitionKind::WorkflowStarted,
                            Some("tool_result"),
                        )
                        .await?;
                    } else if matches!(
                        tc.function.name.as_str(),
                        "workflow_set_phase" | "set_workflow_phase"
                    ) {
                        self.persist_workflow_state().await?;
                        self.record_transition(
                            Some(turn_id),
                            Some(turn_seq),
                            Some(old_phase),
                            parsed["phase"]
                                .as_str()
                                .unwrap_or(DEFAULT_PHASE)
                                .to_string(),
                            WorkflowTransitionKind::PhaseStarted,
                            Some("tool_result"),
                        )
                        .await?;
                    } else if matches!(
                        tc.function.name.as_str(),
                        "workflow_set_phase_result" | "set_phase_result"
                    ) {
                        self.persist_workflow_state().await?;
                        if self.workflow_state.status == WorkflowStatus::WaitingUser {
                            self.record_transition(
                                Some(turn_id),
                                Some(turn_seq),
                                Some(old_phase.clone()),
                                old_phase,
                                WorkflowTransitionKind::WaitingUser,
                                Some("tool_result"),
                            )
                            .await?;
                        }
                    } else if matches!(
                        tc.function.name.as_str(),
                        "workflow_complete" | "complete_workflow"
                    ) {
                        self.persist_workflow_state().await?;
                        let kind = if self.workflow_state.status == WorkflowStatus::Failed {
                            WorkflowTransitionKind::WorkflowFailed
                        } else {
                            WorkflowTransitionKind::WorkflowCompleted
                        };
                        self.record_transition(
                            Some(turn_id),
                            Some(turn_seq),
                            Some(old_phase.clone()),
                            old_phase,
                            kind,
                            Some("tool_result"),
                        )
                        .await?;
                    }
                    self.emit(AgentEvent::WorkflowStateChanged(
                        self.workflow_state.clone(),
                    ));
                }

                self.messages.push(Message {
                    role: "tool".to_string(),
                    content: Some(result),
                    tool_calls: None,
                    tool_call_id: Some(tc.id.clone()),
                });
                {
                    let db = self.db.clone();
                    let sid = self.session_id;
                    let msg = self.messages.last().unwrap().clone();
                    let seq = self.messages.len() as u32;
                    let workflow = self.workflow_state.clone();
                    tokio::task::spawn_blocking(move || {
                        db.append_message(turn_id, sid, seq, &msg, &workflow)
                    })
                    .await??;
                }

                if self.workflow_state.status == WorkflowStatus::Completed {
                    turn_end_reason = "workflow_completed".to_string();
                    break;
                }
                if self.workflow_state.status == WorkflowStatus::Failed {
                    turn_end_reason = "workflow_failed".to_string();
                    break;
                }
                if self.workflow_state.status == WorkflowStatus::WaitingUser {
                    turn_end_reason = "phase_waiting_user".to_string();
                    break;
                }
            }

            if self.workflow_state.status == WorkflowStatus::Interrupted {
                turn_end_reason = "interrupted".to_string();
                break;
            }
            if self.workflow_state.status == WorkflowStatus::Completed {
                turn_end_reason = "workflow_completed".to_string();
                break;
            }
            if self.workflow_state.status == WorkflowStatus::Failed {
                turn_end_reason = "workflow_failed".to_string();
                break;
            }
            if self.workflow_state.status == WorkflowStatus::WaitingUser {
                turn_end_reason = "phase_waiting_user".to_string();
                break;
            }
        }

        if self.workflow_state.status == WorkflowStatus::Completed {
            self.reset_to_default_workflow(
                Some(turn_id),
                Some(turn_seq),
                Some("workflow_completion"),
            )
            .await?;
        } else if self.workflow_state.status != WorkflowStatus::Interrupted
            && self.workflow_state.workflow_name == DEFAULT_WORKFLOW
            && self.workflow_state.phase_name == "EXECUTE"
        {
            self.workflow_state.phase_result = PhaseResult::Passed;
            self.emit_status("phase: EXECUTE -> IDLE");
            self.emit_status("workflow: NORMAL");
            self.workflow_state.phase_name = DEFAULT_PHASE.to_string();
            self.workflow_state.status = WorkflowStatus::Running;
            self.workflow_state.last_updated_turn_seq = Some(turn_seq);
            self.emit(AgentEvent::WorkflowStateChanged(
                self.workflow_state.clone(),
            ));
            self.persist_workflow_state().await?;
            self.record_transition(
                Some(turn_id),
                Some(turn_seq),
                Some("EXECUTE".to_string()),
                self.workflow_state.phase_name.clone(),
                WorkflowTransitionKind::WorkflowCompleted,
                Some("model_completion"),
            )
            .await?;
        }

        let workflow_continues_after_turn = matches!(
            self.workflow_state.status,
            WorkflowStatus::WaitingUser | WorkflowStatus::Interrupted
        );
        let stats = TurnStats {
            llm_rounds,
            tool_calls,
            tokens_in,
            tokens_out,
            tokens_cached,
            last_api_call_tokens_in,
            elapsed_ms: turn_start.elapsed().as_millis(),
        };
        {
            let db = self.db.clone();
            let s = stats.clone();
            let workflow = self.workflow_state.clone();
            let turn_end_reason = turn_end_reason.clone();
            tokio::task::spawn_blocking(move || {
                db.finalize_turn(
                    turn_id,
                    &s,
                    &workflow,
                    workflow_continues_after_turn,
                    &turn_end_reason,
                )
            })
            .await??;
        }
        self.emit(AgentEvent::TurnDone(stats.clone()));
        Ok((final_response, stats))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{FunctionCall, Message, ToolCall};
    use serde_json::json;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.to_string(),
            content: Some(content.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn pure_message_turn_keeps_assistant_text_and_summarizes_tool_reason() {
        let tool_calls = vec![ToolCall {
            id: "call-1".to_string(),
            function: FunctionCall {
                name: "shell_run_command".to_string(),
                arguments: json!({
                    "command": "cargo check",
                    "reason": "validate touched crate"
                })
                .to_string(),
            },
        }];
        let turn = vec![
            msg("user", "please validate it"),
            Message {
                role: "assistant".to_string(),
                content: Some("I will run validation.".to_string()),
                tool_calls: Some(tool_calls),
                tool_call_id: None,
            },
            Message {
                role: "tool".to_string(),
                content: Some("finished".to_string()),
                tool_calls: None,
                tool_call_id: Some("call-1".to_string()),
            },
        ];

        let pure = build_pure_message_turn(&turn);
        assert_eq!(pure.len(), 2);
        assert_eq!(pure[0].role, "user");
        let assistant = &pure[1];
        assert_eq!(assistant.role, "assistant");
        let content = assistant.content.as_deref().unwrap();
        assert!(content.contains("I will run validation."));
        assert!(content.contains("tool call: shell_run_command"));
        assert!(content.contains("reason: validate touched crate"));
        assert!(assistant.tool_calls.is_none());
        assert!(assistant.tool_call_id.is_none());
    }

    #[test]
    fn pure_message_turn_omits_tool_messages() {
        let turn = vec![
            msg("user", "read the file"),
            Message {
                role: "assistant".to_string(),
                content: Some("Reading it now.".to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: "tool".to_string(),
                content: Some("file contents".to_string()),
                tool_calls: None,
                tool_call_id: Some("tool-1".to_string()),
            },
        ];

        let pure = build_pure_message_turn(&turn);
        assert_eq!(pure.len(), 2);
        assert!(pure.iter().all(|m| m.role != "tool"));
    }

    #[test]
    fn estimate_messages_tokens_uses_char_div_ceil_four() {
        let messages = vec![msg("user", "12345678")];
        assert_eq!(estimate_messages_tokens(&messages), 3);
    }

    #[test]
    fn role_context_for_executor_omits_master_and_interactive_actions() {
        let text =
            build_local_agent_role_context_text("smith-1", "smith-1", &["executor".to_string()]);
        assert!(text.contains("Your roles are: executor."));
        assert!(text.contains("Known roles: master=team leader"));
        assert!(text.contains("- executor: Do general implementation"));
        assert!(text.contains("Keep direct chat very short"));
        assert!(!text.contains("- master: Lead the team"));
        assert!(!text.contains("- interactive: Own human-facing conversation"));
    }

    #[test]
    fn role_context_for_master_interactive_includes_matching_actions() {
        let text = build_local_agent_role_context_text(
            "master",
            "master",
            &["master".to_string(), "interactive".to_string()],
        );
        assert!(text.contains("Your roles are: master, interactive."));
        assert!(text.contains("- master: Lead the team"));
        assert!(text.contains("- interactive: Own human-facing conversation"));
        assert!(!text.contains("Keep direct chat very short"));
    }
}
