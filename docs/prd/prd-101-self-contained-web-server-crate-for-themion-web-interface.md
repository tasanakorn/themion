# PRD-101: Self-Contained Web Server Crate for Themion Web Interface

- **Status:** Partially implemented
- **Version:** v0.61.1
- **Scope:** `themion-web`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-04

## Summary

- Themion now has a separate `themion-web` crate and dedicated `themion-web` binary that host a self-contained local browser surface using Rust, Axum, and Leptos.
- The currently landed implementation delivers the isolated crate/binary boundary and a browser terminal surface backed by `portable-pty` plus bundled `xterm.js`, including persistent in-process PTY sessions and resize handling.
- The originally discussed monitoring, SQLite inspection, and config/auth-file inspection features have not landed in this PRD and should move to a follow-up PRD instead of remaining implied here.
- A future browser chat/agent interface comparable to the TUI remains deferred, with Stylos still the preferred future bridge path if and when that surface is later designed.
- This PRD is tightened to the current implemented slice rather than preserving broader unimplemented Phase 1 scope.

## Goals

- Introduce a new self-contained web-server crate in the workspace for a browser-based Themion interface, with its own separate binary rather than embedding the server inside the existing CLI executable.
- Provide a local browser terminal surface using `portable-pty` on the server side and bundled `xterm.js` on the frontend.
- Keep the first landed implementation isolated in the new crate without requiring code changes to `themion-core`, `themion-cli`, or other existing crates.
- Preserve the repository architecture rule that surfaces observe or project runtime truth rather than owning runtime policy, while keeping the first landed implementation intentionally limited in scope.
- Leave room for future browser monitoring and browser agent interaction work, but do not overstate those deferred capabilities as implemented behavior in this PRD.

## Non-goals

- No requirement in this tightened PRD to replace the TUI as the primary interactive surface.
- No requirement in this PRD to multiplex the web server into the existing `themion` CLI binary.
- No requirement in this PRD to change `themion-cli`, `themion-core`, or other existing crates at all.
- No requirement in this PRD to claim landed monitoring through SQLite, read-only database browsing, config-file inspection, or auth-file inspection; those are deferred to a follow-up PRD.
- No requirement in this PRD to implement full browser-based agent chat, tool-call interaction, transcript composition, or other TUI-like agent-control behavior.
- No requirement in this PRD to make the browser UI the canonical owner of runtime state, board policy, workflow policy, or agent scheduling.
- No requirement in this PRD to expose remote multi-user authentication, internet-facing deployment, or public-host hardening.
- No requirement in this PRD to define a stable external HTTP API for third-party clients beyond what the first-party web UI needs.
- No requirement in this PRD to make terminal access a replacement for native local shell tooling; it is a browser convenience surface.

## Background & Motivation

### Current state

Themion is currently organized around two primary crates:

- `themion-core` for reusable harness/runtime logic, provider behavior, tool handling, and database-backed history
- `themion-cli` for terminal UI, config loading, local filesystem-driven flows, and process-local runtime ownership

The current product is terminal-first. It already has strong local runtime capabilities that a browser surface could usefully project:

- runtime-owned app-state snapshots and local agent roster state
- persistent SQLite-backed session and Project Memory data
- durable config/auth files and other local state files
- Stylos-backed and local multi-agent status concepts
- a terminal-mode operational workflow that some users may want to inspect or attach to from a browser

What was missing was a self-contained local web surface that could be added without turning the browser into a second orchestration stack and without folding that work into the existing terminal app crate. That isolated crate/binary boundary has now landed, but the broader monitoring/file-inspection portion originally described here has not.

### Why this matters now

A browser UI could improve several practical workflows without replacing the TUI:

- attaching to a terminal session from a browser when a terminal emulator or SSH workflow is less convenient
- proving out a dedicated browser-surface crate and binary without mixing that work into `tui.rs` or the existing CLI executable
- keeping room for future browser monitoring and browser-agent work without misdescribing those deferred capabilities as already landed

The requested scope originally divided into a broader operational Phase 1 plus a later browser-agent bridge. In practice, the currently landed slice is narrower: dedicated web-crate delivery plus browser terminal access now, with monitoring/file inspection and browser-agent work deferred.

**Alternative considered:** extend `themion-cli` directly with an embedded HTTP server and frontend asset handling. Rejected: the web surface is substantial enough to deserve its own crate and binary boundary, and this PRD now requires the first implementation to avoid changing existing crates entirely.

## Design

### 1. Add a dedicated web-interface crate to the workspace

Themion should have one new crate dedicated to hosting the web interface.

Implemented behavior:

- the workspace now contains `crates/themion-web/`
- the crate compiles to its own separate `themion-web` binary entrypoint rather than extending the existing `themion` executable
- the crate owns HTTP serving, web routing, web asset delivery, and web-specific terminal session plumbing
- the crate uses Rust with Axum for the server layer and Leptos for the frontend UI layer
- the first landed implementation did not require code changes to `themion-core`, `themion-cli`, or other existing crates

### 2. Treat the web interface as a separate local surface

The landed crate is a new local browser surface rather than a new runtime owner.

Implemented behavior:

- `themion-web` runs as its own process with its own local state for browser PTY sessions
- it does not become the canonical home of agent orchestration, workflow policy, board routing, or Stylos policy
- the current landed implementation is intentionally narrow and does not claim runtime-owned monitoring snapshots or agent-control semantics that would require a different bridge design later

### 3. Browser terminal access is the currently landed feature slice

The landed implementation centers on browser terminal access.

