use anyhow::Result;
use std::time::Instant;
use tokio::sync::mpsc;
use crate::client::{Message, OpenRouterClient};
use crate::tools;

#[derive(Debug, Clone)]
pub struct TurnStats {
    pub llm_rounds: u32,
    pub tool_calls: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cached: u64,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    LlmStart,
    ToolStart { detail: String },
    ToolEnd,
    AssistantText(String),
    TurnDone(TurnStats),
}

fn tool_call_detail(name: &str, args_json: &str) -> String {
    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
    match name {
        "bash" => format!("bash: {}", args["command"].as_str().unwrap_or("?")),
        "read_file" => format!("read: {}", args["path"].as_str().unwrap_or("?")),
        "write_file" => format!("write: {}", args["path"].as_str().unwrap_or("?")),
        "list_directory" => format!("ls: {}", args["path"].as_str().unwrap_or("?")),
        _ => name.to_string(),
    }
}

pub struct Agent {
    client: OpenRouterClient,
    model: String,
    system_prompt: String,
    messages: Vec<Message>,
    event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
}

impl Agent {
    pub fn new(client: OpenRouterClient, model: String, system_prompt: String) -> Self {
        Self {
            client,
            model,
            system_prompt,
            messages: Vec::new(),
            event_tx: None,
        }
    }

    pub fn new_verbose(client: OpenRouterClient, model: String, system_prompt: String) -> Self {
        Self {
            client,
            model,
            system_prompt,
            messages: Vec::new(),
            event_tx: None,
        }
    }

    pub fn new_with_events(
        client: OpenRouterClient,
        model: String,
        system_prompt: String,
        tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Self {
        Self {
            client,
            model,
            system_prompt,
            messages: Vec::new(),
            event_tx: Some(tx),
        }
    }

    fn emit(&self, event: AgentEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event);
        }
    }

    pub async fn run_loop(&mut self, user_input: &str) -> Result<(String, TurnStats)> {
        self.messages.push(Message {
            role: "user".to_string(),
            content: Some(user_input.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });

        let turn_start = Instant::now();
        let tool_defs = tools::tool_definitions();
        let mut final_response = String::new();

        let mut llm_rounds = 0u32;
        let mut tool_calls = 0u32;
        let mut tokens_in = 0u64;
        let mut tokens_out = 0u64;
        let mut tokens_cached = 0u64;

        // TODO: make max iterations configurable
        for _ in 0..10 {
            let mut msgs_with_system = vec![Message {
                role: "system".to_string(),
                content: Some(self.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            }];
            msgs_with_system.extend_from_slice(&self.messages);

            self.emit(AgentEvent::LlmStart);
            let (response, usage) = self.client
                .chat_completion(&self.model, &msgs_with_system, &tool_defs)
                .await?;

            llm_rounds += 1;
            if let Some(u) = usage {
                if let Some(pt) = u.prompt_tokens {
                    tokens_in += pt;
                }
                if let Some(ct) = u.completion_tokens {
                    tokens_out += ct;
                }
                if let Some(details) = u.prompt_tokens_details {
                    if let Some(cached) = details.cached_tokens {
                        tokens_cached += cached;
                    }
                }
            }

            // Push assistant message to history
            self.messages.push(Message {
                role: response.role.clone(),
                content: response.content.clone(),
                tool_calls: response.tool_calls.clone(),
                tool_call_id: None,
            });

            if let Some(ref content) = response.content {
                final_response = content.clone();
                self.emit(AgentEvent::AssistantText(content.clone()));
            }

            let tool_calls_vec = match response.tool_calls {
                Some(calls) if !calls.is_empty() => calls,
                _ => break,
            };

            // Execute each tool call and push results
            for tc in &tool_calls_vec {
                let detail = tool_call_detail(&tc.function.name, &tc.function.arguments);
                self.emit(AgentEvent::ToolStart { detail });
                let result = tools::call_tool(&tc.function.name, &tc.function.arguments).await;
                self.emit(AgentEvent::ToolEnd);
                tool_calls += 1;
                self.messages.push(Message {
                    role: "tool".to_string(),
                    content: Some(result),
                    tool_calls: None,
                    tool_call_id: Some(tc.id.clone()),
                });
            }
        }

        let stats = TurnStats { llm_rounds, tool_calls, tokens_in, tokens_out, tokens_cached, elapsed_ms: turn_start.elapsed().as_millis() };
        self.emit(AgentEvent::TurnDone(stats.clone()));
        Ok((final_response, stats))
    }
}
