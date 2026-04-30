use crate::app_state::AppState;
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

pub fn start_tick_loop<T, F>(
    tui_domain: &DomainHandle,
    app_tx: tokio::sync::mpsc::UnboundedSender<T>,
    mut make_tick: F,
) where
    T: Send + 'static,
    F: FnMut() -> T + Send + 'static,
{
    let tui_domain_for_tick = tui_domain.clone();
    tui_domain_for_tick.spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(150));
        loop {
            interval.tick().await;
            if app_tx.send(make_tick()).is_err() {
                break;
            }
        }
    });
}

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
fn wire_stylos_app(
    app: &mut App,
    runtime_domains: &Arc<RuntimeDomains>,
    app_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    app.refresh_stylos_status();
    app.wire_stylos_event_streams(
        &runtime_domains
            .tui()
            .expect("tui runtime available in TUI mode"),
        app_tx,
    );
}

fn build_app(
    session: Session,
    db: Arc<DbHandle>,
    session_id: Uuid,
    project_dir: PathBuf,
    runtime_domains: &Arc<RuntimeDomains>,
    #[cfg(feature = "stylos")] app_tx: &tokio::sync::mpsc::UnboundedSender<crate::tui::AppEvent>,
    #[cfg(feature = "stylos")] stylos_handle: Option<crate::stylos::StylosHandle>,
) -> App {
    App::new(
        session,
        db,
        session_id,
        project_dir,
        runtime_domains
            .background()
            .expect("background runtime available in TUI mode"),
        runtime_domains.core(),
        #[cfg(feature = "stylos")]
        app_tx.clone(),
        #[cfg(feature = "stylos")]
        stylos_handle,
    )
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
    input_shutdown_tx: broadcast::Sender<()>,
    draw_rx: broadcast::Receiver<()>,
    frame_requester: FrameRequester,
}

impl RunnerContext {
    fn build(runtime_domains: &Arc<RuntimeDomains>) -> Self {
        let (app_tx, app_rx) = mpsc::unbounded_channel::<AppEvent>();
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
        start_tick_loop(&tui_domain, app_tx.clone(), || AppEvent::Tick);
        let (_draw_tx, draw_rx, frame_requester) = create_frame_requester(&tui_domain);
        Self {
            app_tx,
            app_rx,
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
    let stylos_handle = Some(crate::app_state::start_stylos(&app_runtime).await?);

    let mut app = build_app(
        app_runtime.session,
        app_runtime.db,
        app_runtime.session_id,
        app_runtime.project_dir,
        &runtime_domains,
        #[cfg(feature = "stylos")]
        &ctx.app_tx,
        #[cfg(feature = "stylos")]
        stylos_handle,
    );

    #[cfg(feature = "stylos")]
    wire_stylos_app(&mut app, &runtime_domains, &ctx.app_tx);

    perform_initial_draw(terminal.terminal_mut(), &mut app, &ctx.frame_requester)?;

    run_event_loop(&mut app, &mut ctx, terminal.terminal_mut()).await?;
    shutdown_app(&mut app, ctx).await;
    Ok(())
}
