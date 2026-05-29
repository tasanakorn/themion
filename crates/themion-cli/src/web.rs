use crate::app_state::{AppRuntimeState, AppSnapshot, AppState};
use crate::surface_runner::{
    handle_surface_app_event, handle_surface_runtime_event, start_snapshot_watch_loop,
    SurfaceRunnerContext,
};
use anyhow::{anyhow, Context};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, put};
use axum::Json;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc};

const DEFAULT_WEB_BIND_ADDR: &str = "127.0.0.1:8420";
const RECENT_EVENT_TEXT_MAX_CHARS: usize = 160;
const RECENT_EVENTS_LIMIT: usize = 8;
const TRANSCRIPT_EVENTS_LIMIT: usize = 24;
const WORKSPACE_TEXT_FILE_MAX_BYTES: u64 = 1_048_576;
const WORKSPACE_PREVIEW_FILE_MAX_BYTES: u64 = 52_428_800;

#[derive(Clone)]
struct WebAppState {
    app_state: Arc<AppState>,
    bind_addr: SocketAddr,
    web_input_tx: mpsc::UnboundedSender<WebInputEvent>,
    terminal_service: crate::web_terminal::TerminalService,
    websocket_sequence: Arc<AtomicU64>,
    agent_event_tx: broadcast::Sender<WebAgentEvent>,
    chat_entries: Arc<std::sync::Mutex<Vec<WebChatEntry>>>,
}

#[derive(Clone)]
enum WebInputEvent {
    SubmitPrompt { agent_id: String, prompt: String },
}

#[derive(Clone, Debug)]
struct WebAgentEvent {
    agent_id: String,
    event_kind: String,
    text: String,
    at_ms: u64,
}

#[derive(Default)]
struct WebSocketSubscriptions {
    agents: HashSet<String>,
    runtime: HashSet<String>,
}

impl WebSocketSubscriptions {
    fn subscribe_agent(&mut self, agent_id: String) {
        self.agents.insert(agent_id);
    }

    fn unsubscribe_agent(&mut self, agent_id: &str) {
        self.agents.remove(agent_id);
    }

    fn is_agent_subscribed(&self, agent_id: &str) -> bool {
        self.agents.contains(agent_id)
    }

    fn subscribe_runtime(&mut self, target_id: String) {
        self.runtime.insert(target_id);
    }

    fn unsubscribe_runtime(&mut self, target_id: &str) {
        self.runtime.remove(target_id);
    }

    fn is_runtime_subscribed(&self, target_id: &str) -> bool {
        self.runtime.contains(target_id)
    }
}

#[derive(Serialize)]
struct WebStatusResponse {
    mode: &'static str,
    bind_addr: String,
    project_dir: String,
    session_id: String,
    primary_agent_id: Option<String>,
    busy: bool,
    activity_status: Option<String>,
    local_agents: Vec<WebAgentStatus>,
    runtime: WebRuntimeSummary,
    recent_events: Vec<WebRecentEvent>,
}

#[derive(Serialize)]
struct WebTranscriptResponse {
    mode: &'static str,
    bind_addr: String,
    session_id: String,
    transcript_events: Vec<WebRecentEvent>,
    chat_entries: Vec<WebChatEntry>,
}

#[derive(Serialize, Clone)]
struct WebChatEntry {
    kind: &'static str,
    agent_id: Option<String>,
    tool_call_id: Option<String>,
    source: Option<&'static str>,
    text: String,
    detail: Option<String>,
    reason: Option<String>,
    stats: Option<String>,
    completed: bool,
}

#[derive(Serialize)]
struct WebAgentsResponse {
    mode: &'static str,
    bind_addr: String,
    session_id: String,
    primary_agent_id: Option<String>,
    activity_status: Option<String>,
    local_agents: Vec<WebAgentStatus>,
}

#[derive(Serialize)]
struct WebAgentStatus {
    agent_id: String,
    label: String,
    roles: Vec<String>,
    busy: bool,
    incoming: bool,
    activity_status: Option<String>,
    activity_label: Option<String>,
    activity_changed_at_ms: Option<u64>,
    provider_connection: Option<themion_core::ProviderConnectionStatus>,
}

