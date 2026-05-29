use leptos::ev::{KeyboardEvent, PointerEvent, SubmitEvent};
use leptos::html::Div;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CloseEvent, Element, ErrorEvent, Event, MessageEvent, WebSocket};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = themionMountCodeEditor)]
    fn themion_mount_code_editor(
        host: &Element,
        content: &str,
        on_change: &JsValue,
        language: &str,
    ) -> bool;
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WebStatusResponse {
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

#[derive(Clone, Debug, Default, Deserialize)]
struct WebTranscriptResponse {
    #[allow(dead_code)]
    transcript_events: Vec<WebRecentEvent>,
    #[serde(default)]
    chat_entries: Vec<WebChatEntry>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WebChatEntry {
    kind: String,
    agent_id: Option<String>,
    tool_call_id: Option<String>,
    source: Option<String>,
    text: String,
    detail: Option<String>,
    reason: Option<String>,
    stats: Option<String>,
    #[serde(default)]
    completed: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WebAgentsResponse {
    bind_addr: String,
    session_id: String,
    primary_agent_id: Option<String>,
    activity_status: Option<String>,
    local_agents: Vec<WebAgentStatus>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WebAgentStatus {
    agent_id: String,
    label: String,
    roles: Vec<String>,
    busy: bool,
    #[serde(default)]
    incoming: bool,
    activity_status: Option<String>,
    activity_label: Option<String>,
    activity_changed_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
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
    #[serde(default)]
    rate_limits: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WebRecentEvent {
    kind: String,
    text: String,
    at_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct WorkspaceTreeResponse {
    path: String,
    entries: Vec<WorkspaceEntry>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct WorkspaceEntry {
    path: String,
    name: String,
    kind: WorkspaceFileKind,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WorkspaceFileResponse {
    path: String,
    kind: WorkspaceFileKind,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
    conflict_token: String,
    content: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct WorkspaceSaveResponse {
    path: String,
    kind: WorkspaceFileKind,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
    conflict_token: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WorkspaceErrorResponse {
    error: WorkspaceErrorDetail,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WorkspaceErrorDetail {
    code: String,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
struct WorkspaceSaveRequest {
    path: String,
    content: String,
    conflict_token: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
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
    #[default]
    Unsupported,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SubscriptionTarget {
    domain: String,
    target_id: String,
}

impl SubscriptionTarget {
    fn new(domain: impl Into<String>, target_id: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            target_id: target_id.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SocketLifecycleState {
    Connecting,
    Open,
    Reconnecting,
    Closed,
}

impl SocketLifecycleState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Open => "open",
            Self::Reconnecting => "reconnecting",
            Self::Closed => "closed",
        }
    }
}

#[derive(Clone)]
struct SharedSocket {
    inner: Rc<RefCell<SocketControllerState>>,
}

struct SocketControllerState {
    ws_url: String,
    socket: Option<Rc<WebSocket>>,
    seq: u64,
    generation: u64,
    retry_attempt: u32,
    reconnect_timer_id: Option<i32>,
    allow_reconnect: bool,
    lifecycle_state: SocketLifecycleState,
    subscriptions: BTreeSet<SubscriptionTarget>,
    socket_state: RwSignal<String>,
    agent_stream: RwSignal<Vec<String>>,
    shell_stream: RwSignal<Vec<String>>,
    status: RwSignal<Option<WebStatusResponse>>,
    transcript: RwSignal<Option<WebTranscriptResponse>>,
    agents: RwSignal<Option<WebAgentsResponse>>,
}

impl SharedSocket {
    fn new(
        socket_state: RwSignal<String>,
        agent_stream: RwSignal<Vec<String>>,
        shell_stream: RwSignal<Vec<String>>,
        status: RwSignal<Option<WebStatusResponse>>,
        transcript: RwSignal<Option<WebTranscriptResponse>>,
        agents: RwSignal<Option<WebAgentsResponse>>,
    ) -> Self {
        let location = web_sys::window().expect("window").location();
        let protocol = match location.protocol().ok().as_deref() {
            Some("https:") => "wss:",
            _ => "ws:",
        };
        let host = location
            .host()
            .unwrap_or_else(|_| "127.0.0.1:8420".to_string());
        let state = SocketControllerState {
            ws_url: format!("{protocol}//{host}/api/ws"),
            socket: None,
            seq: 1,
            generation: 0,
            retry_attempt: 0,
            reconnect_timer_id: None,
            allow_reconnect: true,
            lifecycle_state: SocketLifecycleState::Connecting,
            subscriptions: BTreeSet::new(),
            socket_state,
            agent_stream,
            shell_stream,
            status,
            transcript,
            agents,
        };
        let shared = Self {
            inner: Rc::new(RefCell::new(state)),
        };
        shared.update_socket_state(SocketLifecycleState::Connecting);
        shared.connect();
        shared
    }

    fn send(&self, kind: &str, domain: &str, target_id: &str, payload: serde_json::Value) -> bool {
        let envelope = {
            let mut state = self.inner.borrow_mut();
            if kind == "subscribe" {
                state
                    .subscriptions
                    .insert(SubscriptionTarget::new(domain, target_id));
            } else if kind == "unsubscribe" {
                state
                    .subscriptions
                    .remove(&SubscriptionTarget::new(domain, target_id));
            }
            if !matches!(state.lifecycle_state, SocketLifecycleState::Open) {
                return false;
            }
            let sequence_id = state.seq;
            state.seq += 1;
            WebSocketEnvelope {
                kind: kind.to_string(),
                domain: domain.to_string(),
                target_id: target_id.to_string(),
                sequence_id: Some(sequence_id),
                request_id: Some(format!("req-{sequence_id}")),
                payload,
            }
        };

        self.send_envelope(envelope)
    }

    fn connect(&self) {
        let (url, generation) = {
            let mut state = self.inner.borrow_mut();
            state.generation += 1;
            let generation = state.generation;
            state.socket = None;
            (state.ws_url.clone(), generation)
        };

        let ws = match WebSocket::new(&url) {
            Ok(ws) => Rc::new(ws),
            Err(_) => {
                self.handle_connect_failure(generation);
                return;
            }
        };
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        self.install_handlers(ws.clone(), generation);

        let mut state = self.inner.borrow_mut();
        state.socket = Some(ws);
    }

    fn install_handlers(&self, ws: Rc<WebSocket>, generation: u64) {
        let onopen = Closure::<dyn FnMut(Event)>::new({
            let shared = self.clone();
            move |_| shared.handle_open(generation)
        });
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget();

        let onclose = Closure::<dyn FnMut(CloseEvent)>::new({
            let shared = self.clone();
            move |_| shared.handle_close(generation)
        });
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        onclose.forget();

        let onerror = Closure::<dyn FnMut(ErrorEvent)>::new({
            let shared = self.clone();
            move |_| shared.handle_error(generation)
        });
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();

        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new({
            let shared = self.clone();
            move |event: MessageEvent| shared.handle_message(generation, event)
        });
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();
    }

    fn handle_open(&self, generation: u64) {
        if !self.is_generation_current(generation) {
            return;
        }
        {
            let mut state = self.inner.borrow_mut();
            state.retry_attempt = 0;
        }
        self.clear_reconnect_timer();
        self.update_socket_state(SocketLifecycleState::Open);
        self.resubscribe_all();
        self.refresh_snapshots();
    }

    fn handle_close(&self, generation: u64) {
        if !self.is_generation_current(generation) {
            return;
        }
        self.schedule_reconnect();
    }

    fn handle_error(&self, generation: u64) {
        if !self.is_generation_current(generation) {
            return;
        }
        self.schedule_reconnect();
    }

    fn handle_connect_failure(&self, generation: u64) {
        if !self.is_generation_current(generation) {
            return;
        }
        self.schedule_reconnect();
    }

    fn handle_message(&self, generation: u64, event: MessageEvent) {
        if !self.is_generation_current(generation) {
            return;
        }
        let Some(text) = event.data().as_string() else {
            return;
        };
        let Ok(envelope) = serde_json::from_str::<WebSocketEnvelope>(&text) else {
            return;
        };
        let line = format!(
            "seq={:?} target={} payload={}",
            envelope.sequence_id, envelope.target_id, envelope.payload
        );
        let refresh_transcript = matches!(envelope.domain.as_str(), "agent" | "runtime");
        let refresh_agents = envelope.domain == "runtime";
        let (agent_stream, shell_stream, status_signal, transcript_signal, agents_signal) = {
            let state = self.inner.borrow();
            (
                state.agent_stream,
                state.shell_stream,
                state.status,
                state.transcript,
                state.agents,
            )
        };

        match envelope.domain.as_str() {
            "agent" => agent_stream.update(|lines| lines.push(line)),
            "terminal" => shell_stream.update(|lines| lines.push(line)),
            "runtime" if envelope.target_id == "status" => {
                if let Ok(payload) =
                    serde_json::from_value::<WebStatusResponse>(envelope.payload.clone())
                {
                    status_signal.set(Some(payload));
                }
            }
            _ => {}
        }
        if refresh_transcript {
            leptos::task::spawn_local(async move {
                if let Ok(payload) = fetch_transcript().await {
                    transcript_signal.set(Some(payload));
                }
            });
        }
        if refresh_agents {
            leptos::task::spawn_local(async move {
                if let Ok(payload) = fetch_agents().await {
                    agents_signal.set(Some(payload));
                }
            });
        }
    }

    fn resubscribe_all(&self) {
        let subscriptions = {
            let state = self.inner.borrow();
            state.subscriptions.iter().cloned().collect::<Vec<_>>()
        };
        for subscription in subscriptions {
            let _ = self.send(
                "subscribe",
                &subscription.domain,
                &subscription.target_id,
                serde_json::json!({}),
            );
        }
    }

    fn refresh_snapshots(&self) {
        let (status_signal, transcript_signal, agents_signal) = {
            let state = self.inner.borrow();
            (state.status, state.transcript, state.agents)
        };
        leptos::task::spawn_local(async move {
            if let Ok(payload) = fetch_status().await {
                status_signal.set(Some(payload));
            }
        });
        leptos::task::spawn_local(async move {
            if let Ok(payload) = fetch_transcript().await {
                transcript_signal.set(Some(payload));
            }
        });
        leptos::task::spawn_local(async move {
            if let Ok(payload) = fetch_agents().await {
                agents_signal.set(Some(payload));
            }
        });
    }

    fn send_envelope(&self, envelope: WebSocketEnvelope) -> bool {
        let socket = {
            let state = self.inner.borrow();
            if !matches!(state.lifecycle_state, SocketLifecycleState::Open) {
                return false;
            }
            state.socket.clone()
        };
        let Some(socket) = socket else {
            return false;
        };
        socket
            .send_with_str(&serde_json::to_string(&envelope).unwrap_or_default())
            .is_ok()
    }

    fn schedule_reconnect(&self) {
        let delay_ms = {
            let mut state = self.inner.borrow_mut();
            if !state.allow_reconnect {
                state.lifecycle_state = SocketLifecycleState::Closed;
                let socket_state = state.socket_state;
                drop(state);
                socket_state.set(SocketLifecycleState::Closed.as_str().to_string());
                return;
            }
            if state.reconnect_timer_id.is_some() {
                if !matches!(state.lifecycle_state, SocketLifecycleState::Reconnecting) {
                    let socket_state = state.socket_state;
                    state.lifecycle_state = SocketLifecycleState::Reconnecting;
                    drop(state);
                    socket_state.set(SocketLifecycleState::Reconnecting.as_str().to_string());
                }
                return;
            }
            let delay_ms = retry_delay_ms(state.retry_attempt);
            state.retry_attempt = state.retry_attempt.saturating_add(1);
            state.lifecycle_state = SocketLifecycleState::Reconnecting;
            delay_ms
        };
        self.update_socket_state(SocketLifecycleState::Reconnecting);

        let shared = self.clone();
        let callback = Closure::<dyn FnMut()>::once(move || {
            {
                let mut state = shared.inner.borrow_mut();
                state.reconnect_timer_id = None;
            }
            shared.update_socket_state(SocketLifecycleState::Connecting);
            shared.connect();
        });
        if let Some(window) = web_sys::window() {
            match window.set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                delay_ms,
            ) {
                Ok(timer_id) => {
                    self.inner.borrow_mut().reconnect_timer_id = Some(timer_id);
                    callback.forget();
                }
                Err(_) => {
                    self.inner.borrow_mut().reconnect_timer_id = None;
                    self.update_socket_state(SocketLifecycleState::Connecting);
                    self.connect();
                }
            }
        }
    }

    fn clear_reconnect_timer(&self) {
        let timer_id = self.inner.borrow_mut().reconnect_timer_id.take();
        if let (Some(window), Some(timer_id)) = (web_sys::window(), timer_id) {
            window.clear_timeout_with_handle(timer_id);
        }
    }

    fn update_socket_state(&self, lifecycle_state: SocketLifecycleState) {
        let socket_state = {
            let mut state = self.inner.borrow_mut();
            state.lifecycle_state = lifecycle_state;
            state.socket_state
        };
        socket_state.set(lifecycle_state.as_str().to_string());
    }

    fn is_generation_current(&self, generation: u64) -> bool {
        self.inner.borrow().generation == generation
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewTab {
    Status,
    Transcript,
    Agents,
    Terminal,
    Workspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceViewMode {
    CodeOnly,
    CodePreview,
    PreviewOnly,
}

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let status = RwSignal::new(None::<WebStatusResponse>);
    let transcript = RwSignal::new(None::<WebTranscriptResponse>);
    let agents = RwSignal::new(None::<WebAgentsResponse>);
    let active_tab = RwSignal::new(ViewTab::Transcript);
    let socket_state = RwSignal::new(String::from("connecting"));
    let agent_stream = RwSignal::new(Vec::<String>::new());
    let shell_stream = RwSignal::new(Vec::<String>::new());
    let prompt = RwSignal::new(String::new());
    let active_agent = RwSignal::new(String::from("master"));
    let prompt_error = RwSignal::new(None::<String>);
    let transcript_history_ref = NodeRef::<Div>::new();
    let transcript_trailing = RwSignal::new(true);
    let workspace_tree = RwSignal::new(None::<WorkspaceTreeResponse>);
    let workspace_path = RwSignal::new(String::new());
    let workspace_file = RwSignal::new(None::<WorkspaceFileResponse>);
    let workspace_content = RwSignal::new(String::new());
    let workspace_dirty = RwSignal::new(false);
    let workspace_status = RwSignal::new(String::from("Loading workspace…"));
    let workspace_preview_path = RwSignal::new(None::<String>);
    let workspace_editor_ref = NodeRef::<Div>::new();
    let workspace_editor_mounted = RwSignal::new(false);
    let workspace_view_mode = RwSignal::new(WorkspaceViewMode::CodeOnly);
    let workspace_split_percent = RwSignal::new(50_u8);
    let workspace_grid_ref = NodeRef::<Div>::new();
    let workspace_split_dragging = RwSignal::new(false);

    let shared_socket = create_shared_socket(
        socket_state,
        agent_stream,
        shell_stream,
        status,
        transcript,
        agents,
    );
    let shared_socket_for_effect = shared_socket.clone();
    let shared_socket_for_submit = shared_socket.clone();
    let shared_socket_for_keydown = shared_socket.clone();

    Effect::new(move |_| {
        let shared = shared_socket_for_effect.clone();
        leptos::task::spawn_local(async move {
            let status_payload = fetch_status().await.ok();
            let transcript_payload = fetch_transcript().await.ok();
            let agents_payload = fetch_agents().await.ok();
            if let Some(payload) = status_payload {
                let agent_id = payload
                    .primary_agent_id
                    .clone()
                    .unwrap_or_else(|| "master".to_string());
                active_agent.set(agent_id.clone());
                status.set(Some(payload));
                let _ = shared.send("subscribe", "runtime", "status", serde_json::json!({}));
                let _ = shared.send("subscribe", "agent", &agent_id, serde_json::json!({}));
                let _ = shared.send("subscribe", "terminal", "list", serde_json::json!({}));
            }
            transcript.set(transcript_payload);
            agents.set(agents_payload);
        });
    });

    Effect::new(move |_| {
        let status_payload = status.get();
        let current_agent = active_agent.get();
        if let Some(payload) = status_payload {
            let available = payload
                .local_agents
                .iter()
                .any(|agent| agent.agent_id == current_agent);
            if !available {
                let next_agent = payload
                    .primary_agent_id
                    .clone()
                    .or_else(|| {
                        payload
                            .local_agents
                            .first()
                            .map(|agent| agent.agent_id.clone())
                    })
                    .unwrap_or_else(|| "master".to_string());
                active_agent.set(next_agent);
            }
        }
    });

    Effect::new(move |_| {
        leptos::task::spawn_local(async move {
            match fetch_workspace_tree("").await {
                Ok(payload) => {
                    workspace_status.set(format!(
                        "Workspace root loaded: {} entries",
                        payload.entries.len()
                    ));
                    workspace_tree.set(Some(payload));
                }
                Err(error) => workspace_status.set(error),
            }
        });
    });

    let editor_change = Closure::<dyn FnMut(String)>::new(move |value: String| {
        workspace_content.set(value);
        workspace_dirty.set(true);
    });
    let editor_change_fn = editor_change.as_ref().clone();
    editor_change.forget();

    Effect::new(move |_| {
        let Some(file) = workspace_file.get() else {
            return;
        };
        let Some(host) = workspace_editor_ref.get() else {
            return;
        };
        let language = workspace_language(&file.path, file.kind);
        let content = workspace_content.get_untracked();
        let _ = themion_mount_code_editor(&host, &content, &editor_change_fn, language);
        workspace_editor_mounted.set(true);
    });

    Effect::new(move |_| {
        let selected = active_agent.get();
        let _ = shared_socket.send("subscribe", "agent", &selected, serde_json::json!({}));
    });

    Effect::new(move |_| {
        let entry_count = transcript
            .get()
            .map(|payload| payload.chat_entries.len())
            .unwrap_or_default();
        if entry_count == 0 || !transcript_trailing.get() {
            return;
        }
        if let Some(history) = transcript_history_ref.get() {
            schedule_scroll_transcript_to_recent(history.into());
        }
    });

    let on_transcript_scroll = move |_| {
        if let Some(history) = transcript_history_ref.get_untracked() {
            transcript_trailing.set(transcript_history_is_trailing(&history));
        }
    };

    let on_submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let text = prompt.get_untracked().trim().to_string();
        if text.is_empty() {
            return;
        }
        let agent_id = active_agent.get_untracked();
        let sent = shared_socket_for_submit.send(
            "input",
            "agent",
            &agent_id,
            serde_json::json!({"prompt": text}),
        );
        if sent {
            prompt.set(String::new());
            prompt_error.set(None);
        } else {
            prompt_error.set(Some("WebSocket reconnecting; prompt not sent.".to_string()));
        }
    };

    let on_prompt_keydown = move |ev: KeyboardEvent| {
        if prompt_keydown_should_submit(&ev) {
            ev.prevent_default();
            let text = prompt.get_untracked().trim().to_string();
            if text.is_empty() {
                return;
            }
            let agent_id = active_agent.get_untracked();
            let sent = shared_socket_for_keydown.send(
                "input",
                "agent",
                &agent_id,
                serde_json::json!({"prompt": text}),
            );
            if sent {
                prompt.set(String::new());
                prompt_error.set(None);
            } else {
                prompt_error.set(Some("WebSocket reconnecting; prompt not sent.".to_string()));
            }
        }
    };

    let open_workspace_entry = move |entry: WorkspaceEntry| {
        let path = entry.path.clone();
        if entry.kind == WorkspaceFileKind::Directory {
            workspace_path.set(path.clone());
            workspace_status.set(format!("Opening directory {path}"));
            leptos::task::spawn_local(async move {
                match fetch_workspace_tree(&path).await {
                    Ok(payload) => {
                        workspace_status.set(format!(
                            "Directory loaded: {} entries",
                            payload.entries.len()
                        ));
                        workspace_tree.set(Some(payload));
                    }
                    Err(error) => workspace_status.set(error),
                }
            });
            return;
        }

        if workspace_kind_is_editable(entry.kind) {
            workspace_status.set(format!("Opening file {path}"));
            leptos::task::spawn_local(async move {
                match fetch_workspace_file(&path).await {
                    Ok(payload) => {
                        workspace_content.set(payload.content.clone());
                        workspace_preview_path
                            .set(workspace_kind_preview_url(payload.kind, &payload.path));
                        workspace_file.set(Some(payload));
                        workspace_dirty.set(false);
                        workspace_status.set("File loaded".to_string());
                    }
                    Err(error) => workspace_status.set(error),
                }
            });
        } else if workspace_kind_is_previewable(entry.kind) {
            workspace_file.set(None);
            workspace_content.set(String::new());
            workspace_preview_path.set(Some(workspace_preview_url(&path)));
            workspace_status.set(format!("Previewing {path}"));
        } else {
            workspace_status.set(format!("Unsupported file type: {path}"));
        }
    };

    let start_workspace_split_drag = move |ev: PointerEvent| {
        ev.prevent_default();
        workspace_split_dragging.set(true);
        if let Some(target) = ev.current_target() {
            if let Ok(element) = target.dyn_into::<Element>() {
                let _ = element.set_pointer_capture(ev.pointer_id());
            }
        }
        update_workspace_split_from_client_x(
            workspace_grid_ref,
            workspace_split_percent,
            ev.client_x(),
        );
    };

    let drag_workspace_split = move |ev: PointerEvent| {
        if !workspace_split_dragging.get_untracked() {
            return;
        }
        ev.prevent_default();
        update_workspace_split_from_client_x(
            workspace_grid_ref,
            workspace_split_percent,
            ev.client_x(),
        );
    };

    let stop_workspace_split_drag = move |ev: PointerEvent| {
        workspace_split_dragging.set(false);
        if let Some(target) = ev.current_target() {
            if let Ok(element) = target.dyn_into::<Element>() {
                let _ = element.release_pointer_capture(ev.pointer_id());
            }
        }
    };

    let sidebar_button = move |tab: ViewTab,
                               icon: &'static str,
                               label: &'static str,
                               hint: &'static str| {
        view! {
            <button
                type="button"
                class=move || if sidebar_tab_is_active(active_tab.get(), tab) { "nav-item active" } else { "nav-item" }
                on:click=move |_| active_tab.set(tab)
            >
                <span class="nav-icon">{icon}</span>
                <span class="nav-copy">
                    <strong>{label}</strong>
                    <small>{hint}</small>
                </span>
            </button>
        }
    };

    view! {
        <div class="app-frame">
            <aside class="sidebar">
                <div class="brand-block">
                    <div class="brand-mark">"Θ"</div>
                    <div>
                        <p class="eyebrow">"themion"</p>
                        <h1>"Agent Console"</h1>
                    </div>
                </div>

                <div class="status-card compact">
                    <span class=move || format!("status-dot {}", socket_state.get())></span>
                    <div>
                        <small>"shared websocket"</small>
                        <strong>{move || websocket_status_label(&socket_state.get())}</strong>
                    </div>
                </div>

                <nav class="sidebar-nav" aria-label="main menu">
                    {sidebar_button(ViewTab::Transcript, "󰈙", "Agent", "chat transcript")}
                    {sidebar_button(ViewTab::Workspace, "󰉋", "Workspace", "code editor")}
                    {sidebar_button(ViewTab::Terminal, "", "Terminal", "placeholder page")}
                </nav>

                {move || if active_tab.get() == ViewTab::Workspace {
                    view! {
                        <div class="sidebar-file-explorer">
                            <div class="sidebar-section-title">
                                <strong>"Files"</strong>
                                <small>{move || workspace_path.get().if_empty_label("root")}</small>
                            </div>
                            <div class="file-tree-actions compact">
                                <button
                                    type="button"
                                    class="secondary-action"
                                    on:click=move |_| {
                                        let current = workspace_path.get_untracked();
                                        let parent = workspace_parent_path(&current);
                                        workspace_path.set(parent.clone());
                                        leptos::task::spawn_local(async move {
                                            match fetch_workspace_tree(&parent).await {
                                                Ok(payload) => {
                                                    workspace_status.set(format!("Directory loaded: {} entries", payload.entries.len()));
                                                    workspace_tree.set(Some(payload));
                                                }
                                                Err(error) => workspace_status.set(error),
                                            }
                                        });
                                    }
                                >"Up"</button>
                                <button
                                    type="button"
                                    class="secondary-action"
                                    on:click=move |_| {
                                        let current = workspace_path.get_untracked();
                                        leptos::task::spawn_local(async move {
                                            match fetch_workspace_tree(&current).await {
                                                Ok(payload) => {
                                                    workspace_status.set(format!("Directory loaded: {} entries", payload.entries.len()));
                                                    workspace_tree.set(Some(payload));
                                                }
                                                Err(error) => workspace_status.set(error),
                                            }
                                        });
                                    }
                                >"Refresh"</button>
                            </div>
                            <div class="file-tree-list sidebar-tree">
                                {move || match workspace_tree.get() {
                                    Some(tree) => view! {
                                        <For
                                            each=move || tree.entries.clone().into_iter()
                                            key=|entry| entry.path.clone()
                                            children=move |entry| {
                                                let click_entry = entry.clone();
                                                view! {
                                                    <button type="button" class="file-tree-entry" on:click=move |_| open_workspace_entry(click_entry.clone())>
                                                        <span class="file-kind-icon">{workspace_kind_icon(entry.kind)}</span>
                                                        <span class="file-entry-name">{entry.name}</span>
                                                        <small>{workspace_kind_label(entry.kind)}</small>
                                                    </button>
                                                }
                                            }
                                        />
                                    }.into_any(),
                                    None => view! { <p class="muted">"Loading workspace tree…"</p> }.into_any(),
                                }}
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! { <></> }.into_any()
                }}

                <div class="sidebar-footer">
                    <div class="meta-row">
                        <span>"mode"</span>
                        <strong>"web"</strong>
                    </div>
                    <div class="meta-row">
                        <span>"primary"</span>
                        <strong>{move || status.get().and_then(|s| s.primary_agent_id).unwrap_or_else(|| "master".to_string())}</strong>
                    </div>
                    <div class="meta-row">
                        <span>"activity"</span>
                        <strong>{move || status.get().and_then(|s| s.activity_status).unwrap_or_else(|| "loading".to_string())}</strong>
                    </div>
                </div>
            </aside>

            <main class="workspace">
                <header class=move || if tab_is_standalone(active_tab.get()) { "topbar hidden" } else { "topbar" }>
                    <div>
                        <p class="eyebrow">"themion-cli --web"</p>
                        <h2>{move || match active_tab.get() {
                            ViewTab::Status => "Status",
                            ViewTab::Transcript => "Transcript",
                            ViewTab::Agents => "Agents",
                            ViewTab::Terminal => "Terminal",
                            ViewTab::Workspace => "Workspace",
                        }}</h2>
                    </div>
                    <div class="topbar-pills">
                        <span class="pill">{move || status.get().map(|s| s.runtime.provider).unwrap_or_else(|| "provider…".to_string())}</span>
                        <span class="pill accent">{move || status.get().map(|s| s.runtime.model).unwrap_or_else(|| "model…".to_string())}</span>
                    </div>
                </header>

                <section class=move || if tab_is_standalone(active_tab.get()) { "agent-tab-strip hidden" } else { "agent-tab-strip" } aria-label="agent tabs">
                    {move || match agents.get() {
                        Some(payload) => view! {
                            <For
                                each=move || payload.local_agents.clone().into_iter()
                                key=|agent| agent.agent_id.clone()
                                children=move |agent| {
                                    let agent_id = agent.agent_id.clone();
                                    let subscribe_agent_id = agent.agent_id.clone();
                                    let role_chips = if agent.roles.is_empty() {
                                        vec!["-".to_string()]
                                    } else {
                                        agent.roles.clone()
                                    };
                                    let label = agent.label.clone();
                                    let code = agent.agent_id.clone();
                                    let busy_label = if agent.busy { "busy" } else { "idle" };
                                    view! {
                                        <button
                                            type="button"
                                            class=move || if active_agent.get() == agent_id { "agent-tab active" } else { "agent-tab" }
                                            on:click=move |_| active_agent.set(subscribe_agent_id.clone())
                                        >
                                            <span class="agent-tab-label">{label}</span>
                                            <code>{code}</code>
                                            <small>{busy_label}</small>
                                            <div class="agent-tab-roles">
                                                <For
                                                    each=move || role_chips.clone().into_iter()
                                                    key=|role| role.clone()
                                                    children=move |role| view! {
                                                        <small class="agent-tab-role">{role}</small>
                                                    }
                                                />
                                            </div>
                                        </button>
                                    }
                                }
                            />
                        }.into_any(),
                        None => view! {
                            <For
                                each=move || status.get().map(|s| s.local_agents).unwrap_or_default().into_iter()
                                key=|agent| agent.agent_id.clone()
                                children=move |agent| {
                                    let agent_id = agent.agent_id.clone();
                                    let subscribe_agent_id = agent.agent_id.clone();
                                    let role_chips = if agent.roles.is_empty() {
                                        vec!["-".to_string()]
                                    } else {
                                        agent.roles.clone()
                                    };
                                    let label = agent.label.clone();
                                    let code = agent.agent_id.clone();
                                    let busy_label = if agent.busy { "busy" } else { "idle" };
                                    view! {
                                        <button
                                            type="button"
                                            class=move || if active_agent.get() == agent_id { "agent-tab active" } else { "agent-tab" }
                                            on:click=move |_| active_agent.set(subscribe_agent_id.clone())
                                        >
                                            <span class="agent-tab-label">{label}</span>
                                            <code>{code}</code>
                                            <small>{busy_label}</small>
                                            <div class="agent-tab-roles">
                                                <For
                                                    each=move || role_chips.clone().into_iter()
                                                    key=|role| role.clone()
                                                    children=move |role| view! {
                                                        <small class="agent-tab-role">{role}</small>
                                                    }
                                                />
                                            </div>
                                        </button>
                                    }
                                }
                            />
                        }.into_any(),
                    }}
                </section>

                <section class=move || if tab_is_standalone(active_tab.get()) { "tab-strip hidden" } else { "tab-strip" } aria-label="workspace tabs">
                    <button type="button" class=move || if active_tab.get() == ViewTab::Transcript { "tab active" } else { "tab" } on:click=move |_| active_tab.set(ViewTab::Transcript)>"Transcript"</button>
                    <button type="button" class=move || if active_tab.get() == ViewTab::Status { "tab active" } else { "tab" } on:click=move |_| active_tab.set(ViewTab::Status)>"Status"</button>
                    <button type="button" class=move || if active_tab.get() == ViewTab::Agents { "tab active" } else { "tab" } on:click=move |_| active_tab.set(ViewTab::Agents)>"Agents"</button>
                </section>

                <div class=move || if tab_is_standalone(active_tab.get()) { "content-grid standalone-page" } else { "content-grid" }>
                    {move || match active_tab.get() {
                        ViewTab::Status => view! {
                            <>
                                <section class="panel hero-panel">
                                    <div class="panel-title">
                                        <h3>"Runtime"</h3>
                                        <span class="badge">{move || status.get().map(|s| if s.busy { "busy" } else { "idle" }).unwrap_or("loading")}</span>
                                    </div>
                                    {move || match status.get() {
                                        Some(payload) => view! {
                                            <div class="metric-grid">
                                                <div class="metric"><span>"Bind"</span><strong>{payload.bind_addr}</strong></div>
                                                <div class="metric"><span>"Session"</span><strong>{payload.session_id}</strong></div>
                                                <div class="metric"><span>"Project"</span><strong>{payload.project_dir}</strong></div>
                                                <div class="metric"><span>"Workflow"</span><strong>{format!("{}/{}/{}", payload.runtime.workflow_name, payload.runtime.workflow_phase, payload.runtime.workflow_status)}</strong></div>
                                                <div class="metric"><span>"Tokens"</span><strong>{format!("in={} out={} cached={}", payload.runtime.session_tokens_in, payload.runtime.session_tokens_out, payload.runtime.session_tokens_cached)}</strong></div>
                                                <div class="metric"><span>"Agents"</span><strong>{payload.local_agents.len().to_string()}</strong></div>
                                            </div>
                                        }.into_any(),
                                        None => view! { <p class="muted">"Loading runtime status…"</p> }.into_any(),
                                    }}
                                </section>

                                <section class="panel">
                                    <div class="panel-title">
                                        <h3>"Recent events"</h3>
                                        <span class="badge subtle">"live snapshot"</span>
                                    </div>
                                    <ul class="event-list">
                                        <For
                                            each=move || status.get().map(|s| s.recent_events).unwrap_or_default().into_iter()
                                            key=|event| format!("{}:{}:{}", event.kind, event.at_ms, event.text)
                                            children=move |event| view! {
                                                <li class="event-row">
                                                    <span class="event-kind">{event.kind}</span>
                                                    <code>{event.text}</code>
                                                    <small>{event.at_ms}</small>
                                                </li>
                                            }
                                        />
                                    </ul>
                                </section>
                            </>
                        }.into_any(),
                        ViewTab::Transcript => view! {
                            <section class="panel wide-panel chat-panel">
                                <div class="panel-title">
                                    <h3>{move || format!("Chat · {}", active_agent.get())}</h3>
                                    <span class="badge subtle">"TUI transcript"</span>
                                </div>
                                {move || match transcript.get() {
                                    Some(payload) => view! {
                                        <>
                                            <div
                                                class="chat-history"
                                                node_ref=transcript_history_ref
                                                on:scroll=on_transcript_scroll
                                            >
                                                <For
                                                    each=move || payload.chat_entries.clone().into_iter()
                                                    key=|entry| format!("{}:{}:{}:{}:{:?}", entry.kind, entry.agent_id.clone().unwrap_or_default(), entry.tool_call_id.clone().unwrap_or_default(), entry.text, entry.stats)
                                                    children=move |entry| view! { <ChatEntryRow entry=entry /> }
                                                />
                                            </div>
                                            {move || view! { <ActivityStrip agents=active_agent_activities(status.get(), agents.get()) /> }}
                                        </>
                                    }.into_any(),
                                    None => view! { <p class="muted">"Loading transcript…"</p> }.into_any(),
                                }}
                            </section>
                        }.into_any(),
                        ViewTab::Agents => view! {
                            <section class="panel wide-panel">
                                <div class="panel-title">
                                    <h3>"Agent roster"</h3>
                                    <span class="badge subtle">{move || agents.get().map(|a| a.local_agents.len().to_string()).unwrap_or_else(|| "…".to_string())}</span>
                                </div>
                                {move || match agents.get() {
                                    Some(payload) => view! {
                                        <div class="agent-meta">
                                            <span>{format!("bind {}", payload.bind_addr)}</span>
                                            <span>{format!("session {}", payload.session_id)}</span>
                                            <span>{format!("primary {}", payload.primary_agent_id.clone().unwrap_or_else(|| "unknown".to_string()))}</span>
                                            <span>{format!("activity {}", payload.activity_status.clone().unwrap_or_else(|| "unknown".to_string()))}</span>
                                        </div>
                                        <div class="agent-grid">
                                            <For
                                                each=move || payload.local_agents.clone().into_iter()
                                                key=|agent| agent.agent_id.clone()
                                                children=move |agent| view! {
                                                    <article class="agent-card">
                                                        <div class="agent-avatar">"󰚩"</div>
                                                        <div>
                                                            <h4>{agent.label}</h4>
                                                            <code>{agent.agent_id}</code>
                                                            <p>{format!("roles: {}", agent.roles.join(", "))}</p>
                                                            <div class="agent-flags">
                                                                <span>{if agent.busy { "busy" } else { "idle" }}</span>
                                                                <span>{if agent.incoming { "incoming" } else { "clear" }}</span>
                                                            </div>
                                                        </div>
                                                    </article>
                                                }
                                            />
                                        </div>
                                    }.into_any(),
                                    None => view! { <p class="muted">"Loading agents…"</p> }.into_any(),
                                }}
                            </section>
                        }.into_any(),
                        ViewTab::Workspace => view! {
                            <section class="panel wide-panel workspace-panel">
                                <div class="panel-title">
                                    <div>
                                        <h3>"Project Workspace"</h3>
                                        <p class="muted">"CodeMirror 6 target · JetBrains Mono Nerd Font · project-scoped file APIs"</p>
                                    </div>
                                    <div class="workspace-layout-controls">
                                        <div class="view-mode-toggle" aria-label="code preview layout">
                                            <button type="button" class=move || if workspace_view_mode.get() == WorkspaceViewMode::CodeOnly { "toggle-chip active" } else { "toggle-chip" } on:click=move |_| workspace_view_mode.set(WorkspaceViewMode::CodeOnly)>"code only"</button>
                                            <button type="button" class=move || if workspace_view_mode.get() == WorkspaceViewMode::CodePreview { "toggle-chip active" } else { "toggle-chip" } on:click=move |_| workspace_view_mode.set(WorkspaceViewMode::CodePreview)>"code - preview"</button>
                                            <button type="button" class=move || if workspace_view_mode.get() == WorkspaceViewMode::PreviewOnly { "toggle-chip active" } else { "toggle-chip" } on:click=move |_| workspace_view_mode.set(WorkspaceViewMode::PreviewOnly)>"preview only"</button>
                                        </div>
                                        <span class=move || if workspace_view_mode.get() == WorkspaceViewMode::CodePreview { "split-readout" } else { "split-readout hidden" }>{move || format!("split {}:{}", workspace_split_percent.get(), 100_u8.saturating_sub(workspace_split_percent.get()))}</span>
                                    </div>
                                </div>
                                <div class=move || workspace_editor_grid_class(workspace_view_mode.get()) style=move || workspace_editor_grid_style(workspace_view_mode.get(), workspace_split_percent.get()) node_ref=workspace_grid_ref>
                                    <div class=move || workspace_code_panel_class(workspace_view_mode.get())>
                                        <div class="editor-toolbar">
                                            <span>{move || workspace_file.get().map(|file| file.path).unwrap_or_else(|| "No file open".to_string())}</span>
                                            <span class="badge subtle">{move || if workspace_dirty.get() { "dirty" } else { "clean" }}</span>
                                            <button
                                                type="button"
                                                class="primary-action small"
                                                on:click=move |_| {
                                                    let Some(file) = workspace_file.get_untracked() else { return; };
                                                    let content = workspace_content.get_untracked();
                                                    workspace_status.set(format!("Saving {}", file.path));
                                                    leptos::task::spawn_local(async move {
                                                        match save_workspace_file(&file.path, content, &file.conflict_token).await {
                                                            Ok(saved) => {
                                                                workspace_file.update(|open| {
                                                                    if let Some(open_file) = open {
                                                                        open_file.conflict_token = saved.conflict_token;
                                                                        open_file.size_bytes = saved.size_bytes;
                                                                        open_file.modified_at_ms = saved.modified_at_ms;
                                                                    }
                                                                });
                                                                workspace_dirty.set(false);
                                                                workspace_status.set("File saved".to_string());
                                                            }
                                                            Err(error) => workspace_status.set(error),
                                                        }
                                                    });
                                                }
                                            >"Save"</button>
                                        </div>
                                        <div class="codemirror-host" node_ref=workspace_editor_ref>
                                            <p class="muted editor-placeholder">"Open a text/code file from the project tree…"</p>
                                        </div>
                                        <p class="muted">{move || workspace_status.get()}</p>
                                    </div>
                                    <div
                                        class=move || workspace_splitter_class(workspace_view_mode.get(), workspace_split_dragging.get())
                                        on:pointerdown=start_workspace_split_drag
                                        on:pointermove=drag_workspace_split
                                        on:pointerup=stop_workspace_split_drag
                                        on:pointercancel=stop_workspace_split_drag
                                        title="Drag to resize code and preview"
                                    ></div>
                                    <aside class=move || workspace_preview_panel_class(workspace_view_mode.get())>
                                        <div class="panel-title"><h3>"Preview"</h3></div>
                                        {move || workspace_preview_path.get().map(|url| view! { <WorkspacePreview url=url /> }.into_any()).unwrap_or_else(|| view! { <p class="muted">"Open an image, video, Markdown, HTML, or PDF file to preview it."</p> }.into_any())}
                                    </aside>
                                </div>
                            </section>
                        }.into_any(),
                        ViewTab::Terminal => view! { <></> }.into_any(),
                    }}
                </div>
                <section class=move || if tab_is_standalone(active_tab.get()) { "composer-card composer-bottom hidden" } else { "composer-card composer-bottom" }>
                    <div class="composer-head">
                        <div>
                            <h3>{move || format!("Prompt → {}", active_agent.get())}</h3>
                            <p>"Send input through the shared CLI-owned websocket."</p>
                        </div>
                        <span class="shortcut">"Enter to submit · Shift+Enter for newline"</span>
                    </div>
                    <form on:submit=on_submit class="composer-form">
                        <div class="composer-input-wrap">
                            <div
                                class=move || format!(
                                    "composer-connection {}",
                                    websocket_connection_class(&socket_state.get())
                                )
                                title=move || websocket_connection_title(&socket_state.get())
                                aria-label=move || websocket_connection_title(&socket_state.get())
                            >
                                <span class="composer-connection-dot" aria-hidden="true"></span>
                                <span class="composer-connection-label">{move || websocket_connection_short_label(&socket_state.get())}</span>
                            </div>
                            <textarea
                                prop:value=move || prompt.get()
                                on:input=move |ev| {
                                    prompt.set(event_target_value(&ev));
                                    prompt_error.set(None);
                                }
                                on:keydown=on_prompt_keydown
                                rows="3"
                                placeholder="Ask the active agent…"
                            />
                        </div>
                        <button type="submit" class="primary-action">"Send"</button>
                    </form>
                    {move || prompt_error.get().map(|error| view! { <p class="muted">{error}</p> })}
                </section>
            </main>
        </div>
    }
}

fn active_agent_activities(
    status: Option<WebStatusResponse>,
    agents: Option<WebAgentsResponse>,
) -> Vec<WebAgentStatus> {
    let source_agents = agents
        .map(|payload| payload.local_agents)
        .or_else(|| status.map(|payload| payload.local_agents))
        .unwrap_or_default();
    source_agents
        .into_iter()
        .filter(|agent| {
            agent
                .activity_status
                .as_deref()
                .is_some_and(activity_status_is_active)
        })
        .collect()
}

fn sidebar_tab_is_active(active_tab: ViewTab, tab: ViewTab) -> bool {
    match tab {
        ViewTab::Transcript => matches!(
            active_tab,
            ViewTab::Transcript | ViewTab::Status | ViewTab::Agents
        ),
        ViewTab::Terminal => active_tab == ViewTab::Terminal,
        _ => active_tab == tab,
    }
}

fn tab_is_standalone(tab: ViewTab) -> bool {
    matches!(tab, ViewTab::Terminal | ViewTab::Workspace)
}

fn activity_status_is_active(status: &str) -> bool {
    !matches!(status, "" | "idle" | "nap")
}

fn activity_label_from_status(status: &str) -> String {
    if status.starts_with("streaming") {
        return "Receiving response".to_string();
    }
    match status {
        "preparing" => "Preparing request".to_string(),
        "waiting-model" => "Waiting for model".to_string(),
        "running-tool" => "Running tool".to_string(),
        "waiting-after-tool" => "Waiting for model".to_string(),
        "finalizing" => "Finalizing".to_string(),
        "idle" => "Idle".to_string(),
        "nap" => "Idle for a while".to_string(),
        other => sentence_case_status(other),
    }
}

fn sentence_case_status(status: &str) -> String {
    let normalized = status.replace(['-', '_'], " ");
    let mut chars = normalized.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.collect::<String>()),
        None => "Unknown".to_string(),
    }
}

fn agent_activity_display_label(agent: &WebAgentStatus) -> String {
    agent
        .activity_label
        .clone()
        .or_else(|| {
            agent
                .activity_status
                .as_deref()
                .map(activity_label_from_status)
        })
        .unwrap_or_else(|| "Active".to_string())
}

fn websocket_status_label(state: &str) -> &'static str {
    match state {
        "connecting" => "connecting",
        "open" => "open",
        "reconnecting" => "reconnecting…",
        "closed" => "closed",
        _ => "connecting",
    }
}

fn websocket_connection_class(state: &str) -> &'static str {
    match state {
        "open" => "open",
        "reconnecting" | "closed" => "lost",
        _ => "connecting",
    }
}

fn websocket_connection_short_label(state: &str) -> &'static str {
    match state {
        "open" => "Live",
        "reconnecting" => "Lost",
        "closed" => "Lost",
        _ => "Conn",
    }
}

fn websocket_connection_title(state: &str) -> &'static str {
    match state {
        "open" => "WebSocket connected",
        "reconnecting" => "WebSocket lost; reconnecting",
        "closed" => "WebSocket closed",
        _ => "WebSocket connecting",
    }
}

#[component]
fn ActivityStrip(agents: Vec<WebAgentStatus>) -> impl IntoView {
    if agents.is_empty() {
        return view! { <div class="activity-strip hidden"></div> }.into_any();
    }
    view! {
        <div class="activity-strip" aria-label="active agents">
            <For
                each=move || agents.clone().into_iter()
                key=|agent| agent.agent_id.clone()
                children=move |agent| {
                    let agent_label = if agent.label.is_empty() {
                        agent.agent_id.clone()
                    } else {
                        agent.label.clone()
                    };
                    let status_label = agent_activity_display_label(&agent);
                    let title = agent
                        .activity_changed_at_ms
                        .map(|at_ms| format!("activity changed at {at_ms}ms"))
                        .unwrap_or_else(|| "active agent".to_string());
                    view! {
                        <div class="activity-chip active" title=title>
                            <span class="activity-pulse" aria-hidden="true"></span>
                            <span class="activity-agent">{agent_label}</span>
                            <span class="activity-separator">"·"</span>
                            <span class="activity-label">{status_label}</span>
                        </div>
                    }
                }
            />
        </div>
    }
    .into_any()
}

fn chat_entry_label(entry: &WebChatEntry) -> String {
    if let Some(agent_id) = entry.agent_id.as_ref().filter(|value| !value.is_empty()) {
        return agent_id.clone();
    }
    match entry.kind.as_str() {
        "user" => "user".to_string(),
        "assistant" => "assistant".to_string(),
        "tool_call" | "tool_done" => "tool".to_string(),
        "status" => entry.source.clone().unwrap_or_else(|| "status".to_string()),
        "remote" => entry.source.clone().unwrap_or_else(|| "remote".to_string()),
        "turn_done" => "turn".to_string(),
        "stats" => "stats".to_string(),
        "transcript_omitted" => "transcript".to_string(),
        "banner" => "themion".to_string(),
        _ => entry.kind.clone(),
    }
}

fn chat_entry_kind_label(entry: &WebChatEntry) -> String {
    match (entry.kind.as_str(), entry.completed) {
        ("tool_call", true) => "TOOL_CALL ✓".to_string(),
        ("transcript_omitted", _) => "OMITTED".to_string(),
        _ => entry.kind.to_ascii_uppercase(),
    }
}

#[component]
fn ChatEntryRow(entry: WebChatEntry) -> impl IntoView {
    let class_name = if entry.kind == "transcript_omitted" {
        "chat-row transcript-omitted".to_string()
    } else {
        format!("chat-row {}", entry.kind)
    };
    let label = chat_entry_label(&entry);
    let kind_label = chat_entry_kind_label(&entry);
    view! {
        <article class=class_name>
            <div class="chat-meta">
                <span class="chat-role">{label}</span>
                <span class="chat-kind">{kind_label}</span>
            </div>
            <div class="chat-bubble">
                {move || if entry.kind == "tool_call" {
                    view! {
                        <>
                            <div class="tool-line">" "{entry.detail.clone().unwrap_or_else(|| entry.text.clone())}</div>
                            {entry.reason.clone().map(|reason| view! { <p class="tool-reason">{reason}</p> })}
                        </>
                    }.into_any()
                } else if entry.kind == "turn_done" {
                    view! {
                        <>
                            <div>{entry.text.clone()}</div>
                            {entry.stats.clone().map(|stats| view! { <p class="tool-reason">{format!("stats: {stats}")}</p> })}
                        </>
                    }.into_any()
                } else {
                    view! { <code>{entry.text.clone()}</code> }.into_any()
                }}
            </div>
        </article>
    }
}

const TRANSCRIPT_TRAILING_SCROLL_PX: i32 = 48;

fn schedule_scroll_transcript_to_recent(history: web_sys::HtmlElement) {
    if let Some(window) = web_sys::window() {
        let callback = Closure::<dyn FnMut()>::once(move || scroll_transcript_to_recent(&history));
        let _ = window.request_animation_frame(callback.as_ref().unchecked_ref());
        callback.forget();
    }
}

fn scroll_transcript_to_recent(history: &web_sys::HtmlElement) {
    history.set_scroll_top(history.scroll_height());
}

fn transcript_history_is_trailing(history: &web_sys::HtmlElement) -> bool {
    scroll_position_is_trailing(
        history.scroll_top(),
        history.client_height(),
        history.scroll_height(),
    )
}

fn scroll_position_is_trailing(scroll_top: i32, client_height: i32, scroll_height: i32) -> bool {
    scroll_height - (scroll_top + client_height) <= TRANSCRIPT_TRAILING_SCROLL_PX
}

fn prompt_keydown_should_submit(ev: &KeyboardEvent) -> bool {
    ev.key() == "Enter" && !ev.shift_key() && !ev.alt_key() && !ev.ctrl_key() && !ev.meta_key()
}

fn retry_delay_ms(attempt: u32) -> i32 {
    match attempt {
        0 => 0,
        1 => 250,
        2 => 500,
        3 => 1_000,
        4 => 2_000,
        _ => 5_000,
    }
}

fn create_shared_socket(
    socket_state: RwSignal<String>,
    agent_stream: RwSignal<Vec<String>>,
    shell_stream: RwSignal<Vec<String>>,
    status: RwSignal<Option<WebStatusResponse>>,
    transcript: RwSignal<Option<WebTranscriptResponse>>,
    agents: RwSignal<Option<WebAgentsResponse>>,
) -> SharedSocket {
    SharedSocket::new(
        socket_state,
        agent_stream,
        shell_stream,
        status,
        transcript,
        agents,
    )
}

async fn fetch_status() -> Result<WebStatusResponse, String> {
    let response = gloo_net::http::Request::get("/api/status")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    response
        .json::<WebStatusResponse>()
        .await
        .map_err(|error| error.to_string())
}

async fn fetch_transcript() -> Result<WebTranscriptResponse, String> {
    let response = gloo_net::http::Request::get("/api/transcript")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    response
        .json::<WebTranscriptResponse>()
        .await
        .map_err(|error| error.to_string())
}

async fn fetch_agents() -> Result<WebAgentsResponse, String> {
    let response = gloo_net::http::Request::get("/api/agents")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    response
        .json::<WebAgentsResponse>()
        .await
        .map_err(|error| error.to_string())
}

async fn fetch_workspace_tree(path: &str) -> Result<WorkspaceTreeResponse, String> {
    let response = gloo_net::http::Request::get(&format!(
        "/api/workspace/tree?path={}",
        encode_query_component(path)
    ))
    .send()
    .await
    .map_err(|error| error.to_string())?;
    decode_workspace_response::<WorkspaceTreeResponse>(response).await
}

async fn fetch_workspace_file(path: &str) -> Result<WorkspaceFileResponse, String> {
    let response = gloo_net::http::Request::get(&format!(
        "/api/workspace/file?path={}",
        encode_query_component(path)
    ))
    .send()
    .await
    .map_err(|error| error.to_string())?;
    decode_workspace_response::<WorkspaceFileResponse>(response).await
}

async fn save_workspace_file(
    path: &str,
    content: String,
    conflict_token: &str,
) -> Result<WorkspaceSaveResponse, String> {
    let request = WorkspaceSaveRequest {
        path: path.to_string(),
        content,
        conflict_token: conflict_token.to_string(),
    };
    let response = gloo_net::http::Request::put("/api/workspace/file")
        .json(&request)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    decode_workspace_response::<WorkspaceSaveResponse>(response).await
}

async fn decode_workspace_response<T>(response: gloo_net::http::Response) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    if response.ok() {
        return response
            .json::<T>()
            .await
            .map_err(|error| error.to_string());
    }
    let status = response.status();
    match response.json::<WorkspaceErrorResponse>().await {
        Ok(error) => Err(format!(
            "{}: {} ({})",
            error.error.code, error.error.message, status
        )),
        Err(error) => Err(format!("workspace request failed ({status}): {error}")),
    }
}

fn workspace_preview_url(path: &str) -> String {
    format!(
        "/api/workspace/preview?path={}",
        encode_query_component(path)
    )
}

fn workspace_kind_preview_url(kind: WorkspaceFileKind, path: &str) -> Option<String> {
    workspace_kind_is_previewable(kind).then(|| workspace_preview_url(path))
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

fn workspace_kind_icon(kind: WorkspaceFileKind) -> &'static str {
    match kind {
        WorkspaceFileKind::Directory => "",
        WorkspaceFileKind::Image => "󰋩",
        WorkspaceFileKind::Video => "󰈫",
        WorkspaceFileKind::Markdown => "",
        WorkspaceFileKind::Html => "",
        WorkspaceFileKind::Pdf => "",
        WorkspaceFileKind::Text => "󰈙",
        WorkspaceFileKind::Binary => "󰞷",
        WorkspaceFileKind::Unsupported => "󰈔",
    }
}

fn workspace_kind_label(kind: WorkspaceFileKind) -> &'static str {
    match kind {
        WorkspaceFileKind::Directory => "dir",
        WorkspaceFileKind::Text => "text",
        WorkspaceFileKind::Image => "image",
        WorkspaceFileKind::Video => "video",
        WorkspaceFileKind::Markdown => "md",
        WorkspaceFileKind::Html => "html",
        WorkspaceFileKind::Pdf => "pdf",
        WorkspaceFileKind::Binary => "binary",
        WorkspaceFileKind::Unsupported => "file",
    }
}

fn workspace_editor_grid_class(mode: WorkspaceViewMode) -> &'static str {
    match mode {
        WorkspaceViewMode::CodeOnly => "workspace-editor-grid code-only",
        WorkspaceViewMode::CodePreview => "workspace-editor-grid code-preview",
        WorkspaceViewMode::PreviewOnly => "workspace-editor-grid preview-only",
    }
}

fn workspace_editor_grid_style(mode: WorkspaceViewMode, split_percent: u8) -> String {
    match mode {
        WorkspaceViewMode::CodePreview => {
            let code = split_percent.clamp(25, 75);
            let preview = 100_u8.saturating_sub(code);
            format!("grid-template-columns: minmax(0, {code}fr) 0.45rem minmax(0, {preview}fr);")
        }
        _ => String::new(),
    }
}

fn workspace_code_panel_class(mode: WorkspaceViewMode) -> &'static str {
    match mode {
        WorkspaceViewMode::PreviewOnly => "code-editor-panel hidden",
        _ => "code-editor-panel",
    }
}

fn workspace_splitter_class(mode: WorkspaceViewMode, dragging: bool) -> &'static str {
    match (mode, dragging) {
        (WorkspaceViewMode::CodePreview, true) => "workspace-splitter dragging",
        (WorkspaceViewMode::CodePreview, false) => "workspace-splitter",
        _ => "workspace-splitter hidden",
    }
}

fn workspace_preview_panel_class(mode: WorkspaceViewMode) -> &'static str {
    match mode {
        WorkspaceViewMode::CodeOnly => "preview-panel hidden",
        _ => "preview-panel",
    }
}

