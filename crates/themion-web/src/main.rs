use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use leptos::prelude::*;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use rusqlite::Connection;
use serde::Serialize;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct AppState {
    paths: RuntimePaths,
}

#[derive(Clone, Serialize)]
struct RuntimePaths {
    data_dir: Option<String>,
    config_dir: Option<String>,
    db_path: Option<String>,
    config_path: Option<String>,
    auth_dir: Option<String>,
    legacy_auth_path: Option<String>,
}

#[derive(Serialize)]
struct MonitoringResponse {
    db_path: Option<String>,
    config_path: Option<String>,
    active_profile: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    session_count: Option<i64>,
    project_memory_nodes: Option<i64>,
    note: &'static str,
}

#[derive(Serialize)]
struct FileInfoResponse {
    label: &'static str,
    path: Option<String>,
    exists: bool,
    readable: bool,
    content_base64: Option<String>,
    error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let bind = env::var("THEMION_WEB_BIND").unwrap_or_else(|_| "0.0.0.0:8787".to_string());
    let addr: SocketAddr = bind
        .parse()
        .with_context(|| format!("invalid THEMION_WEB_BIND '{}'", bind))?;
    let state = AppState {
        paths: discover_runtime_paths(),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/api/monitor", get(api_monitor))
        .route("/api/files/config", get(api_config_file))
        .route("/api/files/auth-legacy", get(api_auth_legacy_file))
        .route("/api/files/auth-dir", get(api_auth_dir_listing))
        .route("/api/files/database", get(api_database_file_info))
        .route("/api/terminal", get(ws_terminal))
        .route("/assets/xterm.min.js", get(xterm_js))
        .route("/assets/xterm.min.css", get(xterm_css))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("themion-web listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<String> {
    Html(render_app_shell())
}

async fn api_monitor(State(state): State<AppState>) -> Json<MonitoringResponse> {
    Json(build_monitoring_response(&state.paths))
}

async fn api_config_file(State(state): State<AppState>) -> Json<FileInfoResponse> {
    Json(read_file_info("config", state.paths.config_path.as_deref().map(Path::new)))
}

async fn api_auth_legacy_file(State(state): State<AppState>) -> Json<FileInfoResponse> {
    Json(read_file_info(
        "legacy-auth",
        state.paths.legacy_auth_path.as_deref().map(Path::new),
    ))
}

async fn api_auth_dir_listing(State(state): State<AppState>) -> Json<Vec<FileInfoResponse>> {
    let Some(path) = state.paths.auth_dir.as_deref().map(Path::new) else {
        return Json(vec![]);
    };
    let mut out = Vec::new();
    match fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    out.push(read_file_info("profile-auth", Some(&p)));
                }
            }
        }
        Err(err) => out.push(FileInfoResponse {
            label: "auth-dir",
            path: Some(path.display().to_string()),
            exists: path.exists(),
            readable: false,
            content_base64: None,
            error: Some(err.to_string()),
        }),
    }
    Json(out)
}

async fn api_database_file_info(State(state): State<AppState>) -> Json<FileInfoResponse> {
    Json(read_file_info("database", state.paths.db_path.as_deref().map(Path::new)))
}

async fn ws_terminal(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_terminal_socket)
}

async fn xterm_js() -> impl IntoResponse {
    (
        [("content-type", "application/javascript; charset=utf-8")],
        include_str!("../vendor/xterm/xterm.min.js"),
    )
}

async fn xterm_css() -> impl IntoResponse {
    (
        [("content-type", "text/css; charset=utf-8")],
        include_str!("../vendor/xterm/xterm.min.css"),
    )
}

async fn handle_terminal_socket(socket: WebSocket) {
    let _ = terminal_session(socket).await;
}