#[derive(Serialize)]
struct WebRuntimeSummary {
    configured_profile: String,
    active_profile: String,
    provider: String,
    model: String,
    workflow_name: String,
    workflow_phase: String,
    workflow_status: String,
    workflow_phase_result: String,
    session_tokens_in: u64,
    session_tokens_out: u64,
    session_tokens_cached: u64,
    llm_rounds: u64,
    tool_calls: u64,
    elapsed_ms: u64,
    process_started_at_ms: u64,
    idle_state_changed_at_ms: Option<u64>,
    activity_changed_at_ms: Option<u64>,
    pending_text: Option<String>,
    rate_limits: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct WebRecentEvent {
    kind: String,
    text: String,
    at_ms: u64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct WebSocketEnvelope {
    kind: String,
    domain: String,
    target_id: String,
    #[serde(default)]
    sequence_id: Option<u64>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct TerminalStreamEventPayload {
    terminal_id: u64,
    label: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct AgentStreamEventPayload {
    agent_id: String,
    event_kind: String,
    text: String,
    at_ms: u64,
}

#[derive(Debug, Serialize)]
struct TerminalAttachedPayload {
    terminal_id: u64,
    label: String,
    scrollback: String,
}

#[derive(Debug, Serialize)]
struct TerminalListPayload {
    terminals: Vec<crate::web_terminal::TerminalDescriptor>,
}

#[derive(Debug, Deserialize)]
struct WorkspacePathQuery {
    #[serde(default)]
    path: String,
}

#[derive(Debug, Deserialize)]
struct WorkspaceSaveRequest {
    path: String,
    content: String,
    conflict_token: String,
}

#[derive(Debug, Serialize)]
struct WorkspaceEntry {
    path: String,
    name: String,
    kind: WorkspaceFileKind,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct WorkspaceTreeResponse {
    path: String,
    kind: WorkspaceFileKind,
    entries: Vec<WorkspaceEntry>,
}

#[derive(Debug, Serialize)]
struct WorkspaceFileResponse {
    path: String,
    kind: WorkspaceFileKind,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
    conflict_token: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct WorkspaceMetaResponse {
    path: String,
    kind: WorkspaceFileKind,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
    editable: bool,
    previewable: bool,
    conflict_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceSaveResponse {
    path: String,
    kind: WorkspaceFileKind,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
    conflict_token: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum WorkspaceFileKind {
    Directory,
    Text,
    Image,
    Video,
    Markdown,
    Html,
    Pdf,
    Binary,
    Unsupported,
}

#[derive(Debug, Serialize)]
struct WorkspaceErrorBody {
    error: WorkspaceErrorDetail,
}

#[derive(Debug, Serialize)]
struct WorkspaceErrorDetail {
    code: &'static str,
    message: String,
}

pub fn parse_bind_addr(raw: Option<&str>) -> anyhow::Result<SocketAddr> {
    let candidate = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_WEB_BIND_ADDR);
    candidate
        .parse()
        .with_context(|| format!("invalid web bind address '{candidate}'"))
}

pub fn run(mut app_state: AppState, bind_addr: SocketAddr) -> anyhow::Result<()> {
    let missing_assets = crate::web_assets::missing_spa_assets();
    if !missing_assets.is_empty() {
        anyhow::bail!(
            "themion --web is missing embedded SPA assets: {}. Rebuild them with cargo build -p themion-cli",
            missing_assets.join(", ")
        );
    }
    let runtime_domains = app_state.runtime_domains.clone();
    let web_runtime = runtime_domains.core();
    web_runtime.block_on(async move {
        let (agent_event_tx, _) = broadcast::channel::<WebAgentEvent>(256);
        let chat_entries = Arc::new(std::sync::Mutex::new(Vec::<WebChatEntry>::new()));
        let web_input_tx =
            start_web_surface_loop(&mut app_state, agent_event_tx.clone(), chat_entries.clone())
                .await?;
        let terminal_service = start_terminal_runtime().await?;
        #[cfg(feature = "stylos")]
        let stylos = app_state.runtime.stylos.take();
        let app = router(WebAppState {
            app_state: Arc::new(app_state),
            bind_addr,
            web_input_tx,
            terminal_service,
            websocket_sequence: Arc::new(AtomicU64::new(1)),
            agent_event_tx,
            chat_entries,
        });
        let listener = tokio::net::TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("failed to bind web server on {bind_addr}"))?;
        println!("themion web mode listening on http://{bind_addr}");
        let serve_result = axum::serve(listener, app)
            .await
            .context("web server exited unexpectedly");
        #[cfg(feature = "stylos")]
        if let Some(stylos) = stylos {
            stylos.shutdown().await;
        }
        serve_result
    })
}

fn router(state: WebAppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app", get(index))
        .route("/assets/app.css", get(spa_css_asset))
        .route("/assets/themion_cli_web_ui.js", get(spa_js_asset))
        .route("/assets/themion_cli_web_ui_bg.wasm", get(spa_wasm_asset))
        .route(
            "/assets/codemirror_editor.js",
            get(codemirror_editor_js_asset),
        )
        .route(
            "/assets/fonts/JetBrainsMonoNerdFont-Regular.ttf",
            get(font_asset),
        )
        .route("/transcript", get(index))
        .route("/shell", get(index))
        .route("/api/ws", get(multiplex_ws))
        .route("/health", get(health))
        .route("/agents", get(index))
        .route("/api/status", get(status))
        .route("/api/agents", get(agents_api))
        .route("/api/transcript", get(transcript_api))
        .route("/api/workspace/tree", get(workspace_tree_api))
        .route("/api/workspace/file", get(workspace_file_api))
        .route("/api/workspace/file", put(workspace_save_api))
        .route("/api/workspace/meta", get(workspace_meta_api))
        .route("/api/workspace/preview", get(workspace_preview_api))
        .with_state(state)
}

async fn index(State(_state): State<WebAppState>) -> Response {
    html_response(crate::web_assets::spa_html().to_string())
}

async fn spa_css_asset() -> Response {
    asset_response(
        crate::web_assets::spa_css().as_bytes(),
        "text/css; charset=utf-8",
    )
}

async fn spa_js_asset() -> Response {
    asset_response(
        crate::web_assets::spa_js().as_bytes(),
        "application/javascript; charset=utf-8",
    )
}

async fn codemirror_editor_js_asset() -> Response {
    asset_response(
        crate::web_assets::codemirror_editor_js().as_bytes(),
        "application/javascript; charset=utf-8",
    )
}

async fn font_asset() -> Response {
    binary_asset_response(crate::web_assets::jetbrains_mono_nerd_font(), "font/ttf")
}

async fn spa_wasm_asset() -> Response {
    binary_asset_response(crate::web_assets::spa_wasm(), "application/wasm")
}

async fn multiplex_ws(ws: WebSocketUpgrade, State(state): State<WebAppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(error) = handle_multiplex_socket(socket, state).await {
            eprintln!("websocket ended with error: {error:#}");
        }
    })
}

async fn handle_multiplex_socket(socket: WebSocket, state: WebAppState) -> anyhow::Result<()> {
    let (mut sender, mut receiver) = socket.split();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<WebSocketEnvelope>();
    let subscriptions = Arc::new(tokio::sync::Mutex::new(WebSocketSubscriptions::default()));

    let writer_task = tokio::spawn(async move {
        while let Some(envelope) = outbound_rx.recv().await {
            let text = serde_json::to_string(&envelope)?;
            sender.send(Message::Text(text.into())).await?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let agent_events_task = {
        let outbound_tx = outbound_tx.clone();
        let state = state.clone();
        let subscriptions = subscriptions.clone();
        let mut agent_event_rx = state.agent_event_tx.subscribe();
        tokio::spawn(async move {
            loop {
                let event = match agent_event_rx.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if !subscriptions
                    .lock()
                    .await
                    .is_agent_subscribed(&event.agent_id)
                {
                    continue;
                }
                let payload = match serde_json::to_value(AgentStreamEventPayload {
                    agent_id: event.agent_id.clone(),
                    event_kind: event.event_kind,
                    text: event.text,
                    at_ms: event.at_ms,
                }) {
                    Ok(payload) => payload,
                    Err(_) => break,
                };
                if outbound_tx
                    .send(WebSocketEnvelope {
                        kind: "event".to_string(),
                        domain: "agent".to_string(),
                        target_id: event.agent_id,
                        sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
                        request_id: None,
                        payload,
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    };

    let runtime_events_task = {
        let outbound_tx = outbound_tx.clone();
        let state = state.clone();
        let subscriptions = subscriptions.clone();
        let mut snapshot_rx = state.app_state.snapshot_hub.subscribe();
        tokio::spawn(async move {
            loop {
                if snapshot_rx.changed().await.is_err() {
                    break;
                }
                if !subscriptions.lock().await.is_runtime_subscribed("status") {
                    continue;
                }
                let snapshot = snapshot_rx.borrow().clone();
                let payload = match serde_json::to_value(build_status_response(&state, &snapshot)) {
                    Ok(payload) => payload,
                    Err(_) => break,
                };
                if outbound_tx
                    .send(WebSocketEnvelope {
                        kind: "snapshot".to_string(),
                        domain: "runtime".to_string(),
                        target_id: "status".to_string(),
                        sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
                        request_id: None,
                        payload,
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    };

    while let Some(message) = receiver.next().await {
        match message? {
            Message::Text(text) => {
                let envelope: WebSocketEnvelope = serde_json::from_str(&text)
                    .with_context(|| format!("invalid websocket envelope: {text}"))?;
                handle_websocket_envelope(&state, &outbound_tx, &subscriptions, envelope).await?;
            }
            Message::Binary(_) => {}
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }

    drop(outbound_tx);
    agent_events_task.abort();
    runtime_events_task.abort();
    let _ = agent_events_task.await;
    let _ = runtime_events_task.await;
    writer_task.await??;
    Ok(())
}

async fn handle_websocket_envelope(
    state: &WebAppState,
    outbound_tx: &mpsc::UnboundedSender<WebSocketEnvelope>,
    subscriptions: &Arc<tokio::sync::Mutex<WebSocketSubscriptions>>,
    envelope: WebSocketEnvelope,
) -> anyhow::Result<()> {
    match (envelope.kind.as_str(), envelope.domain.as_str()) {
        ("create", "terminal") => {
            let descriptor = state.terminal_service.create_terminal().await?;
            send_terminal_attach(
                state,
                outbound_tx,
                descriptor.terminal_id,
                envelope.request_id,
            )
            .await
        }
        ("subscribe", "agent") => {
            subscriptions
                .lock()
                .await
                .subscribe_agent(envelope.target_id.clone());
            outbound_tx.send(WebSocketEnvelope {
                kind: "snapshot".to_string(),
                domain: "agent".to_string(),
                target_id: envelope.target_id.clone(),
                sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
                request_id: envelope.request_id,
                payload: serde_json::json!({
                    "agent_id": envelope.target_id,
                    "activity_status": state.app_state.snapshot_hub.current().activity_status,
                    "local_agents": state.app_state.snapshot_hub.current().local_agents.iter().map(|agent| serde_json::json!({
                        "agent_id": agent.agent_id,
                        "label": agent.label,
                        "roles": agent.roles,
                        "busy": agent.busy,
                        "incoming": false,
                    })).collect::<Vec<_>>(),
                    "latest_event": serde_json::Value::Null,
                }),
            }).map_err(|_| anyhow!("websocket outbound channel closed"))?;
            Ok(())
        }
        ("unsubscribe", "agent") => {
            subscriptions
                .lock()
                .await
                .unsubscribe_agent(&envelope.target_id);
            outbound_tx
                .send(WebSocketEnvelope {
                    kind: "ack".to_string(),
                    domain: "agent".to_string(),
                    target_id: envelope.target_id,
                    sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
                    request_id: envelope.request_id,
                    payload: serde_json::json!({"subscribed": false}),
                })
                .map_err(|_| anyhow!("websocket outbound channel closed"))?;
            Ok(())
        }
        ("subscribe", "runtime") => {
            subscriptions
                .lock()
                .await
                .subscribe_runtime(envelope.target_id.clone());
            let snapshot = state.app_state.snapshot_hub.current();
            outbound_tx
                .send(WebSocketEnvelope {
                    kind: "snapshot".to_string(),
                    domain: "runtime".to_string(),
                    target_id: envelope.target_id,
                    sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
                    request_id: envelope.request_id,
                    payload: serde_json::to_value(build_status_response(state, &snapshot))?,
                })
                .map_err(|_| anyhow!("websocket outbound channel closed"))?;
            Ok(())
        }
        ("unsubscribe", "runtime") => {
            subscriptions
                .lock()
                .await
                .unsubscribe_runtime(&envelope.target_id);
            outbound_tx
                .send(WebSocketEnvelope {
                    kind: "ack".to_string(),
                    domain: "runtime".to_string(),
                    target_id: envelope.target_id,
                    sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
                    request_id: envelope.request_id,
                    payload: serde_json::json!({"subscribed": false}),
                })
                .map_err(|_| anyhow!("websocket outbound channel closed"))?;
            Ok(())
        }
        ("subscribe", "terminal") => {
            if envelope.target_id == "list" {
                let terminals = state.terminal_service.list_terminals().await?;
                outbound_tx
                    .send(WebSocketEnvelope {
                        kind: "snapshot".to_string(),
                        domain: "terminal".to_string(),
                        target_id: "list".to_string(),
                        sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
                        request_id: envelope.request_id,
                        payload: serde_json::to_value(TerminalListPayload { terminals })?,
                    })
                    .map_err(|_| anyhow!("websocket outbound channel closed"))?;
                Ok(())
            } else {
                let terminal_id = parse_terminal_target_id(&envelope.target_id)?;
                send_terminal_attach(state, outbound_tx, terminal_id, envelope.request_id).await
            }
        }
        ("input", "agent") => {
            let prompt = extract_prompt_payload(&envelope.payload)?;
            state
                .web_input_tx
                .send(WebInputEvent::SubmitPrompt {
                    agent_id: envelope.target_id.clone(),
                    prompt,
                })
                .map_err(|_| anyhow!("web input channel closed"))?;
            outbound_tx
                .send(WebSocketEnvelope {
                    kind: "ack".to_string(),
                    domain: "agent".to_string(),
                    target_id: envelope.target_id,
                    sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
                    request_id: envelope.request_id,
                    payload: serde_json::json!({"accepted": true}),
                })
                .map_err(|_| anyhow!("websocket outbound channel closed"))?;
            Ok(())
        }
        _ => {
            outbound_tx.send(WebSocketEnvelope {
                kind: "error".to_string(),
                domain: envelope.domain,
                target_id: envelope.target_id,
                sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
                request_id: envelope.request_id,
                payload: serde_json::json!({"message": format!("unsupported websocket message kind={}", envelope.kind)}),
            }).map_err(|_| anyhow!("websocket outbound channel closed"))?;
            Ok(())
        }
    }
}

async fn send_terminal_attach(
    state: &WebAppState,
    outbound_tx: &mpsc::UnboundedSender<WebSocketEnvelope>,
    terminal_id: u64,
    request_id: Option<String>,
) -> anyhow::Result<()> {
    let attach = state.terminal_service.attach_terminal(terminal_id).await?;
    outbound_tx
        .send(WebSocketEnvelope {
            kind: "snapshot".to_string(),
            domain: "terminal".to_string(),
            target_id: attach.descriptor.terminal_id.to_string(),
            sequence_id: Some(state.websocket_sequence.fetch_add(1, Ordering::Relaxed)),
            request_id: request_id.clone(),
            payload: serde_json::to_value(TerminalAttachedPayload {
                terminal_id: attach.descriptor.terminal_id,
                label: attach.descriptor.label.clone(),
                scrollback: attach.scrollback.clone(),
            })?,
        })
        .map_err(|_| anyhow!("websocket outbound channel closed"))?;

    let mut output_rx = attach.output_rx;
    let outbound_tx = outbound_tx.clone();
    let sequence = state.websocket_sequence.clone();
    let target_id = attach.descriptor.terminal_id.to_string();
    let label = attach.descriptor.label;
    tokio::spawn(async move {
        while let Some(data) = output_rx.recv().await {
            let payload = match serde_json::to_value(TerminalStreamEventPayload {
                terminal_id,
                label: label.clone(),
                data,
            }) {
                Ok(payload) => payload,
                Err(_) => break,
            };
            if outbound_tx
                .send(WebSocketEnvelope {
                    kind: "event".to_string(),
                    domain: "terminal".to_string(),
                    target_id: target_id.clone(),
                    sequence_id: Some(sequence.fetch_add(1, Ordering::Relaxed)),
                    request_id: None,
                    payload,
                })
                .is_err()
            {
                break;
            }
        }
    });
    Ok(())
}

fn publish_web_agent_event(
    app: &crate::tui::App,
    event: &crate::app_state::AppRuntimeEvent,
    agent_event_tx: &broadcast::Sender<WebAgentEvent>,
) {
    let crate::app_state::AppRuntimeEvent::Agent(session_id, agent_event) = event else {
        return;
    };
    let agent_id = crate::tui::agent_id_for_session(&app.runtime.agents, *session_id)
        .unwrap_or_else(|| "master".to_string());
    let at_ms = crate::tui::unix_epoch_now_ms();
    match agent_event {
        themion_core::agent::AgentEvent::LlmStart => {
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "status".to_string(),
                text: "model request started".to_string(),
                at_ms,
            });
        }
        themion_core::agent::AgentEvent::AssistantChunk(chunk) => {
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "assistant_chunk".to_string(),
                text: chunk.clone(),
                at_ms,
            });
        }
        themion_core::agent::AgentEvent::AssistantText(text) => {
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "assistant".to_string(),
                text: text.clone(),
                at_ms,
            });
        }
        themion_core::agent::AgentEvent::ToolStart {
            tool_call_id,
            name,
            display_arguments_json,
            arguments_json,
        } => {
            let args = display_arguments_json.as_deref().unwrap_or(arguments_json);
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "tool".to_string(),
                text: format!(
                    "tool start: {} {name} {args}",
                    tool_call_id.as_deref().unwrap_or("?")
                ),
                at_ms,
            });
        }
        themion_core::agent::AgentEvent::ToolEnd { tool_call_id } => {
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "tool".to_string(),
                text: format!("tool finished {}", tool_call_id.as_deref().unwrap_or("?")),
                at_ms,
            });
        }
        themion_core::agent::AgentEvent::ReasoningDelta(text) => {
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "reasoning".to_string(),
                text: text.clone(),
                at_ms,
            });
        }
        themion_core::agent::AgentEvent::Status(text) => {
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "status".to_string(),
                text: text.clone(),
                at_ms,
            });
        }
        themion_core::agent::AgentEvent::Stats(text) => {
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "stats".to_string(),
                text: text.clone(),
                at_ms,
            });
        }
        themion_core::agent::AgentEvent::TurnDone(stats) => {
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "turn".to_string(),
                text: crate::format_stats(stats),
                at_ms,
            });
        }
        themion_core::agent::AgentEvent::WorkflowStateChanged(workflow) => {
            let _ = agent_event_tx.send(WebAgentEvent {
                agent_id,
                event_kind: "workflow".to_string(),
                text: format!(
                    "workflow={}/{} status={} result={}",
                    workflow.workflow_name,
                    workflow.phase_name,
                    workflow.status.as_str(),
                    workflow.phase_result.as_str()
                ),
                at_ms,
            });
        }
    }
}

