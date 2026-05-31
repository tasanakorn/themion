use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set"));
    let repo_root = manifest_dir.join("../..");

    emit_version_metadata(&repo_root);
    build_web_assets(&repo_root);
}

fn emit_version_metadata(repo_root: &Path) {
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

    match detect_git_metadata(repo_root) {
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

fn build_web_assets(repo_root: &Path) {
    println!("cargo:rerun-if-changed=../../Cargo.toml");
    println!("cargo:rerun-if-changed=../themion-cli-web-ui/Cargo.toml");
    emit_rerun_if_changed_recursive(&repo_root.join("crates/themion-cli-web-ui/src"));
    println!("cargo:rerun-if-changed=../themion-cli-web-ui/package.json");
    println!("cargo:rerun-if-changed=../themion-cli-web-ui/package-lock.json");
    emit_rerun_if_changed_recursive(&repo_root.join("crates/themion-cli-web-ui/js"));
    emit_rerun_if_changed_recursive(&repo_root.join("crates/themion-cli-web-ui/assets"));

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR set"));
    let web_out_dir = out_dir.join("web-assets");
    let wasm_target_dir = out_dir.join("web-ui-target");
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let wasm_profile_dir = if profile == "release" {
        "release"
    } else {
        "debug"
    };

    std::fs::create_dir_all(&web_out_dir).expect("create web asset output dir");
    std::fs::create_dir_all(&wasm_target_dir).expect("create web UI target dir");

    build_codemirror_editor_bundle(repo_root);

    run_command(
        Command::new("rustup")
            .args(["target", "add", "wasm32-unknown-unknown"])
            .current_dir(repo_root),
        "install wasm32-unknown-unknown target",
    );
    let mut cargo_build = Command::new(cargo_exe());
    cargo_build
        .args([
            "build",
            "-p",
            "themion-cli-web-ui",
            "--target",
            "wasm32-unknown-unknown",
            "--target-dir",
        ])
        .arg(&wasm_target_dir)
        .current_dir(repo_root);
    if profile == "release" {
        cargo_build.arg("--release");
    }
    run_command(&mut cargo_build, "build themion-cli-web-ui wasm crate");
    run_command(
        Command::new("wasm-bindgen")
            .arg(wasm_target_dir.join(format!(
                "wasm32-unknown-unknown/{wasm_profile_dir}/themion_cli_web_ui.wasm"
            )))
            .args(["--out-dir"])
            .arg(&web_out_dir)
            .args(["--target", "web"])
            .current_dir(repo_root),
        "generate browser bindings for themion-cli-web-ui",
    );

    let font_out_dir = web_out_dir.join("fonts");
    std::fs::create_dir_all(&font_out_dir).expect("create web font output dir");

    let font_source =
        repo_root.join("crates/themion-cli-web-ui/assets/fonts/JetBrainsMonoNerdFont-Regular.ttf");
    let font_bytes = read_bytes(&font_source);
    let font_hash = content_hash(&font_bytes);
    write_bytes(
        &font_out_dir.join("JetBrainsMonoNerdFont-Regular.ttf"),
        &font_bytes,
    );
    write_bytes(
        &font_out_dir.join(format!("JetBrainsMonoNerdFont-Regular.{font_hash}.ttf")),
        &font_bytes,
    );
    write_text(&web_out_dir.join("font.hash"), &font_hash);

    let css_source = repo_root.join("crates/themion-cli-web-ui/assets/app.css");
    let css = read_text(&css_source).replace(
        "/assets/fonts/JetBrainsMonoNerdFont-Regular.ttf",
        &format!("/assets/fonts/JetBrainsMonoNerdFont-Regular.{font_hash}.ttf"),
    );
    let css_hash = content_hash(css.as_bytes());
    write_text(&web_out_dir.join("app.css"), &css);
    write_text(&web_out_dir.join(format!("app.{css_hash}.css")), &css);
    write_text(&web_out_dir.join("app.css.hash"), &css_hash);

    let codemirror_source = repo_root.join("crates/themion-cli-web-ui/assets/codemirror_editor.js");
    let codemirror_js = read_text(&codemirror_source);
    let codemirror_hash = content_hash(codemirror_js.as_bytes());
    write_text(&web_out_dir.join("codemirror_editor.js"), &codemirror_js);
    write_text(
        &web_out_dir.join(format!("codemirror_editor.{codemirror_hash}.js")),
        &codemirror_js,
    );
    write_text(
        &web_out_dir.join("codemirror_editor.js.hash"),
        &codemirror_hash,
    );

    let wasm_bytes = read_bytes(&web_out_dir.join("themion_cli_web_ui_bg.wasm"));
    let wasm_hash = content_hash(&wasm_bytes);
    write_bytes(
        &web_out_dir.join(format!("themion_cli_web_ui_bg.{wasm_hash}.wasm")),
        &wasm_bytes,
    );
    write_text(
        &web_out_dir.join("themion_cli_web_ui_bg.wasm.hash"),
        &wasm_hash,
    );

    let spa_js = read_text(&web_out_dir.join("themion_cli_web_ui.js")).replace(
        "themion_cli_web_ui_bg.wasm",
        &format!("themion_cli_web_ui_bg.{wasm_hash}.wasm"),
    );
    let spa_js_hash = content_hash(spa_js.as_bytes());
    write_text(&web_out_dir.join("themion_cli_web_ui.js"), &spa_js);
    write_text(
        &web_out_dir.join(format!("themion_cli_web_ui.{spa_js_hash}.js")),
        &spa_js,
    );
    write_text(
        &web_out_dir.join("themion_cli_web_ui.js.hash"),
        &spa_js_hash,
    );

    let index = read_text(&repo_root.join("crates/themion-cli-web-ui/assets/index.html"))
        .replace("/assets/app.css", &format!("/assets/app.{css_hash}.css"))
        .replace(
            "/assets/codemirror_editor.js",
            &format!("/assets/codemirror_editor.{codemirror_hash}.js"),
        )
        .replace(
            "/assets/themion_cli_web_ui.js",
            &format!("/assets/themion_cli_web_ui.{spa_js_hash}.js"),
        );
    write_text(&web_out_dir.join("index.html"), &index);
}

fn build_codemirror_editor_bundle(repo_root: &Path) {
    let web_ui_dir = repo_root.join("crates/themion-cli-web-ui");
    if !web_ui_dir.join("node_modules").exists() {
        run_command(
            Command::new("npm").arg("install").current_dir(&web_ui_dir),
            "install themion-cli-web-ui npm dependencies",
        );
    }
    run_command(
        Command::new("npm")
            .args(["run", "build:editor"])
            .current_dir(&web_ui_dir),
        "build CodeMirror editor bundle",
    );
}

fn emit_rerun_if_changed_recursive(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if child.is_dir() {
            emit_rerun_if_changed_recursive(&child);
        } else {
            println!("cargo:rerun-if-changed={}", child.display());
        }
    }
}

fn cargo_exe() -> PathBuf {
    std::env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

fn run_command(command: &mut Command, description: &str) {
    let output = command
        .output()
        .unwrap_or_else(|e| panic!("failed to {description}: {e}"));
    if output.status.success() {
        return;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    panic!(
        "failed to {description} (status {}):\n{}{}",
        output.status, stdout, stderr
    );
}

fn read_bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", path.display()))
}

fn write_text(path: &Path, text: &str) {
    write_bytes(path, text.as_bytes());
}

fn content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn detect_git_metadata(repo_root: &Path) -> Option<(String, bool)> {
    let hash = git_output(repo_root, &["rev-parse", "--short=7", "HEAD"])
        .and_then(|value| normalize_hash(&value))?;
    let dirty = git_status_dirty(repo_root).unwrap_or(false);
    Some((hash, dirty))
}

fn git_output(repo_root: &Path, args: &[&str]) -> Option<String> {
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

fn git_status_dirty(repo_root: &Path) -> Option<bool> {
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
