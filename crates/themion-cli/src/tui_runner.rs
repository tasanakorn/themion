#[cfg(feature = "stylos")]
use crate::app_runtime::SharedStylosStatusHub;
use crate::app_runtime::{AppRuntimeObserverPublisher, AppSnapshotPublisher};
#[cfg(feature = "stylos")]
use crate::board_runtime::LocalBoardClaimRegistry;
use crate::app_state::{start_tick_loop, AppRuntimeEvent, AppState};
use crate::runtime_domains::{DomainHandle, RuntimeDomains};
use crate::tui::{dispatch_terminal_event, draw, App, AppEvent, FrameRequester};
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste, PushKeyboardEnhancementFlags},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use themion_core::db::DbHandle;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::Session;

type TerminalBackend = CrosstermBackend<std::io::Stdout>;
type TuiTerminal = Terminal<TerminalBackend>;

pub fn start_terminal_input_loop(
    dispatch_event: impl Fn(&crossterm::event::Event) -> bool + Send + Sync + 'static,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    std::thread::Builder::new()
        .name("themion-tui-input".to_string())
        .spawn(move || loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            if crossterm::event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
                match crossterm::event::read() {
                    Ok(event) => {
                        if !dispatch_event(&event) {
                            break;
                        }
                    }
                    Err(err) => {
                        if matches!(err.kind(), std::io::ErrorKind::Interrupted) {
                            break;
                        }
                    }
                }
            }
        })
        .expect("failed to spawn terminal input thread");
}

pub fn create_frame_requester(
    domain: &DomainHandle,
) -> (
    tokio::sync::broadcast::Sender<()>,
    tokio::sync::broadcast::Receiver<()>,
    FrameRequester,
) {
    let (draw_tx, draw_rx) = tokio::sync::broadcast::channel::<()>(8);
    let frame_requester = FrameRequester::new(draw_tx.clone(), domain);
    (draw_tx, draw_rx, frame_requester)
}

struct TerminalGuard {
    terminal: TuiTerminal,
}

impl TerminalGuard {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            crossterm::event::EnableMouseCapture,
            EnableBracketedPaste
        )?;
        let _ = execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(
                crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | crossterm::event::KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | crossterm::event::KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        );
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    fn terminal_mut(&mut self) -> &mut TuiTerminal {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            crossterm::event::DisableMouseCapture,
            DisableBracketedPaste,
            crossterm::event::PopKeyboardEnhancementFlags,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

fn install_panic_cleanup_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            crossterm::event::DisableMouseCapture,
            DisableBracketedPaste,
            crossterm::event::PopKeyboardEnhancementFlags,
            LeaveAlternateScreen
        );
        original_hook(info);
    }));
}

#[cfg(feature = "stylos")]
fn wire_stylos_event_streams(
    runtime_domains: &Arc<RuntimeDomains>,
    handle: &mut crate::stylos::StylosHandle,
    runtime_tx: &mpsc::UnboundedSender<AppRuntimeEvent>,
) {
    let tui_domain = runtime_domains
        .tui()
        .expect("tui runtime available in TUI mode");
    if let Some(mut cmd_rx) = handle.take_cmd_rx() {
        let runtime_tx_cmd = runtime_tx.clone();
        tui_domain.spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                let _ = runtime_tx_cmd.send(AppRuntimeEvent::StylosCmd(cmd));
            }
        });
    }
    if let Some(mut prompt_rx) = handle.take_prompt_rx() {
        let runtime_tx_prompt = runtime_tx.clone();
        tui_domain.spawn(async move {
            while let Some(prompt) = prompt_rx.recv().await {
                let _ = runtime_tx_prompt.send(AppRuntimeEvent::IncomingPrompt(prompt));
            }
        });
    }
    if let Some(mut event_rx) = handle.take_event_rx() {
        let runtime_tx_event = runtime_tx.clone();
        tui_domain.spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let _ = runtime_tx_event.send(AppRuntimeEvent::StylosEvent(event));
            }
        });
    }
}

