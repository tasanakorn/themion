use crate::client::{ChatBackend, Message};
use crate::db::DbHandle;
use crate::tools;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use uuid::Uuid;

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
    AssistantChunk(String),
    AssistantText(String),
    Stats(String),
    TurnDone(TurnStats),
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max].iter().collect::<String>() + "…"
    }
}

fn tool_call_detail(name: &str, args_json: &str) -> String {
    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
    let t = |key: &str| truncate(args[key].as_str().unwrap_or("?"), 60);
    match name {
        "bash" => format!("bash: {}", t("command")),
        "read_file" => format!("read: {}", t("path")),
        "write_file" => format!("write: {}", t("path")),
        "list_directory" => format!("ls: {}", t("path")),
        "recall_history" => format!(
            "recall_history: session={}",
            truncate(args["session_id"].as_str().unwrap_or("current"), 60)
        ),
        "search_history" => format!("search_history: {}", t("query")),
        _ => name.to_string(),
    }
}

pub struct Agent {
    client: Box<dyn ChatBackend + Send + Sync>,
    model: String,
    system_prompt: String,
    messages: Vec<Message>,
    event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
    pub session_id: Uuid,
    pub project_dir: PathBuf,
    pub db: Arc<DbHandle>,
    pub window_turns: usize,
    turn_boundaries: Vec<usize>,
    turn_seq_counter: u32,
}

impl Agent {
    pub fn new(
        client: Box<dyn ChatBackend + Send + Sync>,
        model: String,
        system_prompt: String,
    ) -> Self {
        Self {
            client,
            model,
            system_prompt,
            messages: Vec::new(),
            event_tx: None,
            session_id: Uuid::new_v4(),
            project_dir: PathBuf::new(),
            db: DbHandle::open_in_memory().expect("in-memory db"),
            window_turns: 5,
            turn_boundaries: Vec::new(),
            turn_seq_counter: 0,
        }
    }

    pub fn new_verbose(
        client: Box<dyn ChatBackend + Send + Sync>,
        model: String,
        system_prompt: String,
    ) -> Self {
        Self {
            client,
            model,
            system_prompt,
            messages: Vec::new(),
            event_tx: None,
            session_id: Uuid::new_v4(),
            project_dir: PathBuf::new(),
            db: DbHandle::open_in_memory().expect("in-memory db"),
            window_turns: 5,
            turn_boundaries: Vec::new(),
            turn_seq_counter: 0,
        }
    }