fn update_workspace_split_from_client_x(
    grid_ref: NodeRef<Div>,
    split_percent: RwSignal<u8>,
    client_x: i32,
) {
    let Some(grid) = grid_ref.get_untracked() else {
        return;
    };
    let rect = grid.get_bounding_client_rect();
    let width = rect.width();
    if width <= 0.0 {
        return;
    }
    let raw = ((client_x as f64 - rect.left()) / width) * 100.0;
    split_percent.set(raw.round().clamp(25.0, 75.0) as u8);
}

fn workspace_language(path: &str, kind: WorkspaceFileKind) -> &'static str {
    if kind == WorkspaceFileKind::Markdown {
        return "markdown";
    }
    if kind == WorkspaceFileKind::Html {
        return "html";
    }
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "rs" => "rust",
        "toml" => "toml",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "js" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "sh" | "bash" | "zsh" | "fish" => "shell",
        _ => {
            let name = path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(name.as_str(), "makefile" | "dockerfile") {
                "shell"
            } else {
                "text"
            }
        }
    }
}

fn workspace_parent_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn encode_query_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            b' ' => vec!['%', '2', '0'],
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

trait EmptyLabel {
    fn if_empty_label(self, label: &str) -> String;
}

impl EmptyLabel for String {
    fn if_empty_label(self, label: &str) -> String {
        if self.is_empty() {
            label.to_string()
        } else {
            self
        }
    }
}

