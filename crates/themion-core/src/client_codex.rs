use anyhow::{anyhow, Result};
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth::CodexAuth;
use crate::client::{
    ChatBackend, ChatRoundTrace, FunctionCall, Message, ModelInfo, ResponseMessage, ToolCall,
    Usage, UsageDetails,
};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_CONTINUATION_FAILED_NOTICE: &str =
    "codex stream: completed end_turn=false continuation=failed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitWindow {
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitSnapshot {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub primary: Option<RateLimitWindow>,
    pub secondary: Option<RateLimitWindow>,
    pub credits: Option<CreditsSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRateLimitSnapshot {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub limits: Vec<ExtractedLimitWindow>,
    pub credits: Option<CreditsSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCallRateLimitReport {
    pub api_call: String,
    pub source: String,
    pub http_status: Option<u16>,
    pub active_limit: Option<String>,
    pub snapshots: Vec<ExtractedRateLimitSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderQuotaError {
    pub error_type: Option<String>,
    pub message: Option<String>,
    pub plan_type: Option<String>,
    pub resets_at: Option<i64>,
    pub resets_in_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderApiError {
    pub provider: String,
    pub http_status: u16,
    pub raw_body: String,
    pub quota: Option<ProviderQuotaError>,
}

impl fmt::Display for ProviderApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(quota) = &self.quota {
            let provider = display_provider_name(&self.provider);
            let mut attrs = vec![format!("status={}", self.http_status)];
            if let Some(error_type) = quota.error_type.as_deref() {
                attrs.push(format!("type={error_type}"));
            }
            if let Some(plan_type) = quota.plan_type.as_deref() {
                attrs.push(format!("plan={plan_type}"));
            }

            write!(f, "{provider} quota limit reached ({})", attrs.join(", "))?;

            if let Some(message) = quota.message.as_deref() {
                write!(f, ": {message}")?;
            }

            if let Some(reset_sentence) = format_quota_reset_sentence(quota) {
                write!(f, ". {reset_sentence}")?;
            }

            return Ok(());
        }

        write!(
            f,
            "{} API error {}: {}",
            display_provider_name(&self.provider),
            self.http_status,
            self.raw_body
        )
    }
}

impl std::error::Error for ProviderApiError {}

pub struct CodexClient {
    http: reqwest::Client,
    base_url: String,
    auth: Arc<RwLock<CodexAuth>>,
    auth_writer: Box<dyn Fn(&CodexAuth) -> Result<()> + Send + Sync>,
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

#[derive(Debug, Deserialize)]
struct CodexErrorEnvelope {
    error: Option<CodexErrorBody>,
}

#[derive(Debug, Deserialize)]
struct CodexErrorBody {
    #[serde(rename = "type")]
    error_type: Option<String>,
    message: Option<String>,
    plan_type: Option<String>,
    resets_at: Option<i64>,
    resets_in_seconds: Option<i64>,
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
    previous_response_id: Option<String>,
    content: String,
    tool_slots: HashMap<String, ToolCallSlot>,
    usage: Option<Usage>,
    completion_meta: Option<CodexCompletionMeta>,
    pending_completion: Option<CodexCompletionMeta>,
    unhandled_notice_dedup: HashSet<String>,
    notices: Vec<String>,
    streamed_notice_count: usize,
    streamed_rate_limits: Vec<RateLimitSnapshot>,
}

impl CodexStreamState {
    fn new() -> Self {
        Self {
            response_message_id: None,
            response_id: None,
            previous_response_id: None,
            content: String::new(),
            tool_slots: HashMap::new(),
            usage: None,
            completion_meta: None,
            pending_completion: None,
            unhandled_notice_dedup: HashSet::new(),
            notices: Vec::new(),
            streamed_notice_count: 0,
            streamed_rate_limits: Vec::new(),
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

fn build_responses_api_body(model: &str, messages: &[Message], tools: &Value) -> Value {
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
    body
}

fn build_codex_continuation_body(
    model: &str,
    tools: &Value,
    previous_response_id: &str,
    response_message_id: Option<&str>,
) -> Value {
    let translated_tools = translate_tools(tools);
    let mut body = serde_json::json!({
        "model": model,
        "store": false,
        "tools": translated_tools,
        "stream": true,
        "previous_response_id": previous_response_id,
    });
    if let Some(message_id) = response_message_id {
        body["input"] = serde_json::json!([
            {
                "type": "message",
                "role": "assistant",
                "id": message_id,
                "status": "in_progress",
                "content": []
            }
        ]);
    }
    body
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

    async fn auth_headers(&self) -> Result<(String, String)> {
        self.ensure_fresh_token().await?;
        let guard = self.auth.read().await;
        Ok((guard.access_token.clone(), guard.account_id.clone()))
    }

    async fn send_responses_request(
        &self,
        access_token: &str,
        account_id: &str,
        body: &Value,
    ) -> Result<reqwest::Response> {
        let response = self
            .http
            .post(format!("{}/responses", self.base_url))
            .header("Authorization", format!("Bearer {}", access_token))
            .header("chatgpt-account-id", account_id)
            .header("originator", "pi")
            .header("OpenAI-Beta", "responses=experimental")
            .header("accept", "text/event-stream")
            .json(body)
            .send()
            .await?;
        Ok(response)
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
    let v = headers
        .get(name)?
        .to_str()
        .ok()?
        .trim()
        .to_ascii_lowercase();
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
    let unlimited =
        parse_header_bool(headers, &format!("{prefix}-credits-unlimited")).unwrap_or(false);
    let balance = parse_header_string(headers, &format!("{prefix}-credits-balance"));
    Some(CreditsSnapshot {
        has_credits,
        unlimited,
        balance,
    })
}

fn parse_window_from_headers_any(
    headers: &reqwest::header::HeaderMap,
    prefixes: &[String],
    kind: &str,
) -> Option<RateLimitWindow> {
    prefixes
        .iter()
        .find_map(|prefix| parse_window_from_headers(headers, prefix, kind))
}

fn parse_credits_from_headers_any(
    headers: &reqwest::header::HeaderMap,
    prefixes: &[String],
) -> Option<CreditsSnapshot> {
    prefixes
        .iter()
        .find_map(|prefix| parse_credits_from_headers(headers, prefix))
}

fn collect_limit_ids_from_headers(headers: &reqwest::header::HeaderMap) -> Vec<String> {
    use std::collections::BTreeSet;

    let mut ids = BTreeSet::new();
    ids.insert("codex".to_string());

    for name in headers.keys() {
        let name = name.as_str().to_ascii_lowercase();
        for suffix in [
            "-primary-used-percent",
            "-primary-window-minutes",
            "-primary-reset-at",
            "-secondary-used-percent",
            "-secondary-window-minutes",
            "-secondary-reset-at",
            "-credits-has-credits",
            "-credits-unlimited",
            "-credits-balance",
            "-limit-name",
        ] {
            if let Some(prefix) = name.strip_suffix(suffix) {
                if let Some(limit) = prefix.strip_prefix("x-") {
                    ids.insert(normalize_limit_id(limit));
                }
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
    let prefixes = vec![
        format!("x-{header_limit}"),
        format!("x-ratelimit-{header_limit}"),
        format!("x-ratelimit-{limit_id}"),
    ];

    let primary = parse_window_from_headers_any(headers, &prefixes, "primary");
    let secondary = parse_window_from_headers_any(headers, &prefixes, "secondary");
    let credits = parse_credits_from_headers_any(headers, &prefixes);
    let limit_name = prefixes
        .iter()
        .find_map(|prefix| parse_header_string(headers, &format!("{prefix}-limit-name")));

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

    let rounded_days = (window_minutes + (12 * 60)) / (24 * 60);
    if window_minutes <= 7 * 24 * 60 + DAY_WINDOW_GRACE_MINUTES {
        return format!("{}d", rounded_days.max(1));
    }
    if window_minutes <= 30 * 24 * 60 + DAY_WINDOW_GRACE_MINUTES {
        return format!("{}d", rounded_days.max(1));
    }

    let rounded_days = (window_minutes + (12 * 60)) / (24 * 60);
    format!("{}d", rounded_days.max(1))
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
    Some(dt.format("%Y-%m-%d %H:%M local").to_string())
}

fn format_limit_display(percent_left: f64, resets_at: Option<i64>) -> String {
    match resets_at.and_then(format_reset_time) {
        Some(when) => format!("{percent_left:.0}% left (resets {when})"),
        None => format!("{percent_left:.0}% left"),
    }
}

fn format_compact_duration(total_seconds: i64) -> Option<String> {
    if total_seconds < 0 {
        return None;
    }
    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if days > 0 {
        Some(format!("{days}d {hours:02}h"))
    } else if hours > 0 {
        Some(format!("{hours}h {minutes:02}m"))
    } else if minutes > 0 {
        Some(format!("{minutes}m {seconds:02}s"))
    } else {
        Some(format!("{seconds}s"))
    }
}

fn format_quota_reset_sentence(quota: &ProviderQuotaError) -> Option<String> {
    match (
        quota.resets_at.and_then(format_reset_time),
        quota.resets_in_seconds.and_then(format_compact_duration),
    ) {
        (Some(at), Some(relative)) => Some(format!("Resets at {at} (in {relative})")),
        (Some(at), None) => Some(format!("Resets at {at}")),
        (None, Some(relative)) => Some(format!("Resets in {relative}")),
        (None, None) => None,
    }
}

fn display_provider_name(provider: &str) -> &str {
    match provider {
        "codex" => "Codex",
        _ => provider,
    }
}

fn parse_codex_quota_error(body: &str) -> Option<ProviderQuotaError> {
    let envelope: CodexErrorEnvelope = serde_json::from_str(body).ok()?;
    let error = envelope.error?;
    if error.error_type.is_none()
        && error.message.is_none()
        && error.plan_type.is_none()
        && error.resets_at.is_none()
        && error.resets_in_seconds.is_none()
    {
        return None;
    }

    Some(ProviderQuotaError {
        error_type: error.error_type,
        message: error.message,
        plan_type: error.plan_type,
        resets_at: error.resets_at,
        resets_in_seconds: error.resets_in_seconds,
    })
}

fn build_codex_api_error(status: reqwest::StatusCode, body: String) -> ProviderApiError {
    let quota = (status.as_u16() == 429)
        .then(|| parse_codex_quota_error(&body))
        .flatten();
    ProviderApiError {
        provider: "codex".to_string(),
        http_status: status.as_u16(),
        raw_body: body,
        quota,
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
            status_line_display: format!("{} limit {:.0}%", label, left),
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
            status_line_display: format!("{} limit {:.0}%", label, left),
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

fn classify_codex_event(event_type: &str) -> CodexStreamEventCategory {
    match event_type {
        "response.output_text.delta"
        | "response.function_call_arguments.delta"
        | "response.output_item.added"
        | "codex.rate_limits"
        | "response.completed"
        | "response.failed"
        | "error" => CodexStreamEventCategory::Handled,
        "Created"
        | "ServerModel"
        | "ModelVerifications"
        | "ServerReasoningIncluded"
        | "ModelsEtag" => CodexStreamEventCategory::KnownIgnored,
        _ => CodexStreamEventCategory::Unhandled,
    }
}

async fn process_codex_sse_response(
    mut response: reqwest::Response,
    state: &mut CodexStreamState,
    on_chunk: &mut (dyn FnMut(String) + Send),
    on_status: &mut (dyn FnMut(String) + Send),
    should_cancel: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<()> {
    let mut buf: Vec<u8> = Vec::new();
    let mut current_event: Option<String> = None;
    let mut current_data: Option<String> = None;
    let mut stream_done = false;

    while !stream_done {
        if should_cancel.is_some_and(|cancel| cancel()) {
            anyhow::bail!("interrupted");
        }
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
                        stream_done = true;
                        break;
                    }
                    let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
                    match classify_codex_event(&event_type) {
                        CodexStreamEventCategory::Handled => match event_type.as_str() {
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
                            "response.output_item.added" => {
                                let item = &data["item"];
                                if item["type"] == "function_call" {
                                    let item_id = item["id"].as_str().unwrap_or("").to_string();
                                    let name = item["name"].as_str().unwrap_or("").to_string();
                                    let call_id =
                                        item["call_id"].as_str().unwrap_or("").to_string();
                                    state.tool_slots.insert(
                                        item_id,
                                        ToolCallSlot {
                                            id: call_id,
                                            name,
                                            arguments: String::new(),
                                        },
                                    );
                                } else if item["type"] == "message" {
                                    state.response_message_id =
                                        item["id"].as_str().map(ToString::to_string);
                                }
                            }
                            "codex.rate_limits" => {
                                state
                                    .streamed_rate_limits
                                    .push(parse_rate_limit_snapshot(&data));
                            }
                            "response.completed" => {
                                let completion_meta = parse_completion_meta(&data);
                                state.record_notice(
                                    "codex stream: response.completed".to_string(),
                                    on_status,
                                );
                                state.usage = combine_usage(
                                    state.usage.take(),
                                    completion_meta.usage.clone(),
                                );
                                state.pending_completion = Some(completion_meta.clone());
                                state.completion_meta = Some(completion_meta);
                                stream_done = true;
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
                        },
                        CodexStreamEventCategory::KnownIgnored => {}
                        CodexStreamEventCategory::Unhandled => {
                            state.record_unhandled_event(&event_type, on_status);
                        }
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
                    stream_done = true;
                    break;
                }
                current_data = Some(data_str.to_string());
            }
        }
    }

    Ok(())
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
        build_responses_api_body(model, messages, tools)
    }

    async fn chat_completion_stream(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
        mut on_chunk: Box<dyn FnMut(String) + Send + 'static>,
        mut on_status: Box<dyn FnMut(String) + Send + 'static>,
        should_cancel: Option<Box<dyn Fn() -> bool + Send + Sync + 'static>>,
    ) -> Result<(
        ResponseMessage,
        Option<Usage>,
        Option<ApiCallRateLimitReport>,
        ChatRoundTrace,
    )> {
        let (access_token, account_id) = self.auth_headers().await?;

        let initial_body = build_responses_api_body(model, messages, tools);
        let mut trace_request = initial_body.clone();
        let response = self
            .send_responses_request(&access_token, &account_id, &initial_body)
            .await?;

        let response_headers = response.headers().clone();
        let response_status = response.status();
        if !response_status.is_success() {
            let text = response.text().await?;
            if response_status.as_u16() == 429 {
                let _ = parse_active_rate_limit_from_headers(&response_headers);
            }
            return Err(build_codex_api_error(response_status, text).into());
        }

        let active_limit = parse_header_string(&response_headers, "x-codex-active-limit");
        let mut rate_limit_snapshots = parse_all_rate_limits_from_headers(&response_headers);
        let mut state = CodexStreamState::new();

        process_codex_sse_response(
            response,
            &mut state,
            &mut *on_chunk,
            &mut *on_status,
            should_cancel.as_deref(),
        )
        .await?;

        if let Some(meta) = state.pending_completion.take() {
            state.response_id = meta.response_id.clone();
            if meta.end_turn == Some(false) {
                let previous_response_id = meta
                    .response_id
                    .clone()
                    .or_else(|| state.previous_response_id.clone());
                if let Some(previous_response_id) = previous_response_id {
                    state.previous_response_id = Some(previous_response_id.clone());
                    let continuation_body = build_codex_continuation_body(
                        model,
                        tools,
                        &previous_response_id,
                        state.response_message_id.as_deref(),
                    );
                    trace_request = serde_json::json!({
                        "initial": initial_body,
                        "continuations": [continuation_body.clone()]
                    });
                    let continuation_response = self
                        .send_responses_request(&access_token, &account_id, &continuation_body)
                        .await;
                    match continuation_response {
                        Ok(response) if response.status().is_success() => {
                            let headers = response.headers().clone();
                            rate_limit_snapshots
                                .extend(parse_all_rate_limits_from_headers(&headers));
                            process_codex_sse_response(
                                response,
                                &mut state,
                                &mut *on_chunk,
                                &mut *on_status,
                                should_cancel.as_deref(),
                            )
                            .await?;
                            if let Some(meta) = state.pending_completion.take() {
                                state.response_id = meta.response_id.clone();
                                state.completion_meta = Some(meta);
                            }
                        }
                        Ok(response) => {
                            let body = response.text().await.unwrap_or_default();
                            let _ = body;
                            state
                                .notices
                                .push(CODEX_CONTINUATION_FAILED_NOTICE.to_string());
                        }
                        Err(_) => {
                            state
                                .notices
                                .push(CODEX_CONTINUATION_FAILED_NOTICE.to_string());
                        }
                    }
                } else {
                    state
                        .notices
                        .push(CODEX_CONTINUATION_FAILED_NOTICE.to_string());
                }
            }
        }

        rate_limit_snapshots.extend(state.streamed_rate_limits.clone());
        let rate_limit_report = Some(report_for_api_call(
            "responses",
            "response_headers",
            Some(response_status.as_u16()),
            active_limit,
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
        let trace = ChatRoundTrace {
            backend: "responses".to_string(),
            request: trace_request,
            response: Some(serde_json::json!({
                "message": message,
                "codex_completion": state.completion_meta,
            })),
            error: None,
            http_status: Some(response_status.as_u16()),
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
