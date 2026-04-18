use anyhow::Result;
use crate::client::{Message, OpenRouterClient};
use crate::tools;

pub struct Agent {
    client: OpenRouterClient,
    model: String,
    system_prompt: String,
    messages: Vec<Message>,
}

impl Agent {
    pub fn new(client: OpenRouterClient, model: String, system_prompt: String) -> Self {
        Self {
            client,
            model,
            system_prompt,
            messages: Vec::new(),
        }
    }

    pub async fn run_loop(&mut self, user_input: &str) -> Result<String> {
        self.messages.push(Message {
            role: "user".to_string(),
            content: Some(user_input.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });

        let tool_defs = tools::tool_definitions();
        let mut final_response = String::new();

        // TODO: make max iterations configurable
        for _ in 0..10 {
            let mut msgs_with_system = vec![Message {
                role: "system".to_string(),
                content: Some(self.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            }];
            msgs_with_system.extend_from_slice(&self.messages);

            let response = self.client
                .chat_completion(&self.model, &msgs_with_system, &tool_defs)
                .await?;

            // Push assistant message to history
            self.messages.push(Message {
                role: response.role.clone(),
                content: response.content.clone(),
                tool_calls: response.tool_calls.clone(),
                tool_call_id: None,
            });

            if let Some(ref content) = response.content {
                final_response = content.clone();
            }

            let tool_calls = match response.tool_calls {
                Some(calls) if !calls.is_empty() => calls,
                _ => break,
            };

            // Execute each tool call and push results
            for tc in &tool_calls {
                let result = tools::call_tool(&tc.function.name, &tc.function.arguments).await;
                self.messages.push(Message {
                    role: "tool".to_string(),
                    content: Some(result),
                    tool_calls: None,
                    tool_call_id: Some(tc.id.clone()),
                });
            }
        }

        Ok(final_response)
    }
}
