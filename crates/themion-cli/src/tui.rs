use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, EventStream, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame, Terminal,
};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tui_textarea::TextArea;
use uuid::Uuid;
use crate::config::{Config, ProfileConfig, save_profiles};
use themion_core::agent::{Agent, AgentEvent, TurnStats};
use themion_core::client::ChatClient;
use themion_core::client_codex::CodexClient;
use themion_core::db::DbHandle;
use crate::{Session, format_stats};

// ── App events ────────────────────────────────────────────────────────────────

enum AppEvent {
    Key(event::KeyEvent),
    Mouse(event::MouseEvent),
    Agent(AgentEvent),
    AgentReady(Box<Agent>, Uuid),
    Tick,
    LoginPrompt { user_code: String, verification_uri: String },
    LoginComplete(anyhow::Result<themion_core::CodexAuth>),
}

// ── Chat entries ──────────────────────────────────────────────────────────────

enum Entry {
    User(String),
    Assistant(String),
    Banner(String),
    ToolCall(String),   // detail, e.g. "bash: ls -la"
    ToolDone,
    Stats(String),
    Blank,
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct AgentHandle {
    pub agent: Option<Agent>,
    pub session_id: Uuid,
    pub is_interactive: bool,
}

pub struct App<'a> {
    session: Session,
    entries: Vec<Entry>,
    pending: Option<String>,       // current in-progress status shown below entries
    input: TextArea<'a>,
    running: bool,
    agent_busy: bool,
    scroll_offset: usize,          // lines from bottom (0 = pinned to bottom)
    history: Vec<String>,          // submitted messages, oldest first
    history_pos: Option<usize>,    // None = not navigating; Some(i) = showing history[i]
    history_draft: String,         // input saved before starting history navigation
    streaming_idx: Option<usize>,  // index into entries of the live assistant entry
    anim_frame: u8,
    agents: Vec<AgentHandle>,
    db: Arc<DbHandle>,
    project_dir: PathBuf,
    session_tokens: TurnStats,     // cumulative across all turns
    last_ctx_tokens: u64,          // tokens_in from last API call
}

impl<'a> App<'a> {
    pub fn new(session: Session, db: Arc<DbHandle>, session_id: Uuid, project_dir: PathBuf) -> Self {
        let agent = build_agent(&session, session_id, project_dir.clone(), db.clone())
            .expect("failed to build agent");
        let handle = AgentHandle { agent: Some(agent), session_id, is_interactive: true };

        let art = concat!(
            "█████  █   █  █████  █   █  ███   ███   █   █\n",
            "  █    █   █  █      ██ ██   █   █   █  ██  █\n",
            "  █    █████  ████   █ █ █   █   █   █  █ █ █\n",
            "  █    █   █  █      █   █   █   █   █  █  ██\n",
            "  █    █   █  █████  █   █  ███   ███   █   █",
        );
        let project_display = project_dir.display().to_string();
        let initial_entries = vec![
            Entry::Blank,
            Entry::Banner(art.to_string()),
            Entry::Blank,
            Entry::Assistant(format!(
                "version: {}  |  profile: {}  |  model: {}",
                env!("CARGO_PKG_VERSION"),
                session.active_profile,
                session.model,
            )),
            Entry::Assistant(format!("project directory: {}", project_display)),
            Entry::Assistant("type /config to change settings, /exit to quit".to_string()),
            Entry::Blank,
        ];

        Self {
            session,
            entries: initial_entries,
            pending: None,
            input: make_input(),
            running: true,
            agent_busy: false,
            scroll_offset: 0,
            history: Vec::new(),
            history_pos: None,
            history_draft: String::new(),
            streaming_idx: None,
            anim_frame: 0,
            agents: vec![handle],
            db,
            project_dir,
            session_tokens: TurnStats { llm_rounds: 0, tool_calls: 0, tokens_in: 0, tokens_out: 0, tokens_cached: 0, elapsed_ms: 0 },
            last_ctx_tokens: 0,
        }
    }

    #[allow(dead_code)]
    fn interactive_agent_mut(&mut self) -> Option<&mut AgentHandle> {
        self.agents.iter_mut().find(|h| h.is_interactive)
    }

    fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let new_pos = match self.history_pos {
            None => {
                self.history_draft = self.input.lines().join("\n");
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_pos = Some(new_pos);
        set_input_text(&mut self.input, &self.history[new_pos].clone());
    }

    fn history_down(&mut self) {
        match self.history_pos {
            None => {}
            Some(i) if i + 1 < self.history.len() => {
                self.history_pos = Some(i + 1);
                let text = self.history[i + 1].clone();
                set_input_text(&mut self.input, &text);
            }
            Some(_) => {
                self.history_pos = None;
                let draft = self.history_draft.clone();
                set_input_text(&mut self.input, &draft);
            }
        }
    }

    fn thinking_str(&self) -> String {
        const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let ch = SPINNER[self.anim_frame as usize % SPINNER.len()];
        format!("  {} thinking…", ch)
    }

    fn on_tick(&mut self) {
        self.anim_frame = self.anim_frame.wrapping_add(1);
        if self.agent_busy && self.pending.is_some() {
            self.pending = Some(self.thinking_str());
        }
    }

    fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    fn handle_agent_event(&mut self, ev: AgentEvent) {
        match ev {
            AgentEvent::LlmStart => {
                self.pending = Some(self.thinking_str());
                self.streaming_idx = None;
            }
            AgentEvent::AssistantChunk(chunk) => {
                self.pending = None;
                match self.streaming_idx {
                    Some(i) => {
                        if let Some(Entry::Assistant(ref mut text)) = self.entries.get_mut(i) {
                            text.push_str(&chunk);
                        }
                    }
                    None => {
                        self.push(Entry::Assistant(chunk));
                        self.streaming_idx = Some(self.entries.len() - 1);
                    }
                }
            }
            AgentEvent::AssistantText(text) => {
                self.streaming_idx = None;
                self.pending = None;
                self.push(Entry::Assistant(text));
            }
            AgentEvent::ToolStart { detail } => {
                self.streaming_idx = None;
                self.pending = None;
                self.push(Entry::ToolCall(detail));
            }
            AgentEvent::ToolEnd => {
                self.push(Entry::ToolDone);
                self.pending = Some(self.thinking_str());
            }
            AgentEvent::TurnDone(stats) => {
                self.streaming_idx = None;
                self.pending = None;
                self.push(Entry::Stats(format_stats(&stats)));
                self.push(Entry::Blank);
                self.agent_busy = false;
                self.last_ctx_tokens = stats.tokens_in;
                self.session_tokens.tokens_in += stats.tokens_in;
                self.session_tokens.tokens_out += stats.tokens_out;
                self.session_tokens.tokens_cached += stats.tokens_cached;
                self.session_tokens.llm_rounds += stats.llm_rounds;
                self.session_tokens.tool_calls += stats.tool_calls;
                self.session_tokens.elapsed_ms += stats.elapsed_ms;
            }
        }
    }

    fn handle_command(&mut self, input: &str, app_tx: &mpsc::UnboundedSender<AppEvent>) -> Vec<String> {
        let mut out = Vec::new();

        if input == "/login codex" {
            if self.agent_busy {
                return vec!["busy, please wait".to_string()];
            }
            self.agent_busy = true;
            self.push(Entry::Assistant("logging in to OpenAI Codex…".to_string()));
            let tx = app_tx.clone();
            tokio::spawn(async move {
                match crate::login_codex::start_device_flow().await {
                    Err(e) => { tx.send(AppEvent::LoginComplete(Err(e))).ok(); }
                    Ok((info, poll)) => {
                        tx.send(AppEvent::LoginPrompt {
                            user_code: info.user_code,
                            verification_uri: info.verification_uri,
                        }).ok();
                        let result = poll.await;
                        tx.send(AppEvent::LoginComplete(result)).ok();
                    }
                }
            });
            return out;
        }

        if input == "/config" {
            let key_display = match &self.session.api_key {
                Some(k) if k.len() > 8 => format!("{}…", &k[..8]),
                Some(_) => "(set)".to_string(),
                None => "(none)".to_string(),
            };
            out.push(format!("profile  : {}", self.session.active_profile));
            out.push(format!("provider : {}", self.session.provider));
            out.push(format!("model    : {}", self.session.model));
            out.push(format!("endpoint : {}", self.session.base_url));
            out.push(format!("api_key  : {}", key_display));
            return out;
        }

        if let Some(rest) = input.strip_prefix("/config ") {
            let parts: Vec<&str> = rest.splitn(3, ' ').collect();
            match parts.as_slice() {
                ["profile"] | ["profile", "list"] => {
                    let mut names: Vec<String> = self.session.profiles.keys().cloned().collect();
                    names.sort();
                    for name in names {
                        let marker = if name == self.session.active_profile { "* " } else { "  " };
                        out.push(format!("{}{}", marker, name));
                    }
                }
                ["profile", "show"] => {
                    let key_display = match &self.session.api_key {
                        Some(k) if k.len() > 8 => format!("{}…", &k[..8]),
                        Some(_) => "(set)".to_string(),
                        None => "(none)".to_string(),
                    };
                    out.push(format!("profile  : {}", self.session.active_profile));
                    out.push(format!("provider : {}", self.session.provider));
                    out.push(format!("model    : {}", self.session.model));
                    out.push(format!("endpoint : {}", self.session.base_url));
                    out.push(format!("api_key  : {}", key_display));
                }
                ["profile", "create", name] => {
                    let p = ProfileConfig {
                        provider: Some(self.session.provider.clone()),
                        base_url: Some(self.session.base_url.clone()),
                        model: Some(self.session.model.clone()),
                        api_key: self.session.api_key.clone(),
                    };
                    self.session.profiles.insert(name.to_string(), p);
                    self.session.active_profile = name.to_string();
                    if let Err(e) = save_profiles(&self.session.active_profile, &self.session.profiles) {
                        out.push(format!("warning: {}", e));
                    }
                    out.push(format!("profile '{}' created and saved", name));
                }
                ["profile", "use", name] => {
                    if self.session.switch_profile(name) {
                        if let Err(e) = save_profiles(&self.session.active_profile, &self.session.profiles) {
                            out.push(format!("warning: {}", e));
                        }
                        let new_session_id = Uuid::new_v4();
                        match build_agent(&self.session, new_session_id, self.project_dir.clone(), self.db.clone()) {
                            Ok(new_agent) => {
                                let db = self.db.clone();
                                let pdir = self.project_dir.clone();
                                let _ = db.insert_session(new_session_id, &pdir, true);
                                self.agents = vec![AgentHandle { agent: Some(new_agent), session_id: new_session_id, is_interactive: true }];
                                out.push(format!("switched to profile '{}'  provider={}  model={}", name, self.session.provider, self.session.model));
                            }
                            Err(e) => {
                                out.push(format!("error building agent: {}", e));
                            }
                        }
                    } else {
                        let mut names: Vec<String> = self.session.profiles.keys().cloned().collect();
                        names.sort();
                        out.push(format!("unknown profile '{}'.  available: {}", name, names.join(", ")));
                    }
                }
                ["profile", "set", kv] => {
                    if let Some((key, val)) = kv.split_once('=') {
                        match key {
                            "provider" => self.session.provider = val.to_string(),
                            "model"    => self.session.model    = val.to_string(),
                            "endpoint" => self.session.base_url = val.to_string(),
                            "api_key"  => self.session.api_key  = Some(val.to_string()),
                            _ => { out.push(format!("unknown key '{}'.  valid: provider, model, endpoint, api_key", key)); return out; }
                        }
                        self.session.profiles.insert(self.session.active_profile.clone(), ProfileConfig {
                            provider: Some(self.session.provider.clone()),
                            base_url: Some(self.session.base_url.clone()),
                            model:    Some(self.session.model.clone()),
                            api_key:  self.session.api_key.clone(),
                        });
                        if let Err(e) = save_profiles(&self.session.active_profile, &self.session.profiles) {
                            out.push(format!("warning: {}", e));
                        }
                        out.push(format!("{}={} saved", key, if key == "api_key" { "(set)" } else { val }));
                    } else {
                        out.push("usage: /config profile set key=value".to_string());
                    }
                }
                _ => {
                    out.push("commands:".to_string());
                    out.push("  /config                          show current settings".to_string());
                    out.push("  /config profile [list]           list profiles".to_string());
                    out.push("  /config profile show             show active profile".to_string());
                    out.push("  /config profile create <name>    create from current settings".to_string());
                    out.push("  /config profile use <name>       switch profile".to_string());
                    out.push("  /config profile set key=value    set provider/model/endpoint/api_key".to_string());
                }
            }
            return out;
        }

        out.push(format!("unknown command '{}'.  try /config", input));
        out
    }

    fn scroll_up(&mut self) {
        self.scroll_offset += 3;
    }

    fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(3);
    }

