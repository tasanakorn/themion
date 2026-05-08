#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

rustup target add wasm32-unknown-unknown >/dev/null
cargo build -p themion-cli-web-ui --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/debug/themion_cli_web_ui.wasm \
  --out-dir crates/themion-cli/web-assets \
  --target web
cp crates/themion-cli-web-ui/assets/index.html crates/themion-cli/web-assets/index.html
cp crates/themion-cli-web-ui/assets/app.css crates/themion-cli/web-assets/app.css
mkdir -p crates/themion-cli/web-assets/fonts
cp crates/themion-cli-web-ui/assets/fonts/JetBrainsMonoNerdFont-Regular.ttf \
  crates/themion-cli/web-assets/fonts/JetBrainsMonoNerdFont-Regular.ttf

echo "Built themion-cli web assets into crates/themion-cli/web-assets"
