use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;
use crate::db::{DbHandle, RecallArgs, RecallDirection, SearchArgs};

pub struct ToolCtx {
    pub db: Arc<DbHandle>,
    pub session_id: Uuid,
    pub project_dir: PathBuf,
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "read_file",
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
                "name": "write_file",
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
                "name": "list_directory",
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
                "name": "bash",
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
                "name": "recall_history",
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
                "name": "search_history",
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
        "read_file" => {
            let path = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let path = ctx.project_dir.join(path);
            let content = fs::read_to_string(path)?;
            Ok(content)
        }
        "write_file" => {
            let path = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let path = ctx.project_dir.join(path);
            let content = args["content"].as_str().ok_or_else(|| anyhow::anyhow!("missing content"))?;
            fs::write(path, content)?;
            Ok(format!("Written"))
        }
        "list_directory" => {
            let path = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let path = ctx.project_dir.join(path);
            let entries: Vec<String> = fs::read_dir(path)?
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            Ok(entries.join("\n"))
        }
        "bash" => {
            let command = args["command"].as_str().ok_or_else(|| anyhow::anyhow!("missing command"))?;
            // TODO: add timeout
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
        "recall_history" => {
            let session_id = args["session_id"].as_str().and_then(|s| Uuid::parse_str(s).ok())
                .or(Some(ctx.session_id));
            let project_dir = args["project_dir"].as_str().map(PathBuf::from)
                .or_else(|| Some(ctx.project_dir.clone()));
            let limit = args["limit"].as_u64().map(|n| n as u32).unwrap_or(20);
            let direction = match args["direction"].as_str() {
                Some("oldest") => RecallDirection::Oldest,
                _ => RecallDirection::Newest,
            };
            match ctx.db.recall(RecallArgs { session_id, project_dir, limit, direction }) {
                Ok(msgs) => Ok(serde_json::to_string(&msgs.iter().map(|m| serde_json::json!({
                    "turn_seq": m.turn_seq, "role": m.role, "content": m.content,
                    "tool_calls": m.tool_calls_json, "tool_call_id": m.tool_call_id,
                })).collect::<Vec<_>>()).unwrap_or_default()),
                Err(e) => Ok(format!("Error: {e}")),
            }
        }
        "search_history" => {
            let query = args["query"].as_str().unwrap_or("").to_string();
            let session_id = args["session_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());
            let project_dir = args["project_dir"].as_str().map(PathBuf::from)
                .or_else(|| Some(ctx.project_dir.clone()));
            let limit = args["limit"].as_u64().map(|n| n as u32).unwrap_or(10);
            match ctx.db.search(SearchArgs { query, session_id, project_dir, limit }) {
                Ok(hits) => Ok(serde_json::to_string(&hits.iter().map(|h| serde_json::json!({
                    "session_id": h.session_id, "turn_seq": h.turn_seq,
                    "role": h.role, "snippet": h.snippet,
                })).collect::<Vec<_>>()).unwrap_or_default()),
                Err(e) => Ok(format!("Error: {e}")),
            }
        }
        _ => Err(anyhow::anyhow!("unknown tool: {name}")),
    }
}
