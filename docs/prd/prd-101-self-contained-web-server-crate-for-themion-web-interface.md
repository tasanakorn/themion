# PRD-101: Self-Contained Web Server Crate for Themion Web Interface

- **Status:** In progress
- **Version:** >v0.61.1 +minor
- **Scope:** new web crate, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-04

## Summary

- Themion currently has a terminal-first local interface, but it does not provide a self-contained browser-accessible surface for monitoring, configuration inspection, database access, or terminal attachment.
- Add a new workspace crate and a separate dedicated binary that run a self-contained local web server using Rust, Axum, and Leptos to host a Themion web interface.
- Make Phase 1 focus on monitoring through SQLite and config/auth files only, read-only access to the active database/config files under explicit local control, and browser terminal access backed by `portable-pty` plus `xterm.js`.
- Reserve a future browser chat/agent interface comparable to the TUI, but keep its runtime bridge explicitly postponed and treat Stylos as the preferred future bridge path rather than implementing that agent-control surface now.
- Keep the first implementation fully isolated in a separate crate/binary and do not require any code changes to existing crates.

## Goals

- Introduce a new self-contained web-server crate in the workspace for a browser-based Themion interface, with its own separate binary rather than embedding the server inside the existing CLI executable.
- Provide a browser-accessible monitoring surface derived from SQLite data and config/auth files only in Phase 1.
- Provide explicit local access to the active SQLite database file and relevant config/auth files through the web interface, with clear behavior boundaries and without making the browser UI the source of truth for those files; Phase 1 database access should be read-only.
- Provide browser terminal access using `portable-pty` on the server side and `xterm.js` on the frontend.
- Leave room for a future browser chat/agent interface comparable to the TUI, but do not implement, imply, or pre-build that interface in Phase 1.
- Define Stylos as the preferred future bridge path for browser-to-agent interaction, while explicitly postponing implementation of that agent-control surface in this PRD.
- Preserve the repository architecture rule that surfaces observe or project runtime truth rather than owning runtime policy, while requiring the first implementation to achieve that without changing existing crates.

## Non-goals

- No requirement in this PRD to replace the TUI as the primary interactive surface.
- No requirement in this PRD to multiplex the web server into the existing `themion` CLI binary.
- No requirement in this PRD to change `themion-cli`, `themion-core`, or other existing crates at all.
- No requirement in this PRD to implement full browser-based agent chat, tool-call interaction, transcript composition, or other TUI-like agent-control behavior in Phase 1.
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

What is missing is a self-contained local web surface that can reuse runtime truth without turning the browser into a second orchestration stack and without folding that work into the existing terminal app crate.

### Why this matters now

A browser UI could improve several practical workflows without replacing the TUI:

- monitoring a running Themion instance from another window or device on the same machine or local network
- inspecting or carefully editing the active database/config state without dropping into ad hoc manual filesystem work
- attaching to a terminal session from a browser when a terminal emulator or SSH workflow is less convenient
- preparing a future path toward a richer browser-based Themion interface without mixing that work into `tui.rs`

The requested scope naturally divides into two parts:

- a concrete first slice that is local, self-contained, and operationally useful now
- a future agent-interaction slice that should bridge into existing runtime mechanisms rather than inventing a second control plane

**Alternative considered:** extend `themion-cli` directly with an embedded HTTP server and frontend asset handling. Rejected: the web surface is substantial enough to deserve its own crate and binary boundary, and this PRD now requires the first implementation to avoid changing existing crates entirely.

## Design

### 1. Add a dedicated web-interface crate to the workspace

Themion should add one new crate dedicated to hosting the web interface.

Required behavior:

- the workspace should gain a new crate dedicated to browser delivery and web-surface integration
- that crate should compile to its own separate binary entrypoint rather than adding the web server mode into the existing `themion` executable
- the new crate should be self-contained in the sense that it owns HTTP serving, web routing, web asset delivery, and any web-specific session plumbing it needs
- the crate should use Rust with Axum for the server layer and Leptos for the frontend UI layer
- the crate should consume only already-available stable interfaces, files, protocols, and process boundaries rather than requiring new shared adapters in existing crates
- the first implementation must not require code changes to `themion-core`, `themion-cli`, or other existing crates
- the crate should not become the canonical home of agent orchestration, workflow policy, board routing, or Stylos policy

Naming and binary requirement:

- the implementation should choose a crate name consistent with workspace naming, for example `themion-web`, unless a more precise name is agreed later
- the new crate should expose its own binary, for example a `themion-web` executable, rather than piggybacking on the existing `themion` binary

### 2. Treat the web interface as a new surface over runtime-owned state

The web crate should follow the same layering expectations that constrain the TUI.

Required behavior:

- the web interface is a surface, not a new runtime owner
- runtime truth in Phase 1 should be derived only from the active SQLite database and relevant config/auth files
- if a capability cannot be reached without changing an existing crate, that capability should be deferred rather than forcing code changes into current crates
- the web surface may request runtime actions through defined intents or adapters, but it must not become the place where “the system decided” logic lives
- if a future web feature would require reconstructing runtime truth separately from existing app-state ownership, that is a design smell to fix before implementation