#[component]
fn WorkspacePreview(url: String) -> impl IntoView {
    let lowered = url.to_ascii_lowercase();
    if lowered.contains(".png")
        || lowered.contains(".jpg")
        || lowered.contains(".jpeg")
        || lowered.contains(".gif")
        || lowered.contains(".webp")
        || lowered.contains(".svg")
    {
        return view! { <img class="workspace-preview-media" src=url alt="file preview" /> }
            .into_any();
    }
    if lowered.contains(".mp4") || lowered.contains(".webm") || lowered.contains(".ogg") {
        return view! { <video class="workspace-preview-media" src=url controls=true /> }
            .into_any();
    }
    view! { <iframe class="workspace-preview-frame" src=url sandbox="allow-same-origin" /> }
        .into_any()
}

#[cfg(test)]
mod tests {
    use super::{
        retry_delay_ms, sentence_case_status, websocket_connection_class,
        websocket_connection_short_label, websocket_connection_title, websocket_status_label,
        SocketLifecycleState, SubscriptionTarget,
    };

    fn keydown_should_submit(key: &str, shift: bool, alt: bool, ctrl: bool, meta: bool) -> bool {
        key == "Enter" && !shift && !alt && !ctrl && !meta
    }

    fn agent_status(
        agent_id: &str,
        activity_status: Option<&str>,
        activity_label: Option<&str>,
    ) -> super::WebAgentStatus {
        super::WebAgentStatus {
            agent_id: agent_id.to_string(),
            label: agent_id.to_string(),
            roles: Vec::new(),
            busy: activity_status.is_some(),
            incoming: false,
            activity_status: activity_status.map(str::to_string),
            activity_label: activity_label.map(str::to_string),
            activity_changed_at_ms: Some(123),
        }
    }

