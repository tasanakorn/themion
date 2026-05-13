use crate::db::{
    CreateNoteArgs, DbHandle, NoteColumn, NoteKind, RecallArgs, RecallDirection, SessionScope,
};
use crate::memory::{
    metadata_to_string, parse_hashtags_value, parse_nullable_string, CreateNodeArgs, HashtagMatch,
    LinkNodesArgs, OpenGraphArgs, UnifiedSearchMode, UpdateNodeArgs, GLOBAL_PROJECT_DIR,
};
use crate::workflow::{
    allowed_transitions, can_retry_current_phase, can_retry_previous_phase, can_transition,
    normalize_workflow_name, phase_instructions, previous_phase, start_phase_for_workflow,
    PhaseResult, WorkflowState, WorkflowStatus, DEFAULT_AGENT,
};
use anyhow::{Context, Result};

use base64::Engine;
use chrono::Utc;
#[cfg(unix)]
use libc::{getpwuid_r, getuid, passwd, sysconf, ERANGE, _SC_GETPW_R_SIZE_MAX};
use mpatch::{
    detect_patch, parse_auto, try_apply_patch_to_content, ApplyOptions, Patch as MpatchPatch,
    PatchFormat, StrictApplyError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::ffi::CStr;
use std::fs;
use std::future::Future;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::ptr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

fn parse_note_column(value: &str) -> Option<NoteColumn> {
    NoteColumn::from_str(value)
}

fn board_note_to_json(note: &crate::db::BoardNote) -> Value {
    json!({
        "note_id": note.note_id,
        "note_slug": note.note_slug,
        "note_kind": note.note_kind.as_str(),
        "origin_note_id": note.origin_note_id,
        "completion_notified_at_ms": note.completion_notified_at_ms,
        "from_instance": note.from_instance,
        "from_agent_id": note.from_agent_id,
        "to_instance": note.to_instance,
        "to_agent_id": note.to_agent_id,
        "body": note.body,
        "column": note.column.as_str(),
        "result_text": note.result_text,
        "blocked_until_ms": note.blocked_until_ms,
        "injection_state": match note.injection_state { crate::db::NoteInjectionState::Pending => "pending", crate::db::NoteInjectionState::Injected => "injected" },
        "created_at_ms": note.created_at_ms,
        "updated_at_ms": note.updated_at_ms,
        "injected_at_ms": note.injected_at_ms,
    })
}

fn board_note_ack(note: &crate::db::BoardNote, operation: &str, changed: Value) -> Value {
    json!({
        "ok": true,
        "entity": "board_note",
        "operation": operation,
        "note_id": note.note_id,
        "note_slug": note.note_slug,
        "changed": changed,
    })
}

fn board_note_not_found(note_id: &str, operation: &str) -> Value {
    json!({
        "ok": false,
        "entity": "board_note",
        "operation": operation,
        "found": false,
        "note_id": note_id,
    })
}

fn memory_node_ack(node: &crate::memory::MemoryNode, operation: &str) -> Value {
    json!({
        "ok": true,
        "entity": "memory_node",
        "operation": operation,
        "node_id": node.node_id,
        "project_dir": node.project_dir,
        "node_type": node.node_type,
        "title": node.title,
        "created_at_ms": node.created_at_ms,
        "updated_at_ms": node.updated_at_ms,
    })
}

fn memory_node_not_found(node_id: &str, operation: &str) -> Value {
    json!({
        "ok": false,
        "entity": "memory_node",
        "operation": operation,
        "found": false,
        "node_id": node_id,
    })
}

fn memory_edge_ack(edge: &crate::memory::MemoryEdge, operation: &str) -> Value {
    json!({
        "ok": true,
        "entity": "memory_edge",
        "operation": operation,
        "edge_id": edge.edge_id,
        "from_node_id": edge.from_node_id,
        "to_node_id": edge.to_node_id,
        "relation_type": edge.relation_type,
    })
}

fn write_file_ack(path: &str, mode: &str, written_bytes: usize) -> Value {
    json!({
        "ok": true,
        "entity": "file",
        "operation": "write",
        "path": path,
        "mode": mode,
        "written_bytes": written_bytes,
    })
}

fn patch_file_ack(
    ok: bool,
    changed_paths: &[String],
    rejected_paths: &[String],
    message: &str,
) -> Value {
    json!({
        "ok": ok,
        "entity": "file_patch",
        "operation": "apply",
        "changed_paths": changed_paths,
        "rejected_paths": rejected_paths,
        "message": message,
    })
}

fn workflow_state_to_json(state: &WorkflowState) -> Value {
    json!({
        "workflow": state.workflow_name,
        "phase": state.phase_name,
        "status": state.status,
        "phase_result": state.phase_result,
        "agent": state.agent_name,
        "last_updated_turn_seq": state.last_updated_turn_seq,
        "retry_state": state.retry_state,
        "allowed_next_phases": allowed_transitions(&state.workflow_name, &state.phase_name),
        "allowed_retry_current_phase": can_retry_current_phase(&state.workflow_name, &state.phase_name),
        "allowed_retry_previous_phase": can_retry_previous_phase(&state.workflow_name, &state.phase_name),
        "previous_phase": previous_phase(&state.workflow_name, &state.phase_name),
        "phase_instructions": phase_instructions(&state.workflow_name, &state.phase_name),
    })
}

fn parse_workflow_set_phase_result(value: &str) -> Result<PhaseResult> {
    match value {
        "passed" => Ok(PhaseResult::Passed),
        "failed" => Ok(PhaseResult::Failed),
        "user_feedback_required" => Ok(PhaseResult::UserFeedbackRequired),
        other => anyhow::bail!("invalid phase_result: {other}"),
    }
}

fn parse_workflow_set_status(value: &str) -> Result<WorkflowStatus> {
    let status = WorkflowStatus::from_str(value);
    match status {
        WorkflowStatus::Completed | WorkflowStatus::Failed => Ok(status),
        _ => anyhow::bail!("invalid workflow_status: {value}"),
    }
}

fn build_workflow_set_state(
    args: &Value,
    current: &WorkflowState,
    turn_seq: Option<u32>,
) -> Result<WorkflowState> {
    let workflow = args.get("workflow").and_then(Value::as_str);
    let phase_result_raw = args.get("phase_result").and_then(Value::as_str);
    let phase = args.get("phase").and_then(Value::as_str);
    let workflow_status_raw = args.get("workflow_status").and_then(Value::as_str);

    if workflow.is_none()
        && phase_result_raw.is_none()
        && phase.is_none()
        && workflow_status_raw.is_none()
    {
        anyhow::bail!("workflow_set requires at least one field");
    }

    if let Some(workflow_name) = workflow {
        if phase_result_raw.is_some() || phase.is_some() || workflow_status_raw.is_some() {
            anyhow::bail!(
                "workflow cannot be combined with phase_result, phase, or workflow_status"
            );
        }
        let workflow = normalize_workflow_name(workflow_name)
            .ok_or_else(|| anyhow::anyhow!("unknown workflow: {workflow_name}"))?;
        let start_phase = start_phase_for_workflow(workflow)
            .ok_or_else(|| anyhow::anyhow!("workflow missing start phase: {workflow}"))?;
        return Ok(WorkflowState {
            workflow_name: workflow.to_string(),
            phase_name: start_phase.to_string(),
            status: WorkflowStatus::Running,
            phase_result: PhaseResult::Pending,
            agent_name: DEFAULT_AGENT.to_string(),
            last_updated_turn_seq: turn_seq,
            retry_state: WorkflowState::default().retry_state,
        });
    }

    let phase_result = phase_result_raw
        .map(parse_workflow_set_phase_result)
        .transpose()?;
    let workflow_status = workflow_status_raw
        .map(parse_workflow_set_status)
        .transpose()?;

    if phase.is_some() && workflow_status.is_some() {
        anyhow::bail!("phase cannot be combined with workflow_status");
    }

    match (phase_result, phase, workflow_status) {
        (Some(PhaseResult::Passed), Some(next_phase), None) | (None, Some(next_phase), None) => {
            if !can_transition(&current.workflow_name, &current.phase_name, next_phase) {
                anyhow::bail!(
                    "cannot transition workflow {} from {} to {}",
                    current.workflow_name,
                    current.phase_name,
                    next_phase
                );
            }
            let mut next = current.clone();
            next.phase_name = next_phase.to_string();
            next.status = WorkflowStatus::Running;
            next.phase_result = PhaseResult::Pending;
            next.last_updated_turn_seq = turn_seq;
            next.retry_state = WorkflowState::default().retry_state;
            Ok(next)
        }
        (Some(PhaseResult::Failed), Some(_), None) => {
            anyhow::bail!("phase_result=failed cannot be combined with phase")
        }
        (Some(PhaseResult::UserFeedbackRequired), Some(_), None) => {
            anyhow::bail!("phase_result=user_feedback_required cannot be combined with phase")
        }
        (Some(PhaseResult::Pending), Some(_), None) => {
            anyhow::bail!("invalid phase_result")
        }
        (Some(PhaseResult::Passed), None, Some(WorkflowStatus::Completed)) => {
            let mut next = current.clone();
            next.status = WorkflowStatus::Completed;
            next.phase_result = PhaseResult::Passed;
            next.last_updated_turn_seq = turn_seq;
            Ok(next)
        }
        (Some(PhaseResult::Failed), None, Some(WorkflowStatus::Failed)) => {
            let mut next = current.clone();
            next.status = WorkflowStatus::Failed;
            next.phase_result = PhaseResult::Failed;
            next.last_updated_turn_seq = turn_seq;
            Ok(next)
        }
        (Some(PhaseResult::Failed), None, Some(WorkflowStatus::Completed)) => {
            anyhow::bail!("phase_result=failed cannot be combined with workflow_status=completed")
        }
        (Some(PhaseResult::UserFeedbackRequired), None, Some(_)) => {
            anyhow::bail!(
                "phase_result=user_feedback_required cannot be combined with workflow_status"
            )
        }
        (Some(PhaseResult::Passed), None, Some(WorkflowStatus::Failed)) => {
            anyhow::bail!("phase_result=passed cannot be combined with workflow_status=failed")
        }
        (Some(PhaseResult::Pending), None, Some(_)) => {
            anyhow::bail!("invalid phase_result")
        }
        (None, None, Some(WorkflowStatus::Completed)) => {
            if current.phase_result != PhaseResult::Passed {
                anyhow::bail!(
                    "workflow can only complete successfully when current phase_result=passed"
                );
            }
            let mut next = current.clone();
            next.status = WorkflowStatus::Completed;
            next.last_updated_turn_seq = turn_seq;
            Ok(next)
        }
        (None, None, Some(WorkflowStatus::Failed)) => {
            let mut next = current.clone();
            next.status = WorkflowStatus::Failed;
            next.last_updated_turn_seq = turn_seq;
            Ok(next)
        }
        (Some(result), None, None) => {
            let mut next = current.clone();
            next.phase_result = result;
            if result == PhaseResult::UserFeedbackRequired {
                next.status = WorkflowStatus::WaitingUser;
            }
            next.last_updated_turn_seq = turn_seq;
            Ok(next)
        }
        (None, Some(_), Some(_)) => unreachable!(),
        (Some(_), Some(_), Some(_)) => unreachable!(),
        (None, None, Some(_)) => anyhow::bail!("invalid workflow_status"),
        (None, None, None) => unreachable!(),
        _ => anyhow::bail!("invalid workflow_set combination"),
    }
}

const MAX_PATCH_BYTES: usize = 256 * 1024;
const MAX_PATCH_TARGETS: usize = 32;
const MAX_PATCH_HUNKS: usize = 256;
const MAX_PATCH_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
struct PreparedPatchTarget {
    path: String,
    absolute_path: PathBuf,
    original_content: String,
    patched_content: String,
}

fn uses_crlf_line_endings(content: &str, normalized_path: &str) -> FsPatchResult<bool> {
    let has_crlf = content.contains("\r\n");
    let without_crlf = content.replace("\r\n", "");
    if without_crlf.contains('\r') || (has_crlf && without_crlf.contains('\n')) {
        return Err(fs_patch_failure(
            format!(
                "mixed or unsupported line endings in target file: {}",
                normalized_path
            ),
            vec![normalized_path.to_string()],
        ));
    }
    Ok(has_crlf)
}

#[derive(Debug)]
struct FsPatchFailure {
    message: String,
    rejected_paths: Vec<String>,
}

type FsPatchResult<T> = std::result::Result<T, FsPatchFailure>;

fn fs_patch_failure(message: impl Into<String>, rejected_paths: Vec<String>) -> FsPatchFailure {
    FsPatchFailure {
        message: message.into(),
        rejected_paths,
    }
}

fn strip_single_markdown_diff_fence(patch_text: &str) -> Option<String> {
    let trimmed = patch_text.trim();
    if !trimmed.starts_with("```") {
        return None;
    }

    let mut lines = trimmed.lines();
    let first_line = lines.next()?;
    let first_trimmed = first_line.trim();
    let fence_len = first_trimmed.chars().take_while(|c| *c == '`').count();
    if fence_len < 3 {
        return None;
    }

    let info = first_trimmed[fence_len..].trim();
    if !info.is_empty() && info != "diff" && info != "patch" {
        return None;
    }

    let mut collected = Vec::new();
    let mut closed = false;
    for line in lines {
        let trimmed_line = line.trim();
        if trimmed_line.chars().take_while(|c| *c == '`').count() >= fence_len
            && trimmed_line.chars().all(|c| c == '`')
        {
            closed = true;
            break;
        }
        collected.push(line);
    }

    closed.then(|| collected.join("\n"))
}

fn normalize_patch_text(patch_text: &str) -> FsPatchResult<String> {
    if patch_text.len() > MAX_PATCH_BYTES {
        return Err(fs_patch_failure(
            format!("patch exceeds maximum {} bytes", MAX_PATCH_BYTES),
            vec![],
        ));
    }

    let trimmed = patch_text.trim();
    if trimmed.is_empty() {
        return Err(fs_patch_failure("patch must not be empty", vec![]));
    }

    let normalized = strip_single_markdown_diff_fence(trimmed)
        .unwrap_or_else(|| trimmed.to_string())
        .trim()
        .to_string();
    if normalized.is_empty() {
        return Err(fs_patch_failure("patch must not be empty", vec![]));
    }
    if matches!(detect_patch(&normalized), PatchFormat::Conflict) {
        return Err(fs_patch_failure(
            "conflict-marker patches are not supported",
            vec![],
        ));
    }
    Ok(normalized)
}

fn normalize_patch_target_str(raw: &str) -> FsPatchResult<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(fs_patch_failure(
            "patch target path must not be empty",
            vec![],
        ));
    }

    let stripped = raw
        .strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw);
    if stripped.is_empty() {
        return Err(fs_patch_failure(
            "patch target path must not be empty",
            vec![],
        ));
    }

    let candidate = Path::new(stripped);
    if candidate.is_absolute() {
        return Err(fs_patch_failure(
            "absolute patch target paths are not allowed",
            vec![],
        ));
    }

    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                return Err(fs_patch_failure(
                    "patch target path must not contain ..",
                    vec![],
                ))
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(fs_patch_failure(
                    "absolute patch target paths are not allowed",
                    vec![],
                ))
            }
        }
    }

    let normalized_str = normalized
        .to_str()
        .ok_or_else(|| fs_patch_failure("patch target path must be valid UTF-8", vec![]))?
        .to_string();
    if normalized_str.is_empty() {
        return Err(fs_patch_failure(
            "patch target path must not be empty",
            vec![],
        ));
    }
    Ok(normalized_str)
}

