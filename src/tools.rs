use anyhow::Result;
use serde_json::{json, Value};
use std::fs;

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
        }
    ])
}

pub async fn call_tool(name: &str, args_json: &str) -> String {
    match execute_tool(name, args_json).await {
        Ok(output) => output,
        Err(e) => format!("Error: {e}"),
    }
}

async fn execute_tool(name: &str, args_json: &str) -> Result<String> {
    let args: Value = serde_json::from_str(args_json)?;

    match name {
        "read_file" => {
            let path = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let content = fs::read_to_string(path)?;
            Ok(content)
        }
        "write_file" => {
            let path = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let content = args["content"].as_str().ok_or_else(|| anyhow::anyhow!("missing content"))?;
            fs::write(path, content)?;
            Ok(format!("Written to {path}"))
        }
        "list_directory" => {
            let path = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
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
                .output()
                .await?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            Ok(format!("{stdout}{stderr}"))
        }
        _ => Err(anyhow::anyhow!("unknown tool: {name}")),
    }
}