fn build_app(
    session: Session,
    db: Arc<DbHandle>,
    session_id: Uuid,
    project_dir: PathBuf,
    runtime_domains: &Arc<RuntimeDomains>,
    app_tx: &tokio::sync::mpsc::UnboundedSender<crate::tui::AppEvent>,
    runtime_tx: &tokio::sync::mpsc::UnboundedSender<crate::app_state::AppRuntimeEvent>,
    runtime_observer_publisher: AppRuntimeObserverPublisher,
    initial_snapshot: crate::app_state::AppSnapshot,
    #[cfg(feature = "stylos")] stylos_handle: Option<crate::stylos::StylosHandle>,
    #[cfg(feature = "stylos")] watchdog_state: Arc<crate::app_runtime::WatchdogRuntimeState>,
    #[cfg(feature = "stylos")] shared_status_hub: SharedStylosStatusHub,
) -> App {
    #[cfg(feature = "stylos")]
    let stylos_tool_bridge = stylos_handle.as_ref().and_then(crate::stylos::tool_bridge);
    #[cfg(feature = "stylos")]
    let board_claims = Arc::new(LocalBoardClaimRegistry::default());

    App::new(
        session,
        db,
        session_id,
        project_dir,
        runtime_domains
            .background()
            .expect("background runtime available in TUI mode"),
        runtime_domains.core(),
        app_tx.clone(),
        runtime_tx.clone(),
        #[cfg(feature = "stylos")]
        stylos_handle,
        #[cfg(feature = "stylos")]
        stylos_tool_bridge,
        #[cfg(feature = "stylos")]
        watchdog_state,
        #[cfg(feature = "stylos")]
        board_claims,
        #[cfg(feature = "stylos")]
        shared_status_hub,
        runtime_observer_publisher,
        initial_snapshot,
    )
}


fn start_snapshot_watch_loop(
    runtime_domains: &Arc<RuntimeDomains>,
    snapshot_hub: &crate::app_state::AppSnapshotHub,
    app_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    let tui_domain = runtime_domains
        .tui()
        .expect("tui runtime available in TUI mode");
    let mut snapshot_rx = snapshot_hub.subscribe();
    let app_tx_snapshot = app_tx.clone();
    tui_domain.spawn(async move {
        loop {
            if snapshot_rx.changed().await.is_err() {
                break;
            }
            let snapshot = snapshot_rx.borrow().clone();
            if app_tx_snapshot
                .send(AppEvent::SnapshotUpdated(snapshot))
                .is_err()
            {
                break;
            }
        }
    });
}

fn perform_initial_draw(
    terminal: &mut TuiTerminal,
    app: &mut App,
    frame_requester: &FrameRequester,
) -> anyhow::Result<()> {
    terminal.draw(|f| draw(f, app))?;
    app.finish_initial_draw(frame_requester);
    Ok(())
}