    #[test]
    fn plain_enter_submits_prompt() {
        assert!(keydown_should_submit("Enter", false, false, false, false));
    }

    #[test]
    fn shifted_enter_does_not_submit_prompt() {
        assert!(!keydown_should_submit("Enter", true, false, false, false));
    }

    #[test]
    fn modified_enter_does_not_submit_prompt() {
        assert!(!keydown_should_submit("Enter", false, true, false, false));
        assert!(!keydown_should_submit("Enter", false, false, true, false));
        assert!(!keydown_should_submit("Enter", false, false, false, true));
    }

    #[test]
    fn user_chat_entry_label_shows_target_agent() {
        let entry = super::WebChatEntry {
            kind: "user".to_string(),
            agent_id: Some("master".to_string()),
            tool_call_id: None,
            source: None,
            text: "hello".to_string(),
            detail: None,
            reason: None,
            stats: None,
            completed: false,
        };
        assert_eq!(super::chat_entry_label(&entry), "master");
        assert_eq!(super::chat_entry_kind_label(&entry), "USER");
    }

    #[test]
    fn chat_entry_label_prefers_agent_id_for_status_and_remote_rows() {
        let status = super::WebChatEntry {
            kind: "status".to_string(),
            agent_id: Some("smith-1".to_string()),
            tool_call_id: None,
            source: Some("runtime".to_string()),
            text: "turn started".to_string(),
            detail: None,
            reason: None,
            stats: None,
            completed: false,
        };
        assert_eq!(super::chat_entry_label(&status), "smith-1");

        let remote = super::WebChatEntry {
            kind: "remote".to_string(),
            agent_id: Some("smith-2".to_string()),
            tool_call_id: None,
            source: Some("stylos".to_string()),
            text: "Stylos incoming message".to_string(),
            detail: None,
            reason: None,
            stats: None,
            completed: false,
        };
        assert_eq!(super::chat_entry_label(&remote), "smith-2");
    }

