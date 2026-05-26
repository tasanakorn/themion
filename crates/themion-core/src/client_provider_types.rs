use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Shared rate-limit types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Shared error types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// OpenAI error envelope (shared between codex and compat)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct OpenAIErrorEnvelope {
    pub error: Option<OpenAIErrorDetail>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAIErrorDetail {
    #[serde(default)]
    pub error_type: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub resets_at: Option<i64>,
    #[serde(default)]
    pub resets_in_seconds: Option<i64>,
}

pub(crate) fn parse_openai_quota_error(body: &str) -> Option<ProviderQuotaError> {
    let envelope: OpenAIErrorEnvelope = serde_json::from_str(body).ok()?;
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

pub(crate) fn build_provider_api_error(
    provider: &str,
    status: reqwest::StatusCode,
    body: String,
) -> ProviderApiError {
    let quota = (status.as_u16() == 429)
        .then(|| parse_openai_quota_error(&body))
        .flatten();
    ProviderApiError {
        provider: provider.to_string(),
        http_status: status.as_u16(),
        raw_body: body,
        quota,
    }
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

pub fn display_provider_name(provider: &str) -> &str {
    match provider {
        "codex" => "Codex",
        "openai-compat" => "OpenAI",
        _ => provider,
    }
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

pub fn percent_left(used_percent: f64) -> f64 {
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

pub fn format_compact_duration(total_seconds: i64) -> Option<String> {
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

// ---------------------------------------------------------------------------
// Header parsing helpers
// ---------------------------------------------------------------------------

pub(crate) fn normalize_limit_id(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

pub(crate) fn parse_header_f64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<f64> {
    headers.get(name)?.to_str().ok()?.parse::<f64>().ok()
}

pub(crate) fn parse_header_i64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<i64> {
    headers.get(name)?.to_str().ok()?.parse::<i64>().ok()
}

pub(crate) fn parse_header_bool(headers: &reqwest::header::HeaderMap, name: &str) -> Option<bool> {
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

pub(crate) fn parse_header_string(
    headers: &reqwest::header::HeaderMap,
    name: &str,
) -> Option<String> {
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
    ids.insert("default".to_string());

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
    active_limit_header: &str,
) -> Option<(Option<String>, RateLimitSnapshot)> {
    let active_limit = parse_header_string(headers, active_limit_header);
    let limit_id = active_limit
        .as_deref()
        .map(normalize_limit_id)
        .unwrap_or_else(|| "default".to_string());
    let snapshot = parse_rate_limit_for_limit_from_headers(headers, &limit_id)?;
    Some((active_limit, snapshot))
}

// ---------------------------------------------------------------------------
// Rate-limit snapshot extraction / reporting
// ---------------------------------------------------------------------------

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
