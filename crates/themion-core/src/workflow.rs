use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseResult {
    Pending,
    Passed,
    Failed,
    UserFeedbackRequired,
}

impl PhaseResult {
    pub fn as_str(self) -> &'static str {
        match self {
            PhaseResult::Pending => "pending",
            PhaseResult::Passed => "passed",
            PhaseResult::Failed => "failed",
            PhaseResult::UserFeedbackRequired => "user_feedback_required",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "passed" => PhaseResult::Passed,
            "failed" => PhaseResult::Failed,
            "user_feedback_required" => PhaseResult::UserFeedbackRequired,
            _ => PhaseResult::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhaseRetryState {
    pub current_phase_retries: u32,
    pub current_phase_retry_limit: u32,
    pub previous_phase_retries: u32,
    pub previous_phase_retry_limit: u32,
    pub entered_via: PhaseEntryKind,
}

impl Default for PhaseRetryState {
    fn default() -> Self {
        Self {
            current_phase_retries: 0,
            current_phase_retry_limit: MAX_CURRENT_PHASE_RETRIES,
            previous_phase_retries: 0,
            previous_phase_retry_limit: MAX_PREVIOUS_PHASE_RETRIES,
            entered_via: PhaseEntryKind::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseEntryKind {
    Normal,
    RetryCurrentPhase,
    RetryPreviousPhase,
}

impl PhaseEntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PhaseEntryKind::Normal => "normal",
            PhaseEntryKind::RetryCurrentPhase => "retry_current_phase",
            PhaseEntryKind::RetryPreviousPhase => "retry_previous_phase",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "retry_current_phase" => PhaseEntryKind::RetryCurrentPhase,
            "retry_previous_phase" => PhaseEntryKind::RetryPreviousPhase,
            _ => PhaseEntryKind::Normal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowState {
    pub workflow_name: String,
    pub phase_name: String,
    pub status: WorkflowStatus,
    pub phase_result: PhaseResult,
    pub agent_name: String,
    pub last_updated_turn_seq: Option<u32>,
    pub retry_state: PhaseRetryState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Running,
    WaitingUser,
    Completed,
    Failed,
    Interrupted,
}

impl WorkflowStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowStatus::Running => "running",
            WorkflowStatus::WaitingUser => "waiting_user",
            WorkflowStatus::Completed => "completed",
            WorkflowStatus::Failed => "failed",
            WorkflowStatus::Interrupted => "interrupted",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "waiting_user" => WorkflowStatus::WaitingUser,
            "completed" => WorkflowStatus::Completed,
            "failed" => WorkflowStatus::Failed,
            "interrupted" => WorkflowStatus::Interrupted,
            _ => WorkflowStatus::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTransitionKind {
    WorkflowStarted,
    PhaseStarted,
    WaitingUser,
    WorkflowCompleted,
    WorkflowFailed,
    WorkflowInterrupted,
    PhaseRetryCurrent,
    PhaseRetryPrevious,
    PhaseRetryExhausted,
}

impl WorkflowTransitionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowTransitionKind::WorkflowStarted => "workflow_started",
            WorkflowTransitionKind::PhaseStarted => "phase_started",
            WorkflowTransitionKind::WaitingUser => "waiting_user",
            WorkflowTransitionKind::WorkflowCompleted => "workflow_completed",
            WorkflowTransitionKind::WorkflowFailed => "workflow_failed",
            WorkflowTransitionKind::WorkflowInterrupted => "workflow_interrupted",
            WorkflowTransitionKind::PhaseRetryCurrent => "phase_retry_current",
            WorkflowTransitionKind::PhaseRetryPrevious => "phase_retry_previous",
            WorkflowTransitionKind::PhaseRetryExhausted => "phase_retry_exhausted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPromptMode {
    Default,
    Clarify,
    Execute,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowDefinition {
    pub name: &'static str,
    pub start_phase: &'static str,
    pub prompt_mode: WorkflowPromptMode,
}

pub const DEFAULT_WORKFLOW: &str = "NORMAL";
pub const DEFAULT_PHASE: &str = "IDLE";
pub const DEFAULT_AGENT: &str = "main";
pub const LITE_WORKFLOW: &str = "LITE";
pub const MAX_CURRENT_PHASE_RETRIES: u32 = 3;
pub const MAX_PREVIOUS_PHASE_RETRIES: u32 = 3;

pub fn workflow_definition(name: &str) -> Option<WorkflowDefinition> {
    match name {
        DEFAULT_WORKFLOW => Some(WorkflowDefinition {
            name: DEFAULT_WORKFLOW,
            start_phase: "EXECUTE",
            prompt_mode: WorkflowPromptMode::Default,
        }),
        LITE_WORKFLOW => Some(WorkflowDefinition {
            name: LITE_WORKFLOW,
            start_phase: "CLARIFY",
            prompt_mode: WorkflowPromptMode::Clarify,
        }),
        _ => None,
    }
}

pub fn normalize_workflow_name(name: &str) -> Option<&'static str> {
    if name.eq_ignore_ascii_case(DEFAULT_WORKFLOW) {
        Some(DEFAULT_WORKFLOW)
    } else if name.eq_ignore_ascii_case(LITE_WORKFLOW) {
        Some(LITE_WORKFLOW)
    } else {
        None
    }
}

pub fn start_phase_for_workflow(name: &str) -> Option<&'static str> {
    workflow_definition(name).map(|d| d.start_phase)
}

pub fn previous_phase(workflow: &str, phase: &str) -> Option<&'static str> {
    match (workflow, phase) {
        (LITE_WORKFLOW, "EXECUTE") => Some("CLARIFY"),
        (LITE_WORKFLOW, "VALIDATE") => Some("EXECUTE"),
        _ => None,
    }
}

pub fn allowed_transitions(workflow: &str, phase: &str) -> Vec<&'static str> {
    let mut transitions = match (workflow, phase) {
        (DEFAULT_WORKFLOW, "IDLE") => vec!["EXECUTE"],
        (DEFAULT_WORKFLOW, "EXECUTE") => vec!["IDLE"],
        (LITE_WORKFLOW, "CLARIFY") => vec!["EXECUTE"],
        (LITE_WORKFLOW, "EXECUTE") => vec!["VALIDATE"],
        (LITE_WORKFLOW, "VALIDATE") => vec![],
        _ => vec![],
    };

    if let Some(prev) = previous_phase(workflow, phase) {
        if !transitions.contains(&prev) {
            transitions.push(prev);
        }
    }

    transitions
}

pub fn can_transition(workflow: &str, from_phase: &str, to_phase: &str) -> bool {
    allowed_transitions(workflow, from_phase)
        .into_iter()
        .any(|phase| phase == to_phase)
}

pub fn can_retry_current_phase(workflow: &str, phase: &str) -> bool {
    matches!(
        (workflow, phase),
        (LITE_WORKFLOW, "CLARIFY") | (LITE_WORKFLOW, "EXECUTE") | (LITE_WORKFLOW, "VALIDATE")
    )
}

pub fn can_retry_previous_phase(workflow: &str, phase: &str) -> bool {
    previous_phase(workflow, phase).is_some()
}

pub fn activation_marker(input: &str) -> Option<&'static str> {
    for raw in input.split_whitespace() {
        let token = raw
            .trim_matches(|c: char| c.is_ascii_punctuation() && c != ':')
            .to_ascii_lowercase();
        if token == "workflow:lite" || token == "workflow: lite" {
            return Some(LITE_WORKFLOW);
        }
        if token == "workflow:normal" || token == "workflow: normal" {
            return Some(DEFAULT_WORKFLOW);
        }
    }

