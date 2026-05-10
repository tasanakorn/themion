# PRD-117: Web UI WebSocket Auto-Reconnect

- **Status:** Implemented
- **Version:** v0.72.1
- **Scope:** `themion-cli`, `themion-cli-web-ui`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-09

## Summary

- The Web UI currently opens one websocket once and stays disconnected after close or error.
- Add automatic reconnect for the shared `/api/ws` browser connection.
- After reconnect, restore runtime, agent, and terminal subscriptions from browser view state.
- Recover current state from runtime-owned snapshots instead of inventing browser-only truth.
- Keep prompt submission honest while reconnect is pending.

## Goals

- Make `themion-cli --web` recover automatically from temporary websocket drops without requiring a page refresh.
- Restore live runtime status, transcript refreshes, agent activity, and terminal-list updates after reconnect.
- Re-subscribe automatically to the streams the browser was already using before disconnect.
- Keep runtime truth in `themion-cli`; the browser should only restore subscriptions and fetch fresh snapshots.
- Show clear connection state in the browser while reconnect is pending.
- Keep one shared websocket connection at `/api/ws` as required by PRD-106.

## Non-goals

- Do not add a second websocket connection type or split agent and terminal traffic into separate sockets.
- Do not guarantee delivery of every event that happened while the browser was disconnected.
- Do not add offline queueing or automatic replay for prompt submissions in this first slice.
- Do not move runtime policy, roster truth, or subscription ownership into the browser.
- Do not redesign the Web UI layout beyond the small connection-state feedback needed for reconnect.
- Do not revive `crates/themion-web` as an implementation target.

## Background & Motivation

### Current state

PRD-106 established one shared websocket at `/api/ws` for browser realtime traffic under `themion-cli --web`. The current Web UI creates that socket once in `crates/themion-cli-web-ui/src/lib.rs` and only updates local status text when it opens, closes, or errors.

The browser does not retry, recreate the socket, or re-send subscriptions after disconnect. If the socket drops because the server restarts, the browser network changes, or the tab wakes from sleep, live updates stop until the user refreshes the page.

### Why this matters now

The Web UI already depends on the websocket for runtime snapshots, agent events, and terminal traffic. Without reconnect, the surface is fragile during ordinary local use.

The user should not need to reload the page for normal transient failures. The browser should reconnect, fetch fresh runtime-owned state, and continue.

## Design

### 1. Reconnect the shared websocket automatically

The Web UI must recreate the shared websocket when the current connection closes or fails.

Required behavior:

- treat `close` as a reconnect trigger unless the page is intentionally shutting down
- treat `error` as a reconnect trigger if the socket does not recover normally
- keep reconnect logic in one Web UI transport controller rather than spreading it across transcript, status, and terminal widgets
- replace the old socket instance cleanly so old handlers do not keep mutating state after a newer socket exists
- use a simple bounded backoff for retries

Required default retry policy for this slice:

- first retry: immediate
- later retries: increase delay gradually
- maximum delay cap: 5 seconds
- keep retrying until the page unloads or the app intentionally disables reconnect

Visible browser states should include at least:

- `connecting`
- `open`
- `reconnecting`
- `closed` only when reconnect has stopped permanently or the page is shutting down

### 2. Restore subscriptions from browser view state

Reconnect is only useful if the browser returns to the same streams.

Required behavior:

- the browser must remember the subscriptions it currently wants, including:
  - runtime `status`
  - the currently selected agent stream
  - terminal `list`
  - any attached terminal session ids that still matter to the current page
- after the new socket opens, resend subscribe messages for those targets automatically
- the remembered subscription set is browser view state, not a second source of truth for runtime state
- if the selected agent changes while reconnect is pending, the post-reconnect subscribe set must match the newest selected agent, not stale prior state
- duplicate subscribe messages must be safe on reconnect

The first slice only needs to restore the streams the current UI already uses.

### 3. Refresh runtime-owned snapshots after reconnect

The browser must recover current truth from the server after reconnect.

Required behavior:

- after reconnect opens, fetch fresh `/api/status`, `/api/agents`, and `/api/transcript` snapshots unless equivalent websocket snapshots already arrive immediately
- replace stale browser state with the fresh server payloads
- do not try to reconstruct missed runtime or transcript events locally
- terminal session attach/list recovery should use the server's current view, not optimistic local assumptions
- if a terminal session no longer exists after reconnect, the browser should show the current server truth instead of keeping stale attached state forever

This keeps runtime truth in `themion-cli` and keeps the browser as an I/O surface.

### 4. Ignore stale sockets and stale timers

Reconnect logic must not let old connections keep writing into the UI.

Required behavior:

- only one active shared websocket generation is authoritative at a time
- message handlers from older socket generations must stop affecting UI state once a newer reconnect attempt becomes authoritative
- reconnect timers from obsolete socket generations must not create extra later sockets
- late messages from stale socket generations must be ignored safely

