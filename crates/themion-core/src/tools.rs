use crate::db::{
    CreateNoteArgs, DbHandle, NoteColumn, NoteKind, RecallArgs, RecallDirection, SearchArgs,
    SessionScope,
};
use crate::memory::{
    metadata_to_string, parse_hashtags_value, parse_nullable_string, CreateNodeArgs, HashtagMatch,
    LinkNodesArgs, OpenGraphArgs, SearchNodesArgs, UpdateNodeArgs, GLOBAL_PROJECT_DIR,
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
use libc::{ERANGE, _SC_GETPW_R_SIZE_MAX, getpwuid_r, getuid, passwd, sysconf};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::CStr;
use std::fs;
#[cfg(feature = "stylos")]
use std::future::Future;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
#[cfg(feature = "stylos")]
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
        memory_tool("memory_create_node", "Create a Project Memory node. Defaults to the current project; use project_dir=\"[GLOBAL]\" only for cross-project knowledge.", json!({
            "type":"object",
            "properties":{
                "node_id":{"type":"string","description":"Optional UUID. Generated when omitted."},
                "project_dir":{"type":"string","description":"Project context. Default: current project; use [GLOBAL] for cross-project knowledge."},
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
        memory_tool("memory_search", "Search Project Memory nodes by mode, query, project context, hashtags, type, and optional relation filters. Defaults to fts in the current project only; [GLOBAL] searches Global Knowledge only.", json!({
            "type":"object",
            "properties":{
                "query":{"type":"string","description":"Query text."},
                "mode":{"type":"string","enum":["fts","semantic"],"description":"Retrieval mode. Default: fts."},
                "project_dir":{"type":"string","description":"Project context. Default: current project; use [GLOBAL] for Global Knowledge."},
                "hashtags":{"type":"array","items":{"type":"string"}},
                "hashtag_match":{"type":"string","enum":["any","all"],"description":"Defaults to any."},
                "node_type":{"type":"string"},
                "relation_type":{"type":"string"},
                "linked_node_id":{"type":"string"},
                "limit":{"type":"integer","description":"Default 20, max 100."}
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
        memory_tool("memory_list_hashtags", "List Project Memory hashtags, optionally filtered by prefix. Defaults to the current project; [GLOBAL] means Global Knowledge.", json!({
            "type":"object",
            "properties":{
                "project_dir":{"type":"string","description":"Project context. Default: current project; use [GLOBAL] for Global Knowledge."},
                "prefix":{"type":"string"},
                "limit":{"type":"integer","description":"Default 50, max 200."}
            },
            "required":[]
        })),
    ]
}

#[cfg(feature = "stylos")]
type StylosToolFuture = Pin<Box<dyn Future<Output = Result<String>> + Send>>;
#[cfg(feature = "stylos")]
pub type StylosToolInvoker = Arc<dyn Fn(String, Value) -> StylosToolFuture + Send + Sync>;

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
    pub system_inspection: Option<SystemInspectionResult>,
}

fn resolve_board_target(
    requested_to_instance: &str,
    requested_to_agent_id: &str,
    #[cfg(feature = "stylos")] local_instance_id: Option<&str>,
    #[cfg(feature = "stylos")] local_agent_id: Option<&str>,
) -> Result<(String, String)> {
    if requested_to_instance != SELF_TARGET_KEYWORD && requested_to_agent_id != SELF_TARGET_KEYWORD
    {
        return Ok((
            requested_to_instance.to_string(),
            requested_to_agent_id.to_string(),
        ));
    }

    #[cfg(feature = "stylos")]
    {
        let to_instance = if requested_to_instance == SELF_TARGET_KEYWORD {
            local_instance_id
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("SELF requires known local instance id"))?
                .to_string()
        } else {
            requested_to_instance.to_string()
        };
        let to_agent_id = if requested_to_agent_id == SELF_TARGET_KEYWORD {
            local_agent_id
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("SELF requires known local agent id"))?
                .to_string()
        } else {
            requested_to_agent_id.to_string()
        };
        return Ok((to_instance, to_agent_id));
    }

    #[cfg(not(feature = "stylos"))]
    {
        anyhow::bail!("SELF target keyword is unavailable without stylos support");
    }
}

fn resolve_memory_project_dir(args: &Value, ctx: &ToolCtx) -> String {
    match args["project_dir"]
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(GLOBAL_PROJECT_DIR) => GLOBAL_PROJECT_DIR.to_string(),
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
                "description": "Write content to a file.",
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
                "description": "Retrieve earlier conversation messages from persistent history.",
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
                "name": "history_search",
                "description": "Full-text search across conversation history.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "FTS5 search query." },
                        "session_id": { "type": "string", "description": "Optional session selector. Omit for the active session, pass \"*\" for all sessions in the current project, or pass one session UUID in the current project." },
                        "limit": { "type": "integer", "description": "Max results. Default: 10, max: 100." }
                    },
                    "required": ["query"]
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
                "name": "workflow_set_active",
                "description": "Activate a workflow and reset to its start phase.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "workflow": { "type": "string", "description": "Workflow name." },
                        "reason": { "type": "string", "description": "Reason." }
                    },
                    "required": ["workflow"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "workflow_set_phase",
                "description": "Request a phase transition in the active workflow.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "phase": { "type": "string", "description": "Next phase." },
                        "reason": { "type": "string", "description": "Reason." }
                    },
                    "required": ["phase"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "workflow_set_phase_result",
                "description": "Set the current phase result.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "result": { "type": "string", "enum": ["passed", "failed", "user_feedback_required"] },
                        "reason": { "type": "string", "description": "Reason." }
                    },
                    "required": ["result"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_create_note",
                "description": "Create a durable board note. Use SELF for the current local instance or agent.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to_instance": { "type": "string", "description": "Target instance id or SELF." },
                        "to_agent_id": { "type": "string", "description": "Target agent id or SELF." },
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
                "description": "List board notes, optionally filtered by target or column.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to_instance": { "type": "string" },
                        "to_agent_id": { "type": "string" },
                        "column": { "type": "string", "enum": ["todo", "in_progress", "blocked", "done"] }
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
                "name": "board_move_note",
                "description": "Move a board note between columns.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "note_id": { "type": "string" },
                        "column": { "type": "string", "enum": ["todo", "in_progress", "blocked", "done"] }
                    },
                    "required": ["note_id", "column"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "board_update_note_result",
                "description": "Set a board note result.",
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
                "description": "Complete or fail the current workflow. Success requires phase_result=passed.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "outcome": { "type": "string", "enum": ["completed", "failed"] },
                        "reason": { "type": "string", "description": "Reason." }
                    },
                    "required": ["outcome"]
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
        stylos_tool("stylos_request_talk", "Send a user-style message to one instance. Sender identity is automatic; to_agent_id defaults to main.", json!({
            "type":"object","properties":{
                "instance":{"type":"string","description":"Target instance in <hostname>:<pid> form."},
                "to_agent_id":{"type":"string","description":"Target agent id. Default: main."},
                "message":{"type":"string"},
                "request_id":{"type":"string"},
                "wait_for_idle_timeout_ms":{"type":"integer","description":"Optional bounded wait in ms for target availability."}
            },"required":["instance","message"]
        })),
        stylos_tool("stylos_request_task", "Submit a task request for local agent routing.", json!({
            "type":"object","properties":{"instance":{"type":"string"},"task":{"type":"string"},"preferred_agent_id":{"type":"string"},"required_roles":{"type":"array","items":{"type":"string"}},"require_git_repo":{"type":"boolean"},"request_id":{"type":"string"}},"required":["instance","task"]
        })),
        stylos_tool("stylos_query_task_status", "Query a submitted task state.", json!({
            "type":"object","properties":{"instance":{"type":"string"},"task_id":{"type":"string"}},"required":["instance","task_id"]
        })),
        stylos_tool("stylos_query_task_result", "Wait for or retrieve a task result.", json!({
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
                from_instance: args["from_instance"].as_str().map(str::to_string),
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
            let column = args["column"]
                .as_str()
                .map(|v| parse_note_column(v).ok_or_else(|| anyhow::anyhow!("invalid column")))
                .transpose()?;
            let notes = ctx.db.list_board_notes(
                args["to_instance"].as_str(),
                args["to_agent_id"].as_str(),
                column,
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
            let note = ctx.db.move_board_note(note_id, column)?;
            Ok(match note {
                Some(note) => board_note_ack(
                    &note,
                    "update_result",
                    json!({
                        "has_result_text": note.result_text.is_some(),
                        "updated_at_ms": note.updated_at_ms,
                    }),
                )
                .to_string(),
                None => board_note_not_found(note_id, "update_result").to_string(),
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
                .update_board_note_result(note_id, Some(result_text))?;
            Ok(match note {
                Some(note) => board_note_to_json(&note).to_string(),
                None => json!({"found": false, "note_id": note_id}).to_string(),
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
        "memory_search" => {
            let hashtag_match = match args["hashtag_match"].as_str().unwrap_or("any") {
                value => HashtagMatch::from_str(value)
                    .ok_or_else(|| anyhow::anyhow!("invalid hashtag_match"))?,
            };
            let mode = match args["mode"].as_str().unwrap_or("fts") {
                value => crate::memory::MemorySearchMode::from_str(value)
                    .ok_or_else(|| anyhow::anyhow!("invalid memory search mode"))?,
            };
            let nodes = ctx.db.memory_store().search_nodes(SearchNodesArgs {
                query: args["query"].as_str().map(str::to_string),
                project_dir: resolve_memory_project_dir(&args, ctx),
                hashtags: parse_hashtags_value(args.get("hashtags"))?,
                hashtag_match,
                node_type: args["node_type"].as_str().map(str::to_string),
                relation_type: args["relation_type"].as_str().map(str::to_string),
                linked_node_id: args["linked_node_id"].as_str().map(str::to_string),
                limit: args["limit"].as_u64().map(|n| n as u32).unwrap_or(20),
                mode,
            })?;
            Ok(serde_json::to_string(&nodes)?)
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
        "memory_list_hashtags" => {
            let hashtags = ctx.db.memory_store().list_hashtags(
                &resolve_memory_project_dir(&args, ctx),
                args["prefix"].as_str(),
                args["limit"].as_u64().map(|n| n as u32).unwrap_or(50),
            )?;
            Ok(serde_json::to_string(&hashtags)?)
        }
        "history_search" | "search_history" => {
            let query = args["query"].as_str().unwrap_or("").to_string();
            let session_scope = match args["session_id"].as_str() {
                Some("*") => SessionScope::AllInCurrentProject,
                Some(value) => SessionScope::Exact(
                    Uuid::parse_str(value).map_err(|_| anyhow::anyhow!("invalid session_id"))?,
                ),
                None => SessionScope::Exact(ctx.session_id),
            };
            let limit = args["limit"].as_u64().map(|n| n as u32).unwrap_or(10);
            match ctx.db.search(SearchArgs {
                query,
                session_scope,
                current_project_dir: ctx.project_dir.clone(),
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
