use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

use crate::auth::CodexAuth;
use crate::client::{
    ChatBackend, ChatRoundTrace, FunctionCall, Message, ModelInfo, ResponseMessage, ToolCall,
    Usage, UsageDetails,
};
use crate::client_provider_types::{
    parse_active_rate_limit_from_headers, parse_all_rate_limits_from_headers, report_for_api_call,
    ApiCallRateLimitReport, CreditsSnapshot, RateLimitSnapshot, RateLimitWindow,
};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub struct CodexClient {
    http: reqwest::Client,
    base_url: String,
    prompt_cache_key: Option<String>,
    reasoning_effort: String,
    user_agent: String,
    session_id: String,
    thread_id: String,
    auth: Arc<RwLock<CodexAuth>>,
    auth_writer: Box<dyn Fn(&CodexAuth) -> Result<()> + Send + Sync>,
    websocket: Arc<Mutex<Option<CodexWebsocketConnection>>>,
    prompt_context: Arc<Mutex<Option<CodexPromptContext>>>,
}

struct CodexWebsocketConnection {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    status: u16,
    upgrade_rate_limits: Vec<RateLimitSnapshot>,
}

#[derive(Debug, Clone)]
struct CodexPromptContext {
    model: String,
    instructions: String,
    tools: Value,
    input_items: Vec<Value>,
    response_id: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Option<Vec<CodexModelInfo>>,
    models: Option<Vec<CodexModelInfo>>,
}

#[derive(Debug, Deserialize)]
struct CodexModelInfo {
    id: Option<String>,
    slug: Option<String>,
    display_name: Option<String>,
    context_window: Option<u64>,
    max_context_window: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CodexCompletionMeta {
    response_id: Option<String>,
    usage: Option<Usage>,
    end_turn: Option<bool>,
}

#[derive(Debug, Clone)]
enum CodexStreamEventCategory {
    Handled,
    KnownIgnored,
    Unhandled,
}

#[derive(Debug)]
struct CodexStreamState {
    response_message_id: Option<String>,
    response_id: Option<String>,
    content: String,
    tool_slots: HashMap<String, ToolCallSlot>,
    usage: Option<Usage>,
    completion_meta: Option<CodexCompletionMeta>,
    pending_completion: Option<CodexCompletionMeta>,
    unhandled_notice_dedup: HashSet<String>,
    notices: Vec<String>,
    received_response_event: bool,
    streamed_notice_count: usize,
    streamed_rate_limits: Vec<RateLimitSnapshot>,
    reasoning_summary: String,
}

impl CodexStreamState {
    fn new() -> Self {
        Self {
            response_message_id: None,
            response_id: None,
            content: String::new(),
            tool_slots: HashMap::new(),
            usage: None,
            completion_meta: None,
            pending_completion: None,
            unhandled_notice_dedup: HashSet::new(),
            notices: Vec::new(),
            received_response_event: false,
            streamed_notice_count: 0,
            streamed_rate_limits: Vec::new(),
            reasoning_summary: String::new(),
        }
    }

    fn record_notice(&mut self, text: String, on_status: &mut dyn FnMut(String)) {
        self.notices.push(text.clone());
        self.streamed_notice_count += 1;
        on_status(text);
    }

