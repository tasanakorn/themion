use crate::db::{
    CreateNoteArgs, DbHandle, NoteColumn, NoteKind, RecallArgs, RecallDirection, SearchArgs,
};
use crate::workflow::{
    allowed_transitions, can_retry_current_phase, can_retry_previous_phase, can_transition,
    normalize_workflow_name, phase_instructions, previous_phase, start_phase_for_workflow,
    PhaseResult, WorkflowState, WorkflowStatus, DEFAULT_AGENT,
};
use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
#[cfg(feature = "stylos")]
use std::future::Future;
use std::path::PathBuf;
#[cfg(feature = "stylos")]
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

fn parse_note_column(value: &str) -> Option<NoteColumn> {
    NoteColumn::from_str(value)
}

fn stylos_note_to_json(note: &crate::db::StylosNote) -> Value {
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
        "injection_state": match note.injection_state { crate::db::NoteInjectionState::Pending => "pending", crate::db::NoteInjectionState::Injected => "injected" },
        "created_at_ms": note.created_at_ms,
        "updated_at_ms": note.updated_at_ms,
        "injected_at_ms": note.injected_at_ms,
    })
}

#[cfg(feature = "stylos")]
type StylosToolFuture = Pin<Box<dyn Future<Output = Result<String>> + Send>>;
#[cfg(feature = "stylos")]
pub type StylosToolInvoker = Arc<dyn Fn(String, Value) -> StylosToolFuture + Send + Sync>;

const MAX_SLEEP_MS: u64 = 30_000;

pub struct ToolCtx {
    pub db: Arc<DbHandle>,
    pub session_id: Uuid,
    pub project_dir: PathBuf,
    pub workflow_state: Option<WorkflowState>,
    pub turn_seq: Option<u32>,
    #[cfg(feature = "stylos")]
    pub local_agent_id: Option<String>,
    #[cfg(feature = "stylos")]
    pub stylos_tool_invoker: Option<StylosToolInvoker>,
    #[cfg(feature = "stylos")]
    pub stylos_enabled: bool,
}

pub fn tool_definitions() -> Value {
    let mut defs = vec![
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
            "type": "function",
            "function": {
                "name": "time_sleep",
                "description": "Sleep for a short bounded duration without invoking the shell.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ms": { "type": "integer", "description": "Milliseconds to sleep. Max 30000." }
                    },
                    "required": ["ms"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "history_recall",
                "description": "Retrieve earlier conversation messages from persistent history.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "UUID of session. Optional; omitted means use the active session." },
                        "project_dir": { "type": "string", "description": "Filter by project directory." },
                        "limit": { "type": "integer", "description": "Max messages (default 20, max 200)." },
                        "direction": { "type": "string", "enum": ["newest", "oldest"] }
                    },
                    "required": []
                }
            }
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_create_note",
                "description": "Create a durable board note targeted to an instance and agent. When Stylos is enabled, creation uses the Stylos receiver-intake flow rather than direct local DB insertion.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to_instance": { "type": "string" },
                        "to_agent_id": { "type": "string" },
                        "body": { "type": "string" },
                        "note_kind": { "type": "string", "enum": ["work_request", "done_mention"], "description": "Optional note semantics. Defaults to work_request." },
                        "origin_note_id": { "type": "string", "description": "Optional original note reference, used for done mentions." },
                        "from_instance": { "type": "string" },
                        "from_agent_id": { "type": "string" },
                        "request_id": { "type": "string", "description": "Optional request identifier for Stylos-mediated creation." },
                        "note_id": { "type": "string", "description": "Optional UUID note identifier. When omitted, one is generated by the receiver or local DB path as applicable." }
                    },
                    "required": ["to_instance", "to_agent_id", "body"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_list_notes",
                "description": "List durable local board notes, optionally filtered by target instance, agent, or column.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to_instance": { "type": "string" },
                        "to_agent_id": { "type": "string" },
                        "column": { "type": "string", "enum": ["todo", "in_progress", "done"] }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_read_note",
                "description": "Read one durable local board note by note_id.",
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
                "name": "board_move_note",
                "description": "Move a durable local board note between todo, in_progress, and done.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "note_id": { "type": "string" },
                        "column": { "type": "string", "enum": ["todo", "in_progress", "done"] }
                    },
                    "required": ["note_id", "column"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_update_note_result",
                "description": "Attach or update result text on a durable local board note.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "note_id": { "type": "string" },
                        "result_text": { "type": "string" }
                    },
                    "required": ["note_id", "result_text"]
                }
            }
        }),
        json!({
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
        }),
    ];

    #[cfg(feature = "stylos")]
    defs.extend(stylos_tool_definitions());

    Value::Array(defs)
}