fn normalize_patch_target_path(path: &Path) -> FsPatchResult<String> {
    let raw = path
        .to_str()
        .ok_or_else(|| fs_patch_failure("patch target path must be valid UTF-8", vec![]))?;
    normalize_patch_target_str(raw)
}

fn validate_patch_headers(patch_text: &str) -> FsPatchResult<()> {
    let mut pending_old: Option<String> = None;
    let mut seen_targets = BTreeSet::new();

    for line in patch_text.lines() {
        if let Some(old_raw) = line.strip_prefix("--- ") {
            pending_old = Some(old_raw.trim().to_string());
            continue;
        }
        let Some(new_raw) = line.strip_prefix("+++ ") else {
            continue;
        };
        let Some(old_raw) = pending_old.take() else {
            continue;
        };
        let new_raw = new_raw.trim().to_string();

        if old_raw == "/dev/null" {
            let path = normalize_patch_target_str(&new_raw)?;
            return Err(fs_patch_failure(
                format!("file creation is not supported for fs_patch: {}", path),
                vec![path],
            ));
        }
        if new_raw == "/dev/null" {
            let path = normalize_patch_target_str(&old_raw)?;
            return Err(fs_patch_failure(
                format!("file deletion is not supported for fs_patch: {}", path),
                vec![path],
            ));
        }

        let old_path = normalize_patch_target_str(&old_raw)?;
        let new_path = normalize_patch_target_str(&new_raw)?;
        if old_path != new_path {
            return Err(fs_patch_failure(
                format!(
                    "rename-style patch targets are not supported: {} -> {}",
                    old_path, new_path
                ),
                vec![old_path, new_path],
            ));
        }
        if !seen_targets.insert(old_path.clone()) {
            return Err(fs_patch_failure(
                format!("duplicate patch target path: {}", old_path),
                vec![old_path],
            ));
        }
    }

    Ok(())
}

fn validate_patch_structure(patches: &[MpatchPatch]) -> FsPatchResult<Vec<String>> {
    if patches.is_empty() {
        return Err(fs_patch_failure(
            "patch does not contain any file changes",
            vec![],
        ));
    }
    if patches.len() > MAX_PATCH_TARGETS {
        return Err(fs_patch_failure(
            format!("patch exceeds maximum {} target files", MAX_PATCH_TARGETS),
            vec![],
        ));
    }

    let total_hunks: usize = patches.iter().map(|patch| patch.hunks.len()).sum();
    if total_hunks > MAX_PATCH_HUNKS {
        return Err(fs_patch_failure(
            format!("patch exceeds maximum {} hunks", MAX_PATCH_HUNKS),
            vec![],
        ));
    }

    let mut normalized_paths = Vec::with_capacity(patches.len());
    let mut seen = BTreeSet::new();
    for patch in patches {
        let normalized_path = normalize_patch_target_path(&patch.file_path)?;
        if patch.is_creation() {
            return Err(fs_patch_failure(
                format!(
                    "file creation is not supported for fs_patch: {}",
                    normalized_path
                ),
                vec![normalized_path],
            ));
        }
        if !seen.insert(normalized_path.clone()) {
            return Err(fs_patch_failure(
                format!("duplicate patch target path: {}", normalized_path),
                vec![normalized_path],
            ));
        }
        normalized_paths.push(normalized_path);
    }

    Ok(normalized_paths)
}

fn read_patch_target_file(path: &Path, normalized_path: &str) -> FsPatchResult<String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| {
        fs_patch_failure(
            format!("failed to inspect target file {}: {}", normalized_path, e),
            vec![normalized_path.to_string()],
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(fs_patch_failure(
            format!("target is not a regular file: {}", normalized_path),
            vec![normalized_path.to_string()],
        ));
    }

    let bytes = fs::read(path).map_err(|e| {
        fs_patch_failure(
            format!("failed to read target file {}: {}", normalized_path, e),
            vec![normalized_path.to_string()],
        )
    })?;
    if bytes.contains(&0) {
        return Err(fs_patch_failure(
            format!("target file contains NUL bytes: {}", normalized_path),
            vec![normalized_path.to_string()],
        ));
    }

    String::from_utf8(bytes).map_err(|e| {
        fs_patch_failure(
            format!(
                "target file is not valid UTF-8: {} ({})",
                normalized_path, e
            ),
            vec![normalized_path.to_string()],
        )
    })
}

