use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use themion_core::CodexAuth;

const LEGACY_AUTH_FILE: &str = "auth.json";
const PROFILE_AUTH_DIR: &str = "auth";

fn themion_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("themion"))
}

fn sanitize_profile_name(profile: &str) -> String {
    let mut out = String::with_capacity(profile.len());
    for ch in profile.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "profile".to_string()
    } else {
        out
    }
}

fn profile_auth_path(profile: &str) -> Option<PathBuf> {
    themion_config_dir().map(|d| {
        d.join(PROFILE_AUTH_DIR)
            .join(format!("{}.json", sanitize_profile_name(profile)))
    })
}

pub fn legacy_auth_path() -> Option<PathBuf> {
    themion_config_dir().map(|d| d.join(LEGACY_AUTH_FILE))
}

fn load_auth_file(path: &Path) -> Result<Option<CodexAuth>> {
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&s)?))
}

fn save_auth_file(path: &Path, auth: &CodexAuth) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine auth directory"))?;
    std::fs::create_dir_all(parent)?;
    let json = serde_json::to_string_pretty(auth)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn load_for_profile(profile: &str) -> Result<Option<CodexAuth>> {
    let path = match profile_auth_path(profile) {
        Some(path) => path,
        None => return Ok(None),
    };
    load_auth_file(&path)
}

pub fn save_for_profile(profile: &str, auth: &CodexAuth) -> Result<()> {
    let path = profile_auth_path(profile).ok_or_else(|| anyhow::anyhow!("cannot determine config dir"))?;
    save_auth_file(&path, auth)
}

pub fn load_legacy() -> Result<Option<CodexAuth>> {
    let path = match legacy_auth_path() {
        Some(path) => path,
        None => return Ok(None),
    };
    load_auth_file(&path)
}

pub fn migrate_legacy_to_profile(profile: &str) -> Result<Option<CodexAuth>> {
    let Some(auth) = load_legacy()? else {
        return Ok(None);
    };
    if load_for_profile(profile)?.is_none() {
        save_for_profile(profile, &auth)
            .with_context(|| format!("saving migrated auth for profile '{}'", profile))?;
    }
    Ok(Some(auth))
}