async fn run_event_loop(
    app: &mut App,
    ctx: &mut RunnerContext,
    terminal: &mut TuiTerminal,
) -> anyhow::Result<()> {
    while app.is_running() {
        tokio::select! {
            maybe_draw = ctx.draw_rx.recv() => {
                if maybe_draw.is_ok() {
                    app.handle_draw_event(terminal)?;
                }
            }
            event = ctx.app_rx.recv() => {
                if let Some(event) = event {
                    app.handle_app_event(event, &ctx.frame_requester, &ctx.app_tx, terminal).await;
                }
            }
            runtime_event = ctx.runtime_rx.recv() => {
                if let Some(runtime_event) = runtime_event {
                    crate::app_state::handle_runtime_event(app, runtime_event, &ctx.frame_requester, &ctx.app_tx).await;
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "stylos")]
async fn shutdown_app(app: &mut App, ctx: RunnerContext) {
    ctx.shutdown();
    drop(ctx.app_tx);
    if let Some(stylos) = app.shutdown_stylos() {
        stylos.shutdown().await;
    }
}

#[cfg(not(feature = "stylos"))]
async fn shutdown_app(_app: &mut App, ctx: RunnerContext) {
    ctx.shutdown();
    drop(ctx.app_tx);
}

struct RunnerContext {
    app_tx: mpsc::UnboundedSender<AppEvent>,
    app_rx: mpsc::UnboundedReceiver<AppEvent>,
    runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
    runtime_rx: mpsc::UnboundedReceiver<AppRuntimeEvent>,
    input_shutdown_tx: broadcast::Sender<()>,
    draw_rx: broadcast::Receiver<()>,
    frame_requester: FrameRequester,
}

impl RunnerContext {
    fn build(runtime_domains: &Arc<RuntimeDomains>) -> Self {
        let (app_tx, app_rx) = mpsc::unbounded_channel::<AppEvent>();
        let (runtime_tx, runtime_rx) = mpsc::unbounded_channel::<AppRuntimeEvent>();
        let tui_domain = runtime_domains
            .tui()
            .expect("tui runtime available in TUI mode");
        let (input_shutdown_tx, input_shutdown_rx) = broadcast::channel::<()>(1);
        start_terminal_input_loop(
            {
                let app_tx = app_tx.clone();
                move |event| dispatch_terminal_event(&app_tx, event.clone())
            },
            input_shutdown_rx,
        );
        start_tick_loop(runtime_domains, app_tx.clone(), || AppEvent::Tick);
        let (_draw_tx, draw_rx, frame_requester) = create_frame_requester(&tui_domain);
        Self {
            app_tx,
            app_rx,
            runtime_tx,
            runtime_rx,
            input_shutdown_tx,
            draw_rx,
            frame_requester,
        }
    }

    fn shutdown(&self) {
        let _ = self.input_shutdown_tx.send(());
    }
}

pub async fn run(app_runtime: AppState) -> anyhow::Result<()> {
    let mut terminal = TerminalGuard::enter()?;
    install_panic_cleanup_hook();

    let runtime_domains = app_runtime.runtime_domains.clone();
    let mut ctx = RunnerContext::build(&runtime_domains);

    #[cfg(feature = "stylos")]
    let shared_status_hub = SharedStylosStatusHub::new();
    #[cfg(feature = "stylos")]
    let mut stylos_handle = Some(crate::app_state::start_stylos(&app_runtime, Some(shared_status_hub.clone())).await?);
    #[cfg(feature = "stylos")]
    if let Some(handle) = stylos_handle.as_mut() {
        wire_stylos_event_streams(&runtime_domains, handle, &ctx.runtime_tx);
    }

    let snapshot_hub = app_runtime.snapshot_hub.clone();
    let initial_snapshot = snapshot_hub.current();
    let snapshot_publisher = AppSnapshotPublisher::new(snapshot_hub.clone());
    let runtime_observer_publisher = AppRuntimeObserverPublisher::new(snapshot_publisher);
    start_snapshot_watch_loop(&runtime_domains, &snapshot_hub, &ctx.app_tx);

    #[cfg(feature = "stylos")]
    crate::app_state::start_tui_watchdog_loop(&app_runtime, ctx.runtime_tx.clone());

    let mut app = build_app(
        app_runtime.runtime.session,
        app_runtime.runtime.db,
        app_runtime.runtime.session_id,
        app_runtime.runtime.project_dir,
        &runtime_domains,
        &ctx.app_tx,
        &ctx.runtime_tx,
        runtime_observer_publisher,
        initial_snapshot,
        #[cfg(feature = "stylos")]
        stylos_handle,
        #[cfg(feature = "stylos")]
        app_runtime.runtime.watchdog_state.clone(),
        #[cfg(feature = "stylos")]
        shared_status_hub,
    );

    perform_initial_draw(terminal.terminal_mut(), &mut app, &ctx.frame_requester)?;

    run_event_loop(&mut app, &mut ctx, terminal.terminal_mut()).await?;
    shutdown_app(&mut app, ctx).await;
    Ok(())
}
