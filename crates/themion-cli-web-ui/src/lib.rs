use leptos::ev::{KeyboardEvent, SubmitEvent};
use leptos::html::Div;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Event, MessageEvent, WebSocket};

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
    incoming: bool,
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
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WebRecentEvent {
    kind: String,
    text: String,
    at_ms: u64,
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

#[derive(Clone)]
struct SharedSocket {
    socket: Rc<WebSocket>,
    seq: Rc<RefCell<u64>>,
}

impl SharedSocket {
    fn send(&self, kind: &str, domain: &str, target_id: &str, payload: serde_json::Value) {
        let mut seq = self.seq.borrow_mut();
        let envelope = WebSocketEnvelope {
            kind: kind.to_string(),
            domain: domain.to_string(),
            target_id: target_id.to_string(),
            sequence_id: Some(*seq),
            request_id: Some(format!("req-{}", *seq)),
            payload,
        };
        *seq += 1;
        let _ = self
            .socket
            .send_with_str(&serde_json::to_string(&envelope).unwrap_or_default());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewTab {
    Status,
    Transcript,
    Agents,
    Terminal,
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
    let transcript_history_ref = NodeRef::<Div>::new();
    let transcript_trailing = RwSignal::new(true);

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
                shared.send("subscribe", "runtime", "status", serde_json::json!({}));
                shared.send("subscribe", "agent", &agent_id, serde_json::json!({}));
                shared.send("subscribe", "terminal", "list", serde_json::json!({}));
            }
            transcript.set(transcript_payload);
            agents.set(agents_payload);
        });
    });

    Effect::new(move |_| {
        let status_payload = status.get();
        let current_agent = active_agent.get();
        if let Some(payload) = status_payload {
            let available = payload.local_agents.iter().any(|agent| agent.agent_id == current_agent);
            if !available {
                let next_agent = payload
                    .primary_agent_id
                    .clone()
                    .or_else(|| payload.local_agents.first().map(|agent| agent.agent_id.clone()))
                    .unwrap_or_else(|| "master".to_string());
                active_agent.set(next_agent);
            }
        }
    });

    Effect::new(move |_| {
        let selected = active_agent.get();
        shared_socket.send("subscribe", "agent", &selected, serde_json::json!({}));
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
        shared_socket_for_submit.send(
            "input",
            "agent",
            &agent_id,
            serde_json::json!({"prompt": text}),
        );
        prompt.set(String::new());
    };

    let on_prompt_keydown = move |ev: KeyboardEvent| {
        if prompt_keydown_should_submit(&ev) {
            ev.prevent_default();
            let text = prompt.get_untracked().trim().to_string();
            if text.is_empty() {
                return;
            }
            let agent_id = active_agent.get_untracked();
            shared_socket_for_keydown.send(
                "input",
                "agent",
                &agent_id,
                serde_json::json!({"prompt": text}),
            );
            prompt.set(String::new());
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
                        <strong>{move || socket_state.get()}</strong>
                    </div>
                </div>

                <nav class="sidebar-nav" aria-label="main menu">
                    {sidebar_button(ViewTab::Transcript, "󰈙", "Agent", "chat transcript")}
                    {sidebar_button(ViewTab::Terminal, "", "Terminal", "placeholder page")}
                </nav>

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
                <header class=move || if active_tab.get() == ViewTab::Terminal { "topbar hidden" } else { "topbar" }>
                    <div>
                        <p class="eyebrow">"themion-cli --web"</p>
                        <h2>{move || match active_tab.get() {
                            ViewTab::Status => "Status",
                            ViewTab::Transcript => "Transcript",
                            ViewTab::Agents => "Agents",
                            ViewTab::Terminal => "Terminal",
                        }}</h2>
                    </div>
                    <div class="topbar-pills">
                        <span class="pill">{move || status.get().map(|s| s.runtime.provider).unwrap_or_else(|| "provider…".to_string())}</span>
                        <span class="pill accent">{move || status.get().map(|s| s.runtime.model).unwrap_or_else(|| "model…".to_string())}</span>
                    </div>
                </header>

                <section class=move || if active_tab.get() == ViewTab::Terminal { "agent-tab-strip hidden" } else { "agent-tab-strip" } aria-label="agent tabs">
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

                <section class=move || if active_tab.get() == ViewTab::Terminal { "tab-strip hidden" } else { "tab-strip" } aria-label="workspace tabs">
                    <button type="button" class=move || if active_tab.get() == ViewTab::Transcript { "tab active" } else { "tab" } on:click=move |_| active_tab.set(ViewTab::Transcript)>"Transcript"</button>
                    <button type="button" class=move || if active_tab.get() == ViewTab::Status { "tab active" } else { "tab" } on:click=move |_| active_tab.set(ViewTab::Status)>"Status"</button>
                    <button type="button" class=move || if active_tab.get() == ViewTab::Agents { "tab active" } else { "tab" } on:click=move |_| active_tab.set(ViewTab::Agents)>"Agents"</button>
                </section>

                <div class=move || if active_tab.get() == ViewTab::Terminal { "content-grid terminal-empty" } else { "content-grid" }>
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
                        ViewTab::Terminal => view! { <></> }.into_any(),
                    }}
                </div>
                <section class=move || if active_tab.get() == ViewTab::Terminal { "composer-card composer-bottom hidden" } else { "composer-card composer-bottom" }>
                    <div class="composer-head">
                        <div>
                            <h3>{move || format!("Prompt → {}", active_agent.get())}</h3>
                            <p>"Send input through the shared CLI-owned websocket."</p>
                        </div>
                        <span class="shortcut">"Enter to submit · Shift+Enter for newline"</span>
                    </div>
                    <form on:submit=on_submit class="composer-form">
                        <textarea
                            prop:value=move || prompt.get()
                            on:input=move |ev| prompt.set(event_target_value(&ev))
                            on:keydown=on_prompt_keydown
                            rows="3"
                            placeholder="Ask the active agent…"
                        />
                        <button type="submit" class="primary-action">"Send"</button>
                    </form>
                </section>
            </main>
        </div>
    }
}