#[cfg(feature = "stylos")]
fn stylos_tool_definitions() -> Vec<Value> {
    vec![
        stylos_tool("stylos_query_agents_alive", "Ask which Themion instances and agents are currently alive.", json!({
            "type":"object",
            "properties":{
                "exclude_self":{"type":"boolean","description":"Whether to exclude the current Themion instance from discovery results. Defaults to true."}
            },
            "required":[]
        })),
        stylos_tool("stylos_query_agents_free", "Ask which agents are currently free for new work.", json!({
            "type":"object",
            "properties":{
                "exclude_self":{"type":"boolean","description":"Whether to exclude the current Themion instance from discovery results. Defaults to true."}
            },
            "required":[]
        })),
        stylos_tool("stylos_query_agents_git", "Ask which agents are attached to git repositories, optionally matching a specific repo identity. Prefer remote in normalized form like <host>/<owner>/<repo> when you can infer it safely, for example github.com/tasanakorn/stele. If the user names a supported forge explicitly, you may normalize before calling. If the host is omitted and there is no documented safe default, ask for clarification instead of guessing. Do not rely on responders to parse conversational phrases.", json!({
            "type":"object","properties":{
                "remote":{"type":"string","description":"Optional git selector. Prefer normalized comparable identity like <host>/<owner>/<repo>; raw remotes are also accepted when needed."},
                "exclude_self":{"type":"boolean","description":"Whether to exclude the current Themion instance from discovery results. Defaults to true."}
            },"required":[]
        })),
        stylos_tool("stylos_query_nodes", "Ask which Themion nodes are visible on the Stylos network.", json!({"type":"object","properties":{},"required":[]})),
        stylos_tool("stylos_query_status", "Ask one instance for its current process and agent status. Optional agent_id and role filters may be provided independently or together.", json!({
            "type":"object","properties":{"instance":{"type":"string"},"agent_id":{"type":"string"},"role":{"type":"string"}},"required":["instance"]
        })),
        stylos_tool("stylos_request_talk", "Submit a sender-aware user-style message to one target instance. Sender identity is resolved automatically by the app; do not pass sender fields. Optional to_agent_id selects the target local agent and defaults to main.", json!({
            "type":"object","properties":{
                "instance":{"type":"string","description":"Target instance identifier in exact <hostname>:<pid> form."},
                "to_agent_id":{"type":"string","description":"Optional target agent id on the remote instance. Defaults to main."},
                "message":{"type":"string"},
                "request_id":{"type":"string"},
                "wait_for_idle_timeout_ms":{"type":"integer","description":"Optional bounded wait in milliseconds for target availability."}
            },"required":["instance","message"]
        })),
        stylos_tool("stylos_request_task", "Submit a structured task request for local agent routing.", json!({
            "type":"object","properties":{"instance":{"type":"string"},"task":{"type":"string"},"preferred_agent_id":{"type":"string"},"required_roles":{"type":"array","items":{"type":"string"}},"require_git_repo":{"type":"boolean"},"request_id":{"type":"string"}},"required":["instance","task"]
        })),
        stylos_tool("stylos_query_task_status", "Look up the current lifecycle state of a submitted task.", json!({
            "type":"object","properties":{"instance":{"type":"string"},"task_id":{"type":"string"}},"required":["instance","task_id"]
        })),
        stylos_tool("stylos_query_task_result", "Wait for or retrieve the result of a submitted task.", json!({
            "type":"object","properties":{"instance":{"type":"string"},"task_id":{"type":"string"},"wait_timeout_ms":{"type":"integer"}},"required":["instance","task_id"]
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
        "board_create_note" => {
            let to_instance = args["to_instance"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing to_instance"))?;
            let to_agent_id = args["to_agent_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing to_agent_id"))?;
            let body = args["body"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing body"))?;

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
            let note_kind = match args["note_kind"].as_str().unwrap_or("work_request") {
                "work_request" => NoteKind::WorkRequest,
                "done_mention" => NoteKind::DoneMention,
                other => anyhow::bail!("invalid note_kind: {other}"),
            };
            let note = ctx.db.create_stylos_note(CreateNoteArgs {
                note_id,
                note_kind,
                origin_note_id: args["origin_note_id"].as_str().map(str::to_string),
                from_instance: args["from_instance"].as_str().map(str::to_string),
                from_agent_id: args["from_agent_id"].as_str().map(str::to_string),
                to_instance: to_instance.to_string(),
                to_agent_id: to_agent_id.to_string(),
                body: body.to_string(),
            })?;
            Ok(stylos_note_to_json(&note).to_string())
        }
        "board_list_notes" => {
            let column = args["column"]
                .as_str()
                .map(|v| parse_note_column(v).ok_or_else(|| anyhow::anyhow!("invalid column")))
                .transpose()?;
            let notes = ctx.db.list_stylos_notes(
                args["to_instance"].as_str(),
                args["to_agent_id"].as_str(),
                column,
            )?;
            Ok(Value::Array(notes.iter().map(stylos_note_to_json).collect()).to_string())
        }
        "board_read_note" => {
            let note_id = args["note_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing note_id"))?;
            let note = ctx.db.get_stylos_note(note_id)?;
            Ok(match note {
                Some(note) => stylos_note_to_json(&note).to_string(),
                None => json!({"found": false, "note_id": note_id}).to_string(),
            })
        }
        "board_move_note" => {
            let note_id = args["note_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing note_id"))?;
            let column = parse_note_column(
                args["column"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing column"))?,
            )
            .ok_or_else(|| anyhow::anyhow!("invalid column"))?;
            let note = ctx.db.move_stylos_note(note_id, column)?;
            Ok(match note {
                Some(note) => stylos_note_to_json(&note).to_string(),
                None => json!({"found": false, "note_id": note_id}).to_string(),
            })
        }
        "board_update_note_result" => {
            let note_id = args["note_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing note_id"))?;
            let result_text = args["result_text"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing result_text"))?;
            let note = ctx
                .db
                .update_stylos_note_result(note_id, Some(result_text))?;
            Ok(match note {
                Some(note) => stylos_note_to_json(&note).to_string(),
                None => json!({"found": false, "note_id": note_id}).to_string(),
            })
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
        #[cfg(feature = "stylos")]
        "stylos_query_agents_alive"
        | "stylos_query_agents_free"
        | "stylos_query_agents_git"
        | "stylos_query_nodes"
        | "stylos_query_status"
        | "stylos_request_talk"
        | "stylos_request_task"
        | "stylos_query_task_status"
        | "stylos_query_task_result" => {
            let invoker = ctx
                .stylos_tool_invoker
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("stylos tools unavailable"))?;
            invoker(name.to_string(), args).await
        }
        "workflow_get_state" | "get_workflow_state" => {
            let state = ctx
                .workflow_state
                .clone()
                .unwrap_or_else(WorkflowState::default);
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
            let phase = args["phase"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing phase"))?;
            let state = ctx
                .workflow_state
                .clone()
                .unwrap_or_else(WorkflowState::default);
            if !can_transition(&state.workflow_name, &state.phase_name, phase) {
                anyhow::bail!(
                    "cannot transition workflow {} from {} to {}",
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
                "retry_state": state.retry_state,
            })
            .to_string())
        }
        "workflow_set_phase_result" => {
            let result = args["result"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing result"))?;
            let normalized = match result {
                "passed" => PhaseResult::Passed,
                "failed" => PhaseResult::Failed,
                "user_feedback_required" => PhaseResult::UserFeedbackRequired,
                other => anyhow::bail!("invalid phase result: {other}"),
            };
            let state = ctx
                .workflow_state
                .clone()
                .unwrap_or_else(WorkflowState::default);
            Ok(json!({
                "workflow": state.workflow_name,
                "phase": state.phase_name,
                "status": state.status,
                "phase_result": normalized,
                "agent": state.agent_name,
                "reason": args["reason"].as_str(),
                "retry_state": state.retry_state,
            })
            .to_string())
        }
        "workflow_complete" | "complete_workflow" => {
            let outcome = args["outcome"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing outcome"))?;
            let state = ctx
                .workflow_state
                .clone()
                .unwrap_or_else(WorkflowState::default);
            match outcome {
                "completed" => {
                    if state.phase_result != PhaseResult::Passed {
                        anyhow::bail!("workflow can only complete successfully when current phase_result=passed");
                    }
                    Ok(json!({
                        "workflow": state.workflow_name,
                        "phase": state.phase_name,
                        "status": WorkflowStatus::Completed,
                        "phase_result": state.phase_result,
                        "agent": state.agent_name,
                        "reason": args["reason"].as_str(),
                        "retry_state": state.retry_state,
                    })
                    .to_string())
                }
                "failed" => Ok(json!({
                    "workflow": state.workflow_name,
                    "phase": state.phase_name,
                    "status": WorkflowStatus::Failed,
                    "phase_result": state.phase_result,
                    "agent": state.agent_name,
                    "reason": args["reason"].as_str(),
                    "retry_state": state.retry_state,
                })
                .to_string()),
                other => anyhow::bail!("invalid outcome: {other}"),
            }
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}
