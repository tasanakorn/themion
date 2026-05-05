pub mod components;

use anyhow::{anyhow, bail, Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::HeaderValue;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use leptos::prelude::*;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

const APP_CSS: &str = include_str!("../style/app.css");
const APP_JS: &str = include_str!("../style/app.js");
const XTERM_CSS: &str = include_str!("../vendor/xterm/xterm.min.css");
const XTERM_JS: &str = include_str!("../vendor/xterm/xterm.min.js");
const TERMINAL_ROUTE: &str = "/api/terminal/ws";
const TERMINAL_SCROLLBACK_LIMIT_BYTES: usize = 262_144;
const DEFAULT_TERMINAL_COLS: u16 = 120;
const DEFAULT_TERMINAL_ROWS: u16 = 40;

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

async fn index() -> Html<String> {
    Html(render_app_shell())
}

async fn xterm_css() -> impl IntoResponse {
    asset_response(XTERM_CSS, "text/css; charset=utf-8")
}

async fn xterm_js() -> impl IntoResponse {
    asset_response(XTERM_JS, "application/javascript; charset=utf-8")
}

fn asset_response(body: &'static str, content_type: &'static str) -> Response {
    let mut response = Response::new(body.into());
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
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

fn render_app_shell() -> String {
    let body = view! { <AppShell/> }.to_html();
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
fn AppShell() -> impl IntoView {
    use crate::components::ui::card::Card;

    view! {
        <main class="app-shell">
            <input id="nav-dashboard" class="nav-radio" type="radio" name="sidebar-page" checked/>
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
                            <label class="sidebar-item" for="nav-dashboard">"Dashboard"</label>
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
                        <Card class="page-surface">
                            <div class="empty-page">"Dashboard"</div>
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
