use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::agent_runtime::{
    AgentRosterSnapshot, AgentRuntimeEvent, AgentRuntimeService, AgentSnapshot, TranscriptEntry,
};
use crate::terminal_runtime::{TerminalDescriptor, TerminalService};

#[derive(Clone)]
pub struct WebSocketServices {
    pub terminal: TerminalService,
    pub agent: AgentRuntimeService,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "domain", rename_all = "snake_case")]
enum ClientSocketEnvelope {
    Terminal {
        #[serde(flatten)]
        message: TerminalClientMessage,
    },
    Agent {
        #[serde(flatten)]
        message: AgentClientMessage,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalClientMessage {
    CreateTerminal,
    ListTerminals,
    AttachTerminal {
        terminal_id: u64,
    },
    Input {
        terminal_id: u64,
        data: String,
    },
    Resize {
        terminal_id: u64,
        cols: u16,
        rows: u16,
    },
    CloseTerminal {
        terminal_id: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentClientMessage {
    Snapshot,
    Attach {
        agent_id: String,
    },
    PromptSubmit {
        agent_id: String,
        prompt: String,
    },
    Create {
        label: Option<String>,
        roles: Vec<String>,
    },
    Delete {
        agent_id: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "domain", rename_all = "snake_case")]
enum ServerSocketEnvelope {
    Terminal {
        #[serde(flatten)]
        message: TerminalServerMessage,
    },
    Agent {
        #[serde(flatten)]
        message: AgentServerMessage,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalServerMessage {
    TerminalList {
        terminals: Vec<TerminalDescriptor>,
    },
    TerminalCreated {
        terminal: TerminalDescriptor,
    },
    TerminalAttached {
        terminal: TerminalDescriptor,
        scrollback: String,
    },
    TerminalOutput {
        terminal_id: u64,
        data: String,
    },
    TerminalClosed {
        terminal_id: u64,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentServerMessage {
    RosterSnapshot {
        agents: Vec<AgentSummaryWire>,
    },
    AgentCreated {
        agent_id: String,
        label: String,
        roles: Vec<String>,
    },
    AgentDeleted {
        agent_id: String,
    },
    AgentSnapshot {
        agent_id: String,
        label: String,
        roles: Vec<String>,
        busy: bool,
        provider: String,
        model: String,
        transcript: Vec<TranscriptEntryWire>,
        status: String,
        warning: Option<String>,
    },
    BusyState {
        agent_id: String,
        busy: bool,
    },
    TranscriptDelta {
        agent_id: String,
        kind: String,
        text: String,
        replace_last: bool,
    },
    Completed {
        agent_id: String,
    },
    Failed {
        agent_id: String,
        message: String,
    },
}

#[derive(Clone, Debug, Serialize)]
struct AgentSummaryWire {
    agent_id: String,
    label: String,
    roles: Vec<String>,
    busy: bool,
    provider: String,
    model: String,
    status: String,
    warning: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct TranscriptEntryWire {
    kind: String,
    text: String,
}

pub fn shared_ws(ws: WebSocketUpgrade, services: WebSocketServices) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(error) = handle_shared_socket(socket, services).await {
            eprintln!("shared websocket ended with error: {error:#}");
        }
    })
}

async fn handle_shared_socket(socket: WebSocket, services: WebSocketServices) -> Result<()> {
    let (sender, mut receiver) = socket.split();
    let outbound = Arc::new(tokio::sync::Mutex::new(sender));

    send_socket_message(
        &outbound,
        ServerSocketEnvelope::Terminal {
            message: TerminalServerMessage::TerminalList {
                terminals: services.terminal.list_terminals().await?,
            },
        },
    )
    .await?;

    while let Some(message) = receiver.next().await {
        match message? {
            Message::Text(text) => {
                if let Err(error) =
                    handle_client_message(text.as_str(), &services, Arc::clone(&outbound)).await
                {
                    send_socket_message(
                        &outbound,
                        ServerSocketEnvelope::Terminal {
                            message: TerminalServerMessage::Error {
                                message: error.to_string(),
                            },
                        },
                    )
                    .await?;
                }
            }
            Message::Binary(_) => {}
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }

    Ok(())
}

async fn handle_client_message(
    text: &str,
    services: &WebSocketServices,
    outbound: Arc<tokio::sync::Mutex<SplitSink<WebSocket, Message>>>,
) -> Result<()> {
    match serde_json::from_str::<ClientSocketEnvelope>(text)
        .with_context(|| format!("invalid shared websocket payload: {text}"))?
    {
        ClientSocketEnvelope::Terminal { message } => {
            handle_terminal_message(message, &services.terminal, outbound).await?
        }
        ClientSocketEnvelope::Agent { message } => {
            handle_agent_message(message, &services.agent, outbound).await?
        }
    }

    Ok(())
}

async fn handle_terminal_message(
    message: TerminalClientMessage,
    terminal_service: &TerminalService,
    outbound: Arc<tokio::sync::Mutex<SplitSink<WebSocket, Message>>>,
) -> Result<()> {
    match message {
        TerminalClientMessage::CreateTerminal => {
            let terminal = terminal_service.create_terminal().await?;
            send_socket_message(
                &outbound,
                ServerSocketEnvelope::Terminal {
                    message: TerminalServerMessage::TerminalCreated {
                        terminal: terminal.clone(),
                    },
                },
            )
            .await?;
            attach_terminal_stream(
                terminal_service.clone(),
                Arc::clone(&outbound),
                terminal.terminal_id,
            )
            .await?;
        }
        TerminalClientMessage::ListTerminals => {
            send_socket_message(
                &outbound,
                ServerSocketEnvelope::Terminal {
                    message: TerminalServerMessage::TerminalList {
                        terminals: terminal_service.list_terminals().await?,
                    },
                },
            )
            .await?;
        }
        TerminalClientMessage::AttachTerminal { terminal_id } => {
            attach_terminal_stream(terminal_service.clone(), outbound, terminal_id).await?;
        }
        TerminalClientMessage::Input { terminal_id, data } => {
            terminal_service
                .send_input(terminal_id, data.into_bytes())
                .await?;
        }
        TerminalClientMessage::Resize {
            terminal_id,
            cols,
            rows,
        } => {
            terminal_service
                .resize_terminal(terminal_id, cols, rows)
                .await?;
        }
        TerminalClientMessage::CloseTerminal { terminal_id } => {
            terminal_service.close_terminal(terminal_id).await?;
            send_socket_message(
                &outbound,
                ServerSocketEnvelope::Terminal {
                    message: TerminalServerMessage::TerminalClosed { terminal_id },
                },
            )
            .await?;
        }
    }

    Ok(())
}

async fn handle_agent_message(
    message: AgentClientMessage,
    agent_runtime: &AgentRuntimeService,
    outbound: Arc<tokio::sync::Mutex<SplitSink<WebSocket, Message>>>,
) -> Result<()> {
    match message {
        AgentClientMessage::Snapshot => {
            let snapshot = agent_runtime.snapshot().await?;
            send_socket_message(
                &outbound,
                ServerSocketEnvelope::Agent {
                    message: AgentServerMessage::RosterSnapshot {
                        agents: map_roster_snapshot(snapshot),
                    },
                },
            )
            .await?;
        }
        AgentClientMessage::Attach { agent_id } => {
            let mut rx = agent_runtime.subscribe(agent_id.clone()).await?;
            let outbound_for_task = Arc::clone(&outbound);
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let message = map_agent_runtime_event(event);
                    if send_socket_message(
                        &outbound_for_task,
                        ServerSocketEnvelope::Agent { message },
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                }
            });
        }
        AgentClientMessage::PromptSubmit { agent_id, prompt } => {
            agent_runtime.submit_prompt(agent_id, prompt).await?;
        }
        AgentClientMessage::Create { label, roles } => {
            let created = agent_runtime.create_agent(label, roles).await?;
            send_socket_message(
                &outbound,
                ServerSocketEnvelope::Agent {
                    message: AgentServerMessage::AgentCreated {
                        agent_id: created.agent_id,
                        label: created.label,
                        roles: created.roles,
                    },
                },
            )
            .await?;
        }
        AgentClientMessage::Delete { agent_id } => {
            let deleted = agent_runtime.delete_agent(agent_id).await?;
            send_socket_message(
                &outbound,
                ServerSocketEnvelope::Agent {
                    message: AgentServerMessage::AgentDeleted {
                        agent_id: deleted.agent_id,
                    },
                },
            )
            .await?;
        }
    }
    Ok(())
}

async fn attach_terminal_stream(
    terminal_service: TerminalService,
    outbound: Arc<tokio::sync::Mutex<SplitSink<WebSocket, Message>>>,
    terminal_id: u64,
) -> Result<()> {
    let mut handle = terminal_service.attach_terminal(terminal_id).await?;
    send_socket_message(
        &outbound,
        ServerSocketEnvelope::Terminal {
            message: TerminalServerMessage::TerminalAttached {
                terminal: handle.descriptor.clone(),
                scrollback: handle.scrollback,
            },
        },
    )
    .await?;

    tokio::spawn(async move {
        while let Some(data) = handle.output_rx.recv().await {
            if send_socket_message(
                &outbound,
                ServerSocketEnvelope::Terminal {
                    message: TerminalServerMessage::TerminalOutput { terminal_id, data },
                },
            )
            .await
            .is_err()
            {
                break;
            }
        }
    });

    Ok(())
}

async fn send_socket_message(
    outbound: &tokio::sync::Mutex<SplitSink<WebSocket, Message>>,
    message: ServerSocketEnvelope,
) -> Result<()> {
    let payload = serde_json::to_string(&message)?;
    outbound
        .lock()
        .await
        .send(Message::Text(payload.into()))
        .await?;
    Ok(())
}

fn map_roster_snapshot(snapshot: AgentRosterSnapshot) -> Vec<AgentSummaryWire> {
    snapshot
        .agents
        .into_iter()
        .map(|agent| AgentSummaryWire {
            agent_id: agent.agent_id,
            label: agent.label,
            roles: agent.roles,
            busy: agent.busy,
            provider: agent.provider,
            model: agent.model,
            status: agent.status,
            warning: agent.warning,
        })
        .collect()
}

fn map_agent_runtime_event(event: AgentRuntimeEvent) -> AgentServerMessage {
    match event {
        AgentRuntimeEvent::Snapshot(snapshot) => map_agent_snapshot(snapshot),
        AgentRuntimeEvent::RosterUpdated(snapshot) => AgentServerMessage::RosterSnapshot {
            agents: map_roster_snapshot(snapshot),
        },
        AgentRuntimeEvent::Busy { agent_id, busy } => {
            AgentServerMessage::BusyState { agent_id, busy }
        }
        AgentRuntimeEvent::TranscriptDelta(delta) => AgentServerMessage::TranscriptDelta {
            agent_id: delta.agent_id,
            kind: delta.kind,
            text: delta.text,
            replace_last: delta.replace_last,
        },
        AgentRuntimeEvent::Completed { agent_id } => AgentServerMessage::Completed { agent_id },
        AgentRuntimeEvent::Failed { agent_id, message } => {
            AgentServerMessage::Failed { agent_id, message }
        }
        AgentRuntimeEvent::Deleted { agent_id } => AgentServerMessage::AgentDeleted { agent_id },
    }
}

fn map_agent_snapshot(snapshot: AgentSnapshot) -> AgentServerMessage {
    AgentServerMessage::AgentSnapshot {
        agent_id: snapshot.agent_id,
        label: snapshot.label,
        roles: snapshot.roles,
        busy: snapshot.busy,
        provider: snapshot.provider,
        model: snapshot.model,
        transcript: map_transcript(snapshot.transcript),
        status: snapshot.status,
        warning: snapshot.warning,
    }
}

fn map_transcript(entries: Vec<TranscriptEntry>) -> Vec<TranscriptEntryWire> {
    entries
        .into_iter()
        .map(|entry| TranscriptEntryWire {
            kind: entry.kind,
            text: entry.text,
        })
        .collect()
}