A generation counter or equivalent socket identity token is the preferred implementation shape.

### 5. Keep send behavior honest while reconnect is pending

The user may try to send while the socket is down.

Required behavior:

- if the socket is not open, ordinary sends must fail safely instead of panicking
- prompt submission must not claim success if the realtime send could not be delivered
- the first slice may reject sends while reconnect is pending instead of queueing them
- if the UI shows a send failure, keep the message concise and local to the browser surface

**Alternative considered:** queue and replay browser-originated actions automatically after reconnect. Rejected for this slice because it adds delivery semantics and duplicate-action risk beyond simple transport recovery.

### 6. Keep the shared-websocket layering from PRD-106

This change must stay consistent with the CLI-owned web direction.

Required behavior:

- keep one browser websocket at `/api/ws`
- keep server-side subscription and routing ownership in `crates/themion-cli/src/web.rs`
- keep reconnect and re-subscribe behavior in `crates/themion-cli-web-ui/src/lib.rs`
- do not move runtime policy into the browser just because reconnect needs local state
- if the server needs a small reconnect-safe protocol improvement, add it without splitting the transport model

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli-web-ui/src/lib.rs` | Add a reconnect-capable shared websocket controller, subscription memory, post-reconnect resubscribe, snapshot refresh, and safe send behavior when disconnected. |
| `crates/themion-cli/src/web.rs` | Keep websocket subscribe/snapshot behavior reconnect-safe and idempotent where needed. |
| `docs/prd/prd-117-web-ui-websocket-auto-reconnect.md` | Define the reconnect behavior and constraints. |
| `docs/README.md` | Add the PRD entry and status. |

## Edge Cases

- websocket closes because `themion-cli --web` restarts → verify: the browser reconnects automatically and restores live status without a page refresh.
- browser laptop sleeps and wakes → verify: stale socket state is replaced by a new connection and fresh snapshots.
- reconnect delay overlaps with the user switching active agent → verify: the restored subscribe set follows the newest selected agent.
- a late message arrives from an older socket generation → verify: it does not overwrite newer state.
- server receives repeated subscribe requests for the same target → verify: reconnect remains safe and does not break live updates.
- terminal list changed while disconnected → verify: browser state is replaced by the fresh server list.
- user submits a prompt while socket is reconnecting → verify: the UI does not falsely report successful acceptance.
- reconnect keeps failing for an extended period → verify: the UI keeps showing reconnect state and retrying within the defined backoff cap.

## Migration

This is an additive Web UI resilience change. No database migration is required.

The change fits patch scope because it improves recovery behavior for an existing browser surface without changing persistent data or the core runtime ownership model.

## Testing

- start `themion-cli --web`, open the browser, then restart the CLI process → verify: the browser reconnects automatically when the server returns and live status resumes.
- drop the websocket temporarily in a browser/dev test harness → verify: socket state changes to reconnecting and later returns to open.
- reconnect during an active agent turn → verify: fresh `/api/status` or websocket snapshot restores the current runtime and agent state.
- reconnect after transcript activity while disconnected → verify: `/api/transcript` refresh replaces stale browser transcript state.
- change selected agent during reconnect wait → verify: post-reconnect subscribe uses the latest selected agent.
- attach terminal list or terminal session, then reconnect → verify: terminal subscriptions recover from current server state.
- attempt prompt submission while socket is not open → verify: the UI does not claim the prompt was accepted unless send actually succeeded.
- run focused Web UI tests for socket lifecycle helpers → verify: reconnect generation, retry timing, and subscription-restore logic behave correctly.
- run `cargo test -p themion-cli-web-ui` → verify: browser helper/tests still pass.
- run `cargo check -p themion-cli` → verify: default CLI/web build compiles.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build compiles.

## Implementation checklist

- [x] refactor shared websocket setup into a reconnect-capable controller in `crates/themion-cli-web-ui/src/lib.rs`
- [x] track authoritative socket generation so stale handlers cannot overwrite newer state
- [x] remember the current subscription targets needed by the existing UI
- [x] resend subscriptions automatically after reconnect opens
- [x] refresh runtime-owned snapshots after reconnect
- [x] make send behavior fail safely while the socket is not open
- [x] add focused tests for reconnect helpers, retry timing, and subscription restore behavior
- [x] update PRD/docs status notes after implementation lands

## Implementation notes

Implemented in v0.72.1. The Web UI now owns a reconnect-capable shared websocket controller with an authoritative generation counter, remembered subscribe targets, bounded reconnect backoff, and post-reconnect snapshot refresh for `/api/status`, `/api/agents`, and `/api/transcript`.

Prompt sends now fail locally while the shared socket is connecting or reconnecting instead of pretending the runtime accepted them, and the sidebar socket badge exposes `connecting`, `open`, `reconnecting`, and `closed` states. The server websocket protocol stayed unchanged because the existing subscribe/snapshot behavior was already reconnect-safe and idempotent for this slice.