fn prepare_fs_patch(
    patch_text: &str,
    project_dir: &Path,
) -> FsPatchResult<Vec<PreparedPatchTarget>> {
    let normalized = normalize_patch_text(patch_text)?;
    validate_patch_headers(&normalized)?;
    let mut patches = parse_auto(&normalized).map_err(|e| {
        fs_patch_failure(format!("failed to parse unified diff patch: {}", e), vec![])
    })?;
    let normalized_paths = validate_patch_structure(&patches)?;

    let mut prepared = Vec::with_capacity(patches.len());
    for (patch, normalized_path) in patches.iter_mut().zip(normalized_paths.into_iter()) {
        patch.file_path = PathBuf::from(&normalized_path);
        let absolute_path = project_dir.join(&normalized_path);
        let original_content = read_patch_target_file(&absolute_path, &normalized_path)?;
        let use_crlf = uses_crlf_line_endings(&original_content, &normalized_path)?;
        let mut patched = try_apply_patch_to_content(
            patch,
            Some(original_content.as_str()),
            &ApplyOptions::exact(),
        )
        .map_err(|err| match err {
            StrictApplyError::Patch(patch_err) => fs_patch_failure(
                format!(
                    "failed to apply patch to {}: {}",
                    normalized_path, patch_err
                ),
                vec![normalized_path.clone()],
            ),
            StrictApplyError::PartialApply { .. } => fs_patch_failure(
                format!(
                    "patch context did not match current file content: {}",
                    normalized_path
                ),
                vec![normalized_path.clone()],
            ),
            other => fs_patch_failure(
                format!("failed to apply patch to {}: {}", normalized_path, other),
                vec![normalized_path.clone()],
            ),
        })?;
        if use_crlf {
            patched.new_content = patched.new_content.replace("\n", "\r\n");
        }
        if patched.new_content.len() > MAX_PATCH_OUTPUT_BYTES {
            return Err(fs_patch_failure(
                format!(
                    "patched file exceeds maximum {} bytes: {}",
                    MAX_PATCH_OUTPUT_BYTES, normalized_path
                ),
                vec![normalized_path],
            ));
        }
        prepared.push(PreparedPatchTarget {
            path: normalized_path,
            absolute_path,
            original_content,
            patched_content: patched.new_content,
        });
    }

    Ok(prepared)
}

fn apply_fs_patch(patch_text: &str, project_dir: &Path) -> Value {
    let prepared = match prepare_fs_patch(patch_text, project_dir) {
        Ok(prepared) => prepared,
        Err(err) => return patch_file_ack(false, &[], &err.rejected_paths, &err.message),
    };

    let changed_paths: Vec<String> = prepared
        .iter()
        .filter(|target| target.original_content != target.patched_content)
        .map(|target| target.path.clone())
        .collect();

    let mut written_paths: Vec<usize> = Vec::new();
    for (index, target) in prepared.iter().enumerate() {
        if target.original_content != target.patched_content {
            if let Err(error) = fs::write(&target.absolute_path, target.patched_content.as_bytes())
            {
                for written_index in written_paths.into_iter().rev() {
                    let written_target = &prepared[written_index];
                    let _ = fs::write(
                        &written_target.absolute_path,
                        written_target.original_content.as_bytes(),
                    );
                }
                return patch_file_ack(
                    false,
                    &[],
                    std::slice::from_ref(&target.path),
                    &format!("failed to write patched file {}: {}", target.path, error),
                );
            }
            written_paths.push(index);
        }
    }

    let message = if changed_paths.is_empty() {
        "patch applied with no file changes".to_string()
    } else {
        format!("applied patch to {} file(s)", changed_paths.len())
    };
    patch_file_ack(true, &changed_paths, &[], &message)
}

fn memory_tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}

fn shell_path_if_valid(path: PathBuf) -> Option<PathBuf> {
    fs::metadata(&path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|_| path)
}

#[cfg(unix)]
fn user_shell_from_passwd() -> Option<PathBuf> {
    let uid = unsafe { getuid() };
    let suggested_buffer_len = unsafe { sysconf(_SC_GETPW_R_SIZE_MAX) };
    let buffer_len = usize::try_from(suggested_buffer_len)
        .ok()
        .filter(|len| *len > 0)
        .unwrap_or(1024);
    let mut buffer = vec![0; buffer_len];
    let mut passwd_entry = MaybeUninit::<passwd>::uninit();

    loop {
        let mut result = ptr::null_mut();
        let status = unsafe {
            getpwuid_r(
                uid,
                passwd_entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };

        if status == 0 {
            if result.is_null() {
                return None;
            }
            let passwd_entry = unsafe { passwd_entry.assume_init_ref() };
            if passwd_entry.pw_shell.is_null() {
                return None;
            }
            let shell = unsafe { CStr::from_ptr(passwd_entry.pw_shell) }
                .to_string_lossy()
                .into_owned();
            return shell_path_if_valid(PathBuf::from(shell));
        }

        if status != ERANGE {
            return None;
        }

        let new_len = buffer.len().checked_mul(2)?;
        if new_len > 1024 * 1024 {
            return None;
        }
        buffer.resize(new_len, 0);
    }
}

#[cfg(not(unix))]
fn user_shell_from_passwd() -> Option<PathBuf> {
    None
}

fn user_shell_from_env() -> Option<PathBuf> {
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .and_then(shell_path_if_valid)
}

fn default_shell_path() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("cmd")
    } else {
        PathBuf::from("sh")
    }
}

fn resolve_user_shell() -> PathBuf {
    user_shell_from_passwd()
        .or_else(user_shell_from_env)
        .unwrap_or_else(default_shell_path)
}

