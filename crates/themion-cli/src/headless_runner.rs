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

pub async fn run(app_runtime: AppState) -> anyhow::Result<()> {
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

    emit_event(
        "headless_stopped",
        ShutdownData {
            reason: "signal=ctrl_c",
        },
    )?;

    Ok(())
}

pub async fn run_non_interactive(_app_runtime: AppState, _prompt: String) -> anyhow::Result<()> {
    emit_event(
        "headless_result",
        DummyResultData {
            status: "ok",
            note: "dummy non-interactive JSON output placeholder",
        },
    )
}

#[cfg(feature = "semantic-memory")]
pub async fn run_semantic_memory_index(
    app_runtime: AppState,
    force_full: bool,
) -> anyhow::Result<()> {
    let db = app_runtime.runtime.db.clone();
    let report =
        tokio::task::spawn_blocking(move || db.memory_store().index_pending_embeddings(force_full))
            .await
            .map_err(|err| anyhow::anyhow!("semantic index task failed: {}", err))??;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
