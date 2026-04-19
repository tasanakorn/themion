#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowState {
    pub workflow_name: String,
    pub phase_name: String,
    pub status: WorkflowStatus,
    pub agent_name: String,
    pub last_updated_turn_seq: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        }
    }
}

pub const DEFAULT_WORKFLOW: &str = "NORMAL";
pub const DEFAULT_PHASE: &str = "IDLE";
pub const DEFAULT_AGENT: &str = "main";

impl Default for WorkflowState {
    fn default() -> Self {
        Self {
            workflow_name: DEFAULT_WORKFLOW.to_string(),
            phase_name: DEFAULT_PHASE.to_string(),
            status: WorkflowStatus::Running,
            agent_name: DEFAULT_AGENT.to_string(),
            last_updated_turn_seq: None,
        }
    }
}
