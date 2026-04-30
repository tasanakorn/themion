use crate::client::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayForm {
    Full,
    PureMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimateMode {
    Tokenizer,
    RoughFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerResolutionSource {
    ExactModelMatch,
    TrustedFallbackMapping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEstimateMode {
    RawOnly,
    RawPlusEffective,
}

#[derive(Debug, Clone)]
pub struct ToolEstimateReport {
    pub raw_tokens: usize,
    pub effective_tokens: Option<usize>,
    pub mode: ToolEstimateMode,
    pub backend_scope: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSectionKind {
    SystemPrompt,
    CodingGuardrails,
    BoardGuidance,
    MemoryGuidance,
    CodexCliWebSearch,
    AgentsMd,
    WorkflowContext,
    ToolDefinitions,
    HistoryRecallHint,
    HistoryReplay,
}

#[derive(Debug, Clone)]
pub struct PromptSectionReport {
    pub kind: PromptSectionKind,
    pub label: String,
    pub messages: Vec<Message>,
    pub extra_text: Option<String>,
    pub chars: usize,
    pub tokens_estimate: usize,
    pub tool_estimate: Option<ToolEstimateReport>,
}

#[derive(Debug, Clone)]
pub struct HistoryTurnReport {
    pub turn_label: String,
    pub replay_form: ReplayForm,
    pub omitted: bool,
    pub messages: Vec<Message>,
    pub chars: usize,
    pub tokens_estimate: usize,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PromptContextReport {
    pub sections: Vec<PromptSectionReport>,
    pub history_turns: Vec<HistoryTurnReport>,
    pub total_turns: usize,
    pub replayed_turns: usize,
    pub omitted_turns: usize,
    pub cap_omitted_turns: usize,
    pub reduced_turns: usize,
    pub total_chars: usize,
    pub total_tokens_estimate: usize,
    pub t0_exceeds_normal_budget: bool,
    pub t0_exceeds_spike_budget: bool,
    pub estimate_mode: EstimateMode,
    pub tokenizer_name: Option<String>,
    pub tokenizer_resolution_source: Option<TokenizerResolutionSource>,
}

pub fn estimate_message_chars(msg: &Message) -> usize {
    let mut total = msg.role.len();
    if let Some(content) = &msg.content {
        total += content.len();
    }
    if let Some(tool_call_id) = &msg.tool_call_id {
        total += tool_call_id.len();
    }
    if let Some(tool_calls) = &msg.tool_calls {
        for tc in tool_calls {
            total += tc.id.len();
            total += tc.function.name.len();
            total += tc.function.arguments.len();
        }
    }
    total
}

pub fn estimate_messages_chars(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message_chars).sum()
}

pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
    estimate_messages_chars(messages).div_ceil(4)
}

pub fn estimate_text_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}
