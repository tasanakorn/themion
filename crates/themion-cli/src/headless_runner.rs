use crate::app_runtime::CliAppRuntime;
use crate::format_stats;
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

#[cfg(feature = "stylos")]
#[derive(Serialize)]
struct StylosStateData {
    state: &'static str,
    mode: Option<String>,
    realm: Option<String>,
    instance: Option<String>,
    error: Option<String>,
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

#[cfg(feature = "stylos")]
fn stylos_state_data(state: &crate::stylos::StylosRuntimeState) -> StylosStateData {
    match state {
        crate::stylos::StylosRuntimeState::Off => StylosStateData {
            state: "off",
            mode: None,
            realm: None,
            instance: None,
            error: None,
        },
        crate::stylos::StylosRuntimeState::Active {
            mode,
            realm,
            instance,
        } => StylosStateData {
            state: "active",
            mode: Some(mode.clone()),
            realm: Some(realm.clone()),
            instance: Some(instance.clone()),
            error: None,
        },
        crate::stylos::StylosRuntimeState::Error(err) => StylosStateData {
            state: "error",
            mode: None,
            realm: None,
            instance: None,
            error: Some(err.clone()),
        },
    }
}

pub async fn run(app_runtime: CliAppRuntime) -> anyhow::Result<()> {
    #[cfg(feature = "stylos")]
    let stylos = crate::app_runtime::start_stylos(&app_runtime).await?;

    let project_dir = app_runtime.project_dir.display().to_string();
    emit_event(
        "headless_started",
        StartupData {
            project_dir: &project_dir,
            session_id: app_runtime.session_id.to_string(),
        },
    )?;

    #[cfg(feature = "stylos")]
    emit_event("stylos_state", stylos_state_data(stylos.state()))?;

    tokio::signal::ctrl_c().await?;

    emit_event(
        "headless_stopping",
        ShutdownData {
            reason: "signal=ctrl_c",
        },
    )?;

    #[cfg(feature = "stylos")]
    stylos.shutdown().await;

    emit_event(
        "headless_stopped",
        ShutdownData {
            reason: "signal=ctrl_c",
        },
    )?;

    Ok(())
}

pub async fn run_non_interactive(app_runtime: CliAppRuntime, prompt: String) -> anyhow::Result<()> {
    let mut agent = app_runtime.build_agent()?;
    agent.refresh_model_info().await;
    let (result, stats) = agent.run_loop(&prompt).await?;
    println!("{result}");
    eprintln!("{}", format_stats(&stats));
    Ok(())
}
