pub mod components;

use anyhow::{anyhow, bail, Context, Result};
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use chrono::{Local, TimeZone};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use leptos::prelude::*;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use rusqlite::{Connection, OpenFlags};
use themion_core::db::DbHandle;
use themion_core::memory::{HashtagMatch, UnifiedSearchMode, UnifiedSearchQuery, UnifiedSearchResponse, UnifiedSearchResult, UnifiedSearchSourceKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

const APP_CSS: &str = include_str!("../style/app.css");
const APP_JS: &str = include_str!("../style/app.js");
const XTERM_CSS: &str = include_str!("../vendor/xterm/xterm.min.css");
const XTERM_JS: &str = include_str!("../vendor/xterm/xterm.min.js");
const JETBRAINS_MONO_NERD_FONT: &[u8] = include_bytes!("../assets/fonts/JetBrainsMonoNerdFont-Regular.ttf");
const TERMINAL_ROUTE: &str = "/api/terminal/ws";
const TERMINAL_SCROLLBACK_LIMIT_BYTES: usize = 262_144;
const DEFAULT_TERMINAL_COLS: u16 = 120;
const DEFAULT_TERMINAL_ROWS: u16 = 40;
const TOP_HASHTAG_LIMIT: usize = 10;
const RECENT_ACTIVITY_LIMIT: usize = 12;

#[derive(Clone)]
struct AppState {
    terminal_service: TerminalService,
}

#[derive(Clone)]
struct TerminalService {
    request_tx: mpsc::Sender<TerminalRequest>,
}

enum TerminalRequest {
    CreateTerminal {
        response_tx: oneshot::Sender<Result<TerminalDescriptor>>,
    },
    ListTerminals {
        response_tx: oneshot::Sender<Result<Vec<TerminalDescriptor>>>,
    },
    AttachTerminal {
        terminal_id: u64,
        response_tx: oneshot::Sender<Result<TerminalAttachHandle>>,
    },
    Input {
        terminal_id: u64,
        data: Vec<u8>,
        response_tx: oneshot::Sender<Result<()>>,
    },
    Resize {
        terminal_id: u64,
        cols: u16,
        rows: u16,
        response_tx: oneshot::Sender<Result<()>>,
    },
    CloseTerminal {
        terminal_id: u64,
        response_tx: oneshot::Sender<Result<()>>,
    },
}

#[derive(Clone, Debug, Serialize)]
struct TerminalDescriptor {
    terminal_id: u64,
    label: String,
}

struct TerminalAttachHandle {
    descriptor: TerminalDescriptor,
    scrollback: String,
    output_rx: tokio_mpsc::UnboundedReceiver<String>,
}

struct TerminalRegistry {
    next_terminal_id: AtomicU64,
    shell: String,
    cwd: Option<String>,
    terminals: Mutex<HashMap<u64, TerminalEntry>>,
}

struct TerminalEntry {
    descriptor: TerminalDescriptor,
    input_tx: tokio_mpsc::UnboundedSender<Vec<u8>>,
    resize_tx: tokio_mpsc::UnboundedSender<TerminalResize>,
    subscribers: Vec<tokio_mpsc::UnboundedSender<String>>,
    scrollback: String,
    _child: Box<dyn Child + Send + Sync>,
}

#[derive(Clone, Copy)]
struct TerminalResize {
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientSocketMessage {
    CreateTerminal,
    ListTerminals,
    AttachTerminal { terminal_id: u64 },
    Input { terminal_id: u64, data: String },
    Resize { terminal_id: u64, cols: u16, rows: u16 },
    CloseTerminal { terminal_id: u64 },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerSocketMessage {
    TerminalList { terminals: Vec<TerminalDescriptor> },
    TerminalCreated { terminal: TerminalDescriptor },
    TerminalAttached { terminal: TerminalDescriptor, scrollback: String },
    TerminalOutput { terminal_id: u64, data: String },
    TerminalClosed { terminal_id: u64 },
    Error { message: String },
}

#[derive(Clone, Debug)]
struct KnowledgeSummaryPageData {
    db_path: String,
    generated_at_label: String,
    query: KnowledgeQueryPageData,
    state: KnowledgeSummaryState,
}

#[derive(Clone, Debug)]
struct KnowledgeQueryPageData {
    form: KnowledgeQueryFormState,
    state: KnowledgeQueryState,
}

#[derive(Clone, Debug)]
struct KnowledgeQueryFormState {
    query: String,
    mode: UnifiedSearchMode,
    limit: u32,
    source_scope: KnowledgeSourceScope,
    hashtags: Vec<String>,
    hashtag_match: HashtagMatch,
    node_type: String,
    relation_type: String,
    linked_node_id: String,
}

#[derive(Clone, Debug)]
enum KnowledgeSourceScope {
    Memory,
    ChatMessage,
    MemoryAndChat,
    OmittedDefault,
}

#[derive(Clone, Debug)]
enum KnowledgeQueryState {
    Idle,
    Ready(UnifiedSearchResponse),
    Error { message: String },
}

#[derive(Clone, Debug)]
enum KnowledgeSummaryState {
    Ready(KnowledgeSummary),
    MissingDb { message: String },
    IncompatibleSchema { message: String },
    QueryError { message: String },
}

#[derive(Clone, Debug)]
struct KnowledgeSummary {
    overview: KnowledgeOverview,
    node_types: Vec<CountRow>,
    hashtags: Vec<CountRow>,
    relations: Vec<CountRow>,
    scopes: Vec<CountRow>,
    graph_shape: GraphShapeSummary,
    recent_activity: Vec<RecentMemoryNode>,
}

#[derive(Clone, Debug)]
struct KnowledgeOverview {
    total_nodes: i64,
    total_edges: i64,
    distinct_hashtags: i64,
    latest_updated_at_label: String,
    is_empty: bool,
}

#[derive(Clone, Debug)]
struct GraphShapeSummary {
    nodes_with_edges: i64,
    nodes_without_edges: i64,
    edge_to_node_ratio_label: String,
}

#[derive(Clone, Debug)]
struct CountRow {
    label: String,
    count: i64,
}

#[derive(Debug, Default, Deserialize)]
struct KnowledgeQueryParams {
    query: Option<String>,
    source_scope: Option<String>,
    mode: Option<String>,
    limit: Option<u32>,
    hashtags: Option<String>,
    hashtag_match: Option<String>,
    node_type: Option<String>,
    relation_type: Option<String>,
    linked_node_id: Option<String>,
}

#[derive(Clone, Debug)]
struct RecentMemoryNode {
    title: String,
    node_type: String,
    project_dir: String,
    updated_at_label: String,
}

fn main() -> Result<()> {
    let bind = env::var("THEMION_WEB_BIND").unwrap_or_else(|_| "0.0.0.0:8787".to_string());
    let addr: SocketAddr = bind
        .parse()
        .with_context(|| format!("invalid THEMION_WEB_BIND '{}'", bind))?;

    let (background_ready_tx, background_ready_rx) = oneshot::channel();
    let background_thread = spawn_background_service_runtime(background_ready_tx)?;
    let terminal_service = background_ready_rx
        .blocking_recv()
        .context("background service runtime exited before startup completed")??;

    let app_state = AppState { terminal_service };
    let web_runtime = build_web_runtime()?;
    let web_result = web_runtime.block_on(run_web_server(addr, app_state));

    drop(web_runtime);
    background_thread
        .join()
        .map_err(|_| anyhow!("background service runtime thread panicked"))??;

    web_result
}

fn spawn_background_service_runtime(
    ready_tx: oneshot::Sender<Result<TerminalService>>,
) -> Result<thread::JoinHandle<Result<()>>> {
    thread::Builder::new()
        .name("themion-web-background".to_string())
        .spawn(move || {
            let runtime = build_background_runtime()?;
            runtime.block_on(run_background_services(ready_tx))
        })
        .context("failed to spawn background service runtime thread")
}

fn build_web_runtime() -> Result<Runtime> {
    Builder::new_multi_thread()
        .enable_all()
        .thread_name("themion-web")
        .build()
        .context("failed to build web runtime")
}

fn build_background_runtime() -> Result<Runtime> {
    Builder::new_multi_thread()
        .enable_all()
        .thread_name("themion-web-background")
        .build()
        .context("failed to build background service runtime")
}

async fn run_web_server(addr: SocketAddr, app_state: AppState) -> Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/assets/xterm.css", get(xterm_css))
        .route("/assets/xterm.js", get(xterm_js))
        .route("/assets/fonts/JetBrainsMonoNerdFont-Regular.ttf", get(jetbrains_mono_nerd_font))
        .route(TERMINAL_ROUTE, get(terminal_ws))
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("themion-web listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_background_services(
    ready_tx: oneshot::Sender<Result<TerminalService>>,
) -> Result<()> {
    let registry = Arc::new(TerminalRegistry::new()?);
    let (request_tx, request_rx) = mpsc::channel::<TerminalRequest>();
    let service = TerminalService { request_tx };
    let _ = ready_tx.send(Ok(service));
    process_terminal_requests(registry, request_rx).await
}

async fn process_terminal_requests(
    registry: Arc<TerminalRegistry>,
    request_rx: mpsc::Receiver<TerminalRequest>,
) -> Result<()> {
    while let Ok(request) = request_rx.recv() {
        match request {
            TerminalRequest::CreateTerminal { response_tx } => {
                let _ = response_tx.send(registry.create_terminal());
            }
            TerminalRequest::ListTerminals { response_tx } => {
                let _ = response_tx.send(registry.list_terminals());
            }
            TerminalRequest::AttachTerminal {
                terminal_id,
                response_tx,
            } => {
                let _ = response_tx.send(registry.attach_terminal(terminal_id));
            }
            TerminalRequest::Input {
                terminal_id,
                data,
                response_tx,
            } => {
                let _ = response_tx.send(registry.send_input(terminal_id, data));
            }
            TerminalRequest::Resize {
                terminal_id,
                cols,
                rows,
                response_tx,
            } => {
                let _ = response_tx.send(registry.resize_terminal(terminal_id, cols, rows));
            }
            TerminalRequest::CloseTerminal {
                terminal_id,
                response_tx,
            } => {
                let _ = response_tx.send(registry.close_terminal(terminal_id));
            }
        }
    }

    bail!("terminal service request channel closed")
}

impl TerminalRegistry {
    fn new() -> Result<Self> {
        Ok(Self {
            next_terminal_id: AtomicU64::new(1),
            shell: resolve_shell(),
            cwd: env::current_dir()
                .ok()
                .and_then(|path| path.to_str().map(|value| value.to_string())),
            terminals: Mutex::new(HashMap::new()),
        })
    }

    fn create_terminal(self: &Arc<Self>) -> Result<TerminalDescriptor> {
        let terminal_id = self.next_terminal_id.fetch_add(1, Ordering::Relaxed);
        let descriptor = TerminalDescriptor {
            terminal_id,
            label: format!("Shell {terminal_id}"),
        };

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_TERMINAL_ROWS,
                cols: DEFAULT_TERMINAL_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to open pty")?;

        let mut cmd = CommandBuilder::new(&self.shell);
        if let Some(cwd) = self.cwd.as_deref() {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("failed to spawn shell '{}'", self.shell))?;

        let writer = pair.master.take_writer().context("failed to get pty writer")?;
        let reader = pair.master.try_clone_reader().context("failed to clone pty reader")?;
        let resizer = pair.master;

        let (input_tx, input_rx) = tokio_mpsc::unbounded_channel::<Vec<u8>>();
        let (output_tx, output_rx) = tokio_mpsc::unbounded_channel::<String>();
        let (resize_tx, resize_rx) = tokio_mpsc::unbounded_channel::<TerminalResize>();

        spawn_terminal_input_loop(writer, input_rx);
        spawn_terminal_output_loop(reader, output_tx);
        spawn_terminal_resize_loop(resizer, resize_rx);
        spawn_terminal_broadcast_loop(terminal_id, Arc::clone(self), output_rx);

        self.terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?
            .insert(
                terminal_id,
                TerminalEntry {
                    descriptor: descriptor.clone(),
                    input_tx,
                    resize_tx,
                    subscribers: Vec::new(),
                    scrollback: String::new(),
                    _child: child,
                },
            );

        Ok(descriptor)
    }

    fn list_terminals(&self) -> Result<Vec<TerminalDescriptor>> {
        let mut terminals: Vec<_> = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?
            .values()
            .map(|entry| entry.descriptor.clone())
            .collect();
        terminals.sort_by_key(|terminal| terminal.terminal_id);
        Ok(terminals)
    }

    fn attach_terminal(&self, terminal_id: u64) -> Result<TerminalAttachHandle> {
        let (subscriber_tx, subscriber_rx) = tokio_mpsc::unbounded_channel::<String>();
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        let entry = terminals
            .get_mut(&terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal_id {}", terminal_id))?;
        entry.subscribers.push(subscriber_tx);
        Ok(TerminalAttachHandle {
            descriptor: entry.descriptor.clone(),
            scrollback: entry.scrollback.clone(),
            output_rx: subscriber_rx,
        })
    }

    fn send_input(&self, terminal_id: u64, data: Vec<u8>) -> Result<()> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        let entry = terminals
            .get(&terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal_id {}", terminal_id))?;
        entry
            .input_tx
            .send(data)
            .map_err(|_| anyhow!("terminal input channel closed"))
    }

    fn resize_terminal(&self, terminal_id: u64, cols: u16, rows: u16) -> Result<()> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        let entry = terminals
            .get(&terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal_id {}", terminal_id))?;
        entry
            .resize_tx
            .send(TerminalResize { cols, rows })
            .map_err(|_| anyhow!("terminal resize channel closed"))
    }

    fn close_terminal(&self, terminal_id: u64) -> Result<()> {
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        terminals
            .remove(&terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal_id {}", terminal_id))?;
        Ok(())
    }

    fn fan_out_output(&self, terminal_id: u64, data: String) -> Result<()> {
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal registry poisoned"))?;
        let Some(entry) = terminals.get_mut(&terminal_id) else {
            return Ok(());
        };

        entry.scrollback.push_str(&data);
        trim_scrollback(&mut entry.scrollback);
        entry
            .subscribers
            .retain(|subscriber| subscriber.send(data.clone()).is_ok());
        Ok(())
    }
}

fn trim_scrollback(scrollback: &mut String) {
    if scrollback.len() <= TERMINAL_SCROLLBACK_LIMIT_BYTES {
        return;
    }
    let drop_bytes = scrollback.len() - TERMINAL_SCROLLBACK_LIMIT_BYTES;
    let drop_at = scrollback
        .char_indices()
        .find_map(|(index, _)| (index >= drop_bytes).then_some(index))
        .unwrap_or(scrollback.len());
    scrollback.drain(..drop_at);
}

fn spawn_terminal_input_loop(
    mut writer: Box<dyn Write + Send>,
    mut input_rx: tokio_mpsc::UnboundedReceiver<Vec<u8>>,
) {
    tokio::task::spawn_blocking(move || {
        while let Some(bytes) = input_rx.blocking_recv() {
            if writer.write_all(&bytes).is_err() {
                break;
            }
            if writer.flush().is_err() {
                break;
            }
        }
    });
}

fn spawn_terminal_output_loop(
    mut reader: Box<dyn Read + Send>,
    output_tx: tokio_mpsc::UnboundedSender<String>,
) {
    tokio::task::spawn_blocking(move || {
        let mut buf = vec![0_u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(count) => {
                    let text = String::from_utf8_lossy(&buf[..count]).to_string();
                    if output_tx.send(text).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn spawn_terminal_resize_loop(
    master: Box<dyn MasterPty + Send>,
    mut resize_rx: tokio_mpsc::UnboundedReceiver<TerminalResize>,
) {
    tokio::spawn(async move {
        while let Some(resize) = resize_rx.recv().await {
            let _ = master.resize(PtySize {
                rows: resize.rows,
                cols: resize.cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    });
}

fn spawn_terminal_broadcast_loop(
    terminal_id: u64,
    registry: Arc<TerminalRegistry>,
    mut output_rx: tokio_mpsc::UnboundedReceiver<String>,
) {
    tokio::spawn(async move {
        while let Some(data) = output_rx.recv().await {
            let _ = registry.fan_out_output(terminal_id, data);
        }
    });
}

fn resolve_shell() -> String {
    env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

async fn index(Query(params): Query<KnowledgeQueryParams>) -> Html<String> {
    let start_on_query_tab = !KnowledgeQueryFormState::from_params(&params).is_effectively_empty();
    let knowledge_summary = load_knowledge_summary_page_data(&params);
    Html(render_app_shell(&knowledge_summary, start_on_query_tab))
}

async fn xterm_css() -> impl IntoResponse {
    asset_response(XTERM_CSS, "text/css; charset=utf-8")
}

async fn xterm_js() -> impl IntoResponse {
    asset_response(XTERM_JS, "application/javascript; charset=utf-8")
}

async fn jetbrains_mono_nerd_font() -> impl IntoResponse {
    binary_asset_response(
        JETBRAINS_MONO_NERD_FONT,
        "font/ttf",
        Some("public, max-age=31536000, immutable"),
    )
}

fn asset_response(body: &'static str, content_type: &'static str) -> Response {
    let mut response = Response::new(body.into());
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn binary_asset_response(
    body: &'static [u8],
    content_type: &'static str,
    cache_control: Option<&'static str>,
) -> Response {
    let mut response = Response::new(Body::from(body.to_vec()));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Some(cache_control) = cache_control {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    }
    response
}

async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(error) = handle_terminal_socket(socket, app_state.terminal_service).await {
            eprintln!("terminal websocket ended with error: {error:#}");
        }
    })
}

impl TerminalService {
    async fn create_terminal(&self) -> Result<TerminalDescriptor> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::CreateTerminal { response_tx })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped create response")?
    }

    async fn list_terminals(&self) -> Result<Vec<TerminalDescriptor>> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::ListTerminals { response_tx })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped list response")?
    }

    async fn attach_terminal(&self, terminal_id: u64) -> Result<TerminalAttachHandle> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::AttachTerminal {
                terminal_id,
                response_tx,
            })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped attach response")?
    }

    async fn send_input(&self, terminal_id: u64, data: Vec<u8>) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::Input {
                terminal_id,
                data,
                response_tx,
            })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped input response")?
    }

    async fn resize_terminal(&self, terminal_id: u64, cols: u16, rows: u16) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::Resize {
                terminal_id,
                cols,
                rows,
                response_tx,
            })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped resize response")?
    }

    async fn close_terminal(&self, terminal_id: u64) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::CloseTerminal {
                terminal_id,
                response_tx,
            })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped close response")?
    }
}