fn shell_program_name(shell_path: &Path) -> String {
    shell_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn shell_command_argv(command: &str) -> Vec<String> {
    let shell_path = resolve_user_shell();
    let program_name = shell_program_name(&shell_path);
    let shell = shell_path.to_string_lossy().to_string();

    if cfg!(windows) {
        if matches!(
            program_name.as_str(),
            "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
        ) {
            return vec![shell, "-Command".to_string(), command.to_string()];
        }
        return vec![shell, "/c".to_string(), command.to_string()];
    }

    vec![shell, "-lc".to_string(), command.to_string()]
}

fn memory_tool_definitions() -> Vec<Value> {
    vec![
        memory_tool("memory_create_node", "Create a Project Memory node. Use an absolute project path, omit project_dir for the current project, or use project_dir=\"[GLOBAL]\" only for cross-project knowledge.", json!({
            "type":"object",
            "properties":{
                "node_id":{"type":"string","description":"Optional UUID. Generated when omitted."},
                "project_dir":{"type":"string","description":"Project context. Use an absolute path, omit for the current project, or use [GLOBAL] for cross-project knowledge."},
                "node_type":{"type":"string","description":"Node kind. Default: observation."},
                "title":{"type":"string"},
                "content":{"type":"string","description":"Optional descriptive/body text."},
                "hashtags":{"type":"array","items":{"type":"string"},"description":"Flat labels such as #rust or #provider. Stored normalized."},
                "metadata":{"type":"object","description":"Optional lightweight JSON metadata."}
            },
            "required":["title"]
        })),
        memory_tool("memory_update_node", "Update a Project Memory node. Returns a compact acknowledgement.", json!({
            "type":"object",
            "properties":{
                "node_id":{"type":"string"},
                "node_type":{"type":"string"},
                "title":{"type":"string"},
                "content":{"type":["string","null"]},
                "hashtags":{"type":"array","items":{"type":"string"}},
                "metadata":{"type":["object","null"]}
            },
            "required":["node_id"]
        })),
        memory_tool("memory_link_nodes", "Create a typed directed link between two Project Memory nodes. Returns a compact acknowledgement.", json!({
            "type":"object",
            "properties":{
                "edge_id":{"type":"string","description":"Optional UUID. Generated when omitted."},
                "from_node_id":{"type":"string"},
                "to_node_id":{"type":"string"},
                "relation_type":{"type":"string","description":"Relation such as depends_on, mentions, owned_by, blocks, documents, or relates_to."},
                "metadata":{"type":"object"}
            },
            "required":["from_node_id","to_node_id","relation_type"]
        })),
        memory_tool("memory_unlink_nodes", "Remove a Project Memory knowledge-base relationship by edge_id or by from_node_id, to_node_id, and relation_type.", json!({
            "type":"object",
            "properties":{
                "edge_id":{"type":"string"},
                "from_node_id":{"type":"string"},
                "to_node_id":{"type":"string"},
                "relation_type":{"type":"string"}
            },
            "required":[]
        })),
        memory_tool("memory_get_node", "Retrieve one Project Memory knowledge-base node with its content, hashtags, and immediate incoming/outgoing relationships.", json!({
            "type":"object",
            "properties":{"node_id":{"type":"string"}},
            "required":["node_id"]
        })),
        memory_tool("unified_search", "Search indexed content across one or more source kinds with fts, semantic, or hybrid retrieval. Use an absolute project path, omit project_dir for the current project, or use [GLOBAL] for Global Knowledge where supported; omit source_kinds for default human-oriented kinds memory and chat_message.", json!({
            "type":"object",
            "properties":{
                "query":{"type":"string","description":"Query text."},
                "mode":{"type":"string","enum":["fts","semantic","hybrid"],"description":"Retrieval mode. Default: fts."},
                "project_dir":{"type":"string","description":"Project context. Use an absolute path, omit for the current project, or use [GLOBAL] for Global Knowledge."},
                "source_kinds":{"type":"array","items":{"type":"string","enum":["memory","chat_message","tool_call","tool_result"]},"description":"Indexed source kinds. Omit for default human-oriented kinds: memory and chat_message."},
                "hashtags":{"type":"array","items":{"type":"string"}},
                "hashtag_match":{"type":"string","enum":["any","all"],"description":"Defaults to any."},
                "node_type":{"type":"string"},
                "relation_type":{"type":"string"},
                "linked_node_id":{"type":"string"},
                "limit":{"type":"integer","description":"Default 10, max 50."}
            },
            "required":[]
        })),
        memory_tool("memory_open_graph", "Open a bounded local Project Memory graph neighborhood around one or more anchor nodes.", json!({
            "type":"object",
            "properties":{
                "node_id":{"type":"string"},
                "node_ids":{"type":"array","items":{"type":"string"}},
                "depth":{"type":"integer","description":"Default 1, max 3."},
                "limit":{"type":"integer","description":"Default 50, max 200 nodes."}
            },
            "required":[]
        })),
        memory_tool("memory_delete_node", "Delete one Project Memory knowledge-base node and its directly owned relationship and hashtag rows.", json!({
            "type":"object",
            "properties":{"node_id":{"type":"string"}},
            "required":["node_id"]
        })),
        memory_tool("memory_list_hashtags", "List Project Memory hashtags, optionally filtered by prefix. Use an absolute project path, omit project_dir for the current project, or use [GLOBAL] for Global Knowledge.", json!({
            "type":"object",
            "properties":{
                "project_dir":{"type":"string","description":"Project context. Use an absolute path, omit for the current project, or use [GLOBAL] for Global Knowledge."},
                "prefix":{"type":"string"},
                "limit":{"type":"integer","description":"Default 50, max 200."}
            },
            "required":[]
        })),
        memory_tool("unified_search_rebuild", "Rebuild or refresh the generalized unified search index for all or one scoped source kind.", json!({
            "type":"object",
            "properties":{
                "project_dir":{"type":"string","description":"Project scope. Use an absolute path or omit for the current project."},
                "source_kind":{"type":"string","enum":["memory","chat_message","tool_call","tool_result"],"description":"Optional source kind filter."},
                "full":{"type":"boolean","description":"When true, clear derived rows in scope before rebuilding. Default: false."}
            },
            "required":[]
        })),
    ]
}

#[cfg(feature = "stylos")]
type StylosToolFuture = Pin<Box<dyn Future<Output = Result<String>> + Send>>;
#[cfg(feature = "stylos")]
pub type StylosToolInvoker = Arc<dyn Fn(String, Value) -> StylosToolFuture + Send + Sync>;

type LocalAgentToolFuture = Pin<Box<dyn Future<Output = Result<String>> + Send>>;
pub type LocalAgentToolInvoker = Arc<dyn Fn(String, Value) -> LocalAgentToolFuture + Send + Sync>;

const MAX_SLEEP_MS: u64 = 30_000;
const SELF_TARGET_KEYWORD: &str = "SELF";
const DEFAULT_READ_MODE: &str = "base64";
const DEFAULT_WRITE_MODE: &str = "base64";
const DEFAULT_READ_LIMIT: usize = 128 * 1024;
const MAX_READ_LIMIT: usize = 2 * 1024 * 1024;
const DEFAULT_SHELL_RESULT_LIMIT: usize = 16 * 1024;
const DEFAULT_SHELL_TIMEOUT_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemInspectionRateLimits {
    pub api_call: String,
    pub source: String,
    pub http_status: Option<u16>,
    pub active_limit: Option<String>,
    pub snapshot_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemInspectionProvider {
    pub status: String,
    pub active_profile: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub auth_configured: Option<bool>,
    pub base_url_present: Option<bool>,
    pub rate_limits: Option<SystemInspectionRateLimits>,
    pub warnings: Vec<String>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemInspectionTaskRuntime {
    pub status: String,
    pub current_activity: Option<String>,
    pub current_activity_detail: Option<String>,
    pub busy: Option<bool>,
    pub activity_status: Option<String>,
    pub activity_status_changed_at_ms: Option<u64>,
    pub process_started_at_ms: Option<u64>,
    pub uptime_ms: Option<u64>,
    pub recent_window_ms: Option<u64>,
    pub runtime_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemInspectionRuntime {
    pub status: String,
    pub pid: Option<u32>,
    pub now_ms: u64,
    pub session_id: String,
    pub project_dir: String,
    pub workflow_name: Option<String>,
    pub phase_name: Option<String>,
    pub workflow_status: Option<String>,
    pub debug_runtime_lines: Vec<String>,
    pub task_runtime: Option<SystemInspectionTaskRuntime>,
    pub warnings: Vec<String>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemInspectionTools {
    pub status: String,
    pub tool_count: usize,
    pub available_names: Vec<String>,
    pub warnings: Vec<String>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemInspectionResult {
    pub overall_status: String,
    pub summary: String,
    pub runtime: SystemInspectionRuntime,
    pub tools: SystemInspectionTools,
    pub provider: SystemInspectionProvider,
    pub warnings: Vec<String>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSymbolSpan {
    pub start_line: usize,
    pub start_byte: usize,
    pub end_line: usize,
    pub end_byte: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceExtractedSymbol {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_name: Option<String>,
    pub span: SourceSymbolSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceExtractSymbolsResult {
    pub language: String,
    pub path: String,
    pub symbols: Vec<SourceExtractedSymbol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceOutlineResult {
    pub language: String,
    pub path: String,
    pub file: SourceOutlineFile,
    pub symbols: Vec<SourceOutlineSymbol>,
    pub imports: Vec<SourceOutlineImport>,
    pub edges: Vec<SourceOutlineEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceOutlineNormalResult {
    pub language: String,
    pub path: String,
    pub detail: String,
    pub symbols: Vec<SourceOutlineNormalSymbol>,
    pub imports: Vec<SourceOutlineNormalImport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceOutlineNormalSymbol(pub String, pub String, pub [usize; 4], pub Option<String>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceOutlineNormalImport(pub String, pub usize);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceOutlineFile {
    pub id: String,
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceOutlineSymbol {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_name: Option<String>,
    pub span: SourceSymbolSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceOutlineImport {
    pub id: String,
    pub module: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub is_wildcard: bool,
    pub span: SourceSymbolSpan,
    pub resolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceOutlineEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
    pub confidence: String,
}

type SourceAnalysisToolFuture = Pin<Box<dyn Future<Output = Result<String>> + Send>>;
pub type SourceAnalysisToolInvoker =
    Arc<dyn Fn(String, Value) -> SourceAnalysisToolFuture + Send + Sync>;

pub struct ToolCtx {
    pub db: Arc<DbHandle>,
    pub session_id: Uuid,
    pub project_dir: PathBuf,
    pub workflow_state: Option<WorkflowState>,
    pub turn_seq: Option<u32>,
    #[cfg(feature = "stylos")]
    pub local_agent_id: Option<String>,
    #[cfg(feature = "stylos")]
    pub local_instance_id: Option<String>,
    #[cfg(feature = "stylos")]
    pub stylos_tool_invoker: Option<StylosToolInvoker>,
    #[cfg(feature = "stylos")]
    pub stylos_enabled: bool,
    pub local_agent_tool_invoker: Option<LocalAgentToolInvoker>,
    pub source_analysis_tool_invoker: Option<SourceAnalysisToolInvoker>,
    pub system_inspection: Option<SystemInspectionResult>,
}

fn resolve_board_target(
    requested_to_instance: &str,
    requested_to_agent_id: &str,
    #[cfg(feature = "stylos")] local_instance_id: Option<&str>,
    #[cfg(feature = "stylos")] local_agent_id: Option<&str>,
) -> Result<(String, String)> {
    #[cfg(feature = "stylos")]
    let resolved_local_instance = local_instance_id
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("local:{}", std::process::id()));

    #[cfg(not(feature = "stylos"))]
    let resolved_local_instance = format!("local:{}", std::process::id());

    let to_instance = match requested_to_instance {
        SELF_TARGET_KEYWORD | "local" => resolved_local_instance,
        other => other.to_string(),
    };

    #[cfg(feature = "stylos")]
    let to_agent_id = if requested_to_agent_id == SELF_TARGET_KEYWORD {
        local_agent_id
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("SELF requires known local agent id"))?
            .to_string()
    } else {
        requested_to_agent_id.to_string()
    };

    #[cfg(not(feature = "stylos"))]
    let to_agent_id = if requested_to_agent_id == SELF_TARGET_KEYWORD {
        "master".to_string()
    } else {
        requested_to_agent_id.to_string()
    };

    Ok((to_instance, to_agent_id))
}

fn resolve_note_source_instance(
    requested_from_instance: Option<&str>,
    #[cfg(feature = "stylos")] local_instance_id: Option<&str>,
) -> Option<String> {
    let requested = requested_from_instance?.trim();
    if requested.is_empty() {
        return None;
    }

    #[cfg(feature = "stylos")]
    let resolved_local_instance = local_instance_id
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("local:{}", std::process::id()));

    #[cfg(not(feature = "stylos"))]
    let resolved_local_instance = format!("local:{}", std::process::id());

    Some(match requested {
        SELF_TARGET_KEYWORD | "local" => resolved_local_instance,
        other => other.to_string(),
    })
}

fn resolve_memory_project_dir(args: &Value, ctx: &ToolCtx) -> String {
    match args["project_dir"]
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(GLOBAL_PROJECT_DIR) => GLOBAL_PROJECT_DIR.to_string(),
        Some(".") => ctx.project_dir.to_string_lossy().to_string(),
        Some(value) => value.to_string(),
        None => ctx.project_dir.to_string_lossy().to_string(),
    }
}

fn parse_mode<'a>(value: Option<&'a str>, default_mode: &'static str) -> Result<&'a str> {
    match value.unwrap_or(default_mode) {
        "raw" => Ok("raw"),
        "base64" => Ok("base64"),
        other => anyhow::bail!("invalid mode: {other}"),
    }
}

fn parse_nonnegative_offset(args: &Value) -> Result<usize> {
    let Some(value) = args.get("offset") else {
        return Ok(0);
    };
    let offset = value
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("offset must be a non-negative integer"))?;
    usize::try_from(offset).context("offset too large")
}

fn parse_read_limit(args: &Value) -> Result<usize> {
    let limit = match args.get("limit") {
        Some(value) => {
            let limit = value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("limit must be a positive integer"))?;
            usize::try_from(limit).context("limit too large")?
        }
        None => DEFAULT_READ_LIMIT,
    };
    if limit == 0 {
        anyhow::bail!("limit must be greater than 0");
    }
    if limit > MAX_READ_LIMIT {
        anyhow::bail!("limit exceeds maximum {MAX_READ_LIMIT}");
    }
    Ok(limit)
}

fn parse_shell_result_limit(args: &Value) -> Result<usize> {
    match args.get("result_limit") {
        Some(value) => {
            let limit = value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("result_limit must be a positive integer"))?;
            let limit = usize::try_from(limit).context("result_limit too large")?;
            if limit == 0 {
                anyhow::bail!("result_limit must be greater than 0");
            }
            Ok(limit)
        }
        None => Ok(DEFAULT_SHELL_RESULT_LIMIT),
    }
}

fn parse_shell_timeout_ms(args: &Value) -> Result<u64> {
    match args.get("timeout_ms") {
        Some(value) => {
            let timeout_ms = value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("timeout_ms must be a non-negative integer"))?;
            if timeout_ms == 0 {
                anyhow::bail!("timeout_ms must be greater than 0");
            }
            Ok(timeout_ms)
        }
        None => Ok(DEFAULT_SHELL_TIMEOUT_MS),
    }
}

fn truncate_output_with_notice(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = 0usize;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if next > limit {
            break;
        }
        end = next;
    }
    let truncated_bytes = text.len().saturating_sub(end);
    format!(
        "{}\n[truncated: omitted {} byte(s) after result_limit={}]",
        &text[..end],
        truncated_bytes,
        limit
    )
}

pub fn tool_definitions() -> Value {
    let mut base_defs = vec![
        json!({
            "type": "function",
            "function": {
                "name": "fs_read_file",
                "description": "Read file contents.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to read" },
                        "mode": { "type": "string", "enum": ["raw", "base64"], "description": "Encoding. Default: base64." },
                        "offset": { "type": "integer", "description": "Offset. Default: 0." },
                        "limit": { "type": "integer", "description": "Max bytes to read. Default: 131072, max: 2097152." }
                        ,"reason": { "type": "string", "description": "Optional reason." }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "fs_write_file",
                "description": "Create a new file. Replaces the entire target if it already exists; for normal edits to existing text files, use fs_patch instead.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to write" },
                        "content": { "type": "string", "description": "Content to write" },
                        "mode": { "type": "string", "enum": ["raw", "base64"], "description": "Encoding. Default: base64." }
                        ,"reason": { "type": "string", "description": "Optional reason." }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "fs_patch",
                "description": "Apply a targeted standard unified-diff patch to existing text files. Use this for normal edits to existing text files. Patch headers must use project-relative a/path and b/path; absolute paths and .. traversal are unsupported. Do not use *** Begin Patch / *** Update File wrappers.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "patch": { "type": "string", "description": "Standard unified diff, e.g. --- a/path, +++ b/path, and @@ hunks. Do not include *** Begin Patch wrappers." }
                        ,"reason": { "type": "string", "description": "Optional reason." }
                    },
                    "required": ["patch"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "fs_list_directory",
                "description": "List directory entries.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path to list" }
                        ,"reason": { "type": "string", "description": "Optional reason." }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "shell_run_command",
                "description": "Run a shell command and return stdout+stderr.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Shell command to run" },
                        "result_limit": { "type": "integer", "description": "Max returned stdout+stderr bytes. Default: 16384." },
                        "timeout_ms": { "type": "integer", "description": "Timeout in ms. Default: 300000." }
                        ,"reason": { "type": "string", "description": "Optional reason." }
                    },
                    "required": ["command"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "source_extract_symbols",
                "description": "Use this tool to read source code structure from one source file. Symbol-only view; prefer source_outline for app analysis and tracing.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Source file path to analyze" },
                        "reason": { "type": "string", "description": "Optional reason." }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "source_outline",
                "description": "Use this tool to read source code structure from one source file. Return a bounded one-file outline; use detail=normal for compact navigation or full for graph-ready IDs and edges.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Source file path to analyze" },
                        "detail": { "type": "string", "enum": ["normal", "full"], "description": "Output detail. Default: normal for compact navigation. Use full for graph-ready IDs and edges." },
                        "reason": { "type": "string", "description": "Optional reason." }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "system_inspect_local",
                "description": "Inspect the current local Themion process and active agent context. Returns a bounded read-only runtime, tool, and provider snapshot.",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "time_sleep",
                "description": "Sleep for a short bounded duration without invoking the shell.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ms": { "type": "integer", "description": "Milliseconds to sleep. Max: 30000." }
                    },
                    "required": ["ms"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "history_recall",
                "description": "Retrieve earlier conversation messages chronologically.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "Optional session selector. Omit for the active session, pass \"*\" for all sessions in the current project, or pass one session UUID in the current project." },
                        "limit": { "type": "integer", "description": "Max messages. Default: 20, max: 200." },
                        "direction": { "type": "string", "enum": ["newest", "oldest"] }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "workflow_get_state",
                "description": "Get workflow state and allowed transitions.",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "workflow_set",
                "description": "Apply workflow state changes.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "workflow": { "type": "string", "description": "Workflow name to activate." },
                        "phase_result": { "type": "string", "enum": ["passed", "failed", "user_feedback_required"], "description": "Current phase result." },
                        "phase": { "type": "string", "description": "Next phase in the active workflow." },
                        "workflow_status": { "type": "string", "enum": ["completed", "failed"], "description": "Terminal workflow status to set." }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_create_note",
                "description": "Create a durable board note for tracked self-work or delegated work. Include expected result and return path when delegating.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to_instance": { "type": "string", "description": "Target instance id. Prefer local for ordinary self-notes; SELF is also accepted when supported." },
                        "to_agent_id": { "type": "string", "description": "Target agent id. Prefer master for ordinary self-notes; SELF is also accepted when supported." },
                        "body": { "type": "string" },
                        "note_kind": { "type": "string", "enum": ["work_request", "done_mention"], "description": "Kind. Default: work_request." },
                        "origin_note_id": { "type": "string", "description": "Original note id for done mentions." },
                        "from_instance": { "type": "string" },
                        "from_agent_id": { "type": "string" },
                        "request_id": { "type": "string", "description": "Optional Stylos request id." },
                        "note_id": { "type": "string", "description": "Optional note id." },
                        "column": { "type": "string", "enum": ["todo", "blocked"], "description": "Initial column. Default: todo." }
                    },
                    "required": ["to_instance", "to_agent_id", "body"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_list_notes",
                "description": "List board notes filtered by target and optional columns.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to_instance": { "type": "string" },
                        "to_agent_id": { "type": "string" },
                        "columns": {
                            "type": "array",
                            "items": { "type": "string", "enum": ["todo", "in_progress", "blocked", "done"] }
                        }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_read_note",
                "description": "Read a board note by note_id.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "note_id": { "type": "string" }
                    },
                    "required": ["note_id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_update_note",
                "description": "Update one board note. Change column, result text, or both.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "note_id": { "type": "string" },
                        "column": { "type": "string", "enum": ["todo", "in_progress", "blocked", "done"], "description": "New column. Omit to keep current column." },
                        "result_text": { "type": "string", "description": "Result text. Omit to keep current result." }
                    },
                    "required": ["note_id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "local_agent_create",
                "description": "Create a new local agent team member in the current Themion instance.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_id": { "type": "string", "description": "Optional explicit agent id. When omitted, the runtime allocates the next free smith-N worker id." },
                        "label": { "type": "string", "description": "Optional user-visible label. Defaults to agent_id." },
                        "roles": { "type": "array", "items": { "type": "string" }, "description": "Optional role list. Omitted or empty defaults to executor. Must not violate local role invariants." },
                        "reason": { "type": "string", "description": "Optional reason." }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "local_agent_delete",
                "description": "Delete an existing non-leader local agent from the current Themion instance.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_id": { "type": "string", "description": "Target local agent id." },
                        "reason": { "type": "string", "description": "Optional reason." }
                    },
                    "required": ["agent_id"]
                }
            }
        }),
    ];
    base_defs.extend(memory_tool_definitions());

    #[cfg(feature = "stylos")]
    let defs = {
        let mut defs = base_defs;
        defs.extend(stylos_tool_definitions());
        defs
    };

    #[cfg(not(feature = "stylos"))]
    let defs = base_defs;

    Value::Array(defs)
}

