# themion-web

Fresh blank Rust/UI app for Themion.

## Scope

This crate has been reset to a minimal runnable Rust/UI-based app shell.

It currently provides:
- a simple Axum server
- SSR-rendered Leptos markup
- a minimal Rust/UI card, badge, and button demo
- embedded Tailwind-compatible Rust/UI styling from `style/tailwind.css`

## Run

```bash
cargo run -p themion-web
```

Or use a custom bind:

```bash
THEMION_WEB_BIND=127.0.0.1:8877 cargo run -p themion-web
```

## Rust/UI model

This crate uses the real Rust/UI model:
- Rust/UI ecosystem crates such as `leptos_ui`, `tw_merge`, and `icons`
- copied local component source under `src/components/ui/`
- Leptos metadata with `tailwind-input-file = "style/tailwind.css"`
