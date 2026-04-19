#![cfg(feature = "stylos")]

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use stylos_config::{Endpoints, IdentitySection, StylosConfig as SessionConfig, ZenohSection};
use stylos_session::SessionOverrides;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use zenoh::bytes::Encoding;
use zenoh::qos::CongestionControl;

use crate::Session;
use crate::config::StylosConfig;

#[derive(Clone, Debug)]
pub enum StylosRuntimeState {
    Off,
    Active { mode: String, realm: String, instance: String },
    Error(String),
}

pub struct StylosHandle {
    state: StylosRuntimeState,
    session: Option<Arc<zenoh::Session>>,
    heartbeat_task: Option<JoinHandle<()>>,
    queryable_task: Option<JoinHandle<()>>,
}

impl StylosHandle {
    pub fn off() -> Self {
        Self {
            state: StylosRuntimeState::Off,
            session: None,
            heartbeat_task: None,
            queryable_task: None,
        }
    }

    pub fn state(&self) -> &StylosRuntimeState {
        &self.state
    }

    pub async fn shutdown(self) {
        if let Some(task) = self.heartbeat_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.queryable_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(session) = self.session {
            let _ = session.close().await;
        }
    }
}

#[derive(Serialize)]
struct ThemionInfo {
    version: String,
    instance: String,
    realm: String,
    mode: String,
    profile: String,
    model: String,
}

pub async fn start(settings: &StylosConfig, session: &Session) -> StylosHandle {
    if !settings.enabled() {
        return StylosHandle::off();
    }

    match start_inner(settings, session).await {
        Ok(handle) => handle,
        Err(err) => StylosHandle {
            state: StylosRuntimeState::Error(err),
            session: None,
            heartbeat_task: None,
            queryable_task: None,
        },
    }
}

async fn start_inner(settings: &StylosConfig, session: &Session) -> Result<StylosHandle, String> {
    let instance = derive_instance(settings.instance.as_deref()).unwrap_or_else(|| "themion".to_string());
    let realm = settings.realm();
    let mode = settings.mode();

    let cfg = SessionConfig {
        stylos: IdentitySection {
            realm: realm.clone(),
            role: "themion".to_string(),
            instance: instance.clone(),
        },
        zenoh: ZenohSection {
            mode: mode.clone(),
            connect: Endpoints {
                endpoints: settings.connect.clone(),
            },
            listen: Endpoints::default(),
            scouting: None,
        },
    };

    let overrides = SessionOverrides {
        connect: if settings.connect.is_empty() {
            None
        } else {
            Some(settings.connect.clone())
        },
    };

    let session_handle = Arc::new(
        stylos_session::open_session(&cfg, &overrides)
            .await
            .map_err(|e| e.to_string())?,
    );

    let ct = CancellationToken::new();
    let hb_ct = ct.clone();
    let hb_session = session_handle.clone();
    let hb_key = format!("stylos/{}/themion/{}/heartbeat", realm, instance);
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = hb_ct.cancelled() => break,
                _ = interval.tick() => {
                    let _ = hb_session
                        .put(&hb_key, b"alive".to_vec())
                        .encoding(Encoding::APPLICATION_OCTET_STREAM)
                        .congestion_control(CongestionControl::Drop)
                        .await;
                }
            }
        }
    });

    let q_ct = ct.clone();
    let q_session = session_handle.clone();
    let q_key = format!("stylos/{}/themion/{}/info", realm, instance);
    let info = ThemionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        instance: instance.clone(),
        realm: realm.clone(),
        mode: mode.clone(),
        profile: session.active_profile.clone(),
        model: session.model.clone(),
    };
    let queryable_task = tokio::spawn(async move {
        let queryable = match q_session.declare_queryable(&q_key).await {
            Ok(q) => q,
            Err(_) => return,
        };
        let payload = serde_json::to_vec(&info).unwrap_or_default();
        loop {
            tokio::select! {
                _ = q_ct.cancelled() => break,
                res = queryable.recv_async() => match res {
                    Ok(query) => {
                        let _ = query
                            .reply(q_key.clone(), payload.clone())
                            .encoding(Encoding::APPLICATION_JSON)
                            .await;
                    }
                    Err(_) => break,
                }
            }
        }
    });

    Ok(StylosHandle {
        state: StylosRuntimeState::Active { mode, realm, instance },
        session: Some(session_handle),
        heartbeat_task: Some(heartbeat_task),
        queryable_task: Some(queryable_task),
    })
}

fn derive_instance(override_id: Option<&str>) -> Option<String> {
    if let Some(value) = override_id {
        let value = value.trim();
        if is_valid_segment(value) {
            return Some(value.to_string());
        }
    }

    let hostname = hostname::get().ok()?.to_string_lossy().to_lowercase();
    let mapped: String = hostname
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = mapped.trim_matches('-').to_string();
    let capped: String = trimmed.chars().take(32).collect();
    if capped.is_empty() || !is_valid_segment(&capped) {
        None
    } else {
        Some(capped)
    }
}

fn is_valid_segment(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}
