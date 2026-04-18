use anyhow::{anyhow, Result};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth::CodexAuth;
use crate::client::{
    ChatBackend, FunctionCall, Message, ResponseMessage, ToolCall, Usage, UsageDetails,
};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

#[derive(Debug, Clone)]
pub struct RateLimitWindow {
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RateLimitSnapshot {
    pub primary: Option<RateLimitWindow>,
    pub secondary: Option<RateLimitWindow>,
    pub credits: Option<CreditsSnapshot>,
}

pub struct CodexClient {
    http: reqwest::Client,
    base_url: String,
    auth: Arc<RwLock<CodexAuth>>,
    auth_writer: Box<dyn Fn(&CodexAuth) -> Result<()> + Send + Sync>,
}

impl CodexClient {
    pub fn new(
        base_url: String,
        auth: CodexAuth,
        auth_writer: Box<dyn Fn(&CodexAuth) -> Result<()> + Send + Sync>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            auth: Arc::new(RwLock::new(auth)),
            auth_writer,
        }
    }

    async fn ensure_fresh_token(&self) -> Result<()> {
        {
            let guard = self.auth.read().await;
            if !guard.is_expired(60) {
                return Ok(());
            }
        }
        let mut guard = self.auth.write().await;
        if !guard.is_expired(60) {
            return Ok(());
        }

        let refresh_token = guard.refresh_token.clone();
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh_token),
            ("client_id", CLIENT_ID),
        ];
        let resp = self.http.post(TOKEN_URL).form(&params).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await?;
            return Err(anyhow!("token refresh failed {status}: {text}"));
        }

        let json: Value = resp.json().await?;
        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| anyhow!("missing access_token in refresh response"))?
            .to_string();
        let new_refresh_token = json["refresh_token"]
            .as_str()
            .unwrap_or(&refresh_token)
            .to_string();
        let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let expires_at = now + expires_in;

        let new_auth = CodexAuth {
            access_token,
            refresh_token: new_refresh_token,
            expires_at,
            account_id: guard.account_id.clone(),
        };
        (self.auth_writer)(&new_auth)?;
        *guard = new_auth;
        Ok(())
    }

    pub async fn get_rate_limits(&self) -> Result<RateLimitSnapshot> {
        self.ensure_fresh_token().await?;

        let (access_token, account_id) = {
            let guard = self.auth.read().await;
            (guard.access_token.clone(), guard.account_id.clone())
        };

        let mut normalized_base = self.base_url.trim_end_matches('/').to_string();
        if (normalized_base.starts_with("https://chatgpt.com")
            || normalized_base.starts_with("https://chat.openai.com"))
            && !normalized_base.contains("/backend-api")
        {
            normalized_base.push_str("/backend-api");
        }

        let url = if normalized_base.contains("/backend-api") {
            format!("{}/wham/usage", normalized_base)
        } else {
            format!("{}/api/codex/usage", normalized_base)
        };

        let response = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("chatgpt-account-id", &account_id)
            .header("originator", "pi")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            return Err(anyhow!("Codex rate-limit API error {status}: {text}"));
        }

        let json: Value = response.json().await?;
        Ok(parse_rate_limit_snapshot(&json))
    }
}

fn parse_rate_limit_snapshot(json: &Value) -> RateLimitSnapshot {
    let rate_limit = json
        .get("rate_limit")
        .and_then(|v| v.as_object());

    let primary = rate_limit.and_then(|rate_limit| {
        parse_usage_window(rate_limit.get("primary_window"))
    });
    let secondary = rate_limit.and_then(|rate_limit| {
        parse_usage_window(rate_limit.get("secondary_window"))
    });
    let credits = json.get("credits").and_then(parse_credits);

    RateLimitSnapshot {
        primary,
        secondary,
        credits,
    }
}

fn parse_usage_window(value: Option<&Value>) -> Option<RateLimitWindow> {
    let value = value?;
    let value = value.as_object()?;
    let used_percent = value.get("used_percent")?.as_f64()?;
    let window_seconds = value.get("limit_window_seconds").and_then(Value::as_i64);
    let window_minutes = window_seconds.map(|seconds| (seconds + 59) / 60);
    let resets_at = value.get("reset_at").and_then(Value::as_i64);
    Some(RateLimitWindow {
        used_percent,
        window_minutes,
        resets_at,
    })
}