    let lower = input.to_ascii_lowercase();
    if lower.contains("workflow: lite") {
        Some(LITE_WORKFLOW)
    } else if lower.contains("workflow: normal") {
        Some(DEFAULT_WORKFLOW)
    } else {
        None
    }
}

pub fn strip_activation_markers(input: &str) -> String {
    input
        .replace("workflow:lite", "")
        .replace("workflow: lite", "")
        .replace("Workflow:lite", "")
        .replace("Workflow: lite", "")
        .replace("workflow:normal", "")
        .replace("workflow: normal", "")
        .replace("Workflow:normal", "")
        .replace("Workflow: normal", "")
        .trim()
        .to_string()
}

pub fn phase_instructions(workflow: &str, phase: &str) -> Vec<&'static str> {
    match (workflow, phase) {
        (LITE_WORKFLOW, "CLARIFY") => vec![
            "Produce a compact brief with objective, assumptions, and success criteria.",
            "Proceed without asking the user unless ambiguity is genuinely blocking.",
            "If blocked by ambiguity, ask a concise clarification question and stop.",
        ],
        (LITE_WORKFLOW, "EXECUTE") => vec![
            "Implement the smallest working slice that satisfies the clarify brief.",
            "Keep changes narrow and avoid unrelated refactors.",
            "When the slice is complete, advance to VALIDATE.",
        ],
        (LITE_WORKFLOW, "VALIDATE") => vec![
            "Check the success criteria from CLARIFY.",
            "Run a narrow smoke check and return pass or fail.",
            "Do not silently continue implementation work after a failed validation.",
        ],
        (DEFAULT_WORKFLOW, "EXECUTE") => {
            vec!["Solve the user's request directly using available tools as needed."]
        }
        _ => vec![],
    }
}

impl Default for WorkflowState {
    fn default() -> Self {
        Self {
            workflow_name: DEFAULT_WORKFLOW.to_string(),
            phase_name: DEFAULT_PHASE.to_string(),
            status: WorkflowStatus::Running,
            phase_result: PhaseResult::Pending,
            agent_name: DEFAULT_AGENT.to_string(),
            last_updated_turn_seq: None,
            retry_state: PhaseRetryState::default(),
        }
    }
}
