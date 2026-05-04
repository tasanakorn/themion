pub const APP_VERSION: &str = env!("THEMION_APP_VERSION");
pub const APP_VERSION_HASH: &str = env!("THEMION_APP_VERSION_HASH");
pub const APP_VERSION_DIRTY_RAW: &str = env!("THEMION_APP_VERSION_DIRTY");

pub fn app_version_dirty() -> bool {
    APP_VERSION_DIRTY_RAW == "true"
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BuildIdentity {
    pub app_version: String,
    pub app_version_hash: String,
    pub app_version_dirty: bool,
}

impl BuildIdentity {
    pub fn current() -> Self {
        Self {
            app_version: APP_VERSION.to_string(),
            app_version_hash: APP_VERSION_HASH.to_string(),
            app_version_dirty: app_version_dirty(),
        }
    }

    pub fn startup_banner_text(&self) -> String {
        let dirty_suffix = if self.app_version_dirty { " dirty" } else { "" };
        format!(
            "themion v{} ({}{})",
            self.app_version, self.app_version_hash, dirty_suffix
        )
    }
}
