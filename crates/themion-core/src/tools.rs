use crate::db::{DbHandle, RecallArgs, RecallDirection, SearchArgs};
use crate::workflow::{
    allowed_transitions, can_retry_current_phase, can_retry_previous_phase, can_transition,
    normalize_workflow_name, phase_instructions, previous_phase, start_phase_for_workflow,
    PhaseResult, WorkflowState, WorkflowStatus, DEFAULT_AGENT,
};
use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

pub struct ToolCtx {
    pub db: Arc<DbHandle>,
    pub session_id: Uuid,
    pub project_dir: PathBuf,
    pub workflow_state: Option<WorkflowState>,
    pub turn_seq: Option<u32>,
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "fs_read_file",
                "description": "Read file contents",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to read" }
                    },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "fs_write_file",
                "description": "Write content to a file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to write" },
                        "content": { "type": "string", "description": "Content to write" }
                    },
                    "required": ["path", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "fs_list_directory",
                "description": "List directory entries",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path to list" }
                    },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "shell_run_command",
                "description": "Run a shell command, returns stdout+stderr",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Shell command to run" }
                    },
                    "required": ["command"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "history_recall",
                "description": "Retrieve earlier conversation messages from persistent history.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "UUID of session. Defaults to current." },
                        "project_dir": { "type": "string", "description": "Filter by project directory." },
                        "limit": { "type": "integer", "description": "Max messages (default 20, max 200)." },
                        "direction": { "type": "string", "enum": ["newest", "oldest"] }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "history_search",
                "description": "Full-text search across conversation history.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "FTS5 search query." },
                        "session_id": { "type": "string", "description": "Limit to session UUID." },
                        "project_dir": { "type": "string", "description": "Limit to project directory." },
                        "limit": { "type": "integer", "description": "Max results (default 10, max 100)." }
                    },
                    "required": ["query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "workflow_get_state",
                "description": "Return the current workflow, phase, status, phase result, and allowed next transitions.",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "workflow_set_active",
                "description": "Activate a named built-in workflow. Resets the current phase to that workflow's start phase.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "workflow": { "type": "string", "description": "Built-in workflow name, such as NORMAL or LITE." },
                        "reason": { "type": "string", "description": "Reason for switching workflows." }
                    },
                    "required": ["workflow"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "workflow_set_phase",
                "description": "Request a phase transition within the currently active workflow.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "phase": { "type": "string", "description": "Next phase within the active workflow." },
                        "reason": { "type": "string", "description": "Reason for changing phase." }
                    },
                    "required": ["phase"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "workflow_set_phase_result",
                "description": "Set the current phase result to passed, failed, or user_feedback_required before transitioning or completing the workflow.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "result": { "type": "string", "enum": ["passed", "failed", "user_feedback_required"] },
                        "reason": { "type": "string", "description": "Reason for this phase result." }
                    },
                    "required": ["result"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "workflow_complete",
                "description": "Mark the current workflow as passed/completed or failed. Completed requires current phase_result=passed.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "outcome": { "type": "string", "enum": ["completed", "failed"] },
                        "reason": { "type": "string", "description": "Reason for completion/failure." }
                    },
                    "required": ["outcome"]
                }
            }
        }
    ])
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
            let path = args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let path = ctx.project_dir.join(path);
            let content = fs::read_to_string(path)?;
            Ok(content)
        }
        "fs_write_file" | "write_file" => {
            let path = args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let path = ctx.project_dir.join(path);
            let content = args["content"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing content"))?;
            fs::write(path, content)?;
            Ok(format!("Written"))
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
            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&ctx.project_dir)
                .output()
                .await?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            Ok(format!("{stdout}{stderr}"))
        }
        "history_recall" | "recall_history" => {
            let session_id = args["session_id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .or(Some(ctx.session_id));
            let project_dir = args["project_dir"]
                .as_str()
                .map(PathBuf::from)
                .or_else(|| Some(ctx.project_dir.clone()));
            let limit = args["limit"].as_u64().map(|n| n as u32).unwrap_or(20);
            let direction = match args["direction"].as_str() {
                Some("oldest") => RecallDirection::Oldest,
                _ => RecallDirection::Newest,
            };
            match ctx.db.recall(RecallArgs {
                session_id,
                project_dir,
                limit,
                direction,
            }) {
                Ok(msgs) => Ok(serde_json::to_string(
                    &msgs
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "turn_seq": m.turn_seq, "role": m.role, "content": m.content,
                                "tool_calls": m.tool_calls_json, "tool_call_id": m.tool_call_id,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default()),
                Err(e) => Ok(format!("Error: {e}")),
            }
        }
        "history_search" | "search_history" => {
            let query = args["query"].as_str().unwrap_or("").to_string();
            let session_id = args["session_id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok());
            let project_dir = args["project_dir"]
                .as_str()
                .map(PathBuf::from)
                .or_else(|| Some(ctx.project_dir.clone()));
            let limit = args["limit"].as_u64().map(|n| n as u32).unwrap_or(10);
            match ctx.db.search(SearchArgs {
                query,
                session_id,
                project_dir,
                limit,
            }) {
                Ok(hits) => Ok(serde_json::to_string(
                    &hits
                        .iter()
                        .map(|h| {
                            serde_json::json!({
                                "session_id": h.session_id, "turn_seq": h.turn_seq,
                                "role": h.role, "snippet": h.snippet,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default()),
                Err(e) => Ok(format!("Error: {e}")),
            }
        }
        "workflow_get_state" | "get_workflow_state" => {
            let state = ctx.workflow_state.clone().unwrap_or_else(WorkflowState::default);
            Ok(json!({
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
            .to_string())
        }
        "workflow_set_active" | "set_workflow" => {
            let workflow = args["workflow"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing workflow"))?;
            let workflow = normalize_workflow_name(workflow)
                .ok_or_else(|| anyhow::anyhow!("unknown workflow: {workflow}"))?;
            let start_phase = start_phase_for_workflow(workflow)
                .ok_or_else(|| anyhow::anyhow!("workflow missing start phase: {workflow}"))?;
            Ok(json!({
                "workflow": workflow,
                "phase": start_phase,
                "status": WorkflowStatus::Running,
                "phase_result": PhaseResult::Pending,
                "agent": DEFAULT_AGENT,
                "reason": args["reason"].as_str(),
                "retry_state": WorkflowState::default().retry_state,
            })
            .to_string())
        }
        "workflow_set_phase" | "set_workflow_phase" => {
            let state = ctx
                .workflow_state
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("workflow state unavailable"))?;
            let phase = args["phase"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing phase"))?;
            if state.phase_result != PhaseResult::Passed {
                anyhow::bail!("current phase result must be passed before transitioning phases");
            }
            if !can_transition(&state.workflow_name, &state.phase_name, phase) {
                anyhow::bail!(
                    "invalid phase transition: {}:{} -> {}",
                    state.workflow_name,
                    state.phase_name,
                    phase
                );
            }
            Ok(json!({
                "workflow": state.workflow_name,
                "phase": phase,
                "status": WorkflowStatus::Running,
                "phase_result": PhaseResult::Pending,
                "agent": state.agent_name,
                "reason": args["reason"].as_str(),
                "retry_state": WorkflowState::default().retry_state,
            })
            .to_string())
        }
        "workflow_set_phase_result" | "set_phase_result" => {
            let state = ctx
                .workflow_state
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("workflow state unavailable"))?;
            let result = match args["result"].as_str().ok_or_else(|| anyhow::anyhow!("missing result"))? {
                "passed" => PhaseResult::Passed,
                "failed" => PhaseResult::Failed,
                "user_feedback_required" => PhaseResult::UserFeedbackRequired,
                other => anyhow::bail!("invalid result: {other}"),
            };
            Ok(json!({
                "workflow": state.workflow_name,
                "phase": state.phase_name,
                "status": if result == PhaseResult::UserFeedbackRequired { WorkflowStatus::WaitingUser } else { state.status },
                "phase_result": result,
                "agent": state.agent_name,
                "reason": args["reason"].as_str(),
                "retry_state": state.retry_state,
            })
            .to_string())
        }
        "workflow_complete" | "complete_workflow" => {
            let state = ctx
                .workflow_state
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("workflow state unavailable"))?;
            let outcome = args["outcome"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing outcome"))?;
            let status = match outcome {
                "completed" => {
                    if state.phase_result != PhaseResult::Passed {
                        anyhow::bail!("current phase result must be passed before completing workflow");
                    }
                    WorkflowStatus::Completed
                }
                "failed" => WorkflowStatus::Failed,
                _ => anyhow::bail!("invalid outcome: {outcome}"),
            };
            Ok(json!({
                "workflow": state.workflow_name,
                "phase": state.phase_name,
                "status": status,
                "phase_result": state.phase_result,
                "agent": state.agent_name,
                "reason": args["reason"].as_str(),
                "retry_state": state.retry_state,
            })
            .to_string())
        }
        _ => Err(anyhow::anyhow!("unknown tool: {name}")),
    }
}
