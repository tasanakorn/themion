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

// ── SSE / streaming types (private) ──────────────────────────────────────────

#[derive(Deserialize, Default)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Deserialize)]
struct StreamToolCallDelta {
    index: usize,
    id: Option<String>,
    function: Option<StreamFunctionDelta>,
}

#[derive(Deserialize, Default)]
struct StreamFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamChunkData {
    choices: Vec<StreamChoice>,
    usage: Option<Usage>,
}

struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

// ── ChatBackend trait ─────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait ChatBackend: Send + Sync {
    async fn chat_completion_stream(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
        on_chunk: Box<dyn FnMut(String) + Send + 'static>,
    ) -> Result<(ResponseMessage, Option<Usage>)>;
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct ChatClient {
    client: Client,
    api_key: Option<String>,
    base_url: String,
    extra_headers: Vec<(String, String)>,
}

pub type OpenRouterClient = ChatClient;

impl ChatClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url,
            extra_headers: Vec::new(),
        }
    }

    pub fn with_headers(mut self, headers: impl IntoIterator<Item = (String, String)>) -> Self {
        self.extra_headers.extend(headers);
        self
    }

    fn build_request(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut req = self.client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(body);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }
        for (name, value) in &self.extra_headers {
            req = req.header(name.as_str(), value.as_str());
        }
        req
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

        let response = self.build_request(&body).send().await?;

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

    /// Streaming chat completion. Calls `on_chunk` with each text delta as it
    /// arrives, and returns the fully-assembled `ResponseMessage` + `Usage`
    /// once the stream is complete.
    pub async fn chat_completion_stream(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
        mut on_chunk: impl FnMut(String),
    ) -> Result<(ResponseMessage, Option<Usage>)> {
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "tools": tools,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        let mut response = self.build_request(&body).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            anyhow::bail!("API error {status}: {text}");
        }

        let mut buf: Vec<u8> = Vec::new();
        let mut content = String::new();
        let mut tool_calls_acc: Vec<ToolCallAccum> = Vec::new();
        let mut usage: Option<Usage> = None;
        let mut done = false;

        while !done {
            let Some(bytes) = response.chunk().await? else { break };
            buf.extend_from_slice(&bytes);

            // Process all complete lines (\n-terminated) from the buffer.
            // Splitting on the 0x0A byte is safe: LF cannot appear as a UTF-8
            // continuation byte, so we never split a multi-byte character.
            loop {
                let Some(pos) = buf.iter().position(|&b| b == b'\n') else { break };
                let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                // Strip trailing CRLF or LF
                let line_bytes = line_bytes.strip_suffix(b"\r\n")
                    .or_else(|| line_bytes.strip_suffix(b"\n"))
                    .unwrap_or(&line_bytes);

                let Ok(line) = std::str::from_utf8(line_bytes) else { continue };

                let Some(data) = line.strip_prefix("data: ") else { continue };

                if data == "[DONE]" {
                    done = true;
                    break;
                }

                let Ok(chunk) = serde_json::from_str::<StreamChunkData>(data) else { continue };

                if let Some(u) = chunk.usage {
                    usage = Some(u);
                }

                if let Some(choice) = chunk.choices.into_iter().next() {
                    let delta = choice.delta;

                    if let Some(text) = delta.content {
                        if !text.is_empty() {
                            content.push_str(&text);
                            on_chunk(text);
                        }
                    }

                    if let Some(tcs) = delta.tool_calls {
                        for tc in tcs {
                            let idx = tc.index;
                            while tool_calls_acc.len() <= idx {
                                tool_calls_acc.push(ToolCallAccum {
                                    id: String::new(),
                                    name: String::new(),
                                    arguments: String::new(),
                                });
                            }
                            if let Some(id) = tc.id {
                                tool_calls_acc[idx].id = id;
                            }
                            if let Some(f) = tc.function {
                                if let Some(name) = f.name {
                                    tool_calls_acc[idx].name = name;
                                }
                                if let Some(args) = f.arguments {
                                    tool_calls_acc[idx].arguments.push_str(&args);
                                }
                            }
                        }
                    }
                }
            }
        }

        let tool_calls = if tool_calls_acc.is_empty() {
            None
        } else {
            Some(tool_calls_acc.into_iter().map(|acc| ToolCall {
                id: acc.id,
                function: FunctionCall {
                    name: acc.name,
                    arguments: acc.arguments,
                },
            }).collect())
        };

        let message = ResponseMessage {
            role: "assistant".to_string(),
            content: if content.is_empty() { None } else { Some(content) },
            tool_calls,
        };

        Ok((message, usage))
    }
}

#[async_trait::async_trait]
impl ChatBackend for ChatClient {
    async fn chat_completion_stream(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
        on_chunk: Box<dyn FnMut(String) + Send + 'static>,
    ) -> Result<(ResponseMessage, Option<Usage>)> {
        self.chat_completion_stream(model, messages, tools, on_chunk).await
    }
}