    pub fn new_with_events(
        client: Box<dyn ChatBackend + Send + Sync>,
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
            session_id: Uuid::new_v4(),
            project_dir: PathBuf::new(),
            db: DbHandle::open_in_memory().expect("in-memory db"),
            window_turns: 5,
            turn_boundaries: Vec::new(),
            turn_seq_counter: 0,
        }
    }

    pub fn new_with_db(
        client: Box<dyn ChatBackend + Send + Sync>,
        model: String,
        system_prompt: String,
        session_id: Uuid,
        project_dir: PathBuf,
        db: Arc<DbHandle>,
    ) -> Self {
        Self {
            client,
            model,
            system_prompt,
            messages: Vec::new(),
            event_tx: None,
            session_id,
            project_dir,
            db,
            window_turns: 5,
            turn_boundaries: Vec::new(),
            turn_seq_counter: 0,
        }
    }

    pub fn set_event_tx(&mut self, tx: mpsc::UnboundedSender<AgentEvent>) {
        self.event_tx = Some(tx);
    }

    pub fn clear_context(&mut self) {
        self.messages.clear();
        self.turn_boundaries.clear();
    }

    fn emit(&self, event: AgentEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event);
        }
    }

    pub async fn run_loop(&mut self, user_input: &str) -> Result<(String, TurnStats)> {
        self.turn_seq_counter += 1;
        let turn_seq = self.turn_seq_counter;
        self.turn_boundaries.push(self.messages.len());
        let turn_id = {
            let db = self.db.clone();
            let sid = self.session_id;
            tokio::task::spawn_blocking(move || db.begin_turn(sid, turn_seq)).await??
        };

        self.messages.push(Message {
            role: "user".to_string(),
            content: Some(user_input.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });

        {
            let db = self.db.clone();
            let sid = self.session_id;
            let msg = self.messages.last().unwrap().clone();
            let seq = self.messages.len() as u32;
            tokio::task::spawn_blocking(move || db.append_message(turn_id, sid, seq, &msg))
                .await??;
        }

        let turn_start = Instant::now();
        let tool_defs = tools::tool_definitions();
        let mut final_response = String::new();

        let mut llm_rounds = 0u32;
        let mut tool_calls = 0u32;
        let mut tokens_in = 0u64;
        let mut tokens_out = 0u64;
        let mut tokens_cached = 0u64;

        for _ in 0..10 {
            let mut msgs_with_system = vec![Message {
                role: "system".to_string(),
                content: Some(self.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            }];
            let window_start = if self.turn_boundaries.len() > self.window_turns {
                let omitted = self.turn_boundaries.len() - self.window_turns;
                msgs_with_system.push(Message {
                    role: "system".to_string(),
                    content: Some(format!(
                        "Note: {} earlier turn(s) (seq 1–{}) are stored in history. Use recall_history to load a range or search_history to find a keyword.",
                        omitted, omitted
                    )),
                    tool_calls: None,
                    tool_call_id: None,
                });
                self.turn_boundaries[self.turn_boundaries.len() - self.window_turns]
            } else {
                0
            };
            msgs_with_system.extend_from_slice(&self.messages[window_start..]);

            self.emit(AgentEvent::LlmStart);
            let event_tx = self.event_tx.clone();
            let (response, usage, rate_limit_report) = self
                .client
                .chat_completion_stream(
                    &self.model,
                    &msgs_with_system,
                    &tool_defs,
                    Box::new(move |chunk| {
                        if let Some(ref tx) = event_tx {
                            let _ = tx.send(AgentEvent::AssistantChunk(chunk));
                        }
                    }),
                )
                .await?;

            if let Some(report) = rate_limit_report {
                if let Ok(text) = serde_json::to_string(&report) {
                    self.emit(AgentEvent::Stats(format!("[rate-limit] {}", text)));
                }
            }

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

            self.messages.push(Message {
                role: response.role.clone(),
                content: response.content.clone(),
                tool_calls: response.tool_calls.clone(),
                tool_call_id: None,
            });

            {
                let db = self.db.clone();
                let sid = self.session_id;
                let msg = self.messages.last().unwrap().clone();
                let seq = self.messages.len() as u32;
                tokio::task::spawn_blocking(move || db.append_message(turn_id, sid, seq, &msg))
                    .await??;
            }

            if let Some(ref content) = response.content {
                final_response = content.clone();
            }

            let tool_calls_vec = match response.tool_calls {
                Some(calls) if !calls.is_empty() => calls,
                _ => break,
            };

            for tc in &tool_calls_vec {
                let detail = tool_call_detail(&tc.function.name, &tc.function.arguments);
                self.emit(AgentEvent::ToolStart { detail });
                let tool_ctx = crate::tools::ToolCtx {
                    db: self.db.clone(),
                    session_id: self.session_id,
                    project_dir: self.project_dir.clone(),
                };
                let result =
                    tools::call_tool(&tc.function.name, &tc.function.arguments, &tool_ctx).await;
                self.emit(AgentEvent::ToolEnd);
                tool_calls += 1;
                self.messages.push(Message {
                    role: "tool".to_string(),
                    content: Some(result),
                    tool_calls: None,
                    tool_call_id: Some(tc.id.clone()),
                });
                {
                    let db = self.db.clone();
                    let sid = self.session_id;
                    let msg = self.messages.last().unwrap().clone();
                    let seq = self.messages.len() as u32;
                    tokio::task::spawn_blocking(move || db.append_message(turn_id, sid, seq, &msg))
                        .await??;
                }
            }
        }

        let stats = TurnStats {
            llm_rounds,
            tool_calls,
            tokens_in,
            tokens_out,
            tokens_cached,
            elapsed_ms: turn_start.elapsed().as_millis(),
        };
        {
            let db = self.db.clone();
            let s = stats.clone();
            tokio::task::spawn_blocking(move || db.finalize_turn(turn_id, &s)).await??;
        }
        self.emit(AgentEvent::TurnDone(stats.clone()));
        Ok((final_response, stats))
    }
}