    fn record_unhandled_event(&mut self, event_name: &str, on_status: &mut dyn FnMut(String)) {
        if self.unhandled_notice_dedup.insert(event_name.to_string()) {
            self.record_notice(
                format!("codex stream: unhandled event={event_name}"),
                on_status,
            );
        }
    }
}

fn build_responses_api_body(
    model: &str,
    messages: &[Message],
    tools: &Value,
    prompt_cache_key: Option<&str>,
    reasoning_effort: &str,
) -> Value {
    let (instructions, input_items) = translate_messages(messages);
    let instructions = instructions.unwrap_or_else(|| "You are a helpful assistant.".to_string());
    let translated_tools = translate_tools(tools);
    build_responses_api_body_from_parts(
        model,
        instructions,
        input_items,
        translated_tools,
        prompt_cache_key,
        None,
        reasoning_effort,
    )
}

fn build_responses_api_body_from_parts(
    model: &str,
    instructions: String,
    input_items: Vec<Value>,
    translated_tools: Value,
    prompt_cache_key: Option<&str>,
    previous_response_id: Option<&str>,
    reasoning_effort: &str,
) -> Value {
    let mut body = serde_json::json!({
        "model": model,
        "store": false,
        "parallel_tool_calls": true,
        "tool_choice": "auto",
        "text": { "verbosity": "low" },
        "include": ["reasoning.encrypted_content"],
        "instructions": instructions,
        "input": input_items,
        "stream": true,
    });
    if codex_model_supports_reasoning(model) && !reasoning_effort.trim().is_empty() {
        body["reasoning"] = serde_json::json!({ "effort": reasoning_effort, "summary": "auto" });
    }
    if translated_tools
        .as_array()
        .is_some_and(|tools| !tools.is_empty())
    {
        body["tools"] = translated_tools;
    }
    if let Some(cache_key) = prompt_cache_key {
        body["prompt_cache_key"] = Value::String(cache_key.to_string());
    }
    if let Some(response_id) = previous_response_id {
        body["previous_response_id"] = Value::String(response_id.to_string());
    }
    body
}

fn codex_model_supports_reasoning(model: &str) -> bool {
    matches!(model, "gpt-5.4" | "gpt-5.5")
}

fn build_codex_user_agent() -> String {
    format!("pi ({}; {})", std::env::consts::OS, std::env::consts::ARCH)
}

fn short_session_id(session_id: &str) -> String {
    let Some(first_part) = session_id.split('-').next() else {
        return session_id.to_string();
    };
    format!("{first_part}..")
}

impl CodexClient {
    pub fn new(
        base_url: String,
        prompt_cache_key: Option<String>,
        reasoning_effort: String,
        auth: CodexAuth,
        auth_writer: Box<dyn Fn(&CodexAuth) -> Result<()> + Send + Sync>,
    ) -> Self {
        let request_id = prompt_cache_key
            .as_deref()
            .and_then(|key| key.strip_prefix("session_"))
            .map(ToString::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        Self {
            http: reqwest::Client::new(),
            base_url,
            prompt_cache_key,
            reasoning_effort,
            user_agent: build_codex_user_agent(),
            session_id: request_id.clone(),
            thread_id: request_id,
            auth: Arc::new(RwLock::new(auth)),
            auth_writer,
            websocket: Arc::new(Mutex::new(None)),
            prompt_context: Arc::new(Mutex::new(None)),
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

    async fn auth_headers(&self) -> Result<(String, String)> {
        self.ensure_fresh_token().await?;
        let guard = self.auth.read().await;
        Ok((guard.access_token.clone(), guard.account_id.clone()))
    }

    async fn connect_responses_websocket(
        &self,
        access_token: &str,
        account_id: &str,
        on_status: &mut (dyn FnMut(String) + Send),
    ) -> Result<CodexWebsocketConnection> {
        let ws_url = responses_websocket_url(&self.base_url)?;
        let mut request = ws_url
            .as_str()
            .into_client_request()
            .map_err(|err| anyhow!("failed to build codex websocket request: {err}"))?;
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {access_token}").parse()?);
        request
            .headers_mut()
            .insert("chatgpt-account-id", account_id.parse()?);
        request.headers_mut().insert("originator", "pi".parse()?);
        request
            .headers_mut()
            .insert("User-Agent", self.user_agent.parse()?);
        request
            .headers_mut()
            .insert("session-id", self.session_id.parse()?);
        request
            .headers_mut()
            .insert("thread-id", self.thread_id.parse()?);
        request
            .headers_mut()
            .insert("x-client-request-id", self.thread_id.parse()?);
        request
            .headers_mut()
            .insert("OpenAI-Beta", "responses=experimental".parse()?);

        let started_at = std::time::Instant::now();
        let (stream, response) = tokio_tungstenite::connect_async(request).await?;
        on_status(format!(
            "codex stream: websocket connected status={} elapsed_ms={}",
            response.status().as_u16(),
            started_at.elapsed().as_millis()
        ));

        Ok(CodexWebsocketConnection {
            stream,
            status: response.status().as_u16(),
            upgrade_rate_limits: parse_all_rate_limits_from_headers(response.headers()),
        })
    }

    async fn stream_responses_websocket(
        &self,
        access_token: &str,
        account_id: &str,
        body: &Value,
        reconnect_body: Option<&Value>,
        reconnect_status_line: Option<&str>,
        state: &mut CodexStreamState,
        on_chunk: &mut (dyn FnMut(String) + Send),
        on_reasoning_delta: &mut (dyn FnMut(String) + Send),
        on_status: &mut (dyn FnMut(String) + Send),
        should_cancel: Option<&(dyn Fn() -> bool + Send + Sync)>,
    ) -> Result<(u16, Vec<RateLimitSnapshot>, bool)> {
        let mut guard = self.websocket.lock().await;
        let mut retries_before_response = 0;
        let mut request_body = body.clone();
        let mut used_reconnect_body = false;

        loop {
            let reused = guard.is_some();
            if guard.is_none() {
                *guard = Some(
                    self.connect_responses_websocket(access_token, account_id, on_status)
                        .await?,
                );
            } else {
                on_status("codex stream: reusing websocket connection".to_string());
            }

            let connection = guard
                .as_mut()
                .ok_or_else(|| anyhow!("Codex websocket connection is unavailable"))?;
            let response_status = connection.status;
            let rate_limit_snapshots = if reused {
                Vec::new()
            } else {
                connection.upgrade_rate_limits.clone()
            };
            let received_response_event_before_request = state.received_response_event;

            let result = stream_responses_websocket_request(
                connection,
                &request_body,
                state,
                on_chunk,
                on_reasoning_delta,
                on_status,
                should_cancel,
            )
            .await;

            match result {
                Ok(()) => return Ok((response_status, rate_limit_snapshots, used_reconnect_body)),
                Err(err) => {
                    *guard = None;
                    let can_retry_before_response = !received_response_event_before_request
                        && !state.received_response_event
                        && retries_before_response < 2
                        && should_cancel.is_none_or(|cancel| !cancel());
                    if can_retry_before_response {
                        retries_before_response += 1;
                        if let Some(reconnect_body) = reconnect_body {
                            request_body = reconnect_body.clone();
                            used_reconnect_body = true;
                        }
                        on_status(format!(
                            "codex stream: websocket resuming after disconnect reused={} attempt={} error={}",
                            reused,
                            retries_before_response,
                            err.source
                        ));
                        if let Some(status_line) = reconnect_status_line {
                            on_status(status_line.to_string());
                        }
                        continue;
                    }
                    if state.received_response_event {
                        on_status(
                            "codex stream: websocket disconnected before response.completed"
                                .to_string(),
                        );
                    }
                    return Err(err.source);
                }
            }
        }
    }

    pub async fn get_rate_limits(&self) -> Result<RateLimitSnapshot> {
        let (access_token, account_id) = self.auth_headers().await?;

        let response = self
            .http
            .get(format!("{}/models", self.base_url))
            .query(&[("client_version", "1.0.0")])
            .header("Authorization", format!("Bearer {}", access_token))
            .header("chatgpt-account-id", &account_id)
            .header("originator", "pi")
            .header("User-Agent", &self.user_agent)
            .send()
            .await?;

        let status = response.status();
        let headers = response.headers().clone();

        for (name, value) in headers.iter() {
            let key = name.as_str().to_ascii_lowercase();
            if key.contains("codex")
                || key.contains("limit")
                || key.contains("credit")
                || key.contains("usage")
            {
                eprintln!(
                    "[codex usage] header {}: {}",
                    name,
                    value.to_str().unwrap_or("<non-utf8>")
                );
            }
        }

        if !status.is_success() {
            let text = response.text().await?;
            return Err(anyhow!(
                "Codex rate-limit header fetch error {status}: {text}"
            ));
        }

        if let Some((active_limit, snapshot)) =
            parse_active_rate_limit_from_headers(&headers, "x-codex-active-limit")
        {
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
        if let Some(snapshot) = all
            .iter()
            .find(|s| {
                s.limit_id
                    .as_deref()
                    .map(|id| id.eq_ignore_ascii_case("codex"))
                    .unwrap_or(false)
            })
            .cloned()
            .or_else(|| all.into_iter().next())
        {
            eprintln!(
                "[codex usage] using header snapshot primary={} secondary={} credits={}",
                snapshot.primary.is_some(),
                snapshot.secondary.is_some(),
                snapshot.credits.is_some()
            );
            return Ok(snapshot);
        }

        Err(anyhow!(
            "no codex rate-limit headers found on /models response"
        ))
    }
}

async fn stream_responses_websocket_request(
    connection: &mut CodexWebsocketConnection,
    body: &Value,
    state: &mut CodexStreamState,
    on_chunk: &mut (dyn FnMut(String) + Send),
    on_reasoning_delta: &mut (dyn FnMut(String) + Send),
    on_status: &mut (dyn FnMut(String) + Send),
    should_cancel: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> std::result::Result<(), CodexWebsocketRequestError> {
    let mut request_body = body.clone();
    request_body["type"] = Value::String("response.create".to_string());
    connection
        .stream
        .send(WsMessage::Text(
            serde_json::to_string(&request_body)?.into(),
        ))
        .await?;

    loop {
        if should_cancel.is_some_and(|cancel| cancel()) {
            let _ = connection.stream.close(None).await;
            return Err(anyhow!("interrupted").into());
        }

        let Some(message) = connection.stream.next().await else {
            on_status("codex stream: websocket disconnected".to_string());
            return Err(anyhow!("Codex websocket closed before response.completed").into());
        };

        match message {
            Err(err) => {
                on_status(format!("codex stream: websocket disconnected error={err}"));
                return Err(anyhow!(err).into());
            }
            Ok(WsMessage::Text(text)) => {
                let data: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let event_type = data["type"].as_str().unwrap_or("");
                if !event_type.is_empty() {
                    state.received_response_event = true;
                }
                let stream_done = process_codex_json_event(
                    event_type,
                    &data,
                    state,
                    on_chunk,
                    on_reasoning_delta,
                    on_status,
                    should_cancel,
                )?;
                if stream_done {
                    return Ok(());
                }
            }
            Ok(WsMessage::Close(_)) => {
                on_status("codex stream: websocket disconnected".to_string());
                return Err(anyhow!("Codex websocket closed before response.completed").into());
            }
            Ok(WsMessage::Ping(payload)) => {
                connection.stream.send(WsMessage::Pong(payload)).await?;
            }
            Ok(WsMessage::Binary(_)) => {
                return Err(anyhow!("Codex websocket returned unexpected binary message").into());
            }
            Ok(WsMessage::Pong(_)) | Ok(WsMessage::Frame(_)) => {}
        }
    }
}

struct CodexWebsocketRequestError {
    source: anyhow::Error,
}

impl From<anyhow::Error> for CodexWebsocketRequestError {
    fn from(source: anyhow::Error) -> Self {
        Self { source }
    }
}

impl From<serde_json::Error> for CodexWebsocketRequestError {
    fn from(source: serde_json::Error) -> Self {
        Self {
            source: anyhow!(source),
        }
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for CodexWebsocketRequestError {
    fn from(source: tokio_tungstenite::tungstenite::Error) -> Self {
        Self {
            source: anyhow!(source),
        }
    }
}

fn parse_rate_limit_snapshot(json: &Value) -> RateLimitSnapshot {
    let rate_limit = json
        .get("rate_limits")
        .or_else(|| json.get("rateLimits"))
        .or_else(|| json.get("rate_limit"))
        .and_then(|v| v.as_object());

    let primary = rate_limit
        .and_then(|rate_limit| {
            rate_limit
                .get("primary_window")
                .or_else(|| rate_limit.get("primaryWindow"))
                .or_else(|| rate_limit.get("primary"))
        })
        .and_then(parse_usage_window);
    let secondary = rate_limit
        .and_then(|rate_limit| {
            rate_limit
                .get("secondary_window")
                .or_else(|| rate_limit.get("secondaryWindow"))
                .or_else(|| rate_limit.get("secondary"))
        })
        .and_then(parse_usage_window);
    let credits = json
        .get("credits")
        .or_else(|| json.get("creditBalance"))
        .and_then(parse_credits);
    let limit_id = json
        .get("metered_limit_name")
        .or_else(|| json.get("meteredLimitName"))
        .or_else(|| json.get("limit_name"))
        .or_else(|| json.get("limitName"))
        .and_then(Value::as_str)
        .map(crate::client_provider_types::normalize_limit_id)
        .unwrap_or_else(|| "codex".to_string());

    RateLimitSnapshot {
        limit_id: Some(limit_id),
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
        .or_else(|| value.get("usedPercent"))?;
    let used_percent = used_percent
        .as_f64()
        .or_else(|| used_percent.as_i64().map(|value| value as f64))?;
    let window_seconds = value
        .get("limit_window_seconds")
        .or_else(|| value.get("limitWindowSeconds"))
        .and_then(Value::as_i64);
    let window_minutes = value
        .get("window_minutes")
        .or_else(|| value.get("windowMinutes"))
        .or_else(|| value.get("limit_window_minutes"))
        .or_else(|| value.get("limitWindowMinutes"))
        .and_then(Value::as_i64)
        .or_else(|| window_seconds.map(|seconds| (seconds + 59) / 60));
    let resets_at = value
        .get("reset_at")
        .or_else(|| value.get("resetAt"))
        .or_else(|| value.get("resets_at"))
        .or_else(|| value.get("resetsAt"))
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
            .or_else(|| value.get("hasCredits"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        unlimited: value
            .get("unlimited")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        balance: value
            .get("balance")
            .or_else(|| value.get("credit_balance"))
            .or_else(|| value.get("creditBalance"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

#[derive(Debug, Clone)]
struct CodexPromptPlan {
    body: Value,
    full_body: Value,
    full_input_items: Vec<Value>,
    instructions: String,
    translated_tools: Value,
    mode: CodexPromptInputMode,
}

#[derive(Debug, Clone)]
enum CodexPromptInputMode {
    Full {
        reason: &'static str,
        sent_items: usize,
    },
    Incremental {
        previous_response_id: String,
        reused_items: usize,
        sent_items: usize,
    },
}

impl CodexPromptInputMode {
    fn status_line(&self) -> String {
        match self {
            CodexPromptInputMode::Full { reason, sent_items } => {
                format!("codex stream: prompt input mode=full reason={reason} sent_items={sent_items}")
            }
            CodexPromptInputMode::Incremental {
                previous_response_id,
                reused_items,
                sent_items,
            } => format!(
                "codex stream: prompt input mode=incremental previous_response_id={} reused_items={} sent_items={}",
                short_response_id(previous_response_id),
                reused_items,
                sent_items
            ),
        }
    }
}

fn short_response_id(response_id: &str) -> String {
    if response_id.len() <= 14 {
        response_id.to_string()
    } else {
        format!("{}..", &response_id[..12])
    }
}

fn plan_codex_prompt_request(
    model: &str,
    messages: &[Message],
    tools: &Value,
    prompt_cache_key: Option<&str>,
    previous_context: Option<&CodexPromptContext>,
    incremental_allowed: bool,
    reasoning_effort: &str,
) -> CodexPromptPlan {
    let (instructions, full_input_items) = translate_messages(messages);
    let instructions = instructions.unwrap_or_else(|| "You are a helpful assistant.".to_string());
    let translated_tools = translate_tools(tools);

    let full_body = build_responses_api_body_from_parts(
        model,
        instructions.clone(),
        full_input_items.clone(),
        translated_tools.clone(),
        prompt_cache_key,
        None,
        reasoning_effort,
    );

    let incremental = incremental_allowed
        .then_some(())
        .and_then(|_| previous_context)
        .and_then(|context| {
            if context.model != model {
                return None;
            }
            if context.instructions != instructions {
                return None;
            }
            if context.tools != translated_tools {
                return None;
            }
            let reused_items =
                common_codex_item_prefix_len(&context.input_items, &full_input_items);
            if reused_items == 0 || reused_items < context.input_items.len() {
                return None;
            }
            let suffix = full_input_items.get(reused_items..)?.to_vec();
            if suffix.is_empty() {
                return None;
            }
            Some((context.response_id.clone(), reused_items, suffix))
        });

    let (body, mode) = if let Some((previous_response_id, reused_items, suffix)) = incremental {
        let sent_items = suffix.len();
        (
            build_responses_api_body_from_parts(
                model,
                instructions.clone(),
                suffix,
                translated_tools.clone(),
                prompt_cache_key,
                Some(&previous_response_id),
                reasoning_effort,
            ),
            CodexPromptInputMode::Incremental {
                previous_response_id,
                reused_items,
                sent_items,
            },
        )
    } else {
        let sent_items = full_input_items.len();
        (
            full_body.clone(),
            CodexPromptInputMode::Full {
                reason: if previous_context.is_some() && !incremental_allowed {
                    "websocket_reconnected"
                } else if previous_context.is_some() {
                    "context_changed"
                } else {
                    "no_previous_context"
                },
                sent_items,
            },
        )
    };

    CodexPromptPlan {
        body,
        full_body,
        full_input_items,
        instructions,
        translated_tools,
        mode,
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
                    "strict": Value::Null,
                })
            } else {
                t.clone()
            }
        })
        .collect();
    Value::Array(translated)
}

fn translated_response_items(response: &ResponseMessage) -> Vec<Value> {
    let message = Message {
        role: response.role.clone(),
        content: response.content.clone(),
        tool_calls: response.tool_calls.clone(),
        tool_call_id: None,
    };
    let (_, items) = translate_messages(&[message]);
    items
}

fn common_codex_item_prefix_len(previous: &[Value], next: &[Value]) -> usize {
    previous
        .iter()
        .zip(next.iter())
        .take_while(|(left, right)| codex_items_match_ignoring_generated_ids(left, right))
        .count()
}

fn codex_items_match_ignoring_generated_ids(left: &Value, right: &Value) -> bool {
    strip_codex_generated_ids(left) == strip_codex_generated_ids(right)
}

fn strip_codex_generated_ids(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let filtered = map
                .iter()
                .filter(|(key, _)| key.as_str() != "id")
                .map(|(key, value)| (key.clone(), strip_codex_generated_ids(value)))
                .collect();
            Value::Object(filtered)
        }
        Value::Array(items) => Value::Array(items.iter().map(strip_codex_generated_ids).collect()),
        _ => value.clone(),
    }
}

fn combine_usage(existing: Option<Usage>, next: Option<Usage>) -> Option<Usage> {
    match (existing, next) {
        (None, None) => None,
        (Some(usage), None) | (None, Some(usage)) => Some(usage),
        (Some(existing), Some(next)) => Some(Usage {
            prompt_tokens: Some(
                existing.prompt_tokens.unwrap_or(0) + next.prompt_tokens.unwrap_or(0),
            ),
            completion_tokens: Some(
                existing.completion_tokens.unwrap_or(0) + next.completion_tokens.unwrap_or(0),
            ),
            prompt_tokens_details: Some(UsageDetails {
                cached_tokens: Some(
                    existing
                        .prompt_tokens_details
                        .and_then(|d| d.cached_tokens)
                        .unwrap_or(0)
                        + next
                            .prompt_tokens_details
                            .and_then(|d| d.cached_tokens)
                            .unwrap_or(0),
                ),
            }),
        }),
    }
}

fn parse_completion_meta(data: &Value) -> CodexCompletionMeta {
    let usage = parse_usage_from_value(&data["response"]["usage"]);
    CodexCompletionMeta {
        response_id: data["response"]["id"].as_str().map(ToString::to_string),
        usage,
        end_turn: data["response"]["end_turn"].as_bool(),
    }
}

fn parse_usage_from_value(usage_val: &Value) -> Option<Usage> {
    if usage_val.is_null() {
        return None;
    }
    let input_tokens = usage_val["input_tokens"].as_u64();
    let output_tokens = usage_val["output_tokens"].as_u64();
    let cached_tokens = usage_val["input_tokens_details"]["cached_tokens"].as_u64();
    Some(Usage {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        prompt_tokens_details: Some(UsageDetails { cached_tokens }),
    })
}

fn responses_websocket_url(base_url: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(&format!("{}/responses", base_url.trim_end_matches('/')))?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        other => {
            return Err(anyhow!(
                "unsupported Codex base URL scheme for websocket: {other}"
            ))
        }
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow!("failed to set Codex websocket URL scheme"))?;
    Ok(url)
}

fn classify_codex_event(event_type: &str) -> CodexStreamEventCategory {
    match event_type {
        "response.output_text.delta"
        | "response.function_call_arguments.delta"
        | "response.output_item.added"
        | "response.reasoning_summary_text.delta"
        | "response.reasoning_summary_part.done"
        | "response.reasoning_text.delta"
        | "codex.rate_limits"
        | "response.completed"
        | "response.failed"
        | "error" => CodexStreamEventCategory::Handled,
        "Created"
        | "ServerModel"
        | "ModelVerifications"
        | "ServerReasoningIncluded"
        | "ModelsEtag"
        | "response.output_item.done"
        | "response.content_part.done"
        | "response.output_text.done"
        | "response.reasoning_summary_part.added"
        | "response.function_call_arguments.done"
        | "response.content_part.added" => CodexStreamEventCategory::KnownIgnored,
        _ => CodexStreamEventCategory::Unhandled,
    }
}

fn process_codex_json_event(
    event_type: &str,
    data: &Value,
    state: &mut CodexStreamState,
    on_chunk: &mut (dyn FnMut(String) + Send),
    on_reasoning_delta: &mut (dyn FnMut(String) + Send),
    on_status: &mut (dyn FnMut(String) + Send),
    should_cancel: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<bool> {
    match classify_codex_event(event_type) {
        CodexStreamEventCategory::Handled => match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = data["delta"].as_str() {
                    if should_cancel.is_some_and(|cancel| cancel()) {
                        anyhow::bail!("interrupted");
                    }
                    state.content.push_str(delta);
                    on_chunk(delta.to_string());
                }
            }
            "response.function_call_arguments.delta" => {
                if let (Some(item_id), Some(delta)) =
                    (data["item_id"].as_str(), data["delta"].as_str())
                {
                    if let Some(slot) = state.tool_slots.get_mut(item_id) {
                        slot.arguments.push_str(delta);
                    }
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(delta) = data["delta"].as_str() {
                    if !delta.is_empty() {
                        state.reasoning_summary.push_str(delta);
                        on_reasoning_delta(delta.to_string());
                    }
                }
            }
            "response.reasoning_summary_part.done" => {
                state.reasoning_summary.push_str("\n\n");
                on_reasoning_delta("\n\n".to_string());
            }
            "response.output_item.added" => {
                let item = &data["item"];
                if item["type"] == "function_call" {
                    let item_id = item["id"].as_str().unwrap_or("").to_string();
                    let name = item["name"].as_str().unwrap_or("").to_string();
                    let call_id = item["call_id"].as_str().unwrap_or("").to_string();
                    state.tool_slots.insert(
                        item_id,
                        ToolCallSlot {
                            id: call_id,
                            name,
                            arguments: String::new(),
                        },
                    );
                } else if item["type"] == "message" {
                    state.response_message_id = item["id"].as_str().map(ToString::to_string);
                }
            }
            "codex.rate_limits" => {
                state
                    .streamed_rate_limits
                    .push(parse_rate_limit_snapshot(data));
            }
            "response.completed" => {
                let completion_meta = parse_completion_meta(data);
                state.record_notice("codex stream: response.completed".to_string(), on_status);
                state.usage = combine_usage(state.usage.take(), completion_meta.usage.clone());
                state.pending_completion = Some(completion_meta.clone());
                state.completion_meta = Some(completion_meta);
                return Ok(true);
            }
            "response.failed" => {
                let err_msg = data["response"]["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown error");
                return Err(anyhow!("Codex response failed: {}", err_msg));
            }
            "error" => {
                let err_msg = data["message"]
                    .as_str()
                    .or_else(|| data["error"]["message"].as_str())
                    .unwrap_or("unknown error");
                return Err(anyhow!("Codex error: {}", err_msg));
            }
            _ => {}
        },
        CodexStreamEventCategory::KnownIgnored => {}
        CodexStreamEventCategory::Unhandled => {
            state.record_unhandled_event(event_type, on_status);
        }
    }
    Ok(false)
}

#[derive(Debug)]
struct ToolCallSlot {
    id: String,
    name: String,
    arguments: String,
}

#[async_trait::async_trait]
impl ChatBackend for CodexClient {
    fn backend_name(&self) -> &'static str {
        "responses"
    }

    fn build_round_request_payload(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
    ) -> Value {
        build_responses_api_body(
            model,
            messages,
            tools,
            self.prompt_cache_key.as_deref(),
            &self.reasoning_effort,
        )
    }

    async fn chat_completion_stream(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
        mut on_chunk: Box<dyn FnMut(String) + Send + 'static>,
        mut on_reasoning_delta: Box<dyn FnMut(String) + Send + 'static>,
        mut on_status: Box<dyn FnMut(String) + Send + 'static>,
        should_cancel: Option<Box<dyn Fn() -> bool + Send + Sync + 'static>>,
    ) -> Result<(
        ResponseMessage,
        Option<Usage>,
        Option<ApiCallRateLimitReport>,
        ChatRoundTrace,
    )> {
        let (access_token, account_id) = self.auth_headers().await?;

        let tool_count = tools.as_array().map_or(0, Vec::len);
        on_status(format!(
            "codex stream: generating prompt model={model} messages={} tools={tool_count}",
            messages.len()
        ));
        let incremental_allowed = self.websocket.lock().await.is_some();
        let previous_context = self.prompt_context.lock().await.clone();
        let prompt_plan = plan_codex_prompt_request(
            model,
            messages,
            tools,
            self.prompt_cache_key.as_deref(),
            previous_context.as_ref(),
            incremental_allowed,
            &self.reasoning_effort,
        );
        on_status(prompt_plan.mode.status_line());
        on_status(format!(
            "codex stream: sending websocket model={model} session_id={}",
            short_session_id(&self.session_id)
        ));
        let mut state = CodexStreamState::new();
        let reconnect_mode = CodexPromptInputMode::Full {
            reason: "websocket_reconnected",
            sent_items: prompt_plan.full_input_items.len(),
        };
        let reconnect_status_line = reconnect_mode.status_line();
        let (response_status, mut rate_limit_snapshots, used_reconnect_body) = self
            .stream_responses_websocket(
                &access_token,
                &account_id,
                &prompt_plan.body,
                Some(&prompt_plan.full_body),
                Some(&reconnect_status_line),
                &mut state,
                &mut *on_chunk,
                &mut *on_reasoning_delta,
                &mut *on_status,
                should_cancel.as_deref(),
            )
            .await?;

        let prompt_input_mode = if used_reconnect_body {
            reconnect_status_line
        } else {
            prompt_plan.mode.status_line()
        };
        let trace_request = serde_json::json!({
            "transport": "websocket",
            "prompt_input_mode": prompt_input_mode,
            "request": if used_reconnect_body { prompt_plan.full_body.clone() } else { prompt_plan.body.clone() },
        });

        if let Some(meta) = state.pending_completion.take() {
            state.response_id = meta.response_id.clone();
            state.completion_meta = Some(meta);
        }

        rate_limit_snapshots.extend(state.streamed_rate_limits.clone());
        let rate_limit_report = Some(report_for_api_call(
            "responses",
            "websocket",
            Some(response_status),
            None,
            rate_limit_snapshots,
        ));

        let tool_calls = if state.tool_slots.is_empty() {
            None
        } else {
            let mut calls: Vec<ToolCall> = state
                .tool_slots
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

        let final_content = state.content;

        let message = ResponseMessage {
            role: "assistant".to_string(),
            content: if final_content.is_empty() {
                None
            } else {
                Some(final_content)
            },
            tool_calls,
        };
        if let Some(response_id) = state.response_id.clone() {
            let mut input_items = prompt_plan.full_input_items;
            input_items.extend(translated_response_items(&message));
            *self.prompt_context.lock().await = Some(CodexPromptContext {
                model: model.to_string(),
                instructions: prompt_plan.instructions,
                tools: prompt_plan.translated_tools,
                input_items,
                response_id,
            });
        }

        let trace = ChatRoundTrace {
            backend: "responses".to_string(),
            request: trace_request,
            response: Some(serde_json::json!({
                "message": &message,
                "codex_completion": &state.completion_meta,
            })),
            error: None,
            http_status: Some(response_status),
            usage: state.usage.clone(),
            rate_limits: rate_limit_report.clone(),
        };

        Ok((message, state.usage, rate_limit_report, trace))
    }

    async fn fetch_model_info(&self, model: &str) -> Result<Option<ModelInfo>> {
        let (access_token, account_id) = self.auth_headers().await?;
        let response = self
            .http
            .get(format!("{}/models", self.base_url))
            .query(&[("client_version", "1.0.0")])
            .header("Authorization", format!("Bearer {}", access_token))
            .header("chatgpt-account-id", &account_id)
            .header("originator", "pi")
            .header("User-Agent", &self.user_agent)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            return Err(anyhow!("Codex models error {status}: {text}"));
        }

        let payload: ModelsResponse = response.json().await?;
        let models = payload.data.or(payload.models).unwrap_or_default();
        let found = models
            .into_iter()
            .find(|m| m.id.as_deref() == Some(model) || m.slug.as_deref() == Some(model));

        Ok(found.map(|m| ModelInfo {
            id: m.id.or(m.slug).unwrap_or_else(|| model.to_string()),
            display_name: m.display_name,
            context_window: m.context_window,
            max_context_window: m.max_context_window,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_message() -> Vec<Message> {
        vec![Message {
            role: "user".to_string(),
            content: Some("hello".to_string()),
            tool_calls: None,
            tool_call_id: None,
        }]
    }

    #[test]
    fn responses_body_includes_reasoning_for_supported_codex_models() {
        for model in ["gpt-5.4", "gpt-5.5"] {
            let body = build_responses_api_body(
                model,
                &user_message(),
                &serde_json::json!([]),
                None,
                "xhigh",
            );

            assert_eq!(body["model"], model);
            assert_eq!(body["reasoning"]["effort"], "xhigh");
            assert_eq!(body["reasoning"]["summary"], "auto");
        }
    }

    #[test]
    fn responses_body_omits_reasoning_for_other_models() {
        let body = build_responses_api_body(
            "gpt-4o",
            &user_message(),
            &serde_json::json!([]),
            None,
            "high",
        );

        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn process_codex_json_event_streams_reasoning_summary_status() {
        let mut state = CodexStreamState::new();
        let mut statuses = Vec::new();
        let mut reasoning = Vec::new();
        let mut chunks = Vec::new();
        let done = {
            let mut on_status = |text: String| statuses.push(text);
            let mut on_reasoning_delta = |text: String| reasoning.push(text);
            let mut on_chunk = |text: String| chunks.push(text);
            process_codex_json_event(
                "response.reasoning_summary_text.delta",
                &serde_json::json!({
                    "type": "response.reasoning_summary_text.delta",
                    "delta": "checking context"
                }),
                &mut state,
                &mut on_chunk,
                &mut on_reasoning_delta,
                &mut on_status,
                None,
            )
            .expect("reasoning summary event should parse")
        };

        assert!(!done);
        assert!(chunks.is_empty());
        assert_eq!(state.reasoning_summary, "checking context");
        assert!(statuses.is_empty());
        assert_eq!(reasoning, vec!["checking context"]);

        {
            let mut on_status = |text: String| statuses.push(text);
            let mut on_reasoning_delta = |text: String| reasoning.push(text);
            let mut on_chunk = |text: String| chunks.push(text);
            process_codex_json_event(
                "response.reasoning_summary_part.done",
                &serde_json::json!({
                    "type": "response.reasoning_summary_part.done"
                }),
                &mut state,
                &mut on_chunk,
                &mut on_reasoning_delta,
                &mut on_status,
                None,
            )
            .expect("reasoning summary done event should parse");
        }

        assert!(statuses.is_empty());
        assert_eq!(reasoning, vec!["checking context", "\n\n"]);
    }

    #[test]
    fn process_codex_json_event_streams_raw_reasoning_delta() {
        let mut state = CodexStreamState::new();
        let mut statuses = Vec::new();
        let mut reasoning = Vec::new();
        let mut chunks = Vec::new();
        let mut on_status = |text: String| statuses.push(text);
        let mut on_reasoning_delta = |text: String| reasoning.push(text);
        let mut on_chunk = |text: String| chunks.push(text);

        process_codex_json_event(
            "response.reasoning_text.delta",
            &serde_json::json!({ "type": "response.reasoning_text.delta", "delta": "checking context" }),
            &mut state,
            &mut on_chunk,
            &mut on_reasoning_delta,
            &mut on_status,
            None,
        )
        .expect("reasoning text event should parse");
        process_codex_json_event(
            "response.completed",
            &serde_json::json!({ "type": "response.completed", "response": { "id": "resp_1" } }),
            &mut state,
            &mut on_chunk,
            &mut on_reasoning_delta,
            &mut on_status,
            None,
        )
        .expect("completion event should parse");

        assert_eq!(reasoning, vec!["checking context"]);
    }

    #[test]
    fn prompt_plan_uses_incremental_when_context_matches_and_websocket_is_live() {
        let previous_messages = vec![
            Message {
                role: "user".to_string(),
                content: Some("hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: "assistant".to_string(),
                content: Some("hi".to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let (_, previous_items) = translate_messages(&previous_messages);
        let context = CodexPromptContext {
            model: "gpt-5.4".to_string(),
            instructions: "You are a helpful assistant.".to_string(),
            tools: serde_json::json!([]),
            input_items: previous_items,
            response_id: "resp_previous".to_string(),
        };
        let mut next_messages = previous_messages;
        next_messages.push(Message {
            role: "user".to_string(),
            content: Some("next".to_string()),
            tool_calls: None,
            tool_call_id: None,
        });

        let plan = plan_codex_prompt_request(
            "gpt-5.4",
            &next_messages,
            &serde_json::json!([]),
            None,
            Some(&context),
            true,
            "high",
        );

        assert_eq!(plan.body["previous_response_id"], "resp_previous");
        assert_eq!(plan.body["input"].as_array().unwrap().len(), 1);
        assert!(matches!(
            plan.mode,
            CodexPromptInputMode::Incremental { .. }
        ));
    }

    #[test]
    fn prompt_plan_forces_full_when_websocket_was_reconnected() {
        let previous_messages = vec![
            Message {
                role: "user".to_string(),
                content: Some("hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: "assistant".to_string(),
                content: Some("hi".to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let (_, previous_items) = translate_messages(&previous_messages);
        let context = CodexPromptContext {
            model: "gpt-5.4".to_string(),
            instructions: "You are a helpful assistant.".to_string(),
            tools: serde_json::json!([]),
            input_items: previous_items,
            response_id: "resp_previous".to_string(),
        };
        let mut next_messages = previous_messages;
        next_messages.push(Message {
            role: "user".to_string(),
            content: Some("next".to_string()),
            tool_calls: None,
            tool_call_id: None,
        });

        let plan = plan_codex_prompt_request(
            "gpt-5.4",
            &next_messages,
            &serde_json::json!([]),
            None,
            Some(&context),
            false,
            "high",
        );

        assert!(plan.body.get("previous_response_id").is_none());
        assert_eq!(plan.body["input"].as_array().unwrap().len(), 3);
        assert!(matches!(
            plan.mode,
            CodexPromptInputMode::Full {
                reason: "websocket_reconnected",
                ..
            }
        ));
    }

    #[test]
    fn parse_rate_limit_snapshot_reads_websocket_event_shape() {
        let snapshot = parse_rate_limit_snapshot(&serde_json::json!({
            "type": "codex.rate_limits",
            "rate_limits": {
                "primary": {
                    "used_percent": 42,
                    "window_minutes": 300,
                    "reset_at": 1700000000
                },
                "secondary": {
                    "used_percent": 12.5,
                    "window_minutes": 10080,
                    "reset_at": 1700600000
                }
            },
            "credits": {
                "has_credits": true,
                "unlimited": false,
                "balance": "123"
            },
            "metered_limit_name": "codex_other"
        }));

        assert_eq!(snapshot.limit_id.as_deref(), Some("codex_other"));
        let primary = snapshot.primary.expect("missing primary window");
        assert_eq!(primary.used_percent, 42.0);
        assert_eq!(primary.window_minutes, Some(300));
        assert_eq!(primary.resets_at, Some(1700000000));
        let secondary = snapshot.secondary.expect("missing secondary window");
        assert_eq!(secondary.used_percent, 12.5);
        assert_eq!(secondary.window_minutes, Some(10080));
        assert_eq!(secondary.resets_at, Some(1700600000));
        let credits = snapshot.credits.expect("missing credits");
        assert!(credits.has_credits);
        assert!(!credits.unlimited);
        assert_eq!(credits.balance.as_deref(), Some("123"));
    }
}