    fn submit_input(&mut self, app_tx: &mpsc::UnboundedSender<AppEvent>) {
        let text: String = self.input.lines().join("\n");
        let text = text.trim().to_string();
        if text.is_empty() || self.agent_busy {
            return;
        }

        self.history.push(text.clone());
        self.history_pos = None;
        self.history_draft = String::new();
        self.input = make_input();
        self.scroll_offset = 0;

        if text == "/exit" || text == "/quit" {
            self.running = false;
            return;
        }

        if text.starts_with('/') {
            let output = self.handle_command(&text, app_tx);
            self.push(Entry::User(text));
            for line in output {
                self.push(Entry::Assistant(line));
            }
            self.push(Entry::Blank);
            return;
        }

        self.push(Entry::User(text.clone()));
        self.agent_busy = true;
        self.pending = Some(self.thinking_str());

        let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();
        let app_tx_relay = app_tx.clone();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(ev) = rx.recv().await {
                let _ = app_tx_relay.send(AppEvent::Agent(ev));
            }
        });

        let handle = self.agents.iter_mut().find(|h| h.is_interactive).expect("interactive agent");
        let mut agent = handle.agent.take().expect("agent available when not busy");
        let handle_session_id = handle.session_id;
        agent.set_event_tx(event_tx);

        let app_tx_done = app_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = agent.run_loop(&text).await {
                let _ = app_tx_done.send(AppEvent::Agent(AgentEvent::AssistantText(
                    format!("error: {e}"),
                )));
            }
            let _ = app_tx_done.send(AppEvent::AgentReady(Box::new(agent), handle_session_id));
        });
    }
}