Implemented behavior:

- the web crate supports browser terminal sessions backed by `portable-pty`
- frontend terminal rendering uses bundled `xterm.js`
- the terminal surface supports interactive input/output and resize handling over a websocket transport
- the server side manages PTY lifecycle and bridges terminal bytes to the browser UI
- terminal sessions are persistent within the running `themion-web` process and can be reattached from the browser UI
- the initial browser terminal targets the user's default local shell through a PTY surface

### 4. Monitoring and file inspection are deferred to follow-up work

The originally proposed monitoring and file-inspection scope is not part of what landed here.

Deferred behavior:

- no SQLite-backed monitoring views have landed in `themion-web`
- no read-only inspection of the active database file has landed in `themion-web`
- no config-file or auth-file inspection flows have landed in `themion-web`
- these capabilities should move to a follow-up PRD instead of remaining implied in this one

### 5. Browser agent interaction remains deferred

A browser-native agent interface comparable to the TUI remains future work.

Deferred behavior:

- the current implementation does not deliver browser chat/composer/transcript or agent-control behavior
- future browser agent interaction should still prefer a Stylos-mediated or other runtime-owned bridge rather than direct TUI coupling
- this PRD should not overstate that future direction as already implemented behavior

### 6. Keep local-first assumptions explicit

The current implementation remains a local or operator-controlled browser surface.

Implemented behavior:

- `themion-web` binds locally by operator choice using `THEMION_WEB_BIND`, with a documented default local-server startup pattern
- the current PRD still does not claim internet-facing production hardening or multi-user authentication

### 7. Documentation must reflect the tightened implemented scope

The docs should describe the current landed scope truthfully.

Required behavior:

- `docs/README.md` should mark this PRD as partially implemented rather than describing the broader original Phase 1 as still in progress
- `crates/themion-web/README.md` should describe the currently landed shell/terminal-focused implementation rather than a generic blank app shell
- any follow-up work for monitoring, SQLite inspection, or config/auth inspection should move to a new PRD rather than staying ambiguous here

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-101-self-contained-web-server-crate-for-themion-web-interface.md` | Tighten the PRD to the currently landed `themion-web` crate and browser terminal scope, and move unimplemented monitoring/file-inspection work out of this document. |
| `docs/README.md` | Update the PRD-101 entry to reflect partial implementation status and the current implemented scope. |
| `crates/themion-web/README.md` | Update crate docs to describe the current browser terminal implementation instead of a generic blank app shell. |
| `crates/themion-web/` | Existing landed crate that hosts the web server, routing, bundled frontend assets, embedded terminal font asset, and default-shell PTY terminal bridge with its separate web-server binary entrypoint. |

## Edge Cases

- the web UI starts while no terminal sessions are currently open → verify: the UI can create a new shell session cleanly.
- a user opens the browser terminal and resizes the browser pane repeatedly → verify: PTY resize behavior remains stable and readable.
- the browser terminal session disconnects unexpectedly → verify: reconnecting the websocket can re-list and reattach to persistent PTY sessions in the running `themion-web` process.
- the user opens multiple shell tabs in the browser → verify: terminal session tabs remain separately attachable and closable.
- a future monitoring or file-inspection capability is discussed while reviewing this PRD → verify: it is treated as deferred follow-up scope rather than implied current behavior.

## Migration

This is an additive product surface, not a replacement migration.

Expected rollout behavior:

- existing TUI and headless flows remain valid
- the new web surface ships as a separate executable instead of altering the existing CLI startup contract
- the landed browser terminal surface adds an additional local operator workflow
- monitoring, database inspection, and config/auth-file inspection remain deferred to follow-up work rather than being treated as part of the shipped scope here
- a postponed browser-native agent interaction surface can land later without invalidating the currently shipped terminal-focused slice

## Testing

- start the `themion-web` binary locally → verify: the browser UI loads from the dedicated web crate and separate executable.
- open a browser terminal backed by `portable-pty` and interact with it through bundled `xterm.js` → verify: input, output, session creation, and reconnect behavior work correctly.
- resize the browser terminal repeatedly → verify: PTY resize handling stays aligned with the rendered terminal geometry.
- run `cargo check -p themion-web` → verify: the new crate compiles in its default configuration.
- run `cargo check --all-features -p themion-web` → verify: the all-features web build compiles cleanly.

## Implementation checklist

- [x] add a new dedicated web-interface crate to the workspace
- [x] give the new web crate its own separate binary entrypoint instead of extending the existing `themion` executable
- [x] implement browser terminal access using `portable-pty` plus `xterm.js` as a PTY surface running the user's default shell
- [x] keep the landed implementation isolated to the new crate without requiring changes in existing crates
- [x] document the deferred browser-native agent interface and preferred future Stylos bridge direction at the PRD level
- [x] validate the new crate in default and all-features configurations
- [ ] move monitoring, SQLite inspection, and config/auth-file inspection into a follow-up PRD

## Implementation notes

- Landed in `v0.61.1` as a partial implementation of the originally broader PRD.
- Implemented a separate `themion-web` crate and dedicated `themion-web` binary using Axum and Leptos.
- Implemented persistent in-process browser shell sessions backed by `portable-pty` and rendered with bundled `xterm.js`.
- The terminal frontend now defaults to an embedded `JetBrains Mono Nerd Font` at size 14 and keeps PTY resize messages aligned with xterm's actual geometry.
- The originally proposed SQLite monitoring, database inspection, and config/auth-file inspection work has not landed and should be specified in a follow-up PRD instead of remaining inside PRD-101.
