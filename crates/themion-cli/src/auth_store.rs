use themion_core::CodexAuth;
use std::path::PathBuf;
use anyhow::Result;

pub fn auth_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("themion").join("auth.json"))
}

pub fn load() -> Result<Option<CodexAuth>> {
    let path = match auth_path() { Some(p) => p, None => return Ok(None) };
    if !path.exists() { return Ok(None); }
    let s = std::fs::read_to_string(&path)?;
    Ok(Some(serde_json::from_str(&s)?))
}

pub fn save(auth: &CodexAuth) -> Result<()> {
    let path = auth_path().ok_or_else(|| anyhow::anyhow!("cannot determine config dir"))?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let json = serde_json::to_string_pretty(auth)?;
    // atomic write
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)?;
    // chmod 0600 on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