async fn terminal_session(socket: WebSocket) -> Result<()> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let cmd = CommandBuilder::new(shell);
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let writer = Arc::new(Mutex::new(writer));
    let pair_master = Arc::new(Mutex::new(pair.master));

    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let reader_task = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if tx.send(String::from_utf8_lossy(&buf[..n]).to_string()).is_err() {
                break;
            }
        }
        Ok(())
    });

    let send_task = tokio::spawn(async move {
        while let Some(chunk) = rx.recv().await {
            if sender.send(Message::Text(chunk.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                if let Some(rest) = text.strip_prefix("RESIZE ") {
                    let parts: Vec<_> = rest.split_whitespace().collect();
                    if parts.len() == 2 {
                        if let (Ok(cols), Ok(rows)) = (parts[0].parse::<u16>(), parts[1].parse::<u16>()) {
                            let lock = pair_master.lock().expect("pty mutex poisoned");
                            let _ = lock.resize(PtySize {
                                rows,
                                cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            });
                        }
                    }
                } else {
                    let mut lock = writer.lock().expect("writer mutex poisoned");
                    let _ = lock.write_all(text.as_bytes());
                    let _ = lock.flush();
                }
            }
            Message::Binary(bytes) => {
                let mut lock = writer.lock().expect("writer mutex poisoned");
                let _ = lock.write_all(&bytes);
                let _ = lock.flush();
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    send_task.abort();
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader_task.await;
    Ok(())
}

fn discover_runtime_paths() -> RuntimePaths {
    let data_dir = dirs::data_dir().map(|d| d.join("themion"));
    let config_dir = dirs::config_dir().map(|d| d.join("themion"));
    RuntimePaths {
        data_dir: data_dir.as_ref().map(display),
        config_dir: config_dir.as_ref().map(display),
        db_path: data_dir
            .as_ref()
            .map(|d| d.join("system.db"))
            .as_ref()
            .map(display),
        config_path: config_dir
            .as_ref()
            .map(|d| d.join("config.toml"))
            .as_ref()
            .map(display),
        auth_dir: config_dir.as_ref().map(|d| d.join("auth")).as_ref().map(display),
        legacy_auth_path: config_dir
            .as_ref()
            .map(|d| d.join("auth.json"))
            .as_ref()
            .map(display),
    }
}

fn display(path: &PathBuf) -> String {
    path.display().to_string()
}

fn build_monitoring_response(paths: &RuntimePaths) -> MonitoringResponse {
    let mut active_profile = None;
    let mut provider = None;
    let mut model = None;
    if let Some(config_path) = paths.config_path.as_deref() {
        if let Ok(raw) = fs::read_to_string(config_path) {
            if let Ok(value) = raw.parse::<toml::Value>() {
                active_profile = value
                    .get("primary_llm_profile")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                if let Some(profile_name) = active_profile.as_deref() {
                    if let Some(profile) = value.get("profile").and_then(|p| p.get(profile_name)) {
                        provider = profile
                            .get("provider")
                            .and_then(|v| v.as_str())
                            .map(ToOwned::to_owned);
                        model = profile
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(ToOwned::to_owned);
                    }
                }
            }
        }
    }

    let mut session_count = None;
    let mut project_memory_nodes = None;
    if let Some(db_path) = paths.db_path.as_deref() {
        if let Ok(conn) = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            session_count = scalar_query(&conn, "SELECT COUNT(*) FROM agent_sessions");
            project_memory_nodes = scalar_query(&conn, "SELECT COUNT(*) FROM memory_nodes");
        }
    }

    MonitoringResponse {
        db_path: paths.db_path.clone(),
        config_path: paths.config_path.clone(),
        active_profile,
        provider,
        model,
        session_count,
        project_memory_nodes,
        note: "Phase 1 monitoring is read-only and derived only from SQLite plus config/auth files.",
    }
}

fn scalar_query(conn: &Connection, sql: &str) -> Option<i64> {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0)).ok()
}

fn read_file_info(label: &'static str, path: Option<&Path>) -> FileInfoResponse {
    let Some(path) = path else {
        return FileInfoResponse {
            label,
            path: None,
            exists: false,
            readable: false,
            content_base64: None,
            error: None,
        };
    };
    match fs::read(path) {
        Ok(bytes) => FileInfoResponse {
            label,
            path: Some(path.display().to_string()),
            exists: true,
            readable: true,
            content_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            error: None,
        },
        Err(err) => FileInfoResponse {
            label,
            path: Some(path.display().to_string()),
            exists: path.exists(),
            readable: false,
            content_base64: None,
            error: Some(err.to_string()),
        },
    }
}

fn render_app_shell() -> String {
    let body = view! { <AppShell/> }.to_html();
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\" /><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" /><title>Themion Web</title><link rel=\"stylesheet\" href=\"/assets/xterm.min.css\" /><style>{}</style></head><body>{}<script src=\"/assets/xterm.min.js\"></script><script>{}</script></body></html>",
        APP_CSS, body, APP_JS
    )
}

#[component]
fn AppShell() -> impl IntoView {
    view! {
        <main class="page">
            <header class="hero">
                <h1>"Themion Web Phase 1"</h1>
                <p>
                    "Read-only monitoring from SQLite + config/auth files, plus browser PTY access through xterm.js using the default shell."
                </p>
            </header>

            <section class="grid">
                <DataPanel title="Monitoring" element_id="monitor"/>
                <DataPanel title="Config" element_id="config"/>
                <DataPanel title="Legacy Auth" element_id="legacy-auth"/>
                <DataPanel title="Database File" element_id="database"/>
            </section>

            <section class="panel">
                <h2>"Profile Auth Files"</h2>
                <pre id="auth-dir">"loading..."</pre>
            </section>

            <section class="panel">
                <h2>"Terminal"</h2>
                <p>
                    "Uses the user default shell from "
                    <code>"$SHELL"</code>
                    ", falling back to "
                    <code>"/bin/sh"</code>
                    "."
                </p>
                <div class="terminal-toolbar">
                    <button id="term-connect" type="button">"Connect terminal"</button>
                    <span id="term-status" class="terminal-status">"disconnected"</span>
                </div>
                <div id="term" class="term-host"></div>
            </section>
        </main>
    }
}

