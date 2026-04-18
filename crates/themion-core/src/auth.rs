use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct CodexAuth {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64, // unix epoch seconds
    pub account_id: String,
}

impl CodexAuth {
    pub fn is_expired(&self, skew_secs: i64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.expires_at - skew_secs <= now
    }
}
