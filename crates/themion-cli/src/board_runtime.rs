#![cfg(feature = "stylos")]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use themion_core::db::{BoardNote, DbHandle, NoteColumn, NoteKind};

use crate::app_state::{create_done_mention_locally, DoneMentionRequest};
use crate::stylos::{build_board_note_prompt, IncomingPromptRequest, IncomingPromptSource};

pub const WATCHDOG_IDLE_DELAY_MS_DEFAULT: u64 = 2_000;

#[derive(Default)]
pub struct LocalBoardClaimRegistry {
    claimed_note_ids: Mutex<HashSet<String>>,
}

impl LocalBoardClaimRegistry {
    pub fn try_claim(&self, note_id: &str) -> bool {
        let mut claimed = self.claimed_note_ids.lock().expect("board claims lock");
        claimed.insert(note_id.to_string())
    }

    pub fn release(&self, note_id: &str) {
        let mut claimed = self.claimed_note_ids.lock().expect("board claims lock");
        claimed.remove(note_id);
    }
}

#[derive(Clone, Debug)]
pub enum BoardTurnFollowUp {
    None,
    ContinueCurrentNote {
        request: IncomingPromptRequest,
        prompt: String,
    },
    EmitDoneMention {
        log_line: String,
    },
    EmitDoneMentionError {
        status_line: String,
    },
}

fn build_injection_request(note: &BoardNote, trigger: IncomingPromptSource) -> IncomingPromptRequest {
    let prompt = build_board_note_prompt(
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
        trigger,
    );
    let log_line = match trigger {
        IncomingPromptSource::WatchdogBoardNote => format!(
            "Watchdog claimed board note note_slug={} to={} to_agent_id={} column={} after_idle_ms={}",
            note.note_slug,
            note.to_instance,
            note.to_agent_id,
            note.column.as_str(),
            WATCHDOG_IDLE_DELAY_MS_DEFAULT,
        ),
        IncomingPromptSource::RemoteStylos => format!(
            "Board note claimed note_slug={} to={} to_agent_id={} column={}",
            note.note_slug,
            note.to_instance,
            note.to_agent_id,
            note.column.as_str()
        ),
    };
    let _ = log_line;
    IncomingPromptRequest {
        prompt,
        source: trigger,
        agent_id: Some(note.to_agent_id.clone()),
        task_id: None,
        request_id: None,
        from: note.from_instance.clone(),
        from_agent_id: note.from_agent_id.clone(),
        to: Some(note.to_instance.clone()),
        to_agent_id: Some(note.to_agent_id.clone()),
    }
}

pub fn resolve_pending_board_note_injection(
    db: &Arc<DbHandle>,
    local_claims: &Arc<LocalBoardClaimRegistry>,
    local_instance: &str,
    target_agent_id: &str,
    trigger: IncomingPromptSource,
) -> Option<IncomingPromptRequest> {
    let Ok(Some(note)) = db.next_board_note_for_injection(local_instance, target_agent_id) else {
        return None;
    };
    if !local_claims.try_claim(&note.note_id) {
        return None;
    }
    Some(build_injection_request(&note, trigger))
}

pub fn release_board_note_claim(local_claims: &Arc<LocalBoardClaimRegistry>, note_id: &str) {
    local_claims.release(note_id);
}

pub fn board_note_id_from_prompt(prompt: &str) -> Option<&str> {
    if !prompt.starts_with("type=stylos_note ") {
        return None;
    }
    prompt
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .find_map(|part| part.strip_prefix("note_id="))
}

pub fn resolve_completed_note_follow_up(
    db: &Arc<DbHandle>,
    remote: &IncomingPromptRequest,
) -> BoardTurnFollowUp {
    if !remote.prompt.starts_with("type=stylos_note ") {
        return BoardTurnFollowUp::None;
    }
    let header = remote.prompt.lines().next().unwrap_or_default();
    let note_id = header
        .split_whitespace()
        .find_map(|part| part.strip_prefix("note_id="));
    let Some(note_id) = note_id else {
        return BoardTurnFollowUp::None;
    };
    let Ok(Some(note)) = db.get_board_note(note_id) else {
        return BoardTurnFollowUp::None;
    };
    if note.column != NoteColumn::Done {
        let prompt = format!(
            "This turn ended but note {} is still in {}. You still have a pending board task. Continue handling this note now. Decide from the note context whether any real action remains. If no further action is needed, move the note to done in this turn. Otherwise keep progressing it through the board workflow and do not end the turn while it is still pending.",
            note.note_slug,
            note.column.as_str(),
        );
        return BoardTurnFollowUp::ContinueCurrentNote {
            request: remote.clone(),
            prompt,
        };
    }
    if note.note_kind != NoteKind::WorkRequest {
        return BoardTurnFollowUp::None;
    }
    if note.completion_notified_at_ms.is_some() {
        return BoardTurnFollowUp::None;
    }
    let (Some(to_instance), Some(to_agent_id)) =
        (note.from_instance.clone(), note.from_agent_id.clone())
    else {
        return BoardTurnFollowUp::None;
    };
    let result_summary = note
        .result_text
        .clone()
        .unwrap_or_else(|| "completed with no explicit stored result".to_string());
    let request = DoneMentionRequest {
        note_id: note.note_id.clone(),
        note_slug: note.note_slug.clone(),
        from_instance: to_instance.clone(),
        from_agent_id: to_agent_id.clone(),
        completed_by_instance: note.to_instance.clone(),
        completed_by_agent_id: note.to_agent_id.clone(),
        result_summary,
    };

    match create_done_mention_locally(db, &request) {
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
            let _ = db.mark_board_note_completion_notified(&note.note_id);
            BoardTurnFollowUp::EmitDoneMention {
                log_line: format!(
                    "Board done mention note_slug={} origin_note_slug={} to={} to_agent_id={}",
                    created_note_slug, note.note_slug, to_instance, to_agent_id,
                ),
            }
        }
        Err(err) => BoardTurnFollowUp::EmitDoneMentionError {
            status_line: format!(
                "done mention create failed for note_id={}: {}",
                note.note_id, err
            ),
        },
    }
}
