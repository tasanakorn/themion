use std::future::Future;
use std::pin::Pin;
use anyhow::{anyhow, Result};
use base64::Engine;
use themion_core::CodexAuth;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ISSUER: &str = "https://auth.openai.com";
const JWT_CLAIM_NS: &str = "https://api.openai.com/auth";

pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
}

/// Returns (DeviceCodeInfo, polling future).
/// Caller displays DeviceCodeInfo immediately, then awaits the future.
pub async fn start_device_flow() -> Result<(DeviceCodeInfo, Pin<Box<dyn Future<Output = Result<CodexAuth>> + Send + 'static>>)> {
    let client = reqwest::Client::new();

    // Step 1: request user code (JSON body, JSON response)
    let resp = client
        .post(format!("{ISSUER}/api/accounts/deviceauth/usercode"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "client_id": CLIENT_ID }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("device auth request failed {status}: {text}"));
    }

    let json: serde_json::Value = resp.json().await?;
    let device_auth_id = json["device_auth_id"].as_str()
        .ok_or_else(|| anyhow!("missing device_auth_id in response"))?
        .to_string();
    let user_code = json["user_code"]
        .as_str()
        .or_else(|| json["usercode"].as_str())
        .ok_or_else(|| anyhow!("missing user_code in response"))?
        .to_string();
    let interval = json["interval"].as_str()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .or_else(|| json["interval"].as_u64())
        .unwrap_or(5);

    // Verification URL is fixed — not returned by the endpoint
    let verification_uri = format!("{ISSUER}/codex/device");

    let info = DeviceCodeInfo { user_code: user_code.clone(), verification_uri };

    let poll_fut = Box::pin(async move {
        poll_for_auth(client, device_auth_id, user_code, interval).await
    });

    Ok((info, poll_fut))
}

/// Poll /deviceauth/token until the user completes the browser step.
/// 403/404 = still pending; 200 = authorization_code + server-generated PKCE.
async fn poll_for_auth(
    client: reqwest::Client,
    device_auth_id: String,
    user_code: String,
    interval: u64,
) -> Result<CodexAuth> {
    let token_url = format!("{ISSUER}/api/accounts/deviceauth/token");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15 * 60);

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        if std::time::Instant::now() >= deadline {
            return Err(anyhow!("device code login timed out after 15 minutes"));
        }

        let resp = client
            .post(&token_url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "device_auth_id": device_auth_id,
                "user_code": user_code,
            }))
            .send()
            .await?;

        let status = resp.status();

        if status.is_success() {
            let json: serde_json::Value = resp.json().await?;
            let authorization_code = json["authorization_code"].as_str()
                .ok_or_else(|| anyhow!("missing authorization_code in token poll response"))?
                .to_string();
            let code_verifier = json["code_verifier"].as_str()
                .ok_or_else(|| anyhow!("missing code_verifier in token poll response"))?
                .to_string();
            return exchange_code(client, authorization_code, code_verifier).await;
        }

        // 403 or 404 = user hasn't completed browser step yet
        if status.as_u16() == 403 || status.as_u16() == 404 {
            continue;
        }

        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("device auth poll failed {status}: {text}"));
    }
}

/// Exchange the authorization code (+ server-provided code_verifier) for tokens.
async fn exchange_code(
    client: reqwest::Client,
    authorization_code: String,
    code_verifier: String,
) -> Result<CodexAuth> {
    let redirect_uri = format!("{ISSUER}/deviceauth/callback");

    let resp = client
        .post(format!("{ISSUER}/oauth/token"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
            urlencoding::encode(&authorization_code),
            urlencoding::encode(&redirect_uri),
            urlencoding::encode(CLIENT_ID),
            urlencoding::encode(&code_verifier),
        ))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("token exchange failed {status}: {text}"));
    }

    let json: serde_json::Value = resp.json().await?;
    let access_token = json["access_token"].as_str()
        .ok_or_else(|| anyhow!("missing access_token"))?
        .to_string();
    let refresh_token = json["refresh_token"].as_str()
        .ok_or_else(|| anyhow!("missing refresh_token"))?
        .to_string();
    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let expires_at = now + expires_in;

    // Extract account_id from access_token JWT (same as pi-mono)
    let account_id = extract_account_id(&access_token)
        .or_else(|_| {
            // Fallback: try id_token if present
            json["id_token"].as_str()
                .ok_or_else(|| anyhow!("no id_token"))
                .and_then(extract_account_id)
        })?;

    Ok(CodexAuth { access_token, refresh_token, expires_at, account_id })
}

fn extract_account_id(jwt: &str) -> Result<String> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return Err(anyhow!("invalid JWT format"));
    }
    // Add padding if needed
    let payload_b64 = parts[1];
    let padded = match payload_b64.len() % 4 {
        2 => format!("{payload_b64}=="),
        3 => format!("{payload_b64}="),
        _ => payload_b64.to_string(),
    };
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(&padded))
        .map_err(|e| anyhow!("JWT base64 decode error: {e}"))?;
    let payload: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|e| anyhow!("JWT JSON parse error: {e}"))?;
    let account_id = payload[JWT_CLAIM_NS]["chatgpt_account_id"]
        .as_str()
        .ok_or_else(|| anyhow!("missing chatgpt_account_id in JWT"))?
        .to_string();
    Ok(account_id)
}