#[component]
fn DataPanel(title: &'static str, element_id: &'static str) -> impl IntoView {
    view! {
        <section class="panel">
            <h2>{title}</h2>
            <pre id=element_id>"loading..."</pre>
        </section>
    }
}

const APP_CSS: &str = r#"
:root {
  color-scheme: dark;
}
body {
  margin: 0;
  font-family: Inter, ui-sans-serif, system-ui, sans-serif;
  background: #0b1020;
  color: #e5e7eb;
}
.page {
  max-width: 1200px;
  margin: 0 auto;
  padding: 2rem;
}
.hero {
  margin-bottom: 1.5rem;
}
.hero h1 {
  margin: 0 0 0.5rem 0;
}
.hero p {
  margin: 0;
  color: #cbd5e1;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: 1rem;
}
.panel {
  background: #111827;
  border: 1px solid #334155;
  border-radius: 12px;
  padding: 1rem;
  margin-bottom: 1rem;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.18);
}
.panel h2 {
  margin-top: 0;
  font-size: 1rem;
}
pre {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  overflow: auto;
  min-height: 10rem;
  background: #020617;
  border-radius: 8px;
  padding: 0.75rem;
  border: 1px solid #1e293b;
}
code {
  background: #1e293b;
  padding: 0.15rem 0.35rem;
  border-radius: 4px;
}
button {
  background: #2563eb;
  color: white;
  border: 0;
  padding: 0.65rem 0.9rem;
  border-radius: 8px;
  font-weight: 600;
  cursor: pointer;
}
button:hover {
  background: #1d4ed8;
}
.terminal-toolbar {
  margin-bottom: 0.75rem;
  display: flex;
  align-items: center;
  gap: 0.75rem;
}
.terminal-status {
  color: #93c5fd;
  font-size: 0.9rem;
}
.term-host {
  min-height: 24rem;
  background: #000;
  border-radius: 8px;
  border: 1px solid #1e293b;
  padding: 0.35rem;
}
"#;

const APP_JS: &str = r#"
async function loadJson(path, elementId) {
  const el = document.getElementById(elementId);
  try {
    const data = await fetch(path).then((r) => r.json());
    el.textContent = JSON.stringify(data, null, 2);
  } catch (err) {
    el.textContent = `failed to load ${path}: ${err}`;
  }
}

loadJson('/api/monitor', 'monitor');
loadJson('/api/files/config', 'config');
loadJson('/api/files/auth-legacy', 'legacy-auth');
loadJson('/api/files/database', 'database');
loadJson('/api/files/auth-dir', 'auth-dir');

let ws;
let term;
let termHost;
let statusEl;

function updateStatus(text) {
  if (statusEl) statusEl.textContent = text;
}

function fitAndResize() {
  if (!term || !termHost || !ws || ws.readyState !== WebSocket.OPEN) return;
  const cols = Math.max(40, Math.floor(termHost.clientWidth / 9));
  const rows = Math.max(12, Math.floor(termHost.clientHeight / 18));
  try {
    term.resize(cols, rows);
  } catch (_) {}
  ws.send(`RESIZE ${cols} ${rows}`);
}

function ensureTerminal() {
  if (term) return term;
  term = new Terminal({
    cursorBlink: true,
    convertEol: true,
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
    fontSize: 14,
    theme: {
      background: '#000000',
      foreground: '#86efac'
    }
  });
  term.open(termHost);
  term.onData((data) => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(data);
    }
  });
  return term;
}

function connectTerm() {
  ensureTerminal();
  if (ws && ws.readyState === WebSocket.OPEN) {
    term.focus();
    return;
  }
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  ws = new WebSocket(`${proto}://${location.host}/api/terminal`);
  updateStatus('connecting');
  ws.onopen = () => {
    updateStatus('connected');
    term.focus();
    fitAndResize();
  };
  ws.onclose = () => {
    updateStatus('disconnected');
  };
  ws.onerror = () => {
    updateStatus('error');
  };
  ws.onmessage = (ev) => {
    term.write(ev.data);
  };
}

window.addEventListener('load', () => {
  termHost = document.getElementById('term');
  statusEl = document.getElementById('term-status');
  document.getElementById('term-connect').addEventListener('click', connectTerm);
  window.addEventListener('resize', fitAndResize);
});
"#;
