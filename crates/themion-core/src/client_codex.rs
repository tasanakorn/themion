use anyhow::{anyhow, Result};
use chrono::{Local, TimeZone};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth::CodexAuth;
use crate::client::{
    ChatBackend, FunctionCall, Message, ResponseMessage, ToolCall, Usage, UsageDetails,
};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

#[derive(Debug, Clone, Serialize)]
pub struct RateLimitWindow {
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RateLimitSnapshot {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub primary: Option<RateLimitWindow>,
    pub secondary: Option<RateLimitWindow>,
    pub credits: Option<CreditsSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtractedLimitWindow {
    pub kind: String,
    pub status_line_key: Option<String>,
    pub label: String,
    pub window_minutes: Option<i64>,
    pub used_percent: f64,
    pub percent_left: f64,
    pub resets_at: Option<i64>,
    pub display: String,
    pub status_line_display: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtractedRateLimitSnapshot {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub limits: Vec<ExtractedLimitWindow>,
    pub credits: Option<CreditsSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiCallRateLimitReport {
    pub api_call: String,
    pub source: String,
    pub http_status: Option<u16>,
    pub active_limit: Option<String>,
    pub snapshots: Vec<ExtractedRateLimitSnapshot>,
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

        let response = self
            .http
            .get(format!("{}/models", self.base_url))
            .query(&[("client_version", "1.0.0")])
            .header("Authorization", format!("Bearer {}", access_token))
            .header("chatgpt-account-id", &account_id)
            .header("originator", "pi")
            .send()
            .await?;

        let status = response.status();
        let headers = response.headers().clone();

        for (name, value) in headers.iter() {
            let key = name.as_str().to_ascii_lowercase();
            if key.contains("codex") || key.contains("limit") || key.contains("credit") || key.contains("usage") {
                eprintln!(
                    "[codex usage] header {}: {}",
                    name,
                    value.to_str().unwrap_or("<non-utf8>")
                );
            }
        }

        if !status.is_success() {
            let text = response.text().await?;
            return Err(anyhow!("Codex rate-limit header fetch error {status}: {text}"));
        }

        if let Some((active_limit, snapshot)) = parse_active_rate_limit_from_headers(&headers) {
            eprintln!(
                "[codex usage] active header snapshot active_limit={:?} primary={} secondary={}",
                active_limit,
                snapshot.primary.is_some(),
                snapshot.secondary.is_some()
            );
            return Ok(snapshot);
        }

        let all = parse_all_rate_limits_from_headers(&headers);
        eprintln!("[codex usage] parsed {} snapshots from headers", all.len());
        if let Some(snapshot) = all.iter().find(|s| {
            s.limit_id
                .as_deref()
                .map(|id| id.eq_ignore_ascii_case("codex"))
                .unwrap_or(false)
        }).cloned().or_else(|| all.into_iter().next()) {
            eprintln!(
                "[codex usage] using header snapshot primary={} secondary={} credits={}",
                snapshot.primary.is_some(),
                snapshot.secondary.is_some(),
                snapshot.credits.is_some()
            );
            return Ok(snapshot);
        }

        Err(anyhow!("no codex rate-limit headers found on /models response"))
    }

}

fn parse_rate_limit_snapshot(json: &Value) -> RateLimitSnapshot {
    let rate_limit = json
        .get("rate_limit")
        .or_else(|| json.get("rateLimits"))
        .and_then(|v| v.as_object());

    let primary = rate_limit
        .and_then(|rate_limit| {
            rate_limit
                .get("primary_window")
                .or_else(|| rate_limit.get("primaryWindow"))
        })
        .and_then(parse_usage_window);
    let secondary = rate_limit
        .and_then(|rate_limit| {
            rate_limit
                .get("secondary_window")
                .or_else(|| rate_limit.get("secondaryWindow"))
        })
        .and_then(parse_usage_window);
    let credits = json
        .get("credits")
        .or_else(|| json.get("creditBalance"))
        .and_then(parse_credits);

    RateLimitSnapshot {
        limit_id: Some("codex".to_string()),
        limit_name: None,
        primary,
        secondary,
        credits,
    }
}

fn parse_usage_window(value: &Value) -> Option<RateLimitWindow> {
    let value = value.as_object()?;
    let used_percent = value
        .get("used_percent")
        .or_else(|| value.get("usedPercent"))?
        .as_f64()?;
    let window_seconds = value
        .get("limit_window_seconds")
        .or_else(|| value.get("limitWindowSeconds"))
        .and_then(Value::as_i64);
    let window_minutes = value
        .get("window_minutes")
        .or_else(|| value.get("windowMinutes"))
        .and_then(Value::as_i64)
        .or_else(|| window_seconds.map(|seconds| (seconds + 59) / 60));
    let resets_at = value
        .get("reset_at")
        .or_else(|| value.get("resetAt"))
        .and_then(Value::as_i64);
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

fn normalize_limit_id(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn parse_header_f64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<f64> {
    headers.get(name)?.to_str().ok()?.parse::<f64>().ok()
}

fn parse_header_i64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<i64> {
    headers.get(name)?.to_str().ok()?.parse::<i64>().ok()
}

fn parse_header_bool(headers: &reqwest::header::HeaderMap, name: &str) -> Option<bool> {
    let v = headers.get(name)?.to_str().ok()?.trim().to_ascii_lowercase();
    match v.as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_header_string(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(ToString::to_string)
}

fn parse_window_from_headers(
    headers: &reqwest::header::HeaderMap,
    prefix: &str,
    kind: &str,
) -> Option<RateLimitWindow> {
    let used_percent = parse_header_f64(headers, &format!("{prefix}-{kind}-used-percent"))?;
    let window_minutes = parse_header_i64(headers, &format!("{prefix}-{kind}-window-minutes"));
    let resets_at = parse_header_i64(headers, &format!("{prefix}-{kind}-reset-at"));
    Some(RateLimitWindow {
        used_percent,
        window_minutes,
        resets_at,
    })
}

fn parse_credits_from_headers(
    headers: &reqwest::header::HeaderMap,
    prefix: &str,
) -> Option<CreditsSnapshot> {
    let has_credits = parse_header_bool(headers, &format!("{prefix}-credits-has-credits"))?;
    let unlimited = parse_header_bool(headers, &format!("{prefix}-credits-unlimited"))
        .unwrap_or(false);
    let balance = parse_header_string(headers, &format!("{prefix}-credits-balance"));
    Some(CreditsSnapshot {
        has_credits,
        unlimited,
        balance,
    })
}

fn collect_limit_ids_from_headers(headers: &reqwest::header::HeaderMap) -> Vec<String> {
    use std::collections::BTreeSet;

    let mut ids = BTreeSet::new();
    ids.insert("codex".to_string());

    for name in headers.keys() {
        let name = name.as_str().to_ascii_lowercase();
        if let Some(prefix) = name.strip_suffix("-primary-used-percent") {
            if let Some(limit) = prefix.strip_prefix("x-") {
                ids.insert(normalize_limit_id(limit));
            }
        }
    }

    ids.into_iter().collect()
}

fn parse_rate_limit_for_limit_from_headers(
    headers: &reqwest::header::HeaderMap,
    limit_id: &str,
) -> Option<RateLimitSnapshot> {
    let header_limit = limit_id.replace('_', "-");
    let prefix = format!("x-{header_limit}");

    let primary = parse_window_from_headers(headers, &prefix, "primary");
    let secondary = parse_window_from_headers(headers, &prefix, "secondary");
    let credits = parse_credits_from_headers(headers, &prefix);
    let limit_name = parse_header_string(headers, &format!("{prefix}-limit-name"));

    if primary.is_none() && secondary.is_none() && credits.is_none() {
        return None;
    }

    Some(RateLimitSnapshot {
        limit_id: Some(limit_id.to_string()),
        limit_name,
        primary,
        secondary,
        credits,
    })
}

pub fn parse_all_rate_limits_from_headers(
    headers: &reqwest::header::HeaderMap,
) -> Vec<RateLimitSnapshot> {
    collect_limit_ids_from_headers(headers)
        .into_iter()
        .filter_map(|limit_id| parse_rate_limit_for_limit_from_headers(headers, &limit_id))
        .collect()
}

pub fn parse_active_rate_limit_from_headers(
    headers: &reqwest::header::HeaderMap,
) -> Option<(Option<String>, RateLimitSnapshot)> {
    let active_limit = parse_header_string(headers, "x-codex-active-limit");
    let limit_id = active_limit
        .as_deref()
        .map(normalize_limit_id)
        .unwrap_or_else(|| "codex".to_string());
    let snapshot = parse_rate_limit_for_limit_from_headers(headers, &limit_id)?;
    Some((active_limit, snapshot))
}

fn get_limits_duration(window_minutes: i64) -> String {
    const HOUR_WINDOW_GRACE_MINUTES: i64 = 3;
    const DAY_WINDOW_GRACE_MINUTES: i64 = 3;

    if window_minutes <= 24 * 60 + HOUR_WINDOW_GRACE_MINUTES {
        let rounded_hours = (window_minutes + 30) / 60;
        return format!("{}h", rounded_hours.max(1));
    }
    if window_minutes <= 7 * 24 * 60 + DAY_WINDOW_GRACE_MINUTES {
        return "weekly".to_string();
    }
    if window_minutes <= 30 * 24 * 60 + DAY_WINDOW_GRACE_MINUTES {
        return "monthly".to_string();
    }
    "annual".to_string()
}

fn display_window_label(window: &RateLimitWindow, fallback: &str) -> String {
    window
        .window_minutes
        .map(get_limits_duration)
        .unwrap_or_else(|| fallback.to_string())
}

fn percent_left(used_percent: f64) -> f64 {
    (100.0 - used_percent).clamp(0.0, 100.0)
}

fn format_reset_time(resets_at: i64) -> Option<String> {
    let dt = Local.timestamp_opt(resets_at, 0).single()?;
    Some(dt.format("%H:%M on %-d %b").to_string())
}

fn format_limit_display(percent_left: f64, resets_at: Option<i64>) -> String {
    match resets_at.and_then(format_reset_time) {
        Some(when) => format!("{percent_left:.0}% left (resets {when})"),
        None => format!("{percent_left:.0}% left"),
    }
}

pub fn extract_snapshot(snapshot: &RateLimitSnapshot) -> ExtractedRateLimitSnapshot {
    let mut limits = Vec::new();
    let is_codex = snapshot
        .limit_id
        .as_deref()
        .map(|id| id.eq_ignore_ascii_case("codex"))
        .unwrap_or(false);

    if let Some(primary) = &snapshot.primary {
        let label = display_window_label(primary, "5h");
        let left = percent_left(primary.used_percent);
        limits.push(ExtractedLimitWindow {
            kind: "primary".to_string(),
            status_line_key: is_codex.then(|| "five-hour-limit".to_string()),
            label: label.clone(),
            window_minutes: primary.window_minutes,
            used_percent: primary.used_percent,
            percent_left: left,
            resets_at: primary.resets_at,
            display: format_limit_display(left, primary.resets_at),
            status_line_display: format!("{} {:.0}%", label, left),
        });
    }

    if let Some(secondary) = &snapshot.secondary {
        let label = display_window_label(secondary, "weekly");
        let left = percent_left(secondary.used_percent);
        limits.push(ExtractedLimitWindow {
            kind: "secondary".to_string(),
            status_line_key: is_codex.then(|| "weekly-limit".to_string()),
            label: label.clone(),
            window_minutes: secondary.window_minutes,
            used_percent: secondary.used_percent,
            percent_left: left,
            resets_at: secondary.resets_at,
            display: format_limit_display(left, secondary.resets_at),
            status_line_display: format!("{} {:.0}%", label, left),
        });
    }

    ExtractedRateLimitSnapshot {
        limit_id: snapshot.limit_id.clone(),
        limit_name: snapshot.limit_name.clone(),
        limits,
        credits: snapshot.credits.clone(),
    }
}

pub fn report_for_api_call(
    api_call: impl Into<String>,
    source: impl Into<String>,
    http_status: Option<u16>,
    active_limit: Option<String>,
    snapshots: Vec<RateLimitSnapshot>,
) -> ApiCallRateLimitReport {
    ApiCallRateLimitReport {
        api_call: api_call.into(),
        source: source.into(),
        http_status,
        active_limit,
        snapshots: snapshots.iter().map(extract_snapshot).collect(),
    }
}

fn translate_messages(messages: &[Message]) -> (Option<String>, Vec<Value>) {
    let mut instructions: Option<String> = None;
    let mut items: Vec<Value> = Vec::new();
    let mut first_system = true;
    let mut next_message_id: u64 = 1;
    let mut next_fc_id: u64 = 1;

    let mut alloc_message_id = || {
        let id = format!("msg_{next_message_id}");
        next_message_id += 1;
        id
    };
    let mut alloc_fc_id = || {
        let id = format!("fc_{next_fc_id}");
        next_fc_id += 1;
        id
    };

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                let text = msg.content.as_deref().unwrap_or("");
                if first_system {
                    instructions = Some(text.to_string());
                    first_system = false;
                } else {
                    items.push(serde_json::json!({
                        "id": alloc_message_id(),
                        "type": "message",
                        "role": "developer",
                        "content": [{"type": "input_text", "text": text}]
                    }));
                }
            }
            "user" => {
                let text = msg.content.as_deref().unwrap_or("");
                items.push(serde_json::json!({
                    "id": alloc_message_id(),
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
                                "id": alloc_message_id(),
                                "type": "message",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": c}]
                            }));
                        }
                    }
                    for tc in tcs {
                        items.push(serde_json::json!({
                            "id": alloc_fc_id(),
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": tc.function.name,
                            "arguments": tc.function.arguments,
                        }));
                    }
                } else {
                    let text = msg.content.as_deref().unwrap_or("");
                    items.push(serde_json::json!({
                        "id": alloc_message_id(),
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
                    "id": alloc_fc_id(),
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
            let headers = response.headers().clone();
            let text = response.text().await?;
            if status.as_u16() == 429 {
                let _ = parse_active_rate_limit_from_headers(&headers);
            }
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
                            "codex.rate_limits" => {
                                let _ = parse_rate_limit_snapshot(&data);
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
