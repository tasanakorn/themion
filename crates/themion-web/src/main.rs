pub mod components;

use anyhow::{anyhow, bail, Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use leptos::prelude::*;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Deserialize;
use std::env;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::thread;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{mpsc as tokio_mpsc, oneshot};
use std::sync::{mpsc, Arc};

const APP_CSS: &str = include_str!("../style/app.css");
const APP_JS: &str = include_str!("../style/app.js");
const XTERM_CSS: &str = include_str!("../vendor/xterm/xterm.min.css");
const XTERM_JS: &str = include_str!("../vendor/xterm/xterm.min.js");
const TERMINAL_ROUTE: &str = "/api/terminal/ws";

#[derive(Clone)]
struct AppState {
    terminal_service: TerminalService,
}

#[derive(Clone)]
struct TerminalService {
    request_tx: mpsc::Sender<TerminalRequest>,
}

enum TerminalRequest {
    CreateSession {
        response_tx: oneshot::Sender<Result<TerminalSessionHandle>>,
    },
}

#[derive(Clone)]
struct TerminalSessionHandle {
    input_tx: tokio_mpsc::UnboundedSender<Vec<u8>>,
    output_rx: Arc<tokio::sync::Mutex<tokio_mpsc::UnboundedReceiver<Vec<u8>>>>,
    resize_tx: tokio_mpsc::UnboundedSender<TerminalResize>,
}

#[derive(Clone, Copy)]
struct TerminalResize {
    cols: u16,
    rows: u16,
}

struct TerminalManager {
    shell: String,
    cwd: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientTerminalMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
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
    let manager = Arc::new(TerminalManager::new()?);
    let (request_tx, request_rx) = mpsc::channel::<TerminalRequest>();
    let service = TerminalService { request_tx };
    let _ = ready_tx.send(Ok(service));
    process_terminal_requests(manager, request_rx).await
}

async fn process_terminal_requests(
    manager: Arc<TerminalManager>,
    request_rx: mpsc::Receiver<TerminalRequest>,
) -> Result<()> {
    while let Ok(request) = request_rx.recv() {
        match request {
            TerminalRequest::CreateSession { response_tx } => {
                let result = manager.create_session();
                let _ = response_tx.send(result);
            }
        }
    }

    bail!("terminal service request channel closed")
}

impl TerminalManager {
    fn new() -> Result<Self> {
        Ok(Self {
            shell: resolve_shell(),
            cwd: env::current_dir()
                .ok()
                .and_then(|path| path.to_str().map(|value| value.to_string())),
        })
    }

    fn create_session(&self) -> Result<TerminalSessionHandle> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 30,
                cols: 120,
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
        drop(child);

        let writer = pair.master.take_writer().context("failed to get pty writer")?;
        let reader = pair.master.try_clone_reader().context("failed to clone pty reader")?;
        let resizer = pair.master;

        let (input_tx, input_rx) = tokio_mpsc::unbounded_channel::<Vec<u8>>();
        let (output_tx, output_rx) = tokio_mpsc::unbounded_channel::<Vec<u8>>();
        let (resize_tx, resize_rx) = tokio_mpsc::unbounded_channel::<TerminalResize>();

        spawn_terminal_input_loop(writer, input_rx);
        spawn_terminal_output_loop(reader, output_tx);
        spawn_terminal_resize_loop(resizer, resize_rx);

        Ok(TerminalSessionHandle {
            input_tx,
            output_rx: Arc::new(tokio::sync::Mutex::new(output_rx)),
            resize_tx,
        })
    }
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
    output_tx: tokio_mpsc::UnboundedSender<Vec<u8>>,
) {
    tokio::task::spawn_blocking(move || {
        let mut buf = vec![0_u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(count) => {
                    if output_tx.send(buf[..count].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn spawn_terminal_resize_loop(
    master: Box<dyn portable_pty::MasterPty + Send>,
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
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static(content_type),
    );
    response
}

async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let session = app_state
        .terminal_service
        .create_session()
        .await
        .map_err(internal_error)?;

    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(error) = handle_terminal_socket(socket, session).await {
            eprintln!("terminal websocket ended with error: {error:#}");
        }
    }))
}

impl TerminalService {
    async fn create_session(&self) -> Result<TerminalSessionHandle> {
        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(TerminalRequest::CreateSession { response_tx })
            .map_err(|_| anyhow!("terminal service unavailable"))?;
        response_rx
            .await
            .context("terminal service dropped session response")?
    }
}

async fn handle_terminal_socket(socket: WebSocket, session: TerminalSessionHandle) -> Result<()> {
    let (sender, receiver) = socket.split();
    let output_rx = Arc::clone(&session.output_rx);

    let send_task = tokio::spawn(stream_terminal_output(sender, output_rx));
    let receive_task = tokio::spawn(process_terminal_input(receiver, session.clone()));

    let send_result = send_task.await.context("terminal send task join failed")?;
    let receive_result = receive_task
        .await
        .context("terminal receive task join failed")?;

    send_result?;
    receive_result?;
    Ok(())
}

async fn stream_terminal_output(
    mut sender: SplitSink<WebSocket, Message>,
    output_rx: Arc<tokio::sync::Mutex<tokio_mpsc::UnboundedReceiver<Vec<u8>>>>,
) -> Result<()> {
    let mut output_rx = output_rx.lock().await;
    while let Some(bytes) = output_rx.recv().await {
        let text = String::from_utf8_lossy(&bytes).to_string();
        sender.send(Message::Text(text.into())).await?;
    }
    Ok(())
}

async fn process_terminal_input(
    mut receiver: SplitStream<WebSocket>,
    session: TerminalSessionHandle,
) -> Result<()> {
    while let Some(message) = receiver.next().await {
        match message? {
            Message::Text(text) => match serde_json::from_str::<ClientTerminalMessage>(&text) {
                Ok(ClientTerminalMessage::Input { data }) => {
                    session
                        .input_tx
                        .send(data.into_bytes())
                        .map_err(|_| anyhow!("terminal input channel closed"))?;
                }
                Ok(ClientTerminalMessage::Resize { cols, rows }) => {
                    session
                        .resize_tx
                        .send(TerminalResize { cols, rows })
                        .map_err(|_| anyhow!("terminal resize channel closed"))?;
                }
                Err(error) => {
                    return Err(anyhow!("invalid terminal websocket payload: {error}"));
                }
            },
            Message::Binary(bytes) => {
                session
                    .input_tx
                    .send(bytes.to_vec())
                    .map_err(|_| anyhow!("terminal input channel closed"))?;
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }

    Ok(())
}

fn internal_error(error: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
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
                                    <div class="terminal-subtitle">"PTY runs on isolated background runtime"</div>
                                </div>
                                <button id="terminal-reconnect" class="tab-button terminal-action" type="button">"Reconnect"</button>
                            </div>
                            <div id="terminal-status" class="terminal-status" data-state="idle">"Terminal idle"</div>
                            <div id="terminal-root" class="terminal-root"></div>
                        </Card>
                    </section>
                </section>
            </div>
        </main>
    }
}
