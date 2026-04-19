//! Stylos config schema + JSON5 loader.
//!
//! Shape: top-level { stylos: { realm, role, instance }, zenoh: { ... } }.
//! The zenoh block uses UPSTREAM zenoh 1.x field names (listen_private_key etc).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use stylos_common::{Result, StylosError, STYLOS_MULTICAST_ADDR};
use stylos_identity::StylosIdentity;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StylosConfig {
    #[serde(default)]
    pub stylos: IdentitySection,
    #[serde(default)]
    pub zenoh: ZenohSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentitySection {
    pub realm: String,
    pub role: String,
    pub instance: String,
}

impl Default for IdentitySection {
    fn default() -> Self {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
        Self {
            realm: "dev".to_string(),
            role: "cli".to_string(),
            instance: format!("cli-{:x}", (ts as u64) & 0xffffff),
        }
    }
}

impl IdentitySection {
    pub fn to_identity(&self) -> Result<StylosIdentity> {
        StylosIdentity::new(&self.realm, &self.role, &self.instance)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZenohSection {
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub connect: Endpoints,
    #[serde(default)]
    pub listen: Endpoints,
    #[serde(default)]
    pub scouting: Option<ScoutingSection>,
}

impl Default for ZenohSection {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            connect: Endpoints::default(),
            listen: Endpoints::default(),
            scouting: None,
        }
    }
}

fn default_mode() -> String { "peer".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Endpoints {
    #[serde(default)]
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoutingSection {
    #[serde(default)]
    pub multicast: Option<MulticastSection>,
    #[serde(default)]
    pub gossip: Option<GossipSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MulticastSection {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_mcast_addr")]
    pub address: String,
    #[serde(default = "default_iface")]
    pub interface: String,
    #[serde(default)]
    pub autoconnect: Option<Autoconnect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Autoconnect {
    #[serde(default = "default_peer_peer")]
    pub peer: String,
    #[serde(default = "default_peer_peer")]
    pub router: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipSection {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }
fn default_mcast_addr() -> String { STYLOS_MULTICAST_ADDR.to_string() }
fn default_iface() -> String { "auto".to_string() }
fn default_peer_peer() -> String { "peer".to_string() }

impl StylosConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let txt = std::fs::read_to_string(path)
            .map_err(|e| StylosError::Config(format!("read {}: {e}", path.display())))?;
        let cfg: Self = json5::from_str(&txt)?;
        Ok(cfg)
    }

    pub fn load_default() -> Result<Self> {
        if let Ok(p) = std::env::var("STYLOS_CONFIG") {
            let pb = PathBuf::from(p);
            if pb.exists() { return Self::load(&pb); }
        }
        let local = PathBuf::from("./stylos.json5");
        if local.exists() { return Self::load(&local); }
        Ok(Self::default())
    }
}
