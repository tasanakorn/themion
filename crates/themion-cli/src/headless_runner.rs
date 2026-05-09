use crate::app_state::AppState;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
struct HeadlessLogEvent<'a, T: Serialize> {
    event: &'a str,
    timestamp_ms: u128,
    data: T,
}

#[derive(Serialize)]
struct StartupData<'a> {
    project_dir: &'a str,
    session_id: String,
}

#[derive(Serialize)]
struct ShutdownData {
    reason: &'static str,
}

#[derive(Serialize)]
struct DummyResultData<'a> {
    status: &'a str,
    note: &'a str,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn emit_event<T: Serialize>(event: &str, data: T) -> anyhow::Result<()> {
    let line = serde_json::to_string(&HeadlessLogEvent {
        event,
        timestamp_ms: now_ms(),
        data,
    })?;
    println!("{line}");
    Ok(())
}

pub async fn run(mut app_runtime: AppState) -> anyhow::Result<()> {
    let (app_tx, _app_rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = tokio::sync::mpsc::unbounded_channel();
    let watchdog_domain = app_runtime.runtime.background_domain();
    crate::app_state::bootstrap_runtime_owner(
        &mut app_runtime,
        app_tx,
        runtime_tx,
        watchdog_domain,
    )
    .await?;

    let project_dir = app_runtime.runtime.project_dir.display().to_string();
    emit_event(
        "headless_started",
        StartupData {
            project_dir: &project_dir,
            session_id: app_runtime.runtime.session_id.to_string(),
        },
    )?;

    tokio::signal::ctrl_c().await?;

    emit_event(
        "headless_stopping",
        ShutdownData {
            reason: "signal=ctrl_c",
        },
    )?;

    #[cfg(feature = "stylos")]
    if let Some(stylos) = app_runtime.runtime.stylos.take() {
        stylos.shutdown().await;
    }

    emit_event(
        "headless_stopped",
        ShutdownData {
            reason: "signal=ctrl_c",
        },
    )?;

    Ok(())
}

pub async fn run_non_interactive(mut app_runtime: AppState, _prompt: String) -> anyhow::Result<()> {
    let (app_tx, _app_rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = tokio::sync::mpsc::unbounded_channel();
    let watchdog_domain = app_runtime.runtime.background_domain();
    crate::app_state::bootstrap_runtime_owner(
        &mut app_runtime,
        app_tx,
        runtime_tx,
        watchdog_domain,
    )
    .await?;

    emit_event(
        "headless_result",
        DummyResultData {
            status: "ok",
            note: "dummy non-interactive JSON output placeholder",
        },
    )?;

    #[cfg(feature = "stylos")]
    if let Some(stylos) = app_runtime.runtime.stylos.take() {
        stylos.shutdown().await;
    }

    Ok(())
}

#[cfg(feature = "semantic-memory")]
pub async fn run_unified_search_index(
    app_runtime: AppState,
    force_full: bool,
    source_kind: Option<String>,
) -> anyhow::Result<()> {
    let db = app_runtime.runtime.db.clone();
    let report = tokio::task::spawn_blocking(move || {
        db.memory_store().rebuild_unified_search_index(
            Some(&app_runtime.runtime.project_dir.display().to_string()),
            source_kind.as_deref(),
            force_full,
        )
    })
    .await
    .map_err(|err| anyhow::anyhow!("unified-search index task failed: {}", err))??;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
