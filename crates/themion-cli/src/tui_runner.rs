use crate::app_runtime::{AppRuntimeObserverPublisher, AppSnapshotPublisher};
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
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};


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


fn build_app(
    runtime: crate::app_state::AppRuntimeState,
    initial_snapshot: crate::app_state::AppSnapshot,
) -> App {
    App::new(runtime, initial_snapshot)
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


#[cfg(feature = "stylos")]
async fn shutdown_app(_app: &mut App, ctx: RunnerContext) {
    ctx.shutdown();
    drop(ctx.app_tx);
}

pub async fn run(mut app_runtime: AppState) -> anyhow::Result<()> {
    let mut terminal = TerminalGuard::enter()?;
    install_panic_cleanup_hook();

    let runtime_domains = app_runtime.runtime_domains.clone();
    let mut ctx = RunnerContext::build(&runtime_domains);

    #[cfg(feature = "stylos")]
    crate::app_state::start_tui_runtime_services(&mut app_runtime, &ctx.runtime_tx).await?;

    let snapshot_hub = app_runtime.snapshot_hub.clone();
    let initial_snapshot = snapshot_hub.current();
    let snapshot_publisher = AppSnapshotPublisher::new(snapshot_hub.clone());
    let runtime_observer_publisher = AppRuntimeObserverPublisher::new(snapshot_publisher);
    start_snapshot_watch_loop(&runtime_domains, &snapshot_hub, &ctx.app_tx);

    #[cfg(feature = "stylos")]
    crate::app_state::start_tui_watchdog_loop(&app_runtime, ctx.runtime_tx.clone());

    crate::app_state::finalize_tui_runtime_state(
        &mut app_runtime.runtime,
        ctx.app_tx.clone(),
        ctx.runtime_tx.clone(),
        runtime_observer_publisher,
    );

    let mut app = build_app(app_runtime.runtime, initial_snapshot);

    perform_initial_draw(terminal.terminal_mut(), &mut app, &ctx.frame_requester)?;

    run_event_loop(&mut app, &mut ctx, terminal.terminal_mut()).await?;
    #[cfg(feature = "stylos")]
    if let Some(stylos) = app.runtime.stylos.take() {
        stylos.shutdown().await;
    }
    shutdown_app(&mut app, ctx).await;
    Ok(())
}
