use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;
use tokio::task::yield_now;

use crate::client::{
    build_chat_completions_body, ChatBackend, ChatRoundTrace, FunctionCall, Message, ModelInfo,
    ResponseMessage, ToolCall, Usage,
};
use crate::client_provider_types::{
    build_provider_api_error, parse_all_rate_limits_from_headers, parse_header_string,
    report_for_api_call, ApiCallRateLimitReport,
};

pub struct OpenAICompatClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    reasoning_effort: Option<String>,
    extra_headers: Vec<(String, String)>,
}

impl OpenAICompatClient {
    pub fn new(base_url: String, api_key: String, reasoning_effort: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            base_url,
            reasoning_effort,
            extra_headers: Vec::new(),
        }
    }

    pub fn with_headers(mut self, headers: impl IntoIterator<Item = (String, String)>) -> Self {
        self.extra_headers.extend(headers);
        self
    }

    fn build_request(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(body);
        for (name, value) in &self.extra_headers {
            req = req.header(name.as_str(), value.as_str());
        }
        req
    }
}

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

#[async_trait::async_trait]
impl ChatBackend for OpenAICompatClient {
    fn backend_name(&self) -> &'static str {
        "openai_compat"
    }

    fn build_round_request_payload(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
    ) -> Value {
        let mut body = build_chat_completions_body(model, messages, tools, true);
        if let Some(effort) = &self.reasoning_effort {
            body["reasoning_effort"] = Value::String(effort.clone());
        }
        body
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
        let mut body = build_chat_completions_body(model, messages, tools, true);
        if let Some(effort) = &self.reasoning_effort {
            body["reasoning_effort"] = Value::String(effort.clone());
        }

        let tool_count = tools.as_array().map_or(0, Vec::len);
        on_status(format!(
            "openai-compat: generating prompt model={model} messages={} tools={tool_count}",
            messages.len()
        ));

        let mut response = self.build_request(&body).send().await?;
        let response_status = response.status();
        let response_headers = response.headers().clone();

        if !response_status.is_success() {
            let text = response.text().await?;
            return Err(build_provider_api_error("openai-compat", response_status, text).into());
        }

        let active_limit = parse_header_string(&response_headers, "x-ratelimit-active-limit");
        let mut rate_limit_snapshots = parse_all_rate_limits_from_headers(&response_headers);

        let mut buf: Vec<u8> = Vec::new();
        let mut content = String::new();
        let mut tool_calls_acc: Vec<ToolCallAccum> = Vec::new();
        let mut usage: Option<Usage> = None;
        let mut done = false;

        while !done {
            if should_cancel.as_ref().is_some_and(|cancel| cancel()) {
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

                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };

                if data == "[DONE]" {
                    done = true;
                    break;
                }

                let Ok(chunk) = serde_json::from_str::<StreamChunkData>(data) else {
                    continue;
                };

                if let Some(u) = chunk.usage {
                    usage = Some(u);
                }

                if let Some(choice) = chunk.choices.into_iter().next() {
                    let delta = choice.delta;

                    if let Some(text) = delta.content {
                        if !text.is_empty() {
                            if should_cancel.as_ref().is_some_and(|cancel| cancel()) {
                                anyhow::bail!("interrupted");
                            }
                            content.push_str(&text);
                            on_chunk(text);
                            yield_now().await;
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

        // Parse rate limits from final response headers
        let final_headers = response.headers();
        rate_limit_snapshots.extend(parse_all_rate_limits_from_headers(&final_headers));

        let tool_calls = if tool_calls_acc.is_empty() {
            None
        } else {
            Some(
                tool_calls_acc
                    .into_iter()
                    .map(|acc| ToolCall {
                        id: acc.id,
                        function: FunctionCall {
                            name: acc.name,
                            arguments: acc.arguments,
                        },
                    })
                    .collect(),
            )
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

        let rate_limit_report = Some(report_for_api_call(
            "chat_completions",
            "response_headers",
            Some(response_status.as_u16()),
            active_limit,
            rate_limit_snapshots,
        ));

        let trace = ChatRoundTrace {
            backend: "openai_compat".to_string(),
            request: body,
            response: Some(serde_json::to_value(&message)?),
            error: None,
            http_status: Some(response_status.as_u16()),
            usage: usage.clone(),
            rate_limits: rate_limit_report.clone(),
        };

        Ok((message, usage, rate_limit_report, trace))
    }

    async fn fetch_model_info(&self, model: &str) -> Result<Option<ModelInfo>> {
        let response = self
            .http
            .get(format!("{}/models/{}", self.base_url, model))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(None);
        }

        #[derive(Deserialize)]
        struct ModelResponse {
            id: String,
            owned_by: Option<String>,
            context_length: Option<u64>,
            max_context_length: Option<u64>,
        }

        let model_resp: ModelResponse = response.json().await?;
        Ok(Some(ModelInfo {
            id: model_resp.id,
            display_name: model_resp.owned_by,
            context_window: model_resp.context_length,
            max_context_window: model_resp.max_context_length,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_payload_includes_reasoning_effort() {
        let client = OpenAICompatClient::new(
            "http://localhost".to_string(),
            "key".to_string(),
            Some("high".to_string()),
        );
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some("hello".to_string()),
            tool_calls: None,
            tool_call_id: None,
        }];
        let payload =
            client.build_round_request_payload("gpt-4o", &messages, &serde_json::json!([]));
        assert_eq!(payload["reasoning_effort"], "high");
        assert_eq!(payload["model"], "gpt-4o");
        assert_eq!(payload["stream"], true);
    }

    #[test]
    fn build_payload_omits_reasoning_effort_when_none() {
        let client =
            OpenAICompatClient::new("http://localhost".to_string(), "key".to_string(), None);
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some("hello".to_string()),
            tool_calls: None,
            tool_call_id: None,
        }];
        let payload =
            client.build_round_request_payload("gpt-4o", &messages, &serde_json::json!([]));
        assert!(payload.get("reasoning_effort").is_none());
    }
}
