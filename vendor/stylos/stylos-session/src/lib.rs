//! zenoh::Session factory.

use stylos_common::{Result, StylosError, STYLOS_DEFAULT_DATA_PORT, STYLOS_PORT_WALK_CAP};
use stylos_config::StylosConfig;
use stylos_transport::{listen_endpoints, walk_available_port};
use zenoh::Config;

#[derive(Debug, Clone, Default)]
pub struct SessionOverrides {
    pub connect: Option<Vec<String>>,
}

/// JSON5-quote a string.
fn jq(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn jq_arr(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| jq(s)).collect();
    format!("[{}]", parts.join(","))
}

/// Apply an optional JSON5 config mutation; log a warning on failure so
/// silently-broken scouting is visible.
fn soft_set(config: &mut Config, path: &str, value: &str) {
    if let Err(e) = config.insert_json5(path, value) {
        eprintln!("[stylos] warn: insert_json5({path}, {value}) failed: {e:?}");
    }
}

pub async fn open_session(
    cfg: &StylosConfig,
    overrides: &SessionOverrides,
) -> Result<zenoh::Session> {
    let _identity = cfg.stylos.to_identity()?;

    let mut config = Config::default();

    // Mode.
    config
        .insert_json5("mode", &jq(&cfg.zenoh.mode))
        .map_err(|e| StylosError::Config(format!("set mode: {e:?}")))?;

    // Listen endpoints.
    let listen = if cfg.zenoh.listen.endpoints.is_empty() {
        let port = walk_available_port(STYLOS_DEFAULT_DATA_PORT, STYLOS_PORT_WALK_CAP)?;
        listen_endpoints(port)
    } else {
        cfg.zenoh.listen.endpoints.clone()
    };
    config
        .insert_json5("listen/endpoints", &jq_arr(&listen))
        .map_err(|e| StylosError::Transport(format!("set listen: {e:?}")))?;

    // Connect endpoints.
    let connect: Vec<String> = match &overrides.connect {
        Some(v) if !v.is_empty() => v.clone(),
        _ => cfg.zenoh.connect.endpoints.clone(),
    };
    config
        .insert_json5("connect/endpoints", &jq_arr(&connect))
        .map_err(|e| StylosError::Transport(format!("set connect: {e:?}")))?;

    // Scouting.
    if let Some(sc) = &cfg.zenoh.scouting {
        if let Some(mc) = &sc.multicast {
            soft_set(
                &mut config,
                "scouting/multicast/enabled",
                &mc.enabled.to_string(),
            );
            soft_set(&mut config, "scouting/multicast/address", &jq(&mc.address));
            soft_set(
                &mut config,
                "scouting/multicast/interface",
                &jq(&mc.interface),
            );
            if let Some(ac) = &mc.autoconnect {
                soft_set(
                    &mut config,
                    "scouting/multicast/autoconnect/peer",
                    &jq(&ac.peer),
                );
                soft_set(
                    &mut config,
                    "scouting/multicast/autoconnect/router",
                    &jq(&ac.router),
                );
            }
        }
        if let Some(g) = &sc.gossip {
            soft_set(
                &mut config,
                "scouting/gossip/enabled",
                &g.enabled.to_string(),
            );
        }
    } else {
        soft_set(&mut config, "scouting/multicast/enabled", "true");
        soft_set(
            &mut config,
            "scouting/multicast/address",
            &jq(stylos_common::STYLOS_MULTICAST_ADDR),
        );
        soft_set(&mut config, "scouting/gossip/enabled", "true");
    }

    let session = zenoh::open(config)
        .await
        .map_err(|e| StylosError::Zenoh(e.to_string()))?;
    Ok(session)
}

pub async fn log_session_info(session: &zenoh::Session) {
    let info = session.info();
    let zid = info.zid().await;
    let peers: Vec<_> = info.peers_zid().await.collect();
    let routers: Vec<_> = info.routers_zid().await.collect();
    eprintln!(
        "stylos session zid={zid} peers={} routers={}",
        peers.len(),
        routers.len()
    );
}