fn parse_credits(value: &Value) -> Option<CreditsSnapshot> {
    let value = value.as_object()?;
    Some(CreditsSnapshot {
        has_credits: value
            .get("has_credits")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        unlimited: value
            .get("unlimited")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        balance: value
            .get("balance")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

fn translate_messages(messages: &[Message]) -> (Option<String>, Vec<Value>) {
    let mut instructions: Option<String> = None;
    let mut items: Vec<Value> = Vec::new();
    let mut first_system = true;

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                let text = msg.content.as_deref().unwrap_or("");
                if first_system {
                    instructions = Some(text.to_string());
                    first_system = false;
                } else {
                    items.push(serde_json::json!({
                        "type": "message",
                        "role": "developer",
                        "content": [{"type": "input_text", "text": text}]
                    }));
                }
            }
            "user" => {
                let text = msg.content.as_deref().unwrap_or("");
                items.push(serde_json::json!({
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": text}]
                }));
            }
            "assistant" => {
                if let Some(ref tcs) = msg.tool_calls {
                    if let Some(ref c) = msg.content {
                        if !c.is_empty() {
                            items.push(serde_json::json!({
                                "type": "message",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": c}]
                            }));
                        }
                    }
                    for tc in tcs {
                        items.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": tc.function.name,
                            "arguments": tc.function.arguments,
                        }));
                    }
                } else {
                    let text = msg.content.as_deref().unwrap_or("");
                    items.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": text}]
                    }));
                }
            }
            "tool" => {
                let output = msg.content.as_deref().unwrap_or("");
                let call_id = msg.tool_call_id.as_deref().unwrap_or("");
                items.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
            _ => {}
        }
    }

    (instructions, items)
}

fn translate_tools(tools: &Value) -> Value {
    let arr = match tools.as_array() {
        Some(a) => a,
        None => return serde_json::json!([]),
    };
    let translated: Vec<Value> = arr
        .iter()
        .map(|t| {
            if t["type"] == "function" {
                let f = &t["function"];
                serde_json::json!({
                    "type": "function",
                    "name": f["name"],
                    "description": f["description"],
                    "parameters": f["parameters"],
                })
            } else {
                t.clone()
            }
        })
        .collect();
    Value::Array(translated)
}

struct ToolCallSlot {
    id: String,
    name: String,
    arguments: String,
}

#[async_trait::async_trait]
impl ChatBackend for CodexClient {
    async fn chat_completion_stream(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
        mut on_chunk: Box<dyn FnMut(String) + Send + 'static>,
    ) -> Result<(ResponseMessage, Option<Usage>)> {
        self.ensure_fresh_token().await?;

        let (access_token, account_id) = {
            let guard = self.auth.read().await;
            (guard.access_token.clone(), guard.account_id.clone())
        };

        let (instructions, input_items) = translate_messages(messages);
        let translated_tools = translate_tools(tools);

        let mut body = serde_json::json!({
            "model": model,
            "store": false,
            "input": input_items,
            "tools": translated_tools,
            "stream": true,
        });
        if let Some(ref instr) = instructions {
            body["instructions"] = Value::String(instr.clone());
        }

        let response = self
            .http
            .post(format!("{}/responses", self.base_url))
            .header("Authorization", format!("Bearer {}", access_token))
            .header("chatgpt-account-id", &account_id)
            .header("originator", "pi")
            .header("OpenAI-Beta", "responses=experimental")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            return Err(anyhow!("Codex API error {status}: {text}"));
        }

        let mut buf: Vec<u8> = Vec::new();
        let mut content = String::new();
        let mut tool_slots: std::collections::HashMap<String, ToolCallSlot> =
            std::collections::HashMap::new();
        let mut usage: Option<Usage> = None;
        let mut done = false;