    #[test]
    fn chat_entry_label_uses_source_for_non_agent_rows() {
        let remote = super::WebChatEntry {
            kind: "remote".to_string(),
            agent_id: None,
            tool_call_id: None,
            source: Some("stylos".to_string()),
            text: "Stylos talk".to_string(),
            detail: None,
            reason: None,
            stats: None,
            completed: false,
        };
        assert_eq!(super::chat_entry_label(&remote), "stylos");
    }

    #[test]
    fn transcript_omitted_entry_has_clear_label() {
        let entry = super::WebChatEntry {
            kind: "transcript_omitted".to_string(),
            agent_id: None,
            tool_call_id: None,
            source: None,
            text: "older transcript entries omitted: 42".to_string(),
            detail: None,
            reason: None,
            stats: None,
            completed: false,
        };

        assert_eq!(super::chat_entry_label(&entry), "transcript");
        assert_eq!(super::chat_entry_kind_label(&entry), "OMITTED");
    }

    #[test]
    fn chat_entry_kind_label_is_uppercase() {
        let mut entry = super::WebChatEntry {
            kind: "tool_call".to_string(),
            agent_id: Some("master".to_string()),
            tool_call_id: Some("call-1".to_string()),
            source: None,
            text: "shell: df -h".to_string(),
            detail: None,
            reason: None,
            stats: None,
            completed: false,
        };
        assert_eq!(super::chat_entry_kind_label(&entry), "TOOL_CALL");
        entry.completed = true;
        assert_eq!(super::chat_entry_kind_label(&entry), "TOOL_CALL ✓");
    }

