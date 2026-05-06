# themion-web

Local browser surface for Themion hosted by a dedicated `themion-web` binary.

## Current scope

This crate currently provides:
- a self-contained Axum server with SSR-rendered Leptos UI
- a separate `themion-web` executable
- a mixed sidebar navigation model with standalone items plus grouped sections such as `Knowledge` → `Stats` and `Knowledge` → `Query`
- a read-only Project Memory summary page sourced directly from the active SQLite `system.db` file
- a browser query page backed by shared `themion-core` `unified_search` execution using direct-linkable URL state
- browser shell sessions backed by `portable-pty`
- bundled `xterm.js` terminal rendering over websocket transport
- persistent in-process terminal sessions with reconnect and multi-tab support
- embedded `JetBrains Mono Nerd Font` as the default terminal font

This crate does not currently provide:
- general SQLite browsing beyond the summary page
- config/auth-file inspection
- browser-native agent chat or control flows

Those deferred capabilities should land in follow-up PRDs instead of being implied by the current implementation.

## Run

```bash
cargo run -p themion-web
```

Or use a custom bind:

```bash
THEMION_WEB_BIND=127.0.0.1:8877 cargo run -p themion-web
```

## Notes

- Default bind is controlled by `THEMION_WEB_BIND` and otherwise falls back to `0.0.0.0:8787`.
- Terminal sessions run the local default shell from the server process environment.
- Terminal rendering uses bundled `xterm.js` assets served by the Rust binary.
