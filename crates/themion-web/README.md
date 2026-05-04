# themion-web

`themion-web` is the Phase 1 local web surface for Themion.

## Current scope

This binary currently provides:
- read-only monitoring derived only from the active SQLite database plus local config/auth files
- read-only inspection of the detected config, auth, and database files
- browser PTY terminal access backed by the user's default shell, with `/bin/sh` fallback

This crate does not own Themion runtime truth. In Phase 1 it reads only already-existing externalized state.

## Bind behavior

By default the server binds to:

- `0.0.0.0:8787`

Override with:

- `THEMION_WEB_BIND=<host:port>`

Example:

```bash
THEMION_WEB_BIND=127.0.0.1:9000 cargo run -p themion-web
```

## Asset bundling

The terminal frontend uses pinned vendored `xterm.js` assets stored in:

- `vendor/xterm/xterm.min.js`
- `vendor/xterm/xterm.min.css`

These files are compiled into the binary with `include_str!`, so the running binary serves local embedded copies and does not depend on a CDN.

Pinned asset version:
- `@xterm/xterm` `5.5.0`

Pinned asset SHA-256:
- `xterm.min.js`: `4196e242ef1cf4c2adead8d97f4a772a69576076f70b095e004b4abbb049e7bf`
- `xterm.min.css`: `f7f724aea2bb620a6482bfb8e4bdecfae1152b0c7facef55fbda61f3b6cfedb2`

## Running

```bash
cargo run -p themion-web
```

Then open the reported address in a browser.