    #[test]
    fn activity_status_labels_are_human_readable() {
        assert_eq!(
            super::activity_label_from_status("waiting-model"),
            "Waiting for model"
        );
        assert_eq!(
            super::activity_label_from_status("streaming c:3 ch:42"),
            "Receiving response"
        );
        assert_eq!(
            super::activity_label_from_status("running-tool"),
            "Running tool"
        );
        assert_eq!(
            super::activity_label_from_status("unknown-state"),
            "Unknown state"
        );
    }

    #[test]
    fn active_agent_activities_filters_idle_agents_and_keeps_all_active() {
        let status = super::WebStatusResponse {
            bind_addr: String::new(),
            project_dir: String::new(),
            session_id: String::new(),
            primary_agent_id: Some("master".to_string()),
            busy: true,
            activity_status: Some("waiting-model".to_string()),
            local_agents: vec![
                agent_status("master", Some("waiting-model"), None),
                agent_status("smith-1", Some("running-tool"), Some("running tool… shell")),
                agent_status("smith-2", Some("idle"), None),
            ],
            runtime: Default::default(),
            recent_events: Vec::new(),
        };
        let active = super::active_agent_activities(Some(status), None);
        assert_eq!(
            active
                .iter()
                .map(|agent| agent.agent_id.as_str())
                .collect::<Vec<_>>(),
            vec!["master", "smith-1"]
        );
    }