#[cfg(feature = "stylos")]
fn stylos_tool_definitions() -> Vec<Value> {
    vec![
        stylos_tool("stylos_query_agents_alive", "Query which instances and agents are alive.", json!({
            "type":"object",
            "properties":{
                "exclude_self":{"type":"boolean","description":"Exclude the current instance from discovery results. Default: true."}
            },
            "required":[]
        })),
        stylos_tool("stylos_query_agents_free", "Query which agents are free.", json!({
            "type":"object",
            "properties":{
                "exclude_self":{"type":"boolean","description":"Exclude the current instance from discovery results. Default: true."}
            },
            "required":[]
        })),
        stylos_tool("stylos_query_agents_git", "Query agents by git repo. Prefer normalized remote <host>/<owner>/<repo>; ask if ambiguous.", json!({
            "type":"object","properties":{
                "remote":{"type":"string","description":"Optional git selector. Prefer normalized <host>/<owner>/<repo>."},
                "exclude_self":{"type":"boolean","description":"Exclude the current instance from discovery results. Default: true."}
            },"required":[]
        })),
        stylos_tool("stylos_query_nodes", "Query visible Themion nodes on the Stylos network.", json!({"type":"object","properties":{},"required":[]})),
        stylos_tool("stylos_query_status", "Query one instance for current process and agent status.", json!({
            "type":"object","properties":{"instance":{"type":"string"},"agent_id":{"type":"string"},"role":{"type":"string"}},"required":["instance"]
        })),
        stylos_tool("stylos_send_message", "Send a short volatile message to one target agent. Use board notes for delegated work that needs tracking.", json!({
            "type":"object","properties":{
                "instance":{"type":"string","description":"Target instance in <hostname>:<pid> form."},
                "to_agent_id":{"type":"string","description":"Target agent id. Default: master."},
                "message":{"type":"string"},
                "request_id":{"type":"string"}
            },"required":["instance","message"]
        })),
    ]
}