fn parse_terminal_target_id(value: &str) -> anyhow::Result<u64> {
    value
        .parse::<u64>()
        .with_context(|| format!("invalid terminal target_id '{value}'"))
}

fn extract_prompt_payload(payload: &serde_json::Value) -> anyhow::Result<String> {
    let prompt = payload
        .get("prompt")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing non-empty payload.prompt"))?;
    Ok(prompt.to_string())
}

async fn start_terminal_runtime() -> anyhow::Result<crate::web_terminal::TerminalService> {
    let (background_ready_tx, background_ready_rx) = tokio::sync::oneshot::channel();
    let _background_thread =
        crate::web_terminal::spawn_background_service_runtime(background_ready_tx)?;
    background_ready_rx
        .await
        .context("background terminal service runtime exited before startup completed")?
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn status(State(state): State<WebAppState>) -> Json<WebStatusResponse> {
    Json(build_status_response(
        &state,
        &state.app_state.snapshot_hub.current(),
    ))
}

async fn transcript_api(State(state): State<WebAppState>) -> Json<WebTranscriptResponse> {
    Json(build_transcript_response(
        &state,
        &state.app_state.snapshot_hub.current(),
    ))
}

async fn agents_api(State(state): State<WebAppState>) -> Json<WebAgentsResponse> {
    Json(build_agents_response(
        &state,
        &state.app_state.snapshot_hub.current(),
    ))
}

async fn workspace_tree_api(
    State(state): State<WebAppState>,
    Query(query): Query<WorkspacePathQuery>,
) -> Response {
    match build_workspace_tree_response(&state, &query.path) {
        Ok(response) => Json(response).into_response(),
        Err(response) => response,
    }
}

async fn workspace_file_api(
    State(state): State<WebAppState>,
    Query(query): Query<WorkspacePathQuery>,
) -> Response {
    match build_workspace_file_response(&state, &query.path) {
        Ok(response) => Json(response).into_response(),
        Err(response) => response,
    }
}

async fn workspace_meta_api(
    State(state): State<WebAppState>,
    Query(query): Query<WorkspacePathQuery>,
) -> Response {
    match build_workspace_meta_response(&state, &query.path) {
        Ok(response) => Json(response).into_response(),
        Err(response) => response,
    }
}

async fn workspace_save_api(
    State(state): State<WebAppState>,
    Json(request): Json<WorkspaceSaveRequest>,
) -> Response {
    match save_workspace_file(&state, request) {
        Ok(response) => Json(response).into_response(),
        Err(response) => response,
    }
}

async fn workspace_preview_api(
    State(state): State<WebAppState>,
    Query(query): Query<WorkspacePathQuery>,
) -> Response {
    match build_workspace_preview_response(&state, &query.path) {
        Ok(response) => response,
        Err(response) => response,
    }
}

fn build_workspace_tree_response(
    state: &WebAppState,
    raw_path: &str,
) -> Result<WorkspaceTreeResponse, Response> {
    let root = workspace_root(state)?;
    let path = resolve_workspace_path(&root, raw_path)?;
    let metadata = std::fs::metadata(&path).map_err(|error| {
        workspace_error(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("workspace path not found: {error}"),
        )
    })?;
    if !metadata.is_dir() {
        return Err(workspace_error(
            StatusCode::BAD_REQUEST,
            "not_directory",
            "workspace tree path is not a directory",
        ));
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(|error| {
        workspace_error(
            StatusCode::FORBIDDEN,
            "read_dir_failed",
            format!("failed to read directory: {error}"),
        )
    })? {
        let entry = entry.map_err(|error| {
            workspace_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "read_dir_entry_failed",
                format!("failed to read directory entry: {error}"),
            )
        })?;
        let child_path = entry.path();
        let metadata = match std::fs::metadata(&child_path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !path_within_root(&root, &child_path) {
            continue;
        }
        let relative = project_relative_path(&root, &child_path)?;
        entries.push(WorkspaceEntry {
            path: relative,
            name: entry.file_name().to_string_lossy().to_string(),
            kind: workspace_file_kind(&child_path, &metadata),
            size_bytes: metadata.len(),
            modified_at_ms: metadata_modified_at_ms(&metadata),
        });
    }
    entries.sort_by(|left, right| {
        let left_dir = left.kind == WorkspaceFileKind::Directory;
        let right_dir = right.kind == WorkspaceFileKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(WorkspaceTreeResponse {
        path: project_relative_path(&root, &path)?,
        kind: WorkspaceFileKind::Directory,
        entries,
    })
}

fn build_workspace_file_response(
    state: &WebAppState,
    raw_path: &str,
) -> Result<WorkspaceFileResponse, Response> {
    let root = workspace_root(state)?;
    let path = resolve_workspace_path(&root, raw_path)?;
    let metadata = file_metadata_for_edit(&path)?;
    let kind = workspace_file_kind(&path, &metadata);
    if !workspace_kind_is_editable(kind) {
        return Err(workspace_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not_editable",
            "workspace file is not editable text",
        ));
    }
    if metadata.len() > WORKSPACE_TEXT_FILE_MAX_BYTES {
        return Err(workspace_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file_too_large",
            format!("text files must be <= {WORKSPACE_TEXT_FILE_MAX_BYTES} bytes"),
        ));
    }
    let content = std::fs::read_to_string(&path).map_err(|error| {
        workspace_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not_utf8",
            format!("failed to read UTF-8 text file: {error}"),
        )
    })?;
    Ok(WorkspaceFileResponse {
        path: project_relative_path(&root, &path)?,
        kind,
        size_bytes: metadata.len(),
        modified_at_ms: metadata_modified_at_ms(&metadata),
        conflict_token: conflict_token(&metadata),
        content,
    })
}

fn build_workspace_meta_response(
    state: &WebAppState,
    raw_path: &str,
) -> Result<WorkspaceMetaResponse, Response> {
    let root = workspace_root(state)?;
    let path = resolve_workspace_path(&root, raw_path)?;
    let metadata = std::fs::metadata(&path).map_err(|error| {
        workspace_error(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("workspace path not found: {error}"),
        )
    })?;
    let kind = workspace_file_kind(&path, &metadata);
    Ok(WorkspaceMetaResponse {
        path: project_relative_path(&root, &path)?,
        kind,
        size_bytes: metadata.len(),
        modified_at_ms: metadata_modified_at_ms(&metadata),
        editable: workspace_kind_is_editable(kind)
            && metadata.len() <= WORKSPACE_TEXT_FILE_MAX_BYTES,
        previewable: workspace_kind_is_previewable(kind)
            && metadata.len() <= WORKSPACE_PREVIEW_FILE_MAX_BYTES,
        conflict_token: metadata.is_file().then(|| conflict_token(&metadata)),
    })
}

fn save_workspace_file(
    state: &WebAppState,
    request: WorkspaceSaveRequest,
) -> Result<WorkspaceSaveResponse, Response> {
    let root = workspace_root(state)?;
    let path = resolve_workspace_path(&root, &request.path)?;
    let metadata = file_metadata_for_edit(&path)?;
    let kind = workspace_file_kind(&path, &metadata);
    if !workspace_kind_is_editable(kind) {
        return Err(workspace_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not_editable",
            "workspace file is not editable text",
        ));
    }
    if request.content.len() as u64 > WORKSPACE_TEXT_FILE_MAX_BYTES {
        return Err(workspace_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file_too_large",
            format!("saved text content must be <= {WORKSPACE_TEXT_FILE_MAX_BYTES} bytes"),
        ));
    }
    let current_token = conflict_token(&metadata);
    if request.conflict_token != current_token {
        return Err(workspace_error(
            StatusCode::CONFLICT,
            "stale_conflict_token",
            "file changed on disk after it was opened",
        ));
    }
    std::fs::write(&path, request.content).map_err(|error| {
        workspace_error(
            StatusCode::FORBIDDEN,
            "write_failed",
            format!("failed to save workspace file: {error}"),
        )
    })?;
    let metadata = std::fs::metadata(&path).map_err(|error| {
        workspace_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "metadata_failed",
            format!("failed to read saved file metadata: {error}"),
        )
    })?;
    Ok(WorkspaceSaveResponse {
        path: project_relative_path(&root, &path)?,
        kind: workspace_file_kind(&path, &metadata),
        size_bytes: metadata.len(),
        modified_at_ms: metadata_modified_at_ms(&metadata),
        conflict_token: conflict_token(&metadata),
    })
}