async fn handle_terminal_socket(socket: WebSocket, terminal_service: TerminalService) -> Result<()> {
    let (sender, mut receiver) = socket.split();
    let outbound = Arc::new(tokio::sync::Mutex::new(sender));

    send_socket_message(
        &outbound,
        ServerSocketMessage::TerminalList {
            terminals: terminal_service.list_terminals().await?,
        },
    )
    .await?;

    while let Some(message) = receiver.next().await {
        match message? {
            Message::Text(text) => {
                if let Err(error) =
                    handle_client_message(text.as_str(), &terminal_service, Arc::clone(&outbound)).await
                {
                    send_socket_message(
                        &outbound,
                        ServerSocketMessage::Error {
                            message: error.to_string(),
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
    terminal_service: &TerminalService,
    outbound: Arc<tokio::sync::Mutex<SplitSink<WebSocket, Message>>>,
) -> Result<()> {
    match serde_json::from_str::<ClientSocketMessage>(text)
        .with_context(|| format!("invalid terminal websocket payload: {text}"))?
    {
        ClientSocketMessage::CreateTerminal => {
            let terminal = terminal_service.create_terminal().await?;
            send_socket_message(
                &outbound,
                ServerSocketMessage::TerminalCreated {
                    terminal: terminal.clone(),
                },
            )
            .await?;
            attach_terminal_stream(terminal_service.clone(), Arc::clone(&outbound), terminal.terminal_id)
                .await?;
        }
        ClientSocketMessage::ListTerminals => {
            send_socket_message(
                &outbound,
                ServerSocketMessage::TerminalList {
                    terminals: terminal_service.list_terminals().await?,
                },
            )
            .await?;
        }
        ClientSocketMessage::AttachTerminal { terminal_id } => {
            attach_terminal_stream(terminal_service.clone(), outbound, terminal_id).await?;
        }
        ClientSocketMessage::Input { terminal_id, data } => {
            terminal_service.send_input(terminal_id, data.into_bytes()).await?;
        }
        ClientSocketMessage::Resize {
            terminal_id,
            cols,
            rows,
        } => {
            terminal_service.resize_terminal(terminal_id, cols, rows).await?;
        }
        ClientSocketMessage::CloseTerminal { terminal_id } => {
            terminal_service.close_terminal(terminal_id).await?;
            send_socket_message(&outbound, ServerSocketMessage::TerminalClosed { terminal_id }).await?;
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
        ServerSocketMessage::TerminalAttached {
            terminal: handle.descriptor.clone(),
            scrollback: handle.scrollback,
        },
    )
    .await?;

    tokio::spawn(async move {
        while let Some(data) = handle.output_rx.recv().await {
            if send_socket_message(
                &outbound,
                ServerSocketMessage::TerminalOutput { terminal_id, data },
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
    message: ServerSocketMessage,
) -> Result<()> {
    let payload = serde_json::to_string(&message)?;
    outbound
        .lock()
        .await
        .send(Message::Text(payload.into()))
        .await?;
    Ok(())
}

fn load_knowledge_summary_page_data(params: &KnowledgeQueryParams) -> KnowledgeSummaryPageData {
    let db_path = resolve_system_db_path();
    let generated_at_ms = now_ms();
    let generated_at_label = format_timestamp_ms(generated_at_ms);
    let db_path_label = db_path.display().to_string();

    let query = load_knowledge_query_page_data(&db_path, params);

    let state = match load_knowledge_summary(&db_path) {
        Ok(summary) => KnowledgeSummaryState::Ready(summary),
        Err(error) => classify_summary_error(&db_path, error),
    };

    KnowledgeSummaryPageData {
        db_path: db_path_label,
        generated_at_label,
        query,
        state,
    }
}

fn load_knowledge_query_page_data(
    db_path: &Path,
    params: &KnowledgeQueryParams,
) -> KnowledgeQueryPageData {
    let form = KnowledgeQueryFormState::from_params(params);
    let Some(db) = open_themion_db_handle(db_path) else {
        return KnowledgeQueryPageData {
            form,
            state: KnowledgeQueryState::Idle,
        };
    };

    if form.is_effectively_empty() {
        return KnowledgeQueryPageData {
            form,
            state: KnowledgeQueryState::Idle,
        };
    }

    let query = UnifiedSearchQuery {
        query: form.query.clone(),
        project_dir: Some(resolve_web_query_project_dir()),
        source_kinds: form.source_scope.to_source_kinds(),
        mode: Some(form.mode),
        limit: Some(form.limit),
        hashtags: form.hashtags.clone(),
        hashtag_match: Some(form.hashtag_match),
        node_type: normalize_filter_value(&form.node_type),
        relation_type: normalize_filter_value(&form.relation_type),
        linked_node_id: normalize_filter_value(&form.linked_node_id),
    };

    let state = match db.unified_search(query, None) {
        Ok(response) => KnowledgeQueryState::Ready(response),
        Err(error) => KnowledgeQueryState::Error {
            message: error.to_string(),
        },
    };

    KnowledgeQueryPageData { form, state }
}

fn resolve_web_query_project_dir() -> String {
    env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn open_themion_db_handle(db_path: &Path) -> Option<std::sync::Arc<DbHandle>> {
    if !db_path.exists() {
        return None;
    }
    DbHandle::open(db_path).ok()
}

impl KnowledgeQueryFormState {
    fn from_params(params: &KnowledgeQueryParams) -> Self {
        Self {
            query: params.query.clone().unwrap_or_default(),
            mode: params
                .mode
                .as_deref()
                .and_then(|value| UnifiedSearchMode::from_str(value.trim()))
                .unwrap_or(UnifiedSearchMode::Fts),
            limit: params.limit.map(|value| value.clamp(1, 50)).unwrap_or(10),
            source_scope: params
                .source_scope
                .as_deref()
                .and_then(|value| KnowledgeSourceScope::from_str(value.trim()))
                .unwrap_or(KnowledgeSourceScope::Memory),
            hashtags: parse_filter_list(params.hashtags.as_deref()),
            hashtag_match: params
                .hashtag_match
                .as_deref()
                .and_then(HashtagMatch::from_str)
                .unwrap_or(HashtagMatch::Any),
            node_type: params.node_type.clone().unwrap_or_default(),
            relation_type: params.relation_type.clone().unwrap_or_default(),
            linked_node_id: params.linked_node_id.clone().unwrap_or_default(),
        }
    }

    fn is_effectively_empty(&self) -> bool {
        self.query.trim().is_empty()
            && self.hashtags.is_empty()
            && self.node_type.trim().is_empty()
            && self.relation_type.trim().is_empty()
            && self.linked_node_id.trim().is_empty()
    }
}

fn parse_filter_list(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn normalize_filter_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

impl KnowledgeSourceScope {
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "memory" => Some(Self::Memory),
            "chat_message" => Some(Self::ChatMessage),
            "memory+chat_message" | "memory_chat_message" | "memory,chat_message" => Some(Self::MemoryAndChat),
            "default" | "omitted" => Some(Self::OmittedDefault),
            _ => None,
        }
    }

    fn as_param_value(&self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::ChatMessage => "chat_message",
            Self::MemoryAndChat => "memory+chat_message",
            Self::OmittedDefault => "omitted",
        }
    }

    fn as_label(&self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::ChatMessage => "chat_message",
            Self::MemoryAndChat => "memory + chat_message",
            Self::OmittedDefault => "omitted default",
        }
    }

    fn to_source_kinds(&self) -> Option<Vec<UnifiedSearchSourceKind>> {
        match self {
            Self::Memory => Some(vec![UnifiedSearchSourceKind::Memory]),
            Self::ChatMessage => Some(vec![UnifiedSearchSourceKind::ChatMessage]),
            Self::MemoryAndChat => Some(vec![
                UnifiedSearchSourceKind::Memory,
                UnifiedSearchSourceKind::ChatMessage,
            ]),
            Self::OmittedDefault => None,
        }
    }
}

fn load_knowledge_summary(db_path: &Path) -> Result<KnowledgeSummary> {
    if !db_path.exists() {
        bail!("database file does not exist yet")
    }

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("failed to open sqlite database at {}", db_path.display()))?;

    ensure_required_table(&conn, "memory_nodes")?;
    ensure_required_table(&conn, "memory_node_hashtags")?;
    ensure_required_table(&conn, "memory_edges")?;

    let total_nodes: i64 = conn.query_row("SELECT COUNT(*) FROM memory_nodes", [], |row| row.get(0))?;
    let total_edges: i64 = conn.query_row("SELECT COUNT(*) FROM memory_edges", [], |row| row.get(0))?;
    let distinct_hashtags: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT hashtag) FROM memory_node_hashtags",
        [],
        |row| row.get(0),
    )?;
    let latest_updated_at_ms: Option<i64> = conn.query_row(
        "SELECT MAX(updated_at_ms) FROM memory_nodes",
        [],
        |row| row.get(0),
    )?;

    let node_types = query_count_rows(
        &conn,
        "SELECT node_type, COUNT(*) AS count FROM memory_nodes GROUP BY node_type ORDER BY count DESC, node_type ASC",
    )?;
    let hashtags = query_count_rows_limited(
        &conn,
        "SELECT hashtag, COUNT(*) AS count FROM memory_node_hashtags GROUP BY hashtag ORDER BY count DESC, hashtag ASC",
        TOP_HASHTAG_LIMIT,
    )?;
    let relations = query_count_rows(
        &conn,
        "SELECT relation_type, COUNT(*) AS count FROM memory_edges GROUP BY relation_type ORDER BY count DESC, relation_type ASC",
    )?;
    let scopes = query_count_rows(
        &conn,
        "SELECT project_dir, COUNT(*) AS count FROM memory_nodes GROUP BY project_dir ORDER BY count DESC, project_dir ASC",
    )?;

    let nodes_with_edges: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_nodes n WHERE EXISTS (SELECT 1 FROM memory_edges e WHERE e.from_node_id = n.node_id OR e.to_node_id = n.node_id)",
        [],
        |row| row.get(0),
    )?;
    let nodes_without_edges = total_nodes.saturating_sub(nodes_with_edges);

    let mut recent_stmt = conn.prepare(
        "SELECT title, node_type, project_dir, updated_at_ms
         FROM memory_nodes
         ORDER BY updated_at_ms DESC, title ASC
         LIMIT ?1",
    )?;
    let recent_activity = recent_stmt
        .query_map([RECENT_ACTIVITY_LIMIT as i64], |row| {
            Ok(RecentMemoryNode {
                title: row.get::<_, String>(0)?,
                node_type: row.get::<_, String>(1)?,
                project_dir: row.get::<_, String>(2)?,
                updated_at_label: format_timestamp_ms(row.get::<_, i64>(3)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(KnowledgeSummary {
        overview: KnowledgeOverview {
            total_nodes,
            total_edges,
            distinct_hashtags,
            latest_updated_at_label: latest_updated_at_ms
                .map(format_timestamp_ms)
                .unwrap_or_else(|| "No memory updates yet".to_string()),
            is_empty: total_nodes == 0,
        },
        node_types,
        hashtags,
        relations,
        scopes,
        graph_shape: GraphShapeSummary {
            nodes_with_edges,
            nodes_without_edges,
            edge_to_node_ratio_label: ratio_label(total_edges, total_nodes),
        },
        recent_activity,
    })
}

fn ensure_required_table(conn: &Connection, table_name: &str) -> Result<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table_name],
        |row| row.get(0),
    )?;
    if exists == 0 {
        bail!("missing required table '{}'", table_name);
    }
    Ok(())
}

fn query_count_rows(conn: &Connection, sql: &str) -> Result<Vec<CountRow>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CountRow {
                label: row.get::<_, String>(0)?,
                count: row.get::<_, i64>(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn query_count_rows_limited(conn: &Connection, sql: &str, limit: usize) -> Result<Vec<CountRow>> {
    let limited_sql = format!("{sql} LIMIT ?1");
    let mut stmt = conn.prepare(&limited_sql)?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(CountRow {
                label: row.get::<_, String>(0)?,
                count: row.get::<_, i64>(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn classify_summary_error(db_path: &Path, error: anyhow::Error) -> KnowledgeSummaryState {
    if !db_path.exists() {
        return KnowledgeSummaryState::MissingDb {
            message: error.to_string(),
        };
    }

    let text = error.to_string();
    if text.contains("missing required table") {
        return KnowledgeSummaryState::IncompatibleSchema { message: text };
    }

    if text.contains("no such table") {
        return KnowledgeSummaryState::IncompatibleSchema { message: text };
    }

    KnowledgeSummaryState::QueryError { message: text }
}

fn resolve_system_db_path() -> PathBuf {
    if let Ok(path) = env::var("THEMION_WEB_DB_PATH") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if let Ok(xdg_data_home) = env::var("XDG_DATA_HOME") {
        let trimmed = xdg_data_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("themion").join("system.db");
        }
    }

    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("themion")
        .join("system.db")
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn format_timestamp_ms(timestamp_ms: i64) -> String {
    match Local.timestamp_millis_opt(timestamp_ms).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
        None => format!("{timestamp_ms} ms"),
    }
}

fn ratio_label(numerator: i64, denominator: i64) -> String {
    if denominator <= 0 {
        return "0.00 edges per node".to_string();
    }
    format!("{:.2} edges per node", numerator as f64 / denominator as f64)
}

fn render_app_shell(knowledge_summary: &KnowledgeSummaryPageData, start_on_query_tab: bool) -> String {
    let body = AppShell(AppShellProps { knowledge_summary: knowledge_summary.clone(), start_on_query_tab }).to_html();
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\" /><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" /><title>Themion Web</title><style>{}</style><style>{}</style></head><body>{}<script>{}</script><script>{}</script></body></html>",
        APP_CSS,
        XTERM_CSS,
        body,
        XTERM_JS,
        APP_JS,
    )
}

#[component]
fn AppShell(knowledge_summary: KnowledgeSummaryPageData, start_on_query_tab: bool) -> impl IntoView {
    use crate::components::ui::card::Card;
    let knowledge_stats = knowledge_summary.clone();
    let knowledge_query = knowledge_summary.clone();

    view! {
        <main class="app-shell">
            <input id="nav-dashboard" class="nav-radio" type="radio" name="sidebar-page" checked={!start_on_query_tab}/>
            <input id="nav-knowledge-stats" class="nav-radio" type="radio" name="sidebar-page" checked={!start_on_query_tab}/>
            <input id="nav-knowledge-query" class="nav-radio" type="radio" name="sidebar-page" checked={start_on_query_tab}/>
            <input id="nav-example" class="nav-radio" type="radio" name="sidebar-page"/>
            <input id="nav-agent" class="nav-radio" type="radio" name="sidebar-page"/>
            <input id="nav-shell" class="nav-radio" type="radio" name="sidebar-page"/>

            <div id="workspace" class="workspace">
                <aside id="sidebar" class="sidebar">
                    <Card class="sidebar-card p-0">
                        <div class="sidebar-head">
                            <span class="sidebar-title">"Menu"</span>
                        </div>
                        <nav class="sidebar-body" aria-label="Sidebar menu">
                            <label class="sidebar-item" for="nav-knowledge-stats">"Knowledge · Stats"</label>
                            <label class="sidebar-item" for="nav-knowledge-query">"Knowledge · Query"</label>
                            <label class="sidebar-item" for="nav-example">"Example"</label>
                            <label class="sidebar-item" for="nav-agent">"Agent"</label>
                            <label class="sidebar-item" for="nav-shell">"Shell"</label>
                        </nav>
                    </Card>
                </aside>

                <section class="main-pane">
                    <div class="main-topbar">
                        <button id="sidebar-toggle" class="sidebar-toggle" type="button" aria-label="Toggle sidebar" aria-pressed="false">"☰"</button>
                    </div>

                    <section class="page-panel page-dashboard">
                        <Card class="page-surface knowledge-page">
                            {render_knowledge_stats_page(&knowledge_stats)}
                        </Card>
                    </section>

                    <section class="page-panel page-knowledge-query">
                        <Card class="page-surface knowledge-page">
                            {render_knowledge_query_page(&knowledge_query)}
                        </Card>
                    </section>

                    <section class="page-panel page-example">
                        <div class="tabs-row" role="tablist" aria-label="Documents">
                            <button class="tab-button is-active" type="button" data-tab-target="tab-main" aria-selected="true">"main.rs"</button>
                            <button class="tab-button" type="button" data-tab-target="tab-lib" aria-selected="false">"lib.rs"</button>
                            <button class="tab-button" type="button" data-tab-target="tab-notes" aria-selected="false">"notes.md"</button>
                        </div>

                        <Card class="document-surface p-0">
                            <div class="doc-panel" data-tab-panel="tab-main">
                                <pre class="doc-content">"fn main() {\n    println!(\"hello\");\n}"</pre>
                            </div>
                            <div class="doc-panel" data-tab-panel="tab-lib" hidden>
                                <pre class="doc-content">"pub fn ready() -> bool {\n    true\n}"</pre>
                            </div>
                            <div class="doc-panel" data-tab-panel="tab-notes" hidden>
                                <pre class="doc-content">"simple first\n- sidebar is menu\n- tabs are documents"</pre>
                            </div>
                        </Card>
                    </section>

                    <section class="page-panel page-agent">
                        <Card class="page-surface">
                            <div class="empty-page">"Agent"</div>
                        </Card>
                    </section>

                    <section class="page-panel page-shell">
                        <Card class="page-surface terminal-page">
                            <div class="terminal-toolbar">
                                <div>
                                    <div class="terminal-title">"Remote Terminal"</div>
                                    <div class="terminal-subtitle">"Persistent PTY sessions on isolated background runtime"</div>
                                </div>
                                <div class="terminal-actions">
                                    <button id="terminal-new" class="tab-button terminal-action" type="button">"New terminal"</button>
                                    <button id="terminal-reconnect" class="tab-button terminal-action" type="button">"Reconnect socket"</button>
                                </div>
                            </div>
                            <div id="terminal-status" class="terminal-status" data-state="idle">"Connecting terminal service…"</div>
                            <div id="terminal-tabs" class="terminal-tabs" role="tablist" aria-label="Shell terminals"></div>
                            <div id="terminal-panels" class="terminal-panels"></div>
                        </Card>
                    </section>
                </section>
            </div>
        </main>
    }
}

fn render_knowledge_stats_page(data: &KnowledgeSummaryPageData) -> leptos::prelude::AnyView {
    match &data.state {
        KnowledgeSummaryState::Ready(summary) => render_ready_knowledge_stats_page(data, summary),
        KnowledgeSummaryState::MissingDb { message } => render_knowledge_state(
            "Missing database",
            "Themion Web could not find the active system.db file yet.",
            &data.db_path,
            &data.generated_at_label,
            message,
        ),
        KnowledgeSummaryState::IncompatibleSchema { message } => render_knowledge_state(
            "Incompatible schema",
            "The database is readable, but it does not expose the expected Project Memory tables for this summary page.",
            &data.db_path,
            &data.generated_at_label,
            message,
        ),
        KnowledgeSummaryState::QueryError { message } => render_knowledge_state(
            "Query error",
            "Themion Web could not summarize the Project Memory database cleanly.",
            &data.db_path,
            &data.generated_at_label,
            message,
        ),
    }
}

fn render_ready_knowledge_stats_page(
    data: &KnowledgeSummaryPageData,
    summary: &KnowledgeSummary,
) -> leptos::prelude::AnyView {
    let empty_note = if summary.overview.is_empty {
        Some("This database is readable, but it does not contain any Project Memory nodes yet.")
    } else {
        None
    };

    view! {
        <div class="knowledge-layout">
            <div class="knowledge-header">
                <div>
                    <div class="terminal-title">"Project Memory Summary"</div>
                    <div class="terminal-subtitle">"Read-only overview from the active SQLite database"</div>
                </div>
                <div class="knowledge-meta">
                    <div><strong>"Database:"</strong> " " {data.db_path.clone()}</div>
                    <div><strong>"Generated:"</strong> " " {data.generated_at_label.clone()}</div>
                </div>
            </div>

            {empty_note.map(|text| view! { <div class="knowledge-empty-note">{text}</div> })}

            <div class="knowledge-grid knowledge-overview-grid">
                <div class="knowledge-stat-card">
                    <div class="knowledge-stat-label">"Memory nodes"</div>
                    <div class="knowledge-stat-value">{summary.overview.total_nodes}</div>
                </div>
                <div class="knowledge-stat-card">
                    <div class="knowledge-stat-label">"Edges"</div>
                    <div class="knowledge-stat-value">{summary.overview.total_edges}</div>
                </div>
                <div class="knowledge-stat-card">
                    <div class="knowledge-stat-label">"Distinct hashtags"</div>
                    <div class="knowledge-stat-value">{summary.overview.distinct_hashtags}</div>
                </div>
                <div class="knowledge-stat-card">
                    <div class="knowledge-stat-label">"Latest update"</div>
                    <div class="knowledge-stat-value knowledge-stat-value-wide">{summary.overview.latest_updated_at_label.clone()}</div>
                </div>
            </div>

            <div class="knowledge-grid knowledge-section-grid">
                {render_count_section("Node types", "Distribution by memory node_type.", &summary.node_types, QueryPivotKind::NodeType, &data.query.form)}
                {render_count_section("Top hashtags", "Most-used Project Memory hashtags.", &summary.hashtags, QueryPivotKind::Hashtag, &data.query.form)}
                {render_count_section("Relations", "Counts grouped by relation_type.", &summary.relations, QueryPivotKind::RelationType, &data.query.form)}
                {render_count_section("Scopes", "Counts grouped by project_dir, including [GLOBAL] when present.", &summary.scopes, QueryPivotKind::QueryText, &data.query.form)}
            </div>

            <div class="knowledge-grid knowledge-section-grid">
                <section class="knowledge-section-card">
                    <div class="knowledge-section-head">
                        <div class="knowledge-section-title">"Graph shape"</div>
                        <div class="knowledge-section-subtitle">"How connected the Project Memory graph currently is."</div>
                    </div>
                    <div class="knowledge-graph-grid">
                        <div class="knowledge-stat-card compact">
                            <div class="knowledge-stat-label">"Nodes with edges"</div>
                            <div class="knowledge-stat-value">{summary.graph_shape.nodes_with_edges}</div>
                        </div>
                        <div class="knowledge-stat-card compact">
                            <div class="knowledge-stat-label">"Nodes without edges"</div>
                            <div class="knowledge-stat-value">{summary.graph_shape.nodes_without_edges}</div>
                        </div>
                        <div class="knowledge-stat-card compact">
                            <div class="knowledge-stat-label">"Edge density"</div>
                            <div class="knowledge-stat-value knowledge-stat-value-wide">{summary.graph_shape.edge_to_node_ratio_label.clone()}</div>
                        </div>
                    </div>
                </section>

                <section class="knowledge-section-card knowledge-activity-section">
                    <div class="knowledge-section-head">
                        <div class="knowledge-section-title">"Recent activity"</div>
                        <div class="knowledge-section-subtitle">"Most recently updated Project Memory nodes."</div>
                    </div>
                    <div class="knowledge-activity-list">
                        {if summary.recent_activity.is_empty() {
                            view! { <div class="knowledge-empty-row">"No memory updates recorded yet."</div> }.into_any()
                        } else {
                            view! {
                                <ul class="knowledge-activity-items">
                                    {summary.recent_activity.iter().map(|item| {
                                        view! {
                                            <li class="knowledge-activity-item">
                                                <div class="knowledge-activity-title">{item.title.clone()}</div>
                                                <div class="knowledge-activity-meta">
                                                    <span>{item.node_type.clone()}</span>
                                                    <span>{item.project_dir.clone()}</span>
                                                    <span>{item.updated_at_label.clone()}</span>
                                                </div>
                                            </li>
                                        }
                                    }).collect_view()}
                                </ul>
                            }.into_any()
                        }}
                    </div>
                </section>
            </div>
        </div>
    }
    .into_any()
}

fn build_query_href(form: &KnowledgeQueryFormState) -> String {
    let mut params = vec![
        format!("source_scope={}", encode_query_value(form.source_scope.as_param_value())),
        format!("mode={}", encode_query_value(&format!("{:?}", form.mode).to_lowercase())),
        format!("limit={}", form.limit),
    ];
    if !form.query.trim().is_empty() {
        params.push(format!("query={}", encode_query_value(form.query.trim())));
    }
    if !form.hashtags.is_empty() {
        params.push(format!("hashtags={}", encode_query_value(&form.hashtags.join(", "))));
    }
    if !matches!(form.hashtag_match, HashtagMatch::Any) {
        params.push(format!("hashtag_match={}", encode_query_value(hashtag_match_param(form.hashtag_match))));
    }
    if !form.node_type.trim().is_empty() {
        params.push(format!("node_type={}", encode_query_value(form.node_type.trim())));
    }
    if !form.relation_type.trim().is_empty() {
        params.push(format!("relation_type={}", encode_query_value(form.relation_type.trim())));
    }
    if !form.linked_node_id.trim().is_empty() {
        params.push(format!("linked_node_id={}", encode_query_value(form.linked_node_id.trim())));
    }
    format!("/?{}#knowledge-query", params.join("&"))
}

enum QueryPivotKind {
    QueryText,
    Hashtag,
    NodeType,
    RelationType,
}

fn build_pivot_href(form: &KnowledgeQueryFormState, label: &str, pivot_kind: &QueryPivotKind) -> String {
    let mut pivot_form = form.clone();
    match pivot_kind {
        QueryPivotKind::QueryText => {
            pivot_form.query = label.to_string();
        }
        QueryPivotKind::Hashtag => {
            pivot_form.hashtags = vec![label.to_string()];
            pivot_form.source_scope = KnowledgeSourceScope::Memory;
        }
        QueryPivotKind::NodeType => {
            pivot_form.node_type = label.to_string();
            pivot_form.source_scope = KnowledgeSourceScope::Memory;
        }
        QueryPivotKind::RelationType => {
            pivot_form.relation_type = label.to_string();
            pivot_form.source_scope = KnowledgeSourceScope::Memory;
        }
    }
    build_query_href(&pivot_form)
}

fn hashtag_match_param(value: HashtagMatch) -> &'static str {
    match value {
        HashtagMatch::Any => "any",
        HashtagMatch::All => "all",
    }
}

fn encode_query_value(value: &str) -> String {
    value.replace(' ', "+")
}

fn render_knowledge_query_page(data: &KnowledgeSummaryPageData) -> leptos::prelude::AnyView {
    view! {
        <div class="knowledge-layout">
            <div class="knowledge-header">
                <div>
                    <div class="terminal-title">"Project Memory Query"</div>
                    <div class="terminal-subtitle">"Read-only unified_search workspace from the active SQLite database"</div>
                </div>
                <div class="knowledge-meta">
                    <div><strong>"Database:"</strong> " " {data.db_path.clone()}</div>
                    <div><strong>"Generated:"</strong> " " {data.generated_at_label.clone()}</div>
                </div>
            </div>
            {render_knowledge_query_workspace(&data.query)}
        </div>
    }
    .into_any()
}

fn render_knowledge_query_workspace(query: &KnowledgeQueryPageData) -> leptos::prelude::AnyView {
    let scope_label = query.form.source_scope.as_label();
    let submitted_scope_hint = match query.form.source_scope {
        KnowledgeSourceScope::Memory => "Explicit memory-only default keeps this page focused on Project Memory.",
        KnowledgeSourceScope::ChatMessage => "Explicit chat-message scope narrows the shared core search to transcript results.",
        KnowledgeSourceScope::MemoryAndChat => "Explicit mixed scope searches Project Memory and chat messages together.",
        KnowledgeSourceScope::OmittedDefault => "Omitted source_kinds preserves the canonical core default behavior.",
    };
    let mode_value = format!("{:?}", query.form.mode).to_lowercase();

    view! {
        <section id="knowledge-query" class="knowledge-section-card knowledge-query-card">
            <div class="knowledge-section-head">
                <div class="knowledge-section-title">"Knowledge query workspace"</div>
                <div class="knowledge-section-subtitle">"Shared themion-core unified_search execution with a memory-first web default."</div>
            </div>
            <form class="knowledge-query-form" method="get" action="/">
                <label class="knowledge-query-field knowledge-query-field-wide">
                    <span>"Query"</span>
                    <input type="text" name="query" value={query.form.query.clone()} placeholder="Search Project Memory"/>
                </label>
                <label class="knowledge-query-field">
                    <span>"Source scope"</span>
                    <select name="source_scope">
                        <option value="memory" selected={matches!(query.form.source_scope, KnowledgeSourceScope::Memory)}>"memory"</option>
                        <option value="chat_message" selected={matches!(query.form.source_scope, KnowledgeSourceScope::ChatMessage)}>"chat_message"</option>
                        <option value="memory+chat_message" selected={matches!(query.form.source_scope, KnowledgeSourceScope::MemoryAndChat)}>"memory + chat_message"</option>
                        <option value="omitted" selected={matches!(query.form.source_scope, KnowledgeSourceScope::OmittedDefault)}>"omitted default"</option>
                    </select>
                </label>
                <label class="knowledge-query-field">
                    <span>"Mode"</span>
                    <select name="mode">
                        <option value="fts" selected={mode_value == "fts"}>"fts"</option>
                        <option value="semantic" selected={mode_value == "semantic"}>"semantic"</option>
                        <option value="hybrid" selected={mode_value == "hybrid"}>"hybrid"</option>
                    </select>
                </label>
                <label class="knowledge-query-field">
                    <span>"Limit"</span>
                    <input type="number" min="1" max="50" name="limit" value={query.form.limit.to_string()}/>
                </label>
                <label class="knowledge-query-field knowledge-query-field-wide">
                    <span>"Hashtags"</span>
                    <input type="text" name="hashtags" value={query.form.hashtags.join(", ")} placeholder="#rust, #provider or plain tags"/>
                </label>
                <label class="knowledge-query-field">
                    <span>"Hashtag match"</span>
                    <select name="hashtag_match">
                        <option value="any" selected={matches!(query.form.hashtag_match, HashtagMatch::Any)}>"any"</option>
                        <option value="all" selected={matches!(query.form.hashtag_match, HashtagMatch::All)}>"all"</option>
                    </select>
                </label>
                <label class="knowledge-query-field">
                    <span>"Node type"</span>
                    <input type="text" name="node_type" value={query.form.node_type.clone()} placeholder="observation"/>
                </label>
                <label class="knowledge-query-field">
                    <span>"Relation type"</span>
                    <input type="text" name="relation_type" value={query.form.relation_type.clone()} placeholder="depends_on"/>
                </label>
                <label class="knowledge-query-field">
                    <span>"Linked node id"</span>
                    <input type="text" name="linked_node_id" value={query.form.linked_node_id.clone()} placeholder="UUID"/>
                </label>
                <div class="knowledge-query-actions">
                    <button class="tab-button terminal-action knowledge-query-submit" type="submit">"Run query"</button>
                </div>
            </form>
            <div class="knowledge-query-form-summary">
                <div><strong>"Default scope:"</strong> " explicit memory"</div>
                <div><strong>"Selected scope:"</strong> " " {scope_label}</div>
                <div><strong>"Mode:"</strong> " " {mode_value}</div>
                <div><strong>"Limit:"</strong> " " {query.form.limit}</div>
                <div><strong>"Hashtags:"</strong> " " {if query.form.hashtags.is_empty() { "—".to_string() } else { query.form.hashtags.join(", ") }}</div>
                <div><strong>"Node type:"</strong> " " {if query.form.node_type.trim().is_empty() { "—".to_string() } else { query.form.node_type.clone() }}</div>
                <div><strong>"Relation type:"</strong> " " {if query.form.relation_type.trim().is_empty() { "—".to_string() } else { query.form.relation_type.clone() }}</div>
            </div>
            <div class="knowledge-query-hint">{submitted_scope_hint}</div>
            {match &query.state {
                KnowledgeQueryState::Idle => view! {
                    <div class="knowledge-state-card">
                        <div class="knowledge-state-title">"Summary view"</div>
                        <div class="knowledge-state-body">"No query submitted yet. The page stays on the PRD-102 summary until you run a search. Summary rows now link back into this query workspace as lightweight pivots."</div>
                    </div>
                }.into_any(),
                KnowledgeQueryState::Error { message } => view! {
                    <div class="knowledge-state-card">
                        <div class="knowledge-state-title">"Query error"</div>
                        <div class="knowledge-state-body">{message.clone()}</div>
                    </div>
                }.into_any(),
                KnowledgeQueryState::Ready(response) => render_knowledge_query_results(query, response),
            }}
        </section>
    }.into_any()
}

fn render_knowledge_query_results(
    query: &KnowledgeQueryPageData,
    response: &UnifiedSearchResponse,
) -> leptos::prelude::AnyView {
    view! {
        <div class="knowledge-query-results">
            <div class="knowledge-query-result-meta">
                <div><strong>"Submitted query:"</strong> " " {if query.form.query.trim().is_empty() { "—".to_string() } else { query.form.query.clone() }}</div>
                <div><strong>"Results:"</strong> " " {response.results.len()}</div>
                <div><strong>"Degraded:"</strong> " " {if response.degraded { "yes" } else { "no" }}</div>
                <div><strong>"Hashtag match:"</strong> " " {hashtag_match_param(query.form.hashtag_match)}</div>
            </div>
            {if response.results.is_empty() {
                view! { <div class="knowledge-empty-row">"No matches for the current query."</div> }.into_any()
            } else {
                view! {
                    <ul class="knowledge-query-result-list">
                        {response.results.iter().map(render_knowledge_query_result_row).collect::<Vec<_>>() }
                    </ul>
                }.into_any()
            }}
        </div>
    }.into_any()
}

fn render_knowledge_query_result_row(result: &UnifiedSearchResult) -> leptos::prelude::AnyView {
    view! {
        <li class="knowledge-query-result-item">
            <div class="knowledge-query-result-head">
                <span class="knowledge-query-result-kind">{result.source_kind.clone()}</span>
                <span class="knowledge-query-result-title">{result.title.clone()}</span>
                <span class="knowledge-query-result-score">{format!("{:.2}", result.score)}</span>
            </div>
            <div class="knowledge-query-result-snippet">{result.primary_snippet.clone()}</div>
            <div class="knowledge-query-result-meta">
                <span>{result.project_dir.clone()}</span>
                {result.node_type.clone().map(|node_type| view! { <span>{node_type}</span> })}
                {if result.hashtags.is_empty() { None } else { Some(view! { <span>{result.hashtags.join(", ")}</span> }) }}
            </div>
        </li>
    }
    .into_any()
}

fn render_count_section(
    title: &'static str,
    subtitle: &'static str,
    rows: &[CountRow],
    pivot_kind: QueryPivotKind,
    form: &KnowledgeQueryFormState,
) -> leptos::prelude::AnyView {
    view! {
        <section class="knowledge-section-card">
            <div class="knowledge-section-head">
                <div class="knowledge-section-title">{title}</div>
                <div class="knowledge-section-subtitle">{subtitle}</div>
            </div>
            <div class="knowledge-table-wrap">
                {if rows.is_empty() {
                    view! { <div class="knowledge-empty-row">"No rows available."</div> }.into_any()
                } else {
                    view! {
                        <table class="knowledge-table">
                            <tbody>
                                {rows.iter().map(|row| {
                                    view! {
                                        <tr>
                                            <td class="knowledge-table-label">
                                                <a class="knowledge-pivot-link" href={build_pivot_href(form, &row.label, &pivot_kind)}>{row.label.clone()}</a>
                                            </td>
                                            <td class="knowledge-table-count">{row.count}</td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any()
                }}
            </div>
        </section>
    }
    .into_any()
}

fn render_knowledge_state(
    title: &'static str,
    subtitle: &'static str,
    db_path: &str,
    generated_at_label: &str,
    message: &str,
) -> leptos::prelude::AnyView {
    view! {
        <div class="knowledge-layout">
            <div class="knowledge-header">
                <div>
                    <div class="terminal-title">{title}</div>
                    <div class="terminal-subtitle">{subtitle}</div>
                </div>
                <div class="knowledge-meta">
                    <div><strong>"Database:"</strong> " " {db_path.to_string()}</div>
                    <div><strong>"Generated:"</strong> " " {generated_at_label.to_string()}</div>
                </div>
            </div>
            <div class="knowledge-state-card">
                <div class="knowledge-state-title">{title}</div>
                <div class="knowledge-state-body">{message.to_string()}</div>
            </div>
        </div>
    }
    .into_any()
}