fn build_agent(session: &Session, session_id: Uuid, project_dir: PathBuf, db: Arc<DbHandle>) -> anyhow::Result<Agent> {
    use themion_core::ChatBackend;
    let client: Box<dyn ChatBackend + Send + Sync> = match session.provider.as_str() {
        "openai-codex" => {
            let auth = crate::auth_store::load()?
                .ok_or_else(|| anyhow::anyhow!("no codex auth; run /login codex first"))?;
            Box::new(CodexClient::new(
                session.base_url.clone(),
                auth,
                Box::new(|a: &themion_core::CodexAuth| crate::auth_store::save(a)),
            ))
        }
        _ => {
            let mut c = ChatClient::new(session.base_url.clone(), session.api_key.clone());
            if session.provider == "openrouter" {
                c = c.with_headers([
                    ("HTTP-Referer".to_string(), "https://github.com/tasanakorn".to_string()),
                    ("X-Title".to_string(), "themion".to_string()),
                    ("X-OpenRouter-Title".to_string(), "themion".to_string()),
                    ("X-OpenRouter-Categories".to_string(), "developer-tools".to_string()),
                ]);
            }
            Box::new(c)
        }
    };
    Ok(Agent::new_with_db(client, session.model.clone(), session.system_prompt.clone(), session_id, project_dir, db))
}

fn set_input_text(input: &mut TextArea, text: &str) {
    *input = make_input();
    if !text.is_empty() {
        input.insert_str(text);
    }
}

