const SPA_HTML: &str = include_str!(concat!(env!("OUT_DIR"), "/web-assets/index.html"));
const SPA_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/web-assets/app.css"));
const SPA_CSS_HASH: &str = include_str!(concat!(env!("OUT_DIR"), "/web-assets/app.css.hash"));
const SPA_JS: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/web-assets/themion_cli_web_ui.js"
));
const SPA_JS_HASH: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/web-assets/themion_cli_web_ui.js.hash"
));
const SPA_WASM: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/web-assets/themion_cli_web_ui_bg.wasm"
));
const SPA_WASM_HASH: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/web-assets/themion_cli_web_ui_bg.wasm.hash"
));
const CODEMIRROR_EDITOR_JS: &str =
    include_str!(concat!(env!("OUT_DIR"), "/web-assets/codemirror_editor.js"));
const CODEMIRROR_EDITOR_JS_HASH: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/web-assets/codemirror_editor.js.hash"
));
const JETBRAINS_MONO_NERD_FONT: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/web-assets/fonts/JetBrainsMonoNerdFont-Regular.ttf"
));
const JETBRAINS_MONO_NERD_FONT_HASH: &str =
    include_str!(concat!(env!("OUT_DIR"), "/web-assets/font.hash"));

pub fn spa_html() -> &'static str {
    SPA_HTML
}

pub fn spa_css() -> &'static str {
    SPA_CSS
}

pub fn spa_css_hash() -> &'static str {
    SPA_CSS_HASH.trim()
}

pub fn spa_js() -> &'static str {
    SPA_JS
}

pub fn spa_js_hash() -> &'static str {
    SPA_JS_HASH.trim()
}

pub fn spa_wasm() -> &'static [u8] {
    SPA_WASM
}

pub fn spa_wasm_hash() -> &'static str {
    SPA_WASM_HASH.trim()
}

pub fn codemirror_editor_js() -> &'static str {
    CODEMIRROR_EDITOR_JS
}

pub fn codemirror_editor_js_hash() -> &'static str {
    CODEMIRROR_EDITOR_JS_HASH.trim()
}

pub fn jetbrains_mono_nerd_font() -> &'static [u8] {
    JETBRAINS_MONO_NERD_FONT
}

pub fn jetbrains_mono_nerd_font_hash() -> &'static str {
    JETBRAINS_MONO_NERD_FONT_HASH.trim()
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
    if CODEMIRROR_EDITOR_JS.is_empty() {
        missing.push("web-assets/codemirror_editor.js");
    }
    if JETBRAINS_MONO_NERD_FONT.is_empty() {
        missing.push("web-assets/fonts/JetBrainsMonoNerdFont-Regular.ttf");
    }
    if SPA_CSS_HASH.trim().is_empty() {
        missing.push("web-assets/app.css.hash");
    }
    if SPA_JS_HASH.trim().is_empty() {
        missing.push("web-assets/themion_cli_web_ui.js.hash");
    }
    if SPA_WASM_HASH.trim().is_empty() {
        missing.push("web-assets/themion_cli_web_ui_bg.wasm.hash");
    }
    if CODEMIRROR_EDITOR_JS_HASH.trim().is_empty() {
        missing.push("web-assets/codemirror_editor.js.hash");
    }
    if JETBRAINS_MONO_NERD_FONT_HASH.trim().is_empty() {
        missing.push("web-assets/font.hash");
    }
    missing
}