        let mut current_event: Option<String> = None;
        let mut current_data: Option<String> = None;

        let mut response = response;

        while !done {
            let Some(bytes) = response.chunk().await? else {
                break;
            };
            buf.extend_from_slice(&bytes);

            loop {
                let Some(pos) = buf.iter().position(|&b| b == b'\n') else {
                    break;
                };
                let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                let line_bytes = line_bytes
                    .strip_suffix(b"\r\n")
                    .or_else(|| line_bytes.strip_suffix(b"\n"))
                    .unwrap_or(&line_bytes);
                let Ok(line) = std::str::from_utf8(line_bytes) else {
                    continue;
                };

                if line.is_empty() {
                    if let (Some(event_type), Some(data_str)) =
                        (current_event.take(), current_data.take())
                    {
                        if data_str == "[DONE]" {
                            done = true;
                            break;
                        }
                        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
                        match event_type.as_str() {
                            "response.output_text.delta" => {
                                if let Some(delta) = data["delta"].as_str() {
                                    content.push_str(delta);
                                    on_chunk(delta.to_string());
                                }
                            }
                            "response.function_call_arguments.delta" => {
                                if let (Some(item_id), Some(delta)) =
                                    (data["item_id"].as_str(), data["delta"].as_str())
                                {
                                    if let Some(slot) = tool_slots.get_mut(item_id) {
                                        slot.arguments.push_str(delta);
                                    }
                                }
                            }
                            "response.output_item.added" => {
                                let item = &data["item"];
                                if item["type"] == "function_call" {
                                    let item_id = item["id"].as_str().unwrap_or("").to_string();
                                    let name = item["name"].as_str().unwrap_or("").to_string();
                                    let call_id =
                                        item["call_id"].as_str().unwrap_or("").to_string();
                                    tool_slots.insert(
                                        item_id,
                                        ToolCallSlot {
                                            id: call_id,
                                            name,
                                            arguments: String::new(),
                                        },
                                    );
                                }
                            }
                            "response.completed" => {
                                let usage_val = &data["response"]["usage"];
                                if !usage_val.is_null() {
                                    let input_tokens = usage_val["input_tokens"].as_u64();
                                    let output_tokens = usage_val["output_tokens"].as_u64();
                                    let cached_tokens =
                                        usage_val["input_tokens_details"]["cached_tokens"].as_u64();
                                    usage = Some(Usage {
                                        prompt_tokens: input_tokens,
                                        completion_tokens: output_tokens,
                                        prompt_tokens_details: Some(UsageDetails { cached_tokens }),
                                    });
                                }
                                done = true;
                            }
                            "response.failed" => {
                                let err_msg = data["response"]["error"]["message"]
                                    .as_str()
                                    .unwrap_or("unknown error");
                                return Err(anyhow!("Codex response failed: {}", err_msg));
                            }
                            "error" => {
                                let err_msg = data["message"].as_str().unwrap_or("unknown error");
                                return Err(anyhow!("Codex error: {}", err_msg));
                            }
                            _ => {}
                        }
                    } else {
                        current_event = None;
                        current_data = None;
                    }
                    continue;
                }

                if let Some(event_str) = line.strip_prefix("event: ") {
                    current_event = Some(event_str.to_string());
                } else if let Some(data_str) = line.strip_prefix("data: ") {
                    if data_str == "[DONE]" {
                        done = true;
                        break;
                    }
                    current_data = Some(data_str.to_string());
                }
            }
        }

        let tool_calls = if tool_slots.is_empty() {
            None
        } else {
            let mut calls: Vec<ToolCall> = tool_slots
                .into_values()
                .map(|slot| ToolCall {
                    id: slot.id,
                    function: FunctionCall {
                        name: slot.name,
                        arguments: slot.arguments,
                    },
                })
                .collect();
            calls.sort_by(|a, b| a.id.cmp(&b.id));
            Some(calls)
        };

        let message = ResponseMessage {
            role: "assistant".to_string(),
            content: if content.is_empty() {
                None
            } else {
                Some(content)
            },
            tool_calls,
        };

        Ok((message, usage))
    }
}