fn make_input<'a>() -> TextArea<'a> {
    let mut ta = TextArea::default();
    ta.set_block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled("▸ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
            .padding(Padding::left(2)),
    );
    ta.set_cursor_line_style(Style::default());
    ta.set_placeholder_text("message…  (Enter send | Ctrl-C quit)");
    ta
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn build_lines<'a>(entries: &'a [Entry], pending: &'a Option<String>) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();

    for entry in entries {
        match entry {
            Entry::User(text) => {
                lines.push(Line::default());
                for (i, part) in text.lines().enumerate() {
                    let prefix = if i == 0 {
                        Span::styled("▸ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                    } else {
                        Span::raw("  ")
                    };
                    lines.push(Line::from(vec![prefix, Span::styled(part.to_string(), Style::default().add_modifier(Modifier::BOLD))]));
                }
            }
            Entry::Assistant(text) => {
                for part in text.lines() {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::raw(part.to_string()),
                    ]));
                }
            }
            Entry::Banner(text) => {
                for part in text.lines() {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {}", part),
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                        ),
                    ]));
                }
            }
            Entry::ToolCall(detail) => {
                lines.push(Line::from(vec![
                    Span::styled(format!("  ↳ {}", detail), Style::default().fg(Color::Yellow)),
                ]));
            }
            Entry::ToolDone => {
                // merge with previous ToolCall line visually — just show checkmark
                if let Some(last) = lines.last_mut() {
                    let mut spans = last.spans.clone();
                    spans.push(Span::styled("  ✓", Style::default().fg(Color::Green)));
                    *last = Line::from(spans);
                }
            }
            Entry::Stats(s) => {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}", s), Style::default().fg(Color::DarkGray)),
                ]));
            }
            Entry::Blank => {
                lines.push(Line::default());
            }
        }
    }

    if let Some(p) = pending {
        lines.push(Line::from(vec![
            Span::styled(p.as_str(), Style::default().fg(Color::Yellow)),
        ]));
    }

    lines
}

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),      // conversation
            Constraint::Length(3),   // input
            Constraint::Length(1),   // status bar
        ])
        .split(area);

    // ── Conversation ─────────────────────────────────────────────────────────
    let lines = build_lines(&app.entries, &app.pending);
    let height = chunks[0].height as usize;
    let width = chunks[0].width;

    // Use ratatui's own line_count() to get the exact visual row count after
    // word-wrap, then compute a scroll that pins the newest content to the bottom.
    let conv_base = Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(Block::default());
    let total_visual = conv_base.line_count(width);
    let max_scroll = total_visual.saturating_sub(height);
    let scroll = max_scroll.saturating_sub(app.scroll_offset) as u16;

    f.render_widget(conv_base.scroll((scroll, 0)), chunks[0]);

    // ── Input ─────────────────────────────────────────────────────────────────
    f.render_widget(&app.input, chunks[1]);

    // ── Status bar ───────────────────────────────────────────────────────────
    let project_leaf = app.project_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?");
    let fmt = |n: u64| -> String {
        let s = n.to_string();
        let mut out = String::new();
        for (i, ch) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 { out.push(','); }
            out.push(ch);
        }
        out.chars().rev().collect()
    };
    let bar = format!(
        "  {}  |  {}  |  {}  |  in:{} out:{} cached:{}  |  ctx:{}",
        project_leaf,
        app.session.active_profile,
        app.session.model,
        fmt(app.session_tokens.tokens_in),
        fmt(app.session_tokens.tokens_out),
        fmt(app.session_tokens.tokens_cached),
        fmt(app.last_ctx_tokens),
    );
    f.render_widget(
        Paragraph::new(bar).style(Style::default().bg(Color::DarkGray).fg(Color::White)),
        chunks[2],
    );
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(cfg: Config, dir_override: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), crossterm::event::DisableMouseCapture, LeaveAlternateScreen);
        original_hook(info);
    }));

    // Resolve project_dir
    let project_dir = dir_override
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Open DB
    let db = match dirs::data_dir() {
        Some(d) => {
            let db_path = d.join("themion").join("history.db");
            DbHandle::open(&db_path).unwrap_or_else(|e| {
                eprintln!("warning: history persistence disabled: {}", e);
                DbHandle::open_in_memory().expect("in-memory db")
            })
        }
        None => {
            eprintln!("warning: history persistence disabled (no data dir)");
            DbHandle::open_in_memory().expect("in-memory db")
        }
    };

    // Create session
    let session_id = Uuid::new_v4();
    let _ = db.insert_session(session_id, &project_dir, true);

    let session = Session::from_config(cfg);
    let (app_tx, mut app_rx) = mpsc::unbounded_channel::<AppEvent>();

    let app_tx_input = app_tx.clone();
    tokio::spawn(async move {
        let mut stream = EventStream::new();
        while let Some(Ok(ev)) = stream.next().await {
            match ev {
                Event::Key(key) => { let _ = app_tx_input.send(AppEvent::Key(key)); }
                Event::Mouse(m) => { let _ = app_tx_input.send(AppEvent::Mouse(m)); }
                _ => {}
            }
        }
    });

    let app_tx_tick = app_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(150));
        loop {
            interval.tick().await;
            if app_tx_tick.send(AppEvent::Tick).is_err() { break; }
        }
    });

    let mut app = App::new(session, db, session_id, project_dir);

    while app.running {
        terminal.draw(|f| draw(f, &app))?;
        match app_rx.recv().await {
            Some(AppEvent::Mouse(m)) => match m.kind {
                MouseEventKind::ScrollUp => app.scroll_up(),
                MouseEventKind::ScrollDown => app.scroll_down(),
                _ => {}
            },
            Some(AppEvent::Key(key)) => match (key.code, key.modifiers) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => app.running = false,
                (KeyCode::Enter, KeyModifiers::NONE) => {
                    let tx = app_tx.clone();
                    app.submit_input(&tx);
                }
                (KeyCode::PageUp, _) | (KeyCode::Up, KeyModifiers::ALT) => app.scroll_up(),
                (KeyCode::PageDown, _) | (KeyCode::Down, KeyModifiers::ALT) => app.scroll_down(),
                (KeyCode::Up, KeyModifiers::NONE) => app.history_up(),
                (KeyCode::Down, KeyModifiers::NONE) => app.history_down(),
                _ => { app.input.input(key); }
            },
            Some(AppEvent::Tick) => app.on_tick(),
            Some(AppEvent::Agent(ev)) => app.handle_agent_event(ev),
            Some(AppEvent::AgentReady(agent, sid)) => {
                if let Some(h) = app.agents.iter_mut().find(|h| h.session_id == sid) {
                    h.agent = Some(*agent);
                }
                // Clear busy in case TurnDone was never emitted (e.g. error path)
                app.agent_busy = false;
            }
            Some(AppEvent::LoginPrompt { user_code, verification_uri }) => {
                app.push(Entry::Assistant(format!("open {} and enter code {}", verification_uri, user_code)));
            }
            Some(AppEvent::LoginComplete(Ok(auth))) => {
                if let Err(e) = crate::auth_store::save(&auth) {
                    app.push(Entry::Assistant(format!("warning: failed to save auth: {}", e)));
                }
                // Upsert codex profile
                use crate::config::ProfileConfig;
                app.session.profiles.entry("codex".to_string()).or_insert_with(|| ProfileConfig {
                    provider: Some("openai-codex".to_string()),
                    model: Some("gpt-5.4".to_string()),
                    base_url: None,
                    api_key: None,
                });
                app.session.switch_profile("codex");
                if let Err(e) = save_profiles(&app.session.active_profile, &app.session.profiles) {
                    app.push(Entry::Assistant(format!("warning: failed to save config: {}", e)));
                }
                let new_session_id = Uuid::new_v4();
                match build_agent(&app.session, new_session_id, app.project_dir.clone(), app.db.clone()) {
                    Ok(new_agent) => {
                        let _ = app.db.insert_session(new_session_id, &app.project_dir, true);
                        app.agents = vec![AgentHandle { agent: Some(new_agent), session_id: new_session_id, is_interactive: true }];
                        app.push(Entry::Assistant(format!("logged in as {} — switched to codex profile (gpt-5.4)", auth.account_id)));
                    }
                    Err(e) => {
                        app.push(Entry::Assistant(format!("login succeeded but agent build failed: {}", e)));
                    }
                }
                app.agent_busy = false;
            }
            Some(AppEvent::LoginComplete(Err(e))) => {
                app.push(Entry::Assistant(format!("login failed: {}", e)));
                app.agent_busy = false;
            }
            None => {}
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), crossterm::event::DisableMouseCapture, LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