This preserves the repository rule that shared runtime truth belongs outside presentation surfaces and keeps the browser work isolated from existing crates.

### 3. Monitoring is part of the first implementation slice

The first slice should provide a useful monitoring view for a running Themion instance.

Required behavior:

- the web UI should expose only monitoring data that can be derived from the active SQLite database and relevant config/auth files without code changes to current crates
- Stylos-visible status, logs, process inspection, and other external surfaces are not required monitoring inputs for Phase 1
- if a desired monitoring view would require a new runtime snapshot, new API, or new status publisher from an existing crate, that view should be deferred
- the monitoring slice should prioritize read-oriented visibility first: what persisted state exists and what the configured local environment reveals

**Alternative considered:** start with a purely static landing page and postpone monitoring until after a browser chat surface exists. Rejected: monitoring is one of the clearest immediate product wins and does not require the postponed agent-chat bridge.

### 4. Database-file and config access are first-slice capabilities

The web UI should provide explicit access to the active database and relevant config files.

Required behavior:

- the web surface should let a local operator inspect the currently active SQLite database file used by Themion
- the web surface should let a local operator inspect relevant config/auth files used by the running instance
- the product must make the target file identities clear, especially for the active database path and active profile/config scope
- Phase 1 should keep SQLite database access read-only and should preserve file truth in the filesystem/database rather than creating a separate browser-owned data model
- Phase 1 should support safe read-only file operations such as view, download/export, and structured inspection for config/auth files, and should not provide config/auth editing or in-browser database mutation tooling

Safety expectation:

- file access should be explicitly local-operator-oriented and should avoid silent background edits
- Phase 1 should not allow config/auth-file writes

### 5. Browser terminal access is part of the first slice

The first slice should include terminal access through a browser.

Required behavior:

- the web crate should support a browser terminal session backed by `portable-pty`
- frontend terminal rendering should use `xterm.js`
- the terminal surface should support interactive input/output and resize handling
- the server side should manage PTY lifecycle and bridge terminal bytes to the browser UI over a suitable real-time channel
- the product should treat the browser terminal as an operational terminal attachment surface, not as a replacement for runtime-owned agent orchestration

Implementation guidance:

- the initial browser terminal should target the user's default local shell through a PTY surface; it should not be described as a browser-native Themion session controller in Phase 1
- terminal access should remain separate from the postponed browser-native agent-chat surface

### 6. Browser agent interaction is a deferred phase, not part of the first landing

A browser-native agent interface comparable to the TUI is in scope for the overall product direction, but not for the first implementation slice.

Required behavior:

- the PRD may preserve a future product direction for a browser frontend interface to Themion agents comparable in spirit to the TUI, but that direction is explicitly not a Phase 1 commitment
- the first implementation slice should not claim to deliver any chat/composer/transcript or agent-control surface
- the PRD should state clearly that all TUI-like browser agent interaction is deferred until an already-existing external bridge makes it possible without changing current crates, or until a later explicitly approved PRD changes that constraint
- deferred status should not erase the requirement that the design remain compatible with adding that surface later

### 7. Stylos is the preferred future bridge for browser-to-agent interaction

Future browser agent interaction should bridge through existing runtime coordination paths rather than inventing a one-off UI-owned control path.

Required behavior:

- the preferred future bridge for browser-to-agent interaction should be Stylos-mediated runtime coordination or an equivalent runtime-owned bridge derived from the same architecture
- because that bridge is postponed, the first implementation should not add preparatory code to existing crates just to reserve the future path
- the postponed browser agent interface should not assume direct TUI coupling or TUI-owned state as its backend
- when the deferred browser agent surface is later implemented, it should connect to runtime/app-state or Stylos-owned coordination paths rather than bypassing them with ad hoc web-only control logic

**Alternative considered:** make the future browser agent surface talk directly to `tui.rs` or reuse terminal scraping as the control path. Rejected: that would violate the repository’s runtime ownership rules and turn the TUI into a backend.

### 8. Prefer phased delivery with the first phase centered on operational surfaces

This product direction is broad enough that the PRD should be phased.

Required behavior:

- the overall product outcome remains a self-contained Themion web interface hosted by a dedicated web crate and served by its own dedicated binary
- Phase 1 should focus on:
  - web server crate setup
  - monitoring views backed only by the active SQLite database and relevant config/auth files
  - read-only active database inspection and config/auth-file inspection
  - browser terminal access as a PTY surface running the user's default shell
- a later phase may add:
  - browser-native transcript/composer interaction with agents
  - Stylos-bridged action routing
  - richer runtime controls and multi-surface synchronization
- the document should not collapse into “Phase 1 only” language; the larger product direction should remain visible

### 9. Keep local-first assumptions explicit