fn build_workspace_preview_response(
    state: &WebAppState,
    raw_path: &str,
) -> Result<Response, Response> {
    let root = workspace_root(state)?;
    let path = resolve_workspace_path(&root, raw_path)?;
    let metadata = file_metadata_for_edit(&path)?;
    let kind = workspace_file_kind(&path, &metadata);
    if !workspace_kind_is_previewable(kind) {
        return Err(workspace_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not_previewable",
            "workspace file type is not previewable",
        ));
    }
    if metadata.len() > WORKSPACE_PREVIEW_FILE_MAX_BYTES {
        return Err(workspace_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file_too_large",
            format!("preview files must be <= {WORKSPACE_PREVIEW_FILE_MAX_BYTES} bytes"),
        ));
    }
    if kind == WorkspaceFileKind::Markdown {
        let markdown = std::fs::read_to_string(&path).map_err(|error| {
            workspace_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "not_utf8",
                format!("failed to read Markdown preview as UTF-8: {error}"),
            )
        })?;
        return Ok(asset_response(
            markdown_preview_html(&markdown).as_bytes(),
            "text/html; charset=utf-8",
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| {
        workspace_error(
            StatusCode::FORBIDDEN,
            "read_failed",
            format!("failed to read preview file: {error}"),
        )
    })?;
    Ok(binary_asset_response(
        &bytes,
        workspace_content_type(&path, kind),
    ))
}

