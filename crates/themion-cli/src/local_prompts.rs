use themion_core::db::{NoteColumn, NoteKind};

const NOTE_PREFIX: &str = "type=stylos_note";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum IncomingPromptSource {
    RemoteStylos,
    WatchdogBoardNote,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct IncomingPromptRequest {
    pub prompt: String,
    pub source: IncomingPromptSource,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub request_id: Option<String>,
    pub from: Option<String>,
    pub from_agent_id: Option<String>,
    pub to: Option<String>,
    pub to_agent_id: Option<String>,
}

pub fn build_board_note_prompt(
    note_id: &str,
    note_slug: &str,
    note_kind: NoteKind,
    origin_note_id: Option<&str>,
    sender: Option<&str>,
    sender_agent_id: Option<&str>,
    target: &str,
    local_agent_id: &str,
    column: NoteColumn,
    body: &str,
    source: IncomingPromptSource,
) -> String {
    let note_purpose = match note_kind {
        NoteKind::WorkRequest => match column {
            NoteColumn::Blocked => "This is a durable delegated work note that currently starts in blocked because its first useful action is to wait or reassess later. Treat it as deferred board work, not ready backlog. Reassess whether the waiting condition has changed. If it is still waiting, keep it in blocked and update result text with the current blocker when useful. If it becomes actionable, move it back to todo before resuming normal work. Never use Stylos talk in response to this note. Board workflow only.",
            _ => "This is a durable delegated work note. Prefer progressing or completing the requested work through the board workflow. Move the note from todo to in_progress as soon as you begin meaningful work when possible. If you finish the task, update the note result text with the concrete outcome and move it to done before ending the turn. If meaningful progress started and then must wait, move the note to blocked instead of leaving it in ready backlog. Never use Stylos talk in response to this note. Board workflow only.",
        },
        NoteKind::DoneMention => "This is an informational completion mention for prior delegated work. Incoming notes still enter the board in todo and must be actively handled; do not assume storage state means the note is already resolved. Treat this as a durable done notification, not as a fresh request to repeat the same task. Decide whether any concrete action remains based on the note context. If no further action is actually needed, move the note to done in this turn. If follow-up is still required, keep working it through the board workflow until the remaining action is complete. Do not create an automatic done echo in response. Do not send an acknowledgment, summary-only reply, or any other no-op follow-up unless the note clearly requires a concrete next action or correction.",
    };
    let instruction = match source {
        IncomingPromptSource::RemoteStylos => None,
        IncomingPromptSource::WatchdogBoardNote => {
            Some("I found that you have a pending note to handle. Below is that note.".to_string())
        }
    };
    match instruction {
        Some(instruction) => format!(
            "{NOTE_PREFIX} note_id={note_id} note_slug={note_slug} note_kind={} origin_note_id={} from={} from_agent_id={} to={target} to_agent_id={local_agent_id} column={}\n\n{}\n\n{}\n\nNote body:\n{}",
            note_kind.as_str(),
            origin_note_id.unwrap_or("-"),
            sender.unwrap_or("unknown"),
            sender_agent_id.unwrap_or("unknown"),
            column.as_str(),
            instruction,
            note_purpose,
            body
        ),
        None => format!(
            "{NOTE_PREFIX} note_id={note_id} note_slug={note_slug} note_kind={} origin_note_id={} from={} from_agent_id={} to={target} to_agent_id={local_agent_id} column={}\n\n{}\n\nNote body:\n{}",
            note_kind.as_str(),
            origin_note_id.unwrap_or("-"),
            sender.unwrap_or("unknown"),
            sender_agent_id.unwrap_or("unknown"),
            column.as_str(),
            note_purpose,
            body
        ),
    }
}
