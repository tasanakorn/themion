use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCall {
    pub id: String,
    pub function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct UsageDetails {
    pub cached_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub prompt_tokens_details: Option<UsageDetails>,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
pub struct ResponseMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

pub struct ChatClient {
    client: Client,
    api_key: Option<String>,
    base_url: String,
}

pub type OpenRouterClient = ChatClient;

impl ChatClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url,
        }
    }

    pub fn new_openrouter(api_key: String) -> Self {
        Self::new("https://openrouter.ai/api/v1".to_string(), Some(api_key))
    }

    pub async fn chat_completion(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
    ) -> Result<(ResponseMessage, Option<Usage>)> {
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "tools": tools,
        });

        let mut request = self.client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&body);

        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            anyhow::bail!("API error {status}: {text}");
        }

        let chat_response: ChatResponse = response.json().await?;
        let message = chat_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no choices in response"))?
            .message;

        Ok((message, chat_response.usage))
    }
}