fn sidebar_tab_is_active(active_tab: ViewTab, tab: ViewTab) -> bool {
    match tab {
        ViewTab::Transcript => active_tab != ViewTab::Terminal,
        ViewTab::Terminal => active_tab == ViewTab::Terminal,
        _ => active_tab == tab,
    }
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
        "banner" => "themion".to_string(),
        _ => entry.kind.clone(),
    }
}

fn chat_entry_kind_label(entry: &WebChatEntry) -> String {
    match (entry.kind.as_str(), entry.completed) {
        ("tool_call", true) => "TOOL_CALL ✓".to_string(),
        _ => entry.kind.to_ascii_uppercase(),
    }
}

#[component]
fn ChatEntryRow(entry: WebChatEntry) -> impl IntoView {
    let class_name = format!("chat-row {}", entry.kind);
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

fn create_shared_socket(
    socket_state: RwSignal<String>,
    agent_stream: RwSignal<Vec<String>>,
    shell_stream: RwSignal<Vec<String>>,
    status: RwSignal<Option<WebStatusResponse>>,
    transcript: RwSignal<Option<WebTranscriptResponse>>,
    agents: RwSignal<Option<WebAgentsResponse>>,
) -> SharedSocket {
    let location = web_sys::window().expect("window").location();
    let protocol = match location.protocol().ok().as_deref() {
        Some("https:") => "wss:",
        _ => "ws:",
    };
    let host = location
        .host()
        .unwrap_or_else(|_| "127.0.0.1:8420".to_string());
    let ws = WebSocket::new(&format!("{protocol}//{host}/api/ws")).expect("websocket");
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let onopen = Closure::<dyn FnMut(Event)>::new({
        let socket_state = socket_state;
        move |_| socket_state.set("open".to_string())
    });
    ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let onclose = Closure::<dyn FnMut(Event)>::new({
        let socket_state = socket_state;
        move |_| socket_state.set("closed".to_string())
    });
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();

    let onerror = Closure::<dyn FnMut(Event)>::new({
        let socket_state = socket_state;
        move |_| socket_state.set("error".to_string())
    });
    ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    onerror.forget();

    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        if let Some(text) = event.data().as_string() {
            if let Ok(envelope) = serde_json::from_str::<WebSocketEnvelope>(&text) {
                let line = format!(
                    "seq={:?} target={} payload={}",
                    envelope.sequence_id, envelope.target_id, envelope.payload
                );
                let refresh_transcript = matches!(envelope.domain.as_str(), "agent" | "runtime");
                let refresh_agents = envelope.domain == "runtime";
                match envelope.domain.as_str() {
                    "agent" => agent_stream.update(|lines| lines.push(line)),
                    "terminal" => shell_stream.update(|lines| lines.push(line)),
                    "runtime" if envelope.target_id == "status" => {
                        if let Ok(payload) =
                            serde_json::from_value::<WebStatusResponse>(envelope.payload.clone())
                        {
                            status.set(Some(payload));
                        }
                    }
                    _ => {}
                }
                if refresh_transcript {
                    let transcript = transcript;
                    leptos::task::spawn_local(async move {
                        if let Ok(payload) = fetch_transcript().await {
                            transcript.set(Some(payload));
                        }
                    });
                }
                if refresh_agents {
                    let agents = agents;
                    leptos::task::spawn_local(async move {
                        if let Ok(payload) = fetch_agents().await {
                            agents.set(Some(payload));
                        }
                    });
                }
            }
        }
    });
    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    SharedSocket {
        socket: Rc::new(ws),
        seq: Rc::new(RefCell::new(1)),
    }
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

#[cfg(test)]
mod tests {
    fn keydown_should_submit(key: &str, shift: bool, alt: bool, ctrl: bool, meta: bool) -> bool {
        key == "Enter" && !shift && !alt && !ctrl && !meta
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
    fn transcript_scroll_detects_trailing_mode_near_bottom() {
        assert!(super::scroll_position_is_trailing(950, 100, 1_000));
        assert!(super::scroll_position_is_trailing(852, 100, 1_000));
    }

    #[test]
    fn transcript_scroll_detects_browse_mode_away_from_bottom() {
        assert!(!super::scroll_position_is_trailing(851, 100, 1_000));
        assert!(!super::scroll_position_is_trailing(400, 100, 1_000));
    }
}
