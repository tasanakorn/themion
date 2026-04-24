use crate::agents_md;
use crate::client::{ChatBackend, Message, ModelInfo};
use crate::codex_cli_instruction::CODEX_CLI_WEB_SEARCH_INSTRUCTION;
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
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct TurnStats {
    pub llm_rounds: u32,
    pub tool_calls: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cached: u64,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    LlmStart,
    ToolStart { detail: String },
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

const TOOL_DETAIL_MAX_CHARS: usize = 60;
const TOOL_DETAIL_CENTER_TRIM_MARKER: &str = "󱑼";
const MEMORY_KB_GUIDANCE: &str = "Long-term memory knowledge-base guidance: memory_* tools are for intentional durable knowledge that should outlive the current session, not for routine transcript logging or disposable task tracking. Prefer knowledge-base shaped entries: concepts, components, files, tasks, decisions, facts, observations, troubleshooting records, and typed links between them. Use node_type values such as concept, component, file, task, decision, fact, observation, troubleshooting, or person. Use node_type=memory only for genuinely narrative long-term capture when a more specific knowledge-base type is not yet known. Add hashtags for retrieval, and link related nodes when the relationship is useful. Keep ordinary conversation history in session history and coordination work in board notes rather than duplicating it into the memory knowledge base.";

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

fn self_session_id_fallback() -> String {
    "session-bound".to_string()
}

fn tool_call_detail(name: &str, args_json: &str) -> String {
    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
    let t = |key: &str| center_trim(args[key].as_str().unwrap_or("?"), TOOL_DETAIL_MAX_CHARS);
    match name {
        "shell_run_command" | "bash" => format!("shell: {}", t("command")),
        "fs_read_file" | "read_file" => format!("read: {}", t("path")),
        "fs_write_file" | "write_file" => format!("write: {}", t("path")),
        "fs_list_directory" | "list_directory" => format!("ls: {}", t("path")),
        "history_recall" | "recall_history" => format!(
            "history_recall: session={}",
            center_trim(
                &args["session_id"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| self_session_id_fallback()),
                TOOL_DETAIL_MAX_CHARS,
            )
        ),
        "history_search" | "search_history" => format!("history_search: {}", t("query")),
        "workflow_get_state" | "get_workflow_state" => "workflow: inspect".to_string(),
        "workflow_set_active" | "set_workflow" => format!("workflow: set {}", t("workflow")),
        "workflow_set_phase" | "set_workflow_phase" => format!("workflow: phase {}", t("phase")),
        "workflow_complete" | "complete_workflow" => format!("workflow: complete {}", t("outcome")),
        "stylos_request_talk" => format!(
            "stylos_request_talk instance={} to_agent_id={}",
            t("instance"),
            center_trim(
                args["to_agent_id"]
                    .as_str()
                    .or_else(|| args["agent_id"].as_str())
                    .unwrap_or("main"),
                TOOL_DETAIL_MAX_CHARS,
            )
        ),
        "board_create_note" => {
            let raw_to_instance = args["to_instance"].as_str().unwrap_or("?").trim();
            let resolved_to_instance = match raw_to_instance {
                "SELF" => "self",
                _ => raw_to_instance,
            };
            format!(
                "board_create_note to_instance={} to_agent_id={}",
                center_trim(resolved_to_instance, TOOL_DETAIL_MAX_CHARS),
                center_trim(
                    args["to_agent_id"].as_str().unwrap_or("main"),
                    TOOL_DETAIL_MAX_CHARS,
                )
            )
        }
        _ => name.to_string(),
    }
}

pub struct Agent {
    client: Box<dyn ChatBackend + Send + Sync>,
    model: String,
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
    #[cfg(feature = "stylos")]
    local_agent_id: Option<String>,
    #[cfg(feature = "stylos")]
    local_instance_id: Option<String>,
    #[cfg(feature = "stylos")]
    stylos_tool_invoker: Option<crate::tools::StylosToolInvoker>,
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
            #[cfg(feature = "stylos")]
            local_agent_id: None,
            #[cfg(feature = "stylos")]
            local_instance_id: None,
            #[cfg(feature = "stylos")]
            stylos_tool_invoker: None,
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
            #[cfg(feature = "stylos")]
            local_agent_id: None,
            #[cfg(feature = "stylos")]
            local_instance_id: None,
            #[cfg(feature = "stylos")]
            stylos_tool_invoker: None,
        }
    }

    pub fn set_event_tx(&mut self, tx: mpsc::UnboundedSender<AgentEvent>) {
        self.event_tx = Some(tx);
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
            tokio::task::spawn_blocking(move || db.begin_turn(sid, turn_seq, &workflow)).await??
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
        let mut tool_calls = 0u32;
        let mut tokens_in = 0u64;
        let mut tokens_out = 0u64;
        let mut tokens_cached = 0u64;
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

            let mut msgs_with_system = vec![Message {
                role: "system".to_string(),
                content: Some(self.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            }];

            msgs_with_system.push(Message {
                role: "system".to_string(),
                content: Some(PREDEFINED_GUARDRAILS.to_string()),
                tool_calls: None,
                tool_call_id: None,
            });

            #[cfg(feature = "stylos")]
            msgs_with_system.push(Message {
                role: "system".to_string(),
                content: Some({
                    let self_instance = self.local_instance_id.as_deref().unwrap_or("unknown");
                    let self_agent_id = self.local_agent_id.as_deref().unwrap_or("main");
                    format!(
                        "Board guidance: simple direct Q&A without tools usually should not create a self-note. If the task needs tools, edits, validation, or durable follow-up tracking, consider creating a durable board note for yourself to help keep track of the work. Your exact self-note target in this session is to_instance={self_instance} to_agent_id={self_agent_id}. For self-notes, you may also call board_create_note with the exact magic keyword SELF for both to_instance and to_agent_id, and the runtime will replace SELF with those exact values. Do not invent placeholders or guesses other than the exact SELF keyword. Multi-agent collaboration guidance: prefer durable board notes over stylos_request_talk when delegating asynchronous or non-urgent work to another agent. Treat stylos_request_talk as an interrupting realtime path for urgent coordination or brief clarification. When you receive a done-mention note, treat it as an informational completion notification rather than a fresh work request."
                    )
                }),
                tool_calls: None,
                tool_call_id: None,
            });

            msgs_with_system.push(Message {
                role: "system".to_string(),
                content: Some(MEMORY_KB_GUIDANCE.to_string()),
                tool_calls: None,
                tool_call_id: None,
            });

            #[cfg(not(feature = "stylos"))]
            msgs_with_system.push(Message {
                role: "system".to_string(),
                content: Some(
                    "Board guidance: simple direct Q&A without tools usually should not create a self-note. If the task needs tools, edits, validation, or durable follow-up tracking, consider creating a durable board note for yourself to help keep track of the work. For self-notes in this session, use your local board context rather than inventing remote identifiers. Multi-agent collaboration guidance: prefer durable board notes over stylos_request_talk when delegating asynchronous or non-urgent work to another agent. Treat stylos_request_talk as an interrupting realtime path for urgent coordination or brief clarification. When you receive a done-mention note, treat it as an informational completion notification rather than a fresh work request.".to_string(),
                ),
                tool_calls: None,
                tool_call_id: None,
            });

            msgs_with_system.push(Message {
                role: "system".to_string(),
                content: Some(CODEX_CLI_WEB_SEARCH_INSTRUCTION.to_string()),
                tool_calls: None,
                tool_call_id: None,
            });

            if let Some(agents_md_message) = agents_md::build_agents_md_message(&self.project_dir) {
                msgs_with_system.push(Message {
                    role: "user".to_string(),
                    content: Some(agents_md_message),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }

            msgs_with_system.extend(self.build_workflow_context_messages(activation_source));

            let window_start = if self.turn_boundaries.len() > self.window_turns {
                let omitted = self.turn_boundaries.len() - self.window_turns;
                msgs_with_system.push(Message {
                    role: "system".to_string(),
                    content: Some(format!(
                        "Note: {} earlier turn(s) (seq 1–{}) are stored in history. Use history_recall or history_search without session_id for the current session, or pass session_id=\"*\" to search across all sessions in the current project.",
                        omitted, omitted
                    )),
                    tool_calls: None,
                    tool_call_id: None,
                });
                self.turn_boundaries[self.turn_boundaries.len() - self.window_turns]
            } else {
                0
            };
            msgs_with_system.extend_from_slice(&self.messages[window_start..]);

            self.emit(AgentEvent::LlmStart);
            let event_tx = self.event_tx.clone();
            let cancellation_for_stream = cancellation.clone();
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

            let (response, usage, rate_limit_report) = match response_result {
                Ok(v) => v,
                Err(err)
                    if cancellation.as_ref().is_some_and(|c| c.is_interrupted())
                        || err.to_string().contains("interrupted") =>
                {
                    self.interrupt_turn(turn_id, turn_seq).await?;
                    turn_end_reason = "interrupted".to_string();
                    break;
                }
                Err(err) => return Err(err),
            };

            if let Some(report) = rate_limit_report {
                if let Ok(text) = serde_json::to_string(&report) {
                    self.emit(AgentEvent::Stats(format!("[rate-limit] {}", text)));
                }
            }

            llm_rounds += 1;
            if let Some(u) = usage {
                if let Some(pt) = u.prompt_tokens {
                    tokens_in += pt;
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

                let detail = tool_call_detail(&tc.function.name, &tc.function.arguments);
                self.emit(AgentEvent::ToolStart { detail });
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
    use super::{center_trim, tool_call_detail, TOOL_DETAIL_CENTER_TRIM_MARKER};

    #[test]
    fn center_trim_keeps_short_strings_unchanged() {
        assert_eq!(center_trim("short", 10), "short");
    }

    #[test]
    fn center_trim_inserts_marker_and_keeps_ends() {
        let trimmed = center_trim("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(
            trimmed,
            format!("abcd{}vwxyz", TOOL_DETAIL_CENTER_TRIM_MARKER)
        );
    }

    #[test]
    fn center_trim_preserves_unicode_boundaries() {
        let trimmed = center_trim("こんにちは世界さようなら", 8);
        assert!(trimmed.contains(TOOL_DETAIL_CENTER_TRIM_MARKER));
        assert_eq!(trimmed.chars().count(), 8);
    }

    #[test]
    fn tool_call_detail_center_trims_long_paths() {
        let detail = tool_call_detail(
            "fs_read_file",
            r#"{"path":"/very/long/path/to/a/deeply/nested/file/with-important-name.rs"}"#,
        );
        assert!(detail.starts_with("read: /very/long/path/to/a/deeply"));
        assert!(detail.contains(TOOL_DETAIL_CENTER_TRIM_MARKER));
        assert!(detail.ends_with("file/with-important-name.rs"));
    }
}
