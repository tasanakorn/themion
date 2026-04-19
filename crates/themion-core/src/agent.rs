use crate::agents_md;
use crate::client::{ChatBackend, Message, ModelInfo};
use crate::db::DbHandle;
use crate::tools;
use crate::workflow::{
    activation_marker, allowed_transitions, can_transition, normalize_workflow_name,
    phase_instructions, previous_phase, start_phase_for_workflow, PhaseEntryKind,
    PhaseResult, PhaseRetryState, WorkflowState, WorkflowStatus, WorkflowTransitionKind, DEFAULT_AGENT,
    DEFAULT_PHASE, DEFAULT_WORKFLOW, LITE_WORKFLOW,
};
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
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
    AssistantChunk(String),
    AssistantText(String),
    Stats(String),
    WorkflowStateChanged(WorkflowState),
    TurnDone(TurnStats),
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max].iter().collect::<String>() + "…"
    }
}

fn tool_call_detail(name: &str, args_json: &str) -> String {
    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
    let t = |key: &str| truncate(args[key].as_str().unwrap_or("?"), 60);
    match name {
        "bash" => format!("bash: {}", t("command")),
        "read_file" => format!("read: {}", t("path")),
        "write_file" => format!("write: {}", t("path")),
        "list_directory" => format!("ls: {}", t("path")),
        "recall_history" => format!(
            "recall_history: session={}",
            truncate(args["session_id"].as_str().unwrap_or("current"), 60)
        ),
        "search_history" => format!("search_history: {}", t("query")),
        "get_workflow_state" => "workflow: inspect".to_string(),
        "set_workflow" => format!("workflow: set {}", t("workflow")),
        "set_workflow_phase" => format!("workflow: phase {}", t("phase")),
        "complete_workflow" => format!("workflow: complete {}", t("outcome")),
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
        }
    }

    pub fn set_event_tx(&mut self, tx: mpsc::UnboundedSender<AgentEvent>) {
        self.event_tx = Some(tx);
    }

    pub fn clear_context(&mut self) {
        self.messages.clear();
        self.turn_boundaries.clear();
    }

    pub async fn refresh_model_info(&mut self) {
        self.model_info = self.client.fetch_model_info(&self.model).await.ok().flatten();
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

    async fn persist_workflow_state(&self) -> Result<()> {
        let db = self.db.clone();
        let sid = self.session_id;
        let state = self.workflow_state.clone();
        tokio::task::spawn_blocking(move || db.update_session_workflow_state(sid, &state)).await??;
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
        let from_phase = Some(self.workflow_state.phase_name.clone());
        self.workflow_state = WorkflowState::default();
        self.workflow_state.last_updated_turn_seq = turn_seq;
        self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
        let from_phase = Some(self.workflow_state.phase_name.clone());
        self.workflow_state.workflow_name = workflow_name.to_string();
        self.workflow_state.phase_name = start_phase.to_string();
        self.workflow_state.status = WorkflowStatus::Running;
        self.workflow_state.phase_result = PhaseResult::Pending;
        self.workflow_state.agent_name = DEFAULT_AGENT.to_string();
        self.workflow_state.last_updated_turn_seq = turn_seq;
        self.reset_retry_state();
        self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
        self.workflow_state.phase_name = next_phase.to_string();
        self.workflow_state.status = WorkflowStatus::Running;
        self.workflow_state.phase_result = PhaseResult::Pending;
        self.workflow_state.last_updated_turn_seq = turn_seq;
        self.reset_retry_state();
        self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
            self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
                self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
        self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
                "Workflow tools: use get_workflow_state to inspect state, set_workflow to activate a built-in workflow (which always resets phase to that workflow's start phase), set_workflow_phase for valid transitions within the current workflow, and complete_workflow to mark completed or failed."
                    .to_string(),
            ),
            tool_calls: None,
            tool_call_id: None,
        });
        out
    }

    fn apply_workflow_tool_result(&mut self, tool_name: &str, result: &str, turn_seq: u32) -> Result<bool> {
        if result.starts_with("Error:") {
            return Ok(false);
        }
        let parsed: Value = match serde_json::from_str(result) {
            Ok(v) => v,
            Err(_) => return Ok(false),
        };
        match tool_name {
            "set_workflow" => {
                let workflow = parsed["workflow"].as_str().unwrap_or(DEFAULT_WORKFLOW);
                let phase = parsed["phase"].as_str().unwrap_or(DEFAULT_PHASE);
                self.workflow_state.workflow_name = workflow.to_string();
                self.workflow_state.phase_name = phase.to_string();
                self.workflow_state.status = WorkflowStatus::Running;
                self.workflow_state.phase_result = PhaseResult::Pending;
                self.workflow_state.agent_name = parsed["agent"].as_str().unwrap_or(DEFAULT_AGENT).to_string();
                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                self.reset_retry_state();
                Ok(true)
            }
            "set_workflow_phase" => {
                let phase = parsed["phase"].as_str().ok_or_else(|| anyhow::anyhow!("missing phase"))?;
                self.workflow_state.phase_name = phase.to_string();
                self.workflow_state.status = WorkflowStatus::Running;
                self.workflow_state.phase_result = PhaseResult::Pending;
                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                self.reset_retry_state();
                Ok(true)
            }
            "set_phase_result" => {
                self.workflow_state.phase_result = match parsed["phase_result"].as_str().unwrap_or("pending") {
                    "passed" => PhaseResult::Passed,
                    "failed" => PhaseResult::Failed,
                    _ => PhaseResult::Pending,
                };
                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                Ok(true)
            }
            "complete_workflow" => {
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

    pub async fn run_loop(&mut self, user_input: &str) -> Result<(String, TurnStats)> {
        if self.model_info.is_none() {
            self.refresh_model_info().await;
        }

        self.turn_seq_counter += 1;
        let turn_seq = self.turn_seq_counter;
        self.turn_boundaries.push(self.messages.len());

        let requested_workflow = activation_marker(user_input);
        let cleaned_user_input = crate::workflow::strip_activation_markers(user_input);
        let effective_user_input = if cleaned_user_input.is_empty() {
            user_input.trim().to_string()
        } else {
            cleaned_user_input
        };

        if let Some(workflow) = requested_workflow {
            self.set_workflow(workflow, None, Some(turn_seq), Some("user_input"))
                .await?;
        } else if self.workflow_state.workflow_name == DEFAULT_WORKFLOW
            && self.workflow_state.phase_name == DEFAULT_PHASE
        {
            self.workflow_state.phase_name = "EXECUTE".to_string();
            self.workflow_state.status = WorkflowStatus::Running;
            self.workflow_state.phase_result = PhaseResult::Pending;
            self.workflow_state.agent_name = DEFAULT_AGENT.to_string();
            self.workflow_state.last_updated_turn_seq = Some(turn_seq);
            self.reset_retry_state();
            self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
            tokio::task::spawn_blocking(move || db.append_message(turn_id, sid, seq, &msg, &workflow))
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
        let mut turn_end_reason = "workflow_completed".to_string();
        let activation_source = if requested_workflow.is_some() {
            "user_input"
        } else {
            "session_state"
        };

        for _ in 0..10 {
            let mut msgs_with_system = vec![Message {
                role: "system".to_string(),
                content: Some(self.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            }];

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
                        "Note: {} earlier turn(s) (seq 1–{}) are stored in history. Use recall_history to load a range or search_history to find a keyword.",
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
            let (response, usage, rate_limit_report) = self
                .client
                .chat_completion_stream(
                    &self.model,
                    &msgs_with_system,
                    &tool_defs,
                    Box::new(move |chunk| {
                        if let Some(ref tx) = event_tx {
                            let _ = tx.send(AgentEvent::AssistantChunk(chunk));
                        }
                    }),
                )
                .await?;

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
                tokio::task::spawn_blocking(move || db.append_message(turn_id, sid, seq, &msg, &workflow))
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
                                let content = response.content.as_deref().unwrap_or_default().to_ascii_lowercase();
                                if content.contains("?") || content.contains("need clarification") || content.contains("unclear") {
                                    if self.retry_or_fail_phase(turn_id, turn_seq, "CLARIFY", "engine_rule").await? {
                                        if self.workflow_state.phase_name == "CLARIFY" && self.workflow_state.retry_state.entered_via == PhaseEntryKind::RetryCurrentPhase {
                                            continue;
                                        }
                                    }
                                    if self.workflow_state.status == WorkflowStatus::Failed {
                                        turn_end_reason = "workflow_failed".to_string();
                                        break;
                                    }
                                    self.workflow_state.status = WorkflowStatus::WaitingUser;
                                    self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                                    self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
                                self.workflow_state.phase_result = PhaseResult::Passed;
                                self.persist_workflow_state().await?;
                                self.set_phase("EXECUTE", Some(turn_id), Some(turn_seq), Some("engine_rule"))
                                    .await?;
                                continue;
                            }
                            "EXECUTE" => {
                                let content = response.content.as_deref().unwrap_or_default().to_ascii_lowercase();
                                if content.contains("cannot continue") || content.contains("blocked") || content.contains("failed") {
                                    if self.retry_or_fail_phase(turn_id, turn_seq, "EXECUTE", "model_completion").await? {
                                        continue;
                                    }
                                    turn_end_reason = "workflow_failed".to_string();
                                    break;
                                }
                                self.workflow_state.phase_result = PhaseResult::Passed;
                                self.persist_workflow_state().await?;
                                self.set_phase("VALIDATE", Some(turn_id), Some(turn_seq), Some("engine_rule"))
                                    .await?;
                                continue;
                            }
                            "VALIDATE" => {
                                let content = response.content.as_deref().unwrap_or_default().to_ascii_lowercase();
                                if content.contains("fail") {
                                    self.workflow_state.phase_result = PhaseResult::Failed;
                                    self.persist_workflow_state().await?;
                                    if self.retry_or_fail_phase(turn_id, turn_seq, "VALIDATE", "model_completion").await? {
                                        continue;
                                    }
                                    turn_end_reason = "workflow_failed".to_string();
                                    break;
                                }
                                self.workflow_state.phase_result = PhaseResult::Passed;
                                self.workflow_state.status = WorkflowStatus::Completed;
                                self.workflow_state.last_updated_turn_seq = Some(turn_seq);
                                self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
                let detail = tool_call_detail(&tc.function.name, &tc.function.arguments);
                self.emit(AgentEvent::ToolStart { detail });
                let tool_ctx = crate::tools::ToolCtx {
                    db: self.db.clone(),
                    session_id: self.session_id,
                    project_dir: self.project_dir.clone(),
                    workflow_state: Some(self.workflow_state.clone()),
                    turn_seq: Some(turn_seq),
                };
                let result =
                    tools::call_tool(&tc.function.name, &tc.function.arguments, &tool_ctx).await;
                self.emit(AgentEvent::ToolEnd);
                tool_calls += 1;

                let old_phase = self.workflow_state.phase_name.clone();
                if self.apply_workflow_tool_result(&tc.function.name, &result, turn_seq)? {
                    let parsed: Value = serde_json::from_str(&result).unwrap_or_default();
                    if tc.function.name == "set_workflow" {
                        self.persist_workflow_state().await?;
                        self.record_transition(
                            Some(turn_id),
                            Some(turn_seq),
                            Some(old_phase),
                            parsed["phase"].as_str().unwrap_or(DEFAULT_PHASE).to_string(),
                            WorkflowTransitionKind::WorkflowStarted,
                            Some("tool_result"),
                        )
                        .await?;
                    } else if tc.function.name == "set_workflow_phase" {
                        self.persist_workflow_state().await?;
                        self.record_transition(
                            Some(turn_id),
                            Some(turn_seq),
                            Some(old_phase),
                            parsed["phase"].as_str().unwrap_or(DEFAULT_PHASE).to_string(),
                            WorkflowTransitionKind::PhaseStarted,
                            Some("tool_result"),
                        )
                        .await?;
                    } else if tc.function.name == "set_phase_result" {
                        self.persist_workflow_state().await?;
                    } else if tc.function.name == "complete_workflow" {
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
                    self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
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
                    tokio::task::spawn_blocking(move || db.append_message(turn_id, sid, seq, &msg, &workflow))
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
            }

            if self.workflow_state.status == WorkflowStatus::Completed {
                turn_end_reason = "workflow_completed".to_string();
                break;
            }
            if self.workflow_state.status == WorkflowStatus::Failed {
                turn_end_reason = "workflow_failed".to_string();
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
        } else if self.workflow_state.workflow_name == DEFAULT_WORKFLOW {
            let from_phase = self.workflow_state.phase_name.clone();
            self.workflow_state.phase_name = DEFAULT_PHASE.to_string();
            self.workflow_state.status = WorkflowStatus::Completed;
            self.workflow_state.last_updated_turn_seq = Some(turn_seq);
            self.emit(AgentEvent::WorkflowStateChanged(self.workflow_state.clone()));
            self.persist_workflow_state().await?;
            self.record_transition(
                Some(turn_id),
                Some(turn_seq),
                Some(from_phase),
                self.workflow_state.phase_name.clone(),
                WorkflowTransitionKind::WorkflowCompleted,
                Some("model_completion"),
            )
            .await?;
            self.reset_to_default_workflow(
                Some(turn_id),
                Some(turn_seq),
                Some("workflow_completion"),
            )
            .await?;
        }

        let workflow_continues_after_turn = matches!(self.workflow_state.status, WorkflowStatus::WaitingUser);
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
                db.finalize_turn(turn_id, &s, &workflow, workflow_continues_after_turn, &turn_end_reason)
            })
            .await??;
        }
        self.emit(AgentEvent::TurnDone(stats.clone()));
        Ok((final_response, stats))
    }
}