    #[test]
    fn transcript_scroll_detects_trailing_mode_near_bottom() {
        assert!(super::scroll_position_is_trailing(950, 100, 1_000));
        assert!(super::scroll_position_is_trailing(852, 100, 1_000));
    }

    #[test]
    fn transcript_scroll_detects_browse_mode_away_from_bottom() {
        assert!(!super::scroll_position_is_trailing(851, 100, 1_000));
        assert!(!super::scroll_position_is_trailing(400, 100, 1_000));
    }

    #[test]
    fn retry_delay_is_bounded_and_starts_immediately() {
        assert_eq!(retry_delay_ms(0), 0);
        assert_eq!(retry_delay_ms(1), 250);
        assert_eq!(retry_delay_ms(2), 500);
        assert_eq!(retry_delay_ms(4), 2_000);
        assert_eq!(retry_delay_ms(5), 5_000);
        assert_eq!(retry_delay_ms(9), 5_000);
    }

    #[test]
    fn websocket_state_labels_match_ui_text() {
        assert_eq!(
            websocket_status_label(SocketLifecycleState::Connecting.as_str()),
            "connecting"
        );
        assert_eq!(
            websocket_status_label(SocketLifecycleState::Open.as_str()),
            "open"
        );
        assert_eq!(
            websocket_status_label(SocketLifecycleState::Reconnecting.as_str()),
            "reconnecting…"
        );
        assert_eq!(
            websocket_status_label(SocketLifecycleState::Closed.as_str()),
            "closed"
        );
    }

