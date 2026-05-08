use crate::app_state::AppRuntimeEvent;
use crate::runtime_domains::{DomainHandle, RuntimeDomains};
use crate::tui::{App, AppEvent, FrameRequester};
use crate::tui_runner::create_frame_requester;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

pub(crate) struct SurfaceRunnerContext {
    pub(crate) domain: DomainHandle,
    pub(crate) app_tx: mpsc::UnboundedSender<AppEvent>,
    pub(crate) app_rx: mpsc::UnboundedReceiver<AppEvent>,
    pub(crate) runtime_tx: mpsc::UnboundedSender<AppRuntimeEvent>,
    pub(crate) runtime_rx: mpsc::UnboundedReceiver<AppRuntimeEvent>,
    pub(crate) draw_rx: broadcast::Receiver<()>,
    pub(crate) frame_requester: FrameRequester,
}

impl SurfaceRunnerContext {
    pub(crate) fn build(runtime_domains: &Arc<RuntimeDomains>) -> Self {
        let (app_tx, app_rx) = mpsc::unbounded_channel::<AppEvent>();
        let (runtime_tx, runtime_rx) = mpsc::unbounded_channel::<AppRuntimeEvent>();
        let domain = runtime_domains
            .tui()
            .unwrap_or_else(|| runtime_domains.core());
        start_tick_loop_on_domain(&domain, app_tx.clone(), || AppEvent::Tick);
        let (_draw_tx, draw_rx, frame_requester) = create_frame_requester(&domain);
        Self {
            domain,
            app_tx,
            app_rx,
            runtime_tx,
            runtime_rx,
            draw_rx,
            frame_requester,
        }
    }
}


pub(crate) fn start_tick_loop_on_domain<T, F>(
    domain: &DomainHandle,
    app_tx: mpsc::UnboundedSender<T>,
    mut make_tick: F,
) where
    T: Send + 'static,
    F: FnMut() -> T + Send + 'static,
{
    let domain = domain.clone();
    domain.spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(150));
        loop {
            interval.tick().await;
            if app_tx.send(make_tick()).is_err() {
                break;
            }
        }
    });
}


pub(crate) fn start_snapshot_watch_loop(
    runtime_domains: &Arc<RuntimeDomains>,
    snapshot_hub: &crate::app_state::AppSnapshotHub,
    app_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    let domain = runtime_domains
        .tui()
        .unwrap_or_else(|| runtime_domains.core());
    let mut snapshot_rx = snapshot_hub.subscribe();
    let app_tx_snapshot = app_tx.clone();
    domain.spawn(async move {
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

pub(crate) async fn handle_surface_runtime_event(
    app: &mut App,
    runtime_event: AppRuntimeEvent,
    ctx: &SurfaceRunnerContext,
) {
    crate::app_state::handle_runtime_event(app, runtime_event, &ctx.frame_requester, &ctx.app_tx)
        .await;
}

pub(crate) async fn handle_surface_app_event(
    app: &mut App,
    event: AppEvent,
    ctx: &SurfaceRunnerContext,
) {
    match event {
        AppEvent::Tick => app.handle_tick_event(&ctx.frame_requester),
        AppEvent::SnapshotUpdated(snapshot) => {
            app.replace_surface_snapshot(snapshot);
            app.mark_dirty_all();
            app.request_draw(&ctx.frame_requester);
        }
        AppEvent::RuntimeCommand(command) => {
            crate::app_state::handle_runtime_command(app, command, &ctx.frame_requester, &ctx.app_tx);
        }
        AppEvent::LoginPrompt {
            user_code,
            verification_uri,
        } => {
            app.handle_login_prompt_event(user_code, verification_uri, &ctx.frame_requester);
        }
        AppEvent::LoginComplete {
            profile_name,
            auth_result,
        } => {
            crate::app_state::handle_login_complete_event(
                app,
                profile_name,
                auth_result,
                &ctx.frame_requester,
            )
            .await;
        }
        AppEvent::LocalAgentManagement(request) => {
            crate::app_state::handle_local_agent_management_request(app, request, &ctx.frame_requester);
        }
        AppEvent::Key(_) | AppEvent::Mouse(_) | AppEvent::Paste(_) => {}
    }
}

