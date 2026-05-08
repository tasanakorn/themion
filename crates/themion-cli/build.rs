use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=THEMION_APP_VERSION_HASH");
    println!("cargo:rerun-if-env-changed=THEMION_APP_VERSION_DIRTY");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-changed=../../.git/refs");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");

    let app_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=THEMION_APP_VERSION={app_version}");

    if let Ok(override_hash) = std::env::var("THEMION_APP_VERSION_HASH") {
        let override_hash = normalize_hash(&override_hash).unwrap_or_else(|| "unknown".to_string());
        println!("cargo:rustc-env=THEMION_APP_VERSION_HASH={override_hash}");
        let dirty = std::env::var("THEMION_APP_VERSION_DIRTY")
            .ok()
            .as_deref()
            .map(parse_dirty)
            .unwrap_or(false);
        println!(
            "cargo:rustc-env=THEMION_APP_VERSION_DIRTY={}",
            if dirty { "true" } else { "false" }
        );
        return;
    }

    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    match detect_git_metadata(&repo_root) {
        Some((hash, dirty)) => {
            println!("cargo:rustc-env=THEMION_APP_VERSION_HASH={hash}");
            println!(
                "cargo:rustc-env=THEMION_APP_VERSION_DIRTY={}",
                if dirty { "true" } else { "false" }
            );
        }
        None => {
            println!("cargo:rustc-env=THEMION_APP_VERSION_HASH=unknown");
            println!("cargo:rustc-env=THEMION_APP_VERSION_DIRTY=false");
        }
    }
}

fn detect_git_metadata(repo_root: &std::path::Path) -> Option<(String, bool)> {
    let hash = git_output(repo_root, &["rev-parse", "--short=7", "HEAD"])
        .and_then(|value| normalize_hash(&value))?;
    let dirty = git_status_dirty(repo_root).unwrap_or(false);
    Some((hash, dirty))
}

fn git_output(repo_root: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn git_status_dirty(repo_root: &std::path::Path) -> Option<bool> {
    let status = Command::new("git")
        .args(["diff", "--quiet", "--ignore-submodules", "HEAD", "--"])
        .current_dir(repo_root)
        .status()
        .ok()?;
    Some(!status.success())
}

fn normalize_hash(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn parse_dirty(value: &str) -> bool {
    matches!(
        value.trim(),
        "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes"
    )
}