fn html_response(body: String) -> Response {
    let mut response = Html(body).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

fn asset_response(body: &[u8], content_type: &'static str) -> Response {
    let mut response = Response::new(axum::body::Body::from(body.to_vec()));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

fn binary_asset_response(body: &[u8], content_type: &'static str) -> Response {
    asset_response(body, content_type)
}

async fn start_web_surface_loop(
    app_state: &mut AppState,
    agent_event_tx: broadcast::Sender<WebAgentEvent>,
    chat_entries: Arc<std::sync::Mutex<Vec<WebChatEntry>>>,
) -> anyhow::Result<mpsc::UnboundedSender<WebInputEvent>> {
    let runtime_domains = app_state.runtime_domains.clone();
    let mut ctx = SurfaceRunnerContext::build(&runtime_domains);
    start_snapshot_watch_loop(&runtime_domains, &app_state.snapshot_hub, &ctx.app_tx);

    let snapshot_hub = app_state.snapshot_hub.clone();
    let initial_snapshot = snapshot_hub.current();

    crate::app_state::bootstrap_runtime_owner(
        app_state,
        ctx.app_tx.clone(),
        ctx.runtime_tx.clone(),
        ctx.domain.clone(),
    )
    .await?;

    let placeholder_runtime =
        crate::app_state::AppRuntimeState::placeholder_for_surface_transfer_from(
            &app_state.runtime,
        );
    let mut app = crate::tui::App::new(
        std::mem::replace(&mut app_state.runtime, placeholder_runtime),
        initial_snapshot,
    );
    update_web_chat_entries(&chat_entries, &app.entries);
    let web_input_tx = {
        let (tx, mut rx) = mpsc::unbounded_channel::<WebInputEvent>();
        let runtime_tx = ctx.runtime_tx.clone();
        let app_tx = ctx.app_tx.clone();
        ctx.domain.clone().spawn(async move {
            loop {
                tokio::select! {
                    _ = ctx.draw_rx.recv() => {}
                    maybe_app_event = ctx.app_rx.recv() => {
                        let Some(event) = maybe_app_event else { break; };
                        handle_surface_app_event(&mut app, event, &ctx).await;
                        update_web_chat_entries(&chat_entries, &app.entries);
                    }
                    maybe_runtime_event = ctx.runtime_rx.recv() => {
                        let Some(event) = maybe_runtime_event else { break; };
                        publish_web_agent_event(&app, &event, &agent_event_tx);
                        handle_surface_runtime_event(&mut app, event, &ctx).await;
                        update_web_chat_entries(&chat_entries, &app.entries);
                    }
                    maybe_web_input = rx.recv() => {
                        let Some(WebInputEvent::SubmitPrompt { agent_id, prompt }) = maybe_web_input else { break; };
                        submit_web_prompt(&mut app, agent_id, prompt, &app_tx);
                        update_web_chat_entries(&chat_entries, &app.entries);
                        if app.dirty.any() {
                            app.request_draw(&ctx.frame_requester);
                        }
                    }
                }
            }
            drop(runtime_tx);
        });
        tx
    };
    Ok(web_input_tx)
}

fn submit_web_prompt(
    app: &mut crate::tui::App,
    agent_id: String,
    prompt: String,
    app_tx: &mpsc::UnboundedSender<crate::tui::AppEvent>,
) {
    let text = prompt.trim().to_string();
    if text.is_empty() {
        return;
    }

    if text == "/exit" || text == "/quit" || text.starts_with('/') || text.starts_with('!') {
        app.submit_text(text, app_tx);
        return;
    }

    if !crate::app_state::submit_text_to_agent_id(app, &agent_id, text) {
        app.push(crate::tui::Entry::Status {
            agent_id: None,
            source: Some(crate::tui::NonAgentSource::Runtime),
            text: format!("web prompt rejected: target agent {agent_id} not found"),
        });
    }
}

fn build_status_response(state: &WebAppState, snapshot: &AppSnapshot) -> WebStatusResponse {
    WebStatusResponse {
        mode: "web",
        bind_addr: state.bind_addr.to_string(),
        project_dir: state.app_state.runtime.project_dir.display().to_string(),
        session_id: state.app_state.runtime.session_id.to_string(),
        primary_agent_id: snapshot.primary_agent_id.clone(),
        busy: snapshot.busy,
        activity_status: snapshot.activity_status.clone(),
        local_agents: snapshot
            .local_agents
            .iter()
            .map(|agent| WebAgentStatus {
                agent_id: agent.agent_id.clone(),
                label: agent.label.clone(),
                roles: agent.roles.clone(),
                busy: agent.busy,
                incoming: false,
                activity_status: agent.activity_status.clone(),
                activity_label: agent.activity_label.clone(),
                activity_changed_at_ms: agent.activity_changed_at_ms,
                provider_connection: agent.provider_connection.clone(),
            })
            .collect(),
        runtime: build_runtime_summary(&state.app_state.runtime, snapshot),
        recent_events: build_recent_events(&state.app_state.runtime, snapshot, RECENT_EVENTS_LIMIT),
    }
}

fn build_transcript_response(state: &WebAppState, snapshot: &AppSnapshot) -> WebTranscriptResponse {
    WebTranscriptResponse {
        mode: "web",
        bind_addr: state.bind_addr.to_string(),
        session_id: state.app_state.runtime.session_id.to_string(),
        transcript_events: build_recent_events(
            &state.app_state.runtime,
            snapshot,
            TRANSCRIPT_EVENTS_LIMIT,
        ),
        chat_entries: state
            .chat_entries
            .lock()
            .map(|entries| entries.clone())
            .unwrap_or_default(),
    }
}

fn update_web_chat_entries(
    chat_entries: &Arc<std::sync::Mutex<Vec<WebChatEntry>>>,
    entries: &[crate::tui::Entry],
) {
    if let Ok(mut guard) = chat_entries.lock() {
        *guard = build_chat_entries(entries);
    }
}

fn non_agent_source_label(source: crate::tui::NonAgentSource) -> &'static str {
    match source {
        crate::tui::NonAgentSource::Board => "board",
        crate::tui::NonAgentSource::Stylos => "stylos",
        crate::tui::NonAgentSource::Runtime => "runtime",
    }
}

fn build_chat_entries(entries: &[crate::tui::Entry]) -> Vec<WebChatEntry> {
    let mut chat_entries = Vec::new();
    let mut last_tool_entry_index: Option<usize> = None;

    for entry in entries {
        match entry {
            crate::tui::Entry::User { agent_id, text } => {
                last_tool_entry_index = None;
                chat_entries.push(WebChatEntry {
                    kind: "user",
                    agent_id: agent_id.clone(),
                    tool_call_id: None,
                    source: None,
                    text: text.clone(),
                    detail: None,
                    reason: None,
                    stats: None,
                    completed: false,
                });
            }
            crate::tui::Entry::Assistant { agent_id, text } => {
                last_tool_entry_index = None;
                chat_entries.push(WebChatEntry {
                    kind: "assistant",
                    agent_id: agent_id.clone(),
                    tool_call_id: None,
                    source: None,
                    text: text.clone(),
                    detail: None,
                    reason: None,
                    stats: None,
                    completed: false,
                });
            }
            crate::tui::Entry::ToolCall {
                agent_id,
                tool_call_id,
                detail,
                reason,
            } => {
                chat_entries.push(WebChatEntry {
                    kind: "tool_call",
                    agent_id: agent_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    source: None,
                    text: detail.clone(),
                    detail: Some(detail.clone()),
                    reason: reason.clone(),
                    stats: None,
                    completed: false,
                });
                last_tool_entry_index = Some(chat_entries.len() - 1);
            }
            crate::tui::Entry::ToolDone { tool_call_id } => {
                let matching_index = tool_call_id
                    .as_ref()
                    .and_then(|id| {
                        chat_entries.iter().rposition(|entry| {
                            entry.kind == "tool_call"
                                && entry.tool_call_id.as_deref() == Some(id.as_str())
                        })
                    })
                    .or_else(|| last_tool_entry_index.take());
                if let Some(index) = matching_index {
                    if let Some(tool_entry) = chat_entries.get_mut(index) {
                        tool_entry.completed = true;
                    }
                } else {
                    chat_entries.push(WebChatEntry {
                        kind: "tool_done",
                        agent_id: None,
                        tool_call_id: tool_call_id.clone(),
                        source: None,
                        text: "tool finished".to_string(),
                        detail: None,
                        reason: None,
                        stats: None,
                        completed: true,
                    });
                }
            }
            crate::tui::Entry::Status {
                agent_id,
                source,
                text,
            } => {
                last_tool_entry_index = None;
                chat_entries.push(WebChatEntry {
                    kind: "status",
                    agent_id: agent_id.clone(),
                    tool_call_id: None,
                    source: source.map(non_agent_source_label),
                    text: text.clone(),
                    detail: None,
                    reason: None,
                    stats: None,
                    completed: false,
                });
            }
            crate::tui::Entry::Reasoning { agent_id, text } => {
                last_tool_entry_index = None;
                chat_entries.push(WebChatEntry {
                    kind: "reasoning",
                    agent_id: agent_id.clone(),
                    tool_call_id: None,
                    source: None,
                    text: text.clone(),
                    detail: None,
                    reason: None,
                    stats: None,
                    completed: false,
                });
            }
            #[cfg(feature = "stylos")]
            crate::tui::Entry::RemoteEvent {
                agent_id,
                source,
                text,
            } => {
                last_tool_entry_index = None;
                chat_entries.push(WebChatEntry {
                    kind: "remote",
                    agent_id: agent_id.clone(),
                    tool_call_id: None,
                    source: source.map(non_agent_source_label),
                    text: text.clone(),
                    detail: None,
                    reason: None,
                    stats: None,
                    completed: false,
                });
            }
            crate::tui::Entry::TurnDone {
                agent_id,
                summary,
                stats,
            } => {
                last_tool_entry_index = None;
                chat_entries.push(WebChatEntry {
                    kind: "turn_done",
                    agent_id: agent_id.clone(),
                    tool_call_id: None,
                    source: None,
                    text: summary.clone(),
                    detail: None,
                    reason: None,
                    stats: Some(stats.clone()),
                    completed: false,
                });
            }
            crate::tui::Entry::Stats(text) => {
                last_tool_entry_index = None;
                chat_entries.push(WebChatEntry {
                    kind: "stats",
                    agent_id: None,
                    tool_call_id: None,
                    source: None,
                    text: text.clone(),
                    detail: None,
                    reason: None,
                    stats: None,
                    completed: false,
                });
            }
            crate::tui::Entry::TranscriptOmitted { omitted_entries } => {
                last_tool_entry_index = None;
                chat_entries.push(WebChatEntry {
                    kind: "transcript_omitted",
                    agent_id: None,
                    tool_call_id: None,
                    source: None,
                    text: format!("older transcript entries omitted: {}", omitted_entries),
                    detail: None,
                    reason: None,
                    stats: None,
                    completed: false,
                });
            }
            crate::tui::Entry::Banner(_) | crate::tui::Entry::Blank => {
                last_tool_entry_index = None;
            }
        }
    }

    chat_entries
}

fn build_agents_response(state: &WebAppState, snapshot: &AppSnapshot) -> WebAgentsResponse {
    WebAgentsResponse {
        mode: "web",
        bind_addr: state.bind_addr.to_string(),
        session_id: state.app_state.runtime.session_id.to_string(),
        primary_agent_id: snapshot.primary_agent_id.clone(),
        activity_status: snapshot.activity_status.clone(),
        local_agents: snapshot
            .local_agents
            .iter()
            .map(|agent| WebAgentStatus {
                agent_id: agent.agent_id.clone(),
                label: agent.label.clone(),
                roles: agent.roles.clone(),
                busy: agent.busy,
                incoming: false,
                activity_status: agent.activity_status.clone(),
                activity_label: agent.activity_label.clone(),
                activity_changed_at_ms: agent.activity_changed_at_ms,
                provider_connection: agent.provider_connection.clone(),
            })
            .collect(),
    }
}

fn build_runtime_summary(runtime: &AppRuntimeState, snapshot: &AppSnapshot) -> WebRuntimeSummary {
    WebRuntimeSummary {
        configured_profile: runtime.session.configured_profile.clone(),
        active_profile: runtime.session.active_profile.clone(),
        provider: runtime.session.provider.clone(),
        model: runtime.session.model.clone(),
        workflow_name: runtime.workflow_state.workflow_name.clone(),
        workflow_phase: runtime.workflow_state.phase_name.clone(),
        workflow_status: runtime.workflow_state.status.as_str().to_string(),
        workflow_phase_result: runtime.workflow_state.phase_result.as_str().to_string(),
        session_tokens_in: runtime.session_tokens.tokens_in,
        session_tokens_out: runtime.session_tokens.tokens_out,
        session_tokens_cached: runtime.session_tokens.tokens_cached,
        llm_rounds: runtime.session_tokens.llm_rounds,
        tool_calls: runtime.session_tokens.tool_calls,
        elapsed_ms: runtime.session_tokens.elapsed_ms,
        process_started_at_ms: runtime.process_started_at_ms,
        idle_state_changed_at_ms: runtime.idle_status_changed_at,
        activity_changed_at_ms: runtime.agent_activity_changed_at,
        pending_text: runtime.pending.clone(),
        rate_limits: snapshot.rate_limits.clone(),
    }
}

fn build_recent_events(
    runtime: &AppRuntimeState,
    snapshot: &AppSnapshot,
    limit: usize,
) -> Vec<WebRecentEvent> {
    let mut events = runtime
        .recent_events
        .iter()
        .filter(|event| web_recent_event_visible(&event.kind))
        .cloned()
        .map(|event| WebRecentEvent {
            kind: event.kind,
            text: truncate_text(&event.text, RECENT_EVENT_TEXT_MAX_CHARS),
            at_ms: event.at_ms,
        })
        .collect::<Vec<_>>();

    if events.is_empty() {
        events.push(WebRecentEvent {
            kind: "activity".to_string(),
            text: format!(
                "activity={} busy={} local_agents={}",
                snapshot.activity_status.as_deref().unwrap_or("unknown"),
                snapshot.busy,
                snapshot.local_agents.len(),
            ),
            at_ms: runtime.process_started_at_ms,
        });
    }

    events.reverse();
    if events.len() > limit {
        events.truncate(limit);
    }
    events
}

fn web_recent_event_visible(kind: &str) -> bool {
    matches!(
        kind,
        "user" | "assistant" | "tool" | "status" | "remote" | "turn"
    )
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn workspace_root(state: &WebAppState) -> Result<PathBuf, Response> {
    state
        .app_state
        .runtime
        .project_dir
        .canonicalize()
        .map_err(|error| {
            workspace_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "project_root_unavailable",
                format!("failed to resolve project root: {error}"),
            )
        })
}

fn resolve_workspace_path(root: &Path, raw_path: &str) -> Result<PathBuf, Response> {
    let relative = Path::new(raw_path.trim_start_matches('/'));
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        if raw_path.trim().is_empty() {
            return Ok(root.to_path_buf());
        }
        return Err(workspace_error(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "workspace path must be project-relative and cannot contain traversal",
        ));
    }
    let candidate = if raw_path.trim().is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    let canonical = candidate.canonicalize().map_err(|error| {
        workspace_error(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("workspace path not found: {error}"),
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(workspace_error(
            StatusCode::FORBIDDEN,
            "path_escape",
            "workspace path resolves outside the project root",
        ));
    }
    Ok(canonical)
}

fn path_within_root(root: &Path, path: &Path) -> bool {
    path.canonicalize()
        .map(|canonical| canonical.starts_with(root))
        .unwrap_or(false)
}

fn project_relative_path(root: &Path, path: &Path) -> Result<String, Response> {
    let relative = path.strip_prefix(root).map_err(|_| {
        workspace_error(
            StatusCode::FORBIDDEN,
            "path_escape",
            "workspace path resolves outside the project root",
        )
    })?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn file_metadata_for_edit(path: &Path) -> Result<std::fs::Metadata, Response> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        workspace_error(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("workspace file not found: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(workspace_error(
            StatusCode::BAD_REQUEST,
            "not_file",
            "workspace path is not a file",
        ));
    }
    Ok(metadata)
}

fn workspace_file_kind(path: &Path, metadata: &std::fs::Metadata) -> WorkspaceFileKind {
    if metadata.is_dir() {
        return WorkspaceFileKind::Directory;
    }
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "md" | "markdown" => WorkspaceFileKind::Markdown,
        "html" | "htm" => WorkspaceFileKind::Html,
        "pdf" => WorkspaceFileKind::Pdf,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" => WorkspaceFileKind::Image,
        "mp4" | "webm" | "ogg" | "mov" | "m4v" => WorkspaceFileKind::Video,
        "rs" | "toml" | "json" | "yaml" | "yml" | "css" | "js" | "jsx" | "ts" | "tsx" | "sh"
        | "bash" | "zsh" | "fish" | "txt" | "sql" | "py" | "go" | "java" | "c" | "h" | "cpp"
        | "hpp" | "lua" | "rb" => WorkspaceFileKind::Text,
        "lock" => WorkspaceFileKind::Text,
        "bin" | "exe" | "wasm" | "so" | "dylib" | "dll" | "zip" | "gz" | "tar" => {
            WorkspaceFileKind::Binary
        }
        _ => {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "makefile" | "dockerfile" | "license" | "readme"
            ) {
                WorkspaceFileKind::Text
            } else {
                WorkspaceFileKind::Unsupported
            }
        }
    }
}