#[cfg(feature = "stylos")]
fn stylos_tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}

pub async fn call_tool(name: &str, args_json: &str, ctx: &ToolCtx) -> String {
    match execute_tool(name, args_json, ctx).await {
        Ok(output) => output,
        Err(e) => format!("Error: {e}"),
    }
}

async fn execute_tool(name: &str, args_json: &str, ctx: &ToolCtx) -> Result<String> {
    let args: Value = serde_json::from_str(args_json)?;

    match name {
        "fs_read_file" | "read_file" => {
            let path_arg = args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let path = ctx.project_dir.join(path_arg);
            let mode = parse_mode(args["mode"].as_str(), DEFAULT_READ_MODE)?;
            let offset = parse_nonnegative_offset(&args)?;
            let limit = parse_read_limit(&args)?;
            let bytes = fs::read(&path)?;
            let start = offset.min(bytes.len());
            let end = start.saturating_add(limit).min(bytes.len());
            let slice = &bytes[start..end];
            let content = match mode {
                "raw" => std::str::from_utf8(slice)
                    .context("raw mode requires valid UTF-8; use mode=base64 for binary content")?
                    .to_string(),
                "base64" => base64::engine::general_purpose::STANDARD.encode(slice),
                _ => unreachable!(),
            };
            Ok(json!({
                "path": path_arg,
                "mode": mode,
                "offset": start,
                "limit": limit,
                "returned_bytes": slice.len(),
                "file_size": bytes.len(),
                "eof": end >= bytes.len(),
                "content": content,
            })
            .to_string())
        }
        "fs_write_file" | "write_file" => {
            let path = args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let path = ctx.project_dir.join(path);
            let content = args["content"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing content"))?;
            let mode = parse_mode(args["mode"].as_str(), DEFAULT_WRITE_MODE)?;
            let bytes = match mode {
                "raw" => content.as_bytes().to_vec(),
                "base64" => base64::engine::general_purpose::STANDARD
                    .decode(content)
                    .context("invalid base64 content")?,
                _ => unreachable!(),
            };
            fs::write(path, &bytes)?;
            Ok(write_file_ack(args["path"].as_str().unwrap(), mode, bytes.len()).to_string())
        }
        "fs_patch" => {
            let patch = args["patch"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing patch"))?;
            Ok(apply_fs_patch(patch, &ctx.project_dir).to_string())
        }
        "fs_list_directory" | "list_directory" => {
            let path = args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let path = ctx.project_dir.join(path);
            let entries: Vec<String> = fs::read_dir(path)?
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            Ok(entries.join("\n"))
        }
        "shell_run_command" | "bash" => {
            let command = args["command"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing command"))?;
            let result_limit = parse_shell_result_limit(&args)?;
            let timeout_ms = parse_shell_timeout_ms(&args)?;
            let argv = shell_command_argv(command);
            let (program, program_args) = argv
                .split_first()
                .ok_or_else(|| anyhow::anyhow!("failed to build shell command argv"))?;
            let output = match timeout(
                Duration::from_millis(timeout_ms),
                tokio::process::Command::new(program)
                    .args(program_args)
                    .current_dir(&ctx.project_dir)
                    .output(),
            )
            .await
            {
                Ok(output) => output?,
                Err(_) => return Ok(format!("Error: command timed out after {} ms", timeout_ms)),
            };
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}{stderr}");
            Ok(truncate_output_with_notice(&combined, result_limit))
        }
        "time_sleep" => {
            let ms = args["ms"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("missing ms"))?;
            if ms > MAX_SLEEP_MS {
                anyhow::bail!("ms exceeds maximum {MAX_SLEEP_MS}");
            }
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Ok(json!({"slept_ms": ms}).to_string())
        }
        "history_recall" | "recall_history" => {
            let session_scope = match args["session_id"].as_str() {
                Some("*") => SessionScope::AllInCurrentProject,
                Some(value) => SessionScope::Exact(
                    Uuid::parse_str(value).map_err(|_| anyhow::anyhow!("invalid session_id"))?,
                ),
                None => SessionScope::Exact(ctx.session_id),
            };
            let limit = args["limit"].as_u64().map(|n| n as u32).unwrap_or(20);
            let direction = match args["direction"].as_str() {
                Some("oldest") => RecallDirection::Oldest,
                _ => RecallDirection::Newest,
            };
            match ctx.db.recall(RecallArgs {
                session_scope,
                current_project_dir: ctx.project_dir.clone(),
                limit,
                direction,
            }) {
                Ok(msgs) => Ok(serde_json::to_string(
                    &msgs
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "session_id": m.session_id, "turn_seq": m.turn_seq, "role": m.role, "content": m.content,
                                "tool_calls": m.tool_calls_json, "tool_call_id": m.tool_call_id,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default()),
                Err(e) => Ok(format!("Error: {e}")),
            }
        }
        "board_create_note" => {
            let requested_to_instance = args["to_instance"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing to_instance"))?;
            let requested_to_agent_id = args["to_agent_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing to_agent_id"))?;
            let body = args["body"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing body"))?;

            let (to_instance, to_agent_id) = resolve_board_target(
                requested_to_instance,
                requested_to_agent_id,
                #[cfg(feature = "stylos")]
                ctx.local_instance_id.as_deref(),
                #[cfg(feature = "stylos")]
                ctx.local_agent_id.as_deref(),
            )?;

            #[cfg(feature = "stylos")]
            {
                if ctx.stylos_enabled {
                    let invoker = ctx
                        .stylos_tool_invoker
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("stylos tools unavailable"))?;
                    let reply = invoker(
                        "board_create_note".to_string(),
                        json!({
                            "to_instance": to_instance,
                            "to_agent_id": to_agent_id,
                            "body": body,
                            "request_id": args["request_id"].as_str(),
                            "note_id": args["note_id"].as_str(),
                            "_local_agent_id": ctx.local_agent_id.as_deref(),
                        }),
                    )
                    .await?;
                    return Ok(reply);
                }
            }

            let note_id = args["note_id"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let column = match args["column"].as_str().unwrap_or("todo") {
                "todo" => NoteColumn::Todo,
                "blocked" => NoteColumn::Blocked,
                other => anyhow::bail!("invalid create column: {other}"),
            };
            let note_kind = match args["note_kind"].as_str().unwrap_or("work_request") {
                "work_request" => NoteKind::WorkRequest,
                "done_mention" => NoteKind::DoneMention,
                other => anyhow::bail!("invalid note_kind: {other}"),
            };
            let note = ctx.db.create_board_note(CreateNoteArgs {
                note_id,
                note_kind,
                column,
                origin_note_id: args["origin_note_id"].as_str().map(str::to_string),
                from_instance: resolve_note_source_instance(
                    args["from_instance"].as_str(),
                    #[cfg(feature = "stylos")]
                    ctx.local_instance_id.as_deref(),
                ),
                from_agent_id: args["from_agent_id"].as_str().map(str::to_string),
                to_instance,
                to_agent_id,
                body: body.to_string(),
                meta_json: None,
            })?;
            Ok(board_note_ack(
                &note,
                "create",
                json!({
                    "column": note.column.as_str(),
                    "note_kind": note.note_kind.as_str(),
                    "to_instance": note.to_instance,
                    "to_agent_id": note.to_agent_id,
                    "created_at_ms": note.created_at_ms,
                }),
            )
            .to_string())
        }
        "board_list_notes" => {
            let mut columns = Vec::new();
            if let Some(values) = args["columns"].as_array() {
                for value in values {
                    let raw = value
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("invalid columns"))?;
                    let column =
                        parse_note_column(raw).ok_or_else(|| anyhow::anyhow!("invalid column"))?;
                    if !columns.contains(&column) {
                        columns.push(column);
                    }
                }
            }
            let notes = ctx.db.list_board_notes(
                args["to_instance"].as_str(),
                args["to_agent_id"].as_str(),
                &columns,
            )?;
            Ok(Value::Array(notes.iter().map(board_note_to_json).collect()).to_string())
        }
        "board_read_note" => {
            let note_id = args["note_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing note_id"))?;
            let note = ctx.db.get_board_note(note_id)?;
            Ok(match note {
                Some(note) => board_note_ack(
                    &note,
                    "move",
                    json!({
                        "column": note.column.as_str(),
                        "updated_at_ms": note.updated_at_ms,
                    }),
                )
                .to_string(),
                None => board_note_not_found(note_id, "move").to_string(),
            })
        }
        "board_update_note" => {
            let note_id = args["note_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing note_id"))?;
            let column = match args.get("column").and_then(Value::as_str) {
                Some(value) => Some(
                    parse_note_column(value).ok_or_else(|| anyhow::anyhow!("invalid column"))?,
                ),
                None => None,
            };
            let result_text = args
                .get("result_text")
                .map(|value| {
                    value
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("result_text must be a string"))
                })
                .transpose()?;

            if column.is_none() && result_text.is_none() {
                anyhow::bail!("board_update_note requires column or result_text");
            }

            let note = ctx
                .db
                .update_board_note(note_id, column, result_text.map(Some))?;
            Ok(match note {
                Some(note) => {
                    let mut changed = serde_json::Map::new();
                    if args.get("column").is_some() {
                        changed.insert(
                            "column".to_string(),
                            Value::String(note.column.as_str().to_string()),
                        );
                    }
                    if args.get("result_text").is_some() {
                        changed.insert(
                            "has_result_text".to_string(),
                            Value::Bool(note.result_text.is_some()),
                        );
                    }
                    changed.insert("updated_at_ms".to_string(), Value::from(note.updated_at_ms));
                    board_note_ack(&note, "update", Value::Object(changed)).to_string()
                }
                None => board_note_not_found(note_id, "update").to_string(),
            })
        }
        "memory_create_node" => {
            let title = args["title"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing title"))?;
            let node = ctx.db.memory_store().create_node(CreateNodeArgs {
                node_id: args["node_id"].as_str().map(str::to_string),
                project_dir: resolve_memory_project_dir(&args, ctx),
                node_type: args["node_type"]
                    .as_str()
                    .unwrap_or("observation")
                    .to_string(),
                title: title.to_string(),
                content: args["content"].as_str().map(str::to_string),
                hashtags: parse_hashtags_value(args.get("hashtags"))?,
                metadata_json: metadata_to_string(args.get("metadata"))?,
            })?;
            Ok(memory_node_ack(&node, "create").to_string())
        }
        "memory_update_node" => {
            let node_id = args["node_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing node_id"))?;
            let node = ctx.db.memory_store().update_node(
                node_id,
                UpdateNodeArgs {
                    node_type: args["node_type"].as_str().map(str::to_string),
                    title: args["title"].as_str().map(str::to_string),
                    content: parse_nullable_string(args.get("content"))?,
                    hashtags: if args.get("hashtags").is_some() {
                        Some(parse_hashtags_value(args.get("hashtags"))?)
                    } else {
                        None
                    },
                    metadata_json: match args.get("metadata") {
                        Some(Value::Null) => Some(None),
                        Some(value) => Some(Some(serde_json::to_string(value)?)),
                        None => None,
                    },
                },
            )?;
            Ok(match node {
                Some(node) => memory_node_ack(&node, "update").to_string(),
                None => memory_node_not_found(node_id, "update").to_string(),
            })
        }
        "memory_link_nodes" => {
            let edge = ctx.db.memory_store().link_nodes(LinkNodesArgs {
                edge_id: args["edge_id"].as_str().map(str::to_string),
                from_node_id: args["from_node_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing from_node_id"))?
                    .to_string(),
                to_node_id: args["to_node_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing to_node_id"))?
                    .to_string(),
                relation_type: args["relation_type"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing relation_type"))?
                    .to_string(),
                metadata_json: metadata_to_string(args.get("metadata"))?,
            })?;
            Ok(memory_edge_ack(&edge, "link").to_string())
        }
        "memory_unlink_nodes" => {
            let deleted = ctx.db.memory_store().unlink_nodes(
                args["edge_id"].as_str(),
                args["from_node_id"].as_str(),
                args["to_node_id"].as_str(),
                args["relation_type"].as_str(),
            )?;
            Ok(json!({"deleted_edges": deleted}).to_string())
        }
        "memory_get_node" => {
            let node_id = args["node_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing node_id"))?;
            let node = ctx.db.memory_store().get_node_with_links(node_id)?;
            Ok(match node {
                Some(node) => serde_json::to_string(&node)?,
                None => json!({"found": false, "node_id": node_id}).to_string(),
            })
        }
        "unified_search" => {
            let query = crate::memory::UnifiedSearchQuery {
                query: args["query"].as_str().unwrap_or("").to_string(),
                project_dir: Some(resolve_memory_project_dir(&args, ctx)),
                source_kinds: args["source_kinds"]
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .map(|value| {
                                let raw = value.as_str().ok_or_else(|| {
                                    anyhow::anyhow!("source_kinds entries must be strings")
                                })?;
                                crate::memory::UnifiedSearchSourceKind::from_str(raw)
                                    .ok_or_else(|| anyhow::anyhow!("invalid source_kind: {raw}"))
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?,
                mode: match args["mode"].as_str() {
                    Some(value) => Some(
                        UnifiedSearchMode::from_str(value)
                            .ok_or_else(|| anyhow::anyhow!("invalid unified search mode"))?,
                    ),
                    None => None,
                },
                limit: args["limit"].as_u64().map(|n| n as u32),
                hashtags: parse_hashtags_value(args.get("hashtags"))?,
                hashtag_match: match args["hashtag_match"].as_str() {
                    Some(value) => Some(
                        HashtagMatch::from_str(value)
                            .ok_or_else(|| anyhow::anyhow!("invalid hashtag_match"))?,
                    ),
                    None => None,
                },
                node_type: args["node_type"].as_str().map(str::to_string),
                relation_type: args["relation_type"].as_str().map(str::to_string),
                linked_node_id: args["linked_node_id"].as_str().map(str::to_string),
            };
            let response = ctx.db.unified_search(query, None)?;
            Ok(serde_json::to_string(&response)?)
        }
        "memory_open_graph" => {
            let parsed: OpenGraphArgs = serde_json::from_value(args.clone())?;
            let (node_ids, depth, limit) = parsed.into_parts();
            if node_ids.is_empty() {
                anyhow::bail!("memory_open_graph requires node_id or node_ids");
            }
            let graph = ctx.db.memory_store().open_graph(node_ids, depth, limit)?;
            Ok(serde_json::to_string(&graph)?)
        }
        "memory_delete_node" => {
            let node_id = args["node_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing node_id"))?;
            let deleted = ctx.db.memory_store().delete_node(node_id)?;
            Ok(json!({"deleted": deleted, "node_id": node_id}).to_string())
        }

        "unified_search_rebuild" => {
            let project_dir = resolve_memory_project_dir(&args, ctx);
            let report = ctx.db.memory_store().rebuild_unified_search_index(
                Some(&project_dir),
                args["source_kind"].as_str(),
                args["full"].as_bool().unwrap_or(false),
            )?;
            Ok(serde_json::to_string(&report)?)
        }
        "memory_list_hashtags" => {
            let hashtags = ctx.db.memory_store().list_hashtags(
                &resolve_memory_project_dir(&args, ctx),
                args["prefix"].as_str(),
                args["limit"].as_u64().map(|n| n as u32).unwrap_or(50),
            )?;
            Ok(serde_json::to_string(&hashtags)?)
        }
        "source_extract_symbols" | "source_outline" => {
            let invoker = ctx
                .source_analysis_tool_invoker
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("source analysis tools unavailable"))?;
            invoker(name.to_string(), args).await
        }
        "local_agent_create" | "local_agent_delete" => {
            let invoker = ctx
                .local_agent_tool_invoker
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("local agent management tools unavailable"))?;
            invoker(name.to_string(), args).await
        }
        #[cfg(feature = "stylos")]
        "stylos_query_agents_alive"
        | "stylos_query_agents_free"
        | "stylos_query_agents_git"
        | "stylos_query_nodes"
        | "stylos_query_status"
        | "stylos_send_message" => {
            let invoker = ctx
                .stylos_tool_invoker
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("stylos tools unavailable"))?;
            invoker(name.to_string(), args).await
        }
        "system_inspect_local" => {
            let mut result = ctx.system_inspection.clone().unwrap_or_else(|| {
                let mut runtime = SystemInspectionRuntime {
                    status: "ok".to_string(),
                    pid: Some(std::process::id()),
                    now_ms: Utc::now().timestamp_millis().max(0) as u64,
                    session_id: ctx.session_id.to_string(),
                    project_dir: ctx.project_dir.display().to_string(),
                    workflow_name: ctx.workflow_state.as_ref().map(|w| w.workflow_name.clone()),
                    phase_name: ctx.workflow_state.as_ref().map(|w| w.phase_name.clone()),
                    workflow_status: ctx
                        .workflow_state
                        .as_ref()
                        .map(|w| format!("{:?}", w.status)),
                    debug_runtime_lines: vec![
                        "debug runtime snapshot unavailable in fallback inspection path"
                            .to_string(),
                    ],
                    task_runtime: Some(SystemInspectionTaskRuntime {
                        status: "partial".to_string(),
                        current_activity: None,
                        current_activity_detail: None,
                        busy: None,
                        activity_status: None,
                        activity_status_changed_at_ms: None,
                        process_started_at_ms: None,
                        uptime_ms: None,
                        recent_window_ms: None,
                        runtime_notes: vec![
                            "task runtime inspection unavailable in fallback inspection path"
                                .to_string(),
                        ],
                    }),
                    warnings: Vec::new(),
                    issues: Vec::new(),
                };
                let tool_names = tool_definitions()
                    .as_array()
                    .into_iter()
                    .flat_map(|defs| defs.iter())
                    .filter_map(|entry| entry.get("function")?.get("name")?.as_str())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                let tools = SystemInspectionTools {
                    status: "ok".to_string(),
                    tool_count: tool_names.len(),
                    available_names: tool_names,
                    warnings: Vec::new(),
                    issues: Vec::new(),
                };
                if runtime.workflow_name.is_none() {
                    runtime
                        .warnings
                        .push("workflow state unavailable".to_string());
                }
                SystemInspectionResult {
                    overall_status: "ok".to_string(),
                    summary: "local inspection snapshot available".to_string(),
                    runtime,
                    tools,
                    provider: SystemInspectionProvider {
                        status: "unknown".to_string(),
                        active_profile: None,
                        provider: None,
                        model: None,
                        auth_configured: None,
                        base_url_present: None,
                        rate_limits: None,
                        warnings: vec![
                            "provider readiness unavailable in this execution path".to_string()
                        ],
                        issues: Vec::new(),
                    },
                    warnings: vec![
                        "using fallback local inspection without CLI runtime snapshot".to_string(),
                    ],
                    issues: Vec::new(),
                }
            });
            result.tools.available_names.sort();
            result.tools.available_names.dedup();
            result.tools.tool_count = result.tools.available_names.len();
            Ok(serde_json::to_string(&result)?)
        }
        "workflow_get_state" => {
            let state = ctx
                .workflow_state
                .clone()
                .unwrap_or_else(WorkflowState::default);
            Ok(workflow_state_to_json(&state).to_string())
        }
        "workflow_set" => {
            let current = ctx
                .workflow_state
                .clone()
                .unwrap_or_else(WorkflowState::default);
            let next = build_workflow_set_state(&args, &current, ctx.turn_seq)?;
            Ok(workflow_state_to_json(&next).to_string())
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use crate::workflow::{PhaseResult, WorkflowState, WorkflowStatus};

    fn workflow_test_ctx(state: WorkflowState) -> ToolCtx {
        let project_dir = tempdir().unwrap();
        let project_dir_path = project_dir.keep();
        let db = DbHandle::open_in_memory().unwrap();
        let session_id = Uuid::new_v4();
        db.insert_session(session_id, &project_dir_path, true)
            .unwrap();
        ToolCtx {
            db,
            session_id,
            project_dir: project_dir_path,
            workflow_state: Some(state),
            turn_seq: Some(7),
            local_agent_tool_invoker: None,
            source_analysis_tool_invoker: None,
            system_inspection: None,
        }
    }

    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[tokio::test]
    async fn workflow_tool_definitions_expose_only_get_and_set() {
        let defs = tool_definitions();
        let functions = defs.as_array().expect("tool definitions array");
        assert!(functions
            .iter()
            .any(|entry| entry["function"]["name"] == "workflow_get_state"));
        let workflow_set = functions
            .iter()
            .find(|entry| entry["function"]["name"] == "workflow_set")
            .expect("workflow_set definition");
        assert_eq!(
            workflow_set["function"]["parameters"]["properties"]["workflow_status"]["enum"],
            json!(["completed", "failed"])
        );
        for removed in [
            "workflow_set_active",
            "workflow_set_phase",
            "workflow_set_phase_result",
            "workflow_complete",
        ] {
            assert!(functions
                .iter()
                .all(|entry| entry["function"]["name"] != removed));
        }
    }

    #[tokio::test]
    async fn workflow_set_combines_pass_and_phase_move() {
        let mut state = WorkflowState::default();
        state.workflow_name = "LITE".to_string();
        state.phase_name = "EXECUTE".to_string();
        state.phase_result = PhaseResult::Failed;
        state.status = WorkflowStatus::WaitingUser;
        state.retry_state.current_phase_retries = 2;
        let ctx = workflow_test_ctx(state);

        let result = execute_tool(
            "workflow_set",
            r#"{"phase_result":"passed","phase":"VALIDATE"}"#,
            &ctx,
        )
        .await
        .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["workflow"], json!("LITE"));
        assert_eq!(parsed["phase"], json!("VALIDATE"));
        assert_eq!(parsed["status"], json!("running"));
        assert_eq!(parsed["phase_result"], json!("pending"));
        assert_eq!(parsed["retry_state"]["current_phase_retries"], json!(0));
    }

    #[tokio::test]
    async fn workflow_set_user_feedback_required_sets_waiting_user() {
        let mut state = WorkflowState::default();
        state.workflow_name = "LITE".to_string();
        state.phase_name = "CLARIFY".to_string();
        let ctx = workflow_test_ctx(state);

        let result = execute_tool(
            "workflow_set",
            r#"{"phase_result":"user_feedback_required"}"#,
            &ctx,
        )
        .await
        .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["status"], json!("waiting_user"));
        assert_eq!(parsed["phase_result"], json!("user_feedback_required"));
    }

    #[tokio::test]
    async fn workflow_set_rejects_invalid_combination() {
        let ctx = workflow_test_ctx(WorkflowState::default());
        let result = execute_tool(
            "workflow_set",
            r#"{"phase_result":"failed","phase":"EXECUTE"}"#,
            &ctx,
        )
        .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("cannot be combined with phase"));
    }

    #[tokio::test]
    async fn workflow_set_rejects_completion_without_passed_phase() {
        let mut state = WorkflowState::default();
        state.phase_result = PhaseResult::Failed;
        let ctx = workflow_test_ctx(state);
        let result = execute_tool("workflow_set", r#"{"workflow_status":"completed"}"#, &ctx).await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("phase_result=passed"));
    }

    #[test]
    fn tool_definitions_include_fs_patch_schema() {
        let defs = tool_definitions();
        let functions = defs.as_array().expect("tool definitions array");
        let fs_patch = functions
            .iter()
            .find(|entry| entry["function"]["name"] == "fs_patch")
            .expect("fs_patch definition");

        assert_eq!(
            fs_patch["function"]["description"],
            "Apply a targeted standard unified-diff patch to existing text files. Use this for normal edits to existing text files. Patch headers must use project-relative a/path and b/path; absolute paths and .. traversal are unsupported. Do not use *** Begin Patch / *** Update File wrappers."
        );
        assert_eq!(
            fs_patch["function"]["parameters"]["required"],
            json!(["patch"])
        );
        assert_eq!(
            fs_patch["function"]["parameters"]["properties"]["patch"]["description"],
            "Standard unified diff, e.g. --- a/path, +++ b/path, and @@ hunks. Do not include *** Begin Patch wrappers."
        );

        let fs_write = functions
            .iter()
            .find(|entry| entry["function"]["name"] == "fs_write_file")
            .expect("fs_write_file definition");
        assert_eq!(
            fs_write["function"]["description"],
            "Create a new file. Replaces the entire target if it already exists; for normal edits to existing text files, use fs_patch instead."
        );
    }

    #[test]
    fn fs_patch_applies_markdown_wrapped_patch() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("src/main.rs");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(&file_path, "fn main() {\n    println!(\"old\");\n}\n").unwrap();

        let result = apply_fs_patch(
            r#"```diff
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!("old");
+    println!("new");
 }
```"#,
            dir.path(),
        );

        assert_eq!(result["ok"], json!(true));
        assert_eq!(result["changed_paths"], json!(["src/main.rs"]));
        assert_eq!(result["rejected_paths"], json!([]));
        assert_eq!(
            fs::read_to_string(&file_path).unwrap(),
            "fn main() {\n    println!(\"new\");\n}\n"
        );
    }

    #[test]
    fn fs_patch_preserves_crlf_line_endings() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, b"alpha\r\nbeta\r\n").unwrap();

        let result = apply_fs_patch(
            "--- a/notes.txt\n+++ b/notes.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+gamma\n",
            dir.path(),
        );

        assert_eq!(result["ok"], json!(true));
        assert_eq!(fs::read(&file_path).unwrap(), b"alpha\r\ngamma\r\n");
    }

    #[test]
    fn fs_patch_rejects_stale_context_atomically() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("a.txt");
        let second = dir.path().join("b.txt");
        fs::write(&first, "one\n").unwrap();
        fs::write(&second, "two\n").unwrap();

        let patch = "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+ONE\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-wrong\n+TWO\n";
        let result = apply_fs_patch(patch, dir.path());

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["changed_paths"], json!([]));
        assert_eq!(result["rejected_paths"], json!(["b.txt"]));
        assert_eq!(fs::read_to_string(&first).unwrap(), "one\n");
        assert_eq!(fs::read_to_string(&second).unwrap(), "two\n");
    }

    #[test]
    fn fs_patch_rejects_missing_file_with_target_path() {
        let dir = tempdir().unwrap();
        let result = apply_fs_patch(
            "--- a/missing.txt\n+++ b/missing.txt\n@@ -1 +1 @@\n-old\n+new\n",
            dir.path(),
        );

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["changed_paths"], json!([]));
        assert_eq!(result["rejected_paths"], json!(["missing.txt"]));
    }

    #[test]
    fn fs_patch_rejects_file_creation() {
        let dir = tempdir().unwrap();
        let result = apply_fs_patch(
            "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n",
            dir.path(),
        );

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["changed_paths"], json!([]));
        assert_eq!(result["rejected_paths"], json!(["new.txt"]));
    }

    #[test]
    fn fs_patch_rejects_file_deletion() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("delete-me.txt");
        fs::write(&file_path, "hello\n").unwrap();

        let result = apply_fs_patch(
            "--- a/delete-me.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-hello\n",
            dir.path(),
        );

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["changed_paths"], json!([]));
        assert_eq!(result["rejected_paths"], json!(["delete-me.txt"]));
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "hello\n");
    }

    #[test]
    fn fs_patch_allows_empty_final_file_without_deletion() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("empty.txt");
        fs::write(&file_path, "only\n").unwrap();

        let result = apply_fs_patch(
            "--- a/empty.txt\n+++ b/empty.txt\n@@ -1 +0,0 @@\n-only\n",
            dir.path(),
        );

        assert_eq!(result["ok"], json!(true));
        assert_eq!(result["changed_paths"], json!(["empty.txt"]));
        assert!(file_path.exists());
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "");
    }

    #[test]
    fn fs_patch_rejects_duplicate_targets() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("dup.txt");
        fs::write(&file_path, "one\ntwo\n").unwrap();

        let patch = "--- a/dup.txt\n+++ b/dup.txt\n@@ -1 +1 @@\n-one\n+ONE\n--- a/dup.txt\n+++ b/dup.txt\n@@ -2 +2 @@\n-two\n+TWO\n";
        let result = apply_fs_patch(patch, dir.path());

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["changed_paths"], json!([]));
        assert_eq!(result["rejected_paths"], json!(["dup.txt"]));
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn fs_patch_rejects_binary_targets() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("bin.dat");
        fs::write(&file_path, b"abc\0def").unwrap();

        let result = apply_fs_patch(
            "--- a/bin.dat\n+++ b/bin.dat\n@@ -1 +1 @@\n-abc\n+xyz\n",
            dir.path(),
        );

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["changed_paths"], json!([]));
        assert_eq!(result["rejected_paths"], json!(["bin.dat"]));
    }

    #[test]
    fn fs_patch_rejects_invalid_utf8_targets() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("utf8.dat");
        fs::write(&file_path, [0xff, 0xfe, 0xfd]).unwrap();

        let result = apply_fs_patch(
            "--- a/utf8.dat\n+++ b/utf8.dat\n@@ -1 +1 @@\n-old\n+new\n",
            dir.path(),
        );

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["changed_paths"], json!([]));
        assert_eq!(result["rejected_paths"], json!(["utf8.dat"]));
    }

    #[cfg(unix)]
    #[test]
    fn fs_patch_rejects_symlink_targets() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, "hello\n").unwrap();
        symlink(&target, &link).unwrap();

        let result = apply_fs_patch(
            "--- a/link.txt\n+++ b/link.txt\n@@ -1 +1 @@\n-hello\n+goodbye\n",
            dir.path(),
        );

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["changed_paths"], json!([]));
        assert_eq!(result["rejected_paths"], json!(["link.txt"]));
        assert_eq!(fs::read_to_string(&target).unwrap(), "hello\n");
    }
}