    #[test]
    fn websocket_prompt_indicator_maps_lost_states() {
        assert_eq!(
            websocket_connection_class(SocketLifecycleState::Open.as_str()),
            "open"
        );
        assert_eq!(
            websocket_connection_class(SocketLifecycleState::Connecting.as_str()),
            "connecting"
        );
        assert_eq!(
            websocket_connection_class(SocketLifecycleState::Reconnecting.as_str()),
            "lost"
        );
        assert_eq!(
            websocket_connection_class(SocketLifecycleState::Closed.as_str()),
            "lost"
        );
        assert_eq!(
            websocket_connection_short_label(SocketLifecycleState::Open.as_str()),
            "Live"
        );
        assert_eq!(
            websocket_connection_short_label(SocketLifecycleState::Reconnecting.as_str()),
            "Lost"
        );
        assert_eq!(
            websocket_connection_title(SocketLifecycleState::Reconnecting.as_str()),
            "WebSocket lost; reconnecting"
        );
    }

    #[test]
    fn subscription_target_deduplicates_same_domain_and_target() {
        let mut set = std::collections::BTreeSet::new();
        set.insert(SubscriptionTarget::new("agent", "master"));
        set.insert(SubscriptionTarget::new("agent", "master"));
        set.insert(SubscriptionTarget::new("runtime", "status"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn sentence_case_status_normalizes_hyphenated_values() {
        assert_eq!(
            sentence_case_status("waiting-after-tool"),
            "Waiting after tool"
        );
        assert_eq!(sentence_case_status(""), "Unknown");
    }
}