fn workspace_kind_is_editable(kind: WorkspaceFileKind) -> bool {
    matches!(
        kind,
        WorkspaceFileKind::Text | WorkspaceFileKind::Markdown | WorkspaceFileKind::Html
    )
}

fn workspace_kind_is_previewable(kind: WorkspaceFileKind) -> bool {
    matches!(
        kind,
        WorkspaceFileKind::Image
            | WorkspaceFileKind::Video
            | WorkspaceFileKind::Markdown
            | WorkspaceFileKind::Html
            | WorkspaceFileKind::Pdf
    )
}

fn workspace_content_type(path: &Path, kind: WorkspaceFileKind) -> &'static str {
    match kind {
        WorkspaceFileKind::Image => match path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "bmp" => "image/bmp",
            "ico" => "image/x-icon",
            _ => "image/png",
        },
        WorkspaceFileKind::Video => match path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "webm" => "video/webm",
            "ogg" => "video/ogg",
            "mov" => "video/quicktime",
            _ => "video/mp4",
        },
        WorkspaceFileKind::Markdown => "text/markdown; charset=utf-8",
        WorkspaceFileKind::Html => "text/html; charset=utf-8",
        WorkspaceFileKind::Pdf => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn markdown_preview_html(markdown: &str) -> String {
    let mut body = String::new();
    let mut in_code = false;
    for line in markdown.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("```") {
            if in_code {
                body.push_str("</code></pre>");
            } else {
                body.push_str("<pre><code>");
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            body.push_str(&escape_html(trimmed));
            body.push('\n');
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
        if (1..=6).contains(&hashes) && trimmed.chars().nth(hashes) == Some(' ') {
            let text = escape_html(trimmed[hashes + 1..].trim());
            body.push_str(&format!("<h{hashes}>{text}</h{hashes}>"));
        } else if let Some(item) = trimmed.strip_prefix("- ") {
            body.push_str("<ul><li>");
            body.push_str(&escape_html(item));
            body.push_str("</li></ul>");
        } else {
            body.push_str("<p>");
            body.push_str(&escape_html(trimmed));
            body.push_str("</p>");
        }
    }
    if in_code {
        body.push_str("</code></pre>");
    }
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><style>body{{margin:0;padding:0.75rem;background:#020617;color:#dce3f0;font:14px/1.55 "JetBrains Mono Nerd",ui-monospace,monospace;}}pre,p,ul{{margin:0 0 .75rem;}}pre{{padding:.55rem;background:#0f172a;overflow:auto;}}code{{color:#dbeafe;}}h1,h2,h3,h4,h5,h6{{color:#bae6fd;margin:.2rem 0 .55rem;}}</style></head><body>{body}</body></html>"#
    )
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn metadata_modified_at_ms(metadata: &std::fs::Metadata) -> Option<u64> {
    metadata.modified().ok().and_then(system_time_ms)
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn conflict_token(metadata: &std::fs::Metadata) -> String {
    format!(
        "size:{}:mtime:{}",
        metadata.len(),
        metadata_modified_at_ms(metadata).unwrap_or_default()
    )
}

fn workspace_error(status: StatusCode, code: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(WorkspaceErrorBody {
            error: WorkspaceErrorDetail {
                code,
                message: message.into(),
            },
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bind_addr_uses_default_when_missing() {
        let addr = parse_bind_addr(None).expect("default bind addr");
        assert_eq!(addr, DEFAULT_WEB_BIND_ADDR.parse().unwrap());
    }

    #[test]
    fn spa_index_asset_contains_module_bootstrap() {
        let html = crate::web_assets::spa_html();
        assert!(html.contains("/assets/app.css"));
        assert!(html.contains("/assets/themion_cli_web_ui.js"));
        assert!(html.contains(r#"type="module""#));
    }

    #[test]
    fn transcript_omitted_entry_projects_to_web_marker() {
        let entries = vec![crate::tui::Entry::TranscriptOmitted {
            omitted_entries: 42,
        }];

        let chat_entries = build_chat_entries(&entries);

        assert_eq!(chat_entries.len(), 1);
        assert_eq!(chat_entries[0].kind, "transcript_omitted");
        assert_eq!(chat_entries[0].text, "older transcript entries omitted: 42");
        assert!(!chat_entries[0].completed);
    }

    #[test]
    fn tool_done_merges_into_previous_tool_call() {
        let entries = vec![
            crate::tui::Entry::ToolCall {
                agent_id: Some("master".to_string()),
                tool_call_id: Some("call-1".to_string()),
                detail: "shell: df -h".to_string(),
                reason: None,
            },
            crate::tui::Entry::ToolDone {
                tool_call_id: Some("call-1".to_string()),
            },
        ];

        let chat_entries = build_chat_entries(&entries);
        assert_eq!(chat_entries.len(), 1);
        assert_eq!(chat_entries[0].kind, "tool_call");
        assert_eq!(chat_entries[0].agent_id.as_deref(), Some("master"));
        assert!(chat_entries[0].completed);
    }

    #[test]
    fn workspace_rejects_parent_traversal() {
        let root = std::env::current_dir().unwrap().canonicalize().unwrap();
        let error = resolve_workspace_path(&root, "../Cargo.toml").unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn workspace_classifies_common_preview_and_code_files() {
        let metadata = std::fs::metadata("Cargo.toml").unwrap();
        assert_eq!(
            workspace_file_kind(Path::new("Cargo.toml"), &metadata),
            WorkspaceFileKind::Text
        );
        assert!(workspace_kind_is_editable(WorkspaceFileKind::Markdown));
        assert!(workspace_kind_is_previewable(WorkspaceFileKind::Pdf));
    }

    #[test]
    fn markdown_preview_escapes_script_content() {
        let html = markdown_preview_html("# Hi\n<script>alert(1)</script>");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
    }
}
