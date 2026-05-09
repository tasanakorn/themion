const SPA_HTML: &str = include_str!(concat!(env!("OUT_DIR"), "/web-assets/index.html"));
const SPA_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/web-assets/app.css"));
const SPA_JS: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/web-assets/themion_cli_web_ui.js"
));
const SPA_WASM: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/web-assets/themion_cli_web_ui_bg.wasm"
));
const JETBRAINS_MONO_NERD_FONT: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/web-assets/fonts/JetBrainsMonoNerdFont-Regular.ttf"
));

pub fn spa_html() -> &'static str {
    SPA_HTML
}

pub fn spa_css() -> &'static str {
    SPA_CSS
}

pub fn spa_js() -> &'static str {
    SPA_JS
}

pub fn spa_wasm() -> &'static [u8] {
    SPA_WASM
}

pub fn jetbrains_mono_nerd_font() -> &'static [u8] {
    JETBRAINS_MONO_NERD_FONT
}

pub fn missing_spa_assets() -> Vec<&'static str> {
    let mut missing = Vec::new();
    if SPA_HTML.is_empty() {
        missing.push("web-assets/index.html");
    }
    if SPA_CSS.is_empty() {
        missing.push("web-assets/app.css");
    }
    if SPA_JS.is_empty() {
        missing.push("web-assets/themion_cli_web_ui.js");
    }
    if SPA_WASM.is_empty() {
        missing.push("web-assets/themion_cli_web_ui_bg.wasm");
    }
    if JETBRAINS_MONO_NERD_FONT.is_empty() {
        missing.push("web-assets/fonts/JetBrainsMonoNerdFont-Regular.ttf");
    }
    missing
}