The first version should be optimized for local or operator-controlled use rather than broad remote hosting.

Required behavior:

- the first product target is a self-hosted Themion web surface controlled by the same operator or environment that runs Themion
- local-only or trusted-network assumptions should be explicit in docs and implementation notes
- if authentication, origin restrictions, or bind-address policy are needed for safe default behavior, they should be defined explicitly during implementation rather than left to accidental framework defaults
- this PRD does not yet claim internet-facing production hardening

### 10. Documentation and workspace guidance must evolve with the new surface

Adding a new web crate changes the documented product shape and workspace structure.

Required behavior:

- `docs/architecture.md` should be updated when implementation lands so the workspace layout and surface-layer description include the new web crate as an independent surface that consumes only already-existing external interfaces
- if the final implementation introduces new operator workflows, startup modes, or config semantics, the relevant docs should be updated in the same change
- `docs/README.md` should gain the new PRD entry in sorted order
- if the accepted implementation adds durable repository guidance about surface ownership or web/runtime interaction rules, the relevant guidance docs should be updated alongside the code

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-101-self-contained-web-server-crate-for-themion-web-interface.md` | Define the product requirement for a dedicated Axum + Leptos web surface, the first-slice operational scope, and the deferred Stylos-bridged browser agent interface. |
| `docs/README.md` | Add the new PRD entry in sorted order. |
| `docs/architecture.md` | Update workspace and surface-layer documentation when the web crate lands. |
| new workspace crate such as `crates/themion-web/` | Host the web server, routing, Leptos frontend, default-shell PTY terminal bridge, and SQLite/config-file integrations that rely only on already-existing external interfaces, with a separate web-server binary entrypoint. |

## Edge Cases

- the web UI starts while no interactive Themion session is currently running → verify: monitoring and file/terminal surfaces fail clearly or show unavailable state without inventing runtime truth.
- the active database file path differs from the canonical default → verify: the web UI shows the actual in-use path rather than assuming `~/.local/share/themion/system.db`.
- a user opens the browser terminal and resizes the browser pane repeatedly → verify: PTY resize behavior remains stable and readable.
- the browser terminal session disconnects unexpectedly → verify: PTY/session cleanup behavior is explicit and does not leak orphaned processes indefinitely.
- the web UI can inspect config/auth files but write access is disabled in the current implementation slice → verify: the product still presents that limitation clearly instead of implying editable support.
- a desired monitoring panel depends on data that is not already exposed through the active SQLite database or relevant config/auth files → verify: that panel remains deferred rather than forcing changes into existing crates.
- a future browser agent-control implementation is not yet present → verify: the web UI clearly distinguishes monitoring/terminal capabilities from postponed browser-native chat capabilities.
- Stylos is not enabled in the current build → verify: the first-slice web monitoring/terminal/file-access surfaces can still exist without pretending the deferred browser-agent bridge has landed.

## Migration

This is an additive product surface, not a replacement migration.

Expected rollout behavior:

- existing TUI and headless flows remain valid
- the new web surface ships as a separate executable instead of altering the existing CLI startup contract
- the new web crate adds an additional surface rather than moving runtime ownership out of existing layers
- rollout should not require code changes to existing crates; any unsupported capability stays deferred until a later explicitly approved PRD
- operators may adopt the web interface incrementally for monitoring, file access, and browser terminal workflows
- the postponed browser-native agent interaction can land later without invalidating the Phase 1 operational surface

## Testing

- start the new web server binary from the new web crate against a normal local Themion runtime → verify: the browser UI loads and shows runtime monitoring data derived from runtime-owned state.
- inspect the active DB/config targets from the web UI → verify: the UI identifies the actual active file paths and shows the expected content or metadata, with database access remaining read-only.
- open a browser terminal backed by `portable-pty` and interact with it through `xterm.js` → verify: input, output, and resize behavior work correctly.
- run `cargo check -p <new-web-crate>` after implementation → verify: the new crate compiles in its default configuration.
- run `cargo check --all-features -p <new-web-crate>` after implementation if the new crate uses feature flags → verify: all-features web build compiles cleanly.

## Implementation checklist

- [ ] add a new dedicated web-interface crate to the workspace
- [ ] give the new web crate its own separate binary entrypoint instead of extending the existing `themion` executable
- [ ] implement monitoring/state access only through already-existing external interfaces without adding new adapters to existing crates
- [ ] defer any capability that cannot be delivered without changing an existing crate
- [ ] implement the first-slice monitoring views only from the active SQLite database and relevant config/auth files
- [ ] implement read-only database inspection and read-only config/auth-file access flows for the web UI
- [ ] implement browser terminal access using `portable-pty` plus `xterm.js` as a PTY surface running the user's default shell
- [ ] document the deferred browser-native agent interface and preferred future Stylos bridge clearly in the shipped UX/docs
- [ ] update active architecture/workspace docs when the web crate lands
- [ ] validate the new crate and any touched existing crates in the required feature configurations
