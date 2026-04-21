# PRD-019: Basic Stylos Support in `themion-cli`

- **Status:** Implemented
- **Version:** v0.10.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Add a basic optional Stylos integration to `themion-cli` so the TUI can join a Stylos mesh when the `stylos` cargo feature is enabled at build time.
- Keep Stylos support feature-gated so normal builds remain unchanged unless the feature is intentionally turned on.
- Scope the first implementation to a small, observable foundation in `themion-cli`: session startup, clean shutdown, and lightweight visibility in the TUI.
- Preserve the current separation of concerns by keeping Stylos-specific local runtime wiring in `themion-cli` rather than pushing terminal-app lifecycle logic into `themion-core`.
- Establish a minimal integration path that can later support richer cross-process presence, discovery, or lightweight coordination features without committing to them yet.

## Non-goals

- No default-on Stylos support for normal non-feature builds. A normal `cargo build` or `cargo run -p themion-cli` must behave exactly as it does today.
- No migration of Themion's provider traffic, tool calls, persistent history, workflow state, or shell execution onto Stylos.
- No requirement to add Stylos support to `themion-core` in this first pass unless a tiny shared type is truly unavoidable.
- No new TUI command surface for Stylos chat, pub/sub inspection, or arbitrary key-expression interaction.
- No auth, ACL, encryption, or production-hardening story for Stylos in this PRD.
- No dependency on Stylos being installed system-wide; the integration should assume local path/workspace development during the initial implementation.

## Background & Motivation

### Current state

Themion today is a local terminal AI agent with:

- a Ratatui TUI in `themion-cli`
- a harness/runtime and persistent history layer in `themion-core`
- provider-backed streaming model interaction
- no built-in cross-process or cross-host local-mesh presence layer

The current architecture docs describe a single-process local app model. `themion-cli` owns startup wiring, session creation, TUI behavior, and app lifecycle. `themion-core` owns agent logic, provider clients, tools, workflows, and SQLite history.

In parallel, the sibling `../stele` workspace already has a usable Stylos foundation:

- `apps/stylos/` provides reusable zenoh-based session/config/identity crates
- `apps/stele/crates/stele-server/src/stylos_module.rs` embeds a basic Stylos session into a long-running app
- current Stylos transport posture is UDP + TCP on port `31747`
- the Stele integration publishes heartbeats and exposes a small info queryable

That means the infrastructure pattern already exists nearby, but Themion currently has no equivalent opt-in integration.

### Why start in `themion-cli`

If Themion adopts Stylos at all, the most natural first consumer is `themion-cli` because it already owns:

- process startup and shutdown
- local config loading
- TUI-visible runtime state
- the interactive session boundary

A first-pass CLI integration can prove whether joining a Stylos mesh is useful for Themion without forcing any change in the core harness loop or in default builds.

### Why feature-gated and off by default

Stylos is a meaningful additive dependency and changes the runtime shape of the CLI process. Making it default-on would introduce:

- extra dependency footprint
- extra socket binding behavior
- extra config surface
- extra runtime complexity for users who did not ask for networked local-mesh behavior

Themion should therefore treat Stylos as an explicit capability build, not a silent default behavior change.

## Design

### Add a `stylos` cargo feature to `themion-cli`

`themion-cli` should gain a new optional cargo feature named `stylos` that pulls in the Stylos crates via path dependencies to the sibling `../stele/apps/stylos/` workspace.

Normative behavior:

- the feature is **not** included in `default` features
- normal builds continue to compile and run without Stylos code or Stylos dependencies
- Stylos-specific code paths are guarded with `#[cfg(feature = "stylos")]`
- when the feature is off, no Stylos config is loaded, no session is opened, and no TUI stylos indicator is shown

The intended build shape is explicit, for example:

```bash
cargo run -p themion-cli --features stylos
```

**Alternative considered:** make Stylos always compiled but runtime-disabled by config. Rejected: the user explicitly asked for feature-flag disable by default, and compile-time gating also keeps normal builds lighter and lower-risk.

### Keep the first implementation local to `themion-cli`

The first pass should add a small `stylos` integration module under `crates/themion-cli/src/` that owns:

- translating Themion-local settings into Stylos config
- opening the Stylos session during CLI startup
- exposing a lightweight status snapshot to the TUI
- clean shutdown when the app exits

This mirrors the existing Stele pattern, but adapted for a terminal app rather than a long-running server process.

The session lifecycle belongs in `themion-cli` because it is tied to UI process startup and teardown, not to the reusable model/tool harness.

**Alternative considered:** add Stylos session ownership to `themion-core::agent::Agent`. Rejected: that would mix local process/network lifecycle into the reusable core harness and would make the feature harder to keep optional.

### Minimal runtime behavior: join mesh, publish presence, expose info

The first-pass Stylos feature should do only three runtime things when enabled:

1. open a Stylos session for the Themion process
2. publish a small periodic heartbeat or presence signal under a Themion-specific key namespace
3. register a small info queryable or equivalent status responder for basic observability

A canonical Themion key shape should follow the Stylos grammar:

```text
stylos/<realm>/themion/instances/<instance>/<leaf>
```

Initial leaves:

- `heartbeat`
- `info`

Suggested examples:

- `stylos/dev/themion/laptop-a/heartbeat`
- `stylos/dev/themion/laptop-a/info`

The payloads should stay intentionally minimal in v0.10.0:

- heartbeat: literal bytes such as `b"alive"`
- info: small JSON object with fields such as version, instance, active profile, and model

This mirrors the already-proven Stele pattern while keeping the scope basic.

**Alternative considered:** add pub/sub features directly into the TUI on day one. Rejected: the first goal is infrastructure wiring and observability, not a user-facing Stylos command suite.

### Add a Themion-local Stylos settings block as feature-enabled overrides

`themion-cli` should gain an optional Stylos config surface for the feature-enabled build. The config should be small and mirror the Stele-side shape where practical:

- `enabled: bool`
- `mode: String` (`peer`, `router`, or `client`)
- `realm: String`
- `instance: Option<String>`
- `connect: Vec<String>`

Recommended behavior:

- built-in defaults exist only when the `stylos` feature is compiled
- in feature-enabled builds, Stylos starts by default when the `[stylos]` block is absent
- the config block acts as an override surface rather than a required enable switch
- `enabled = false` explicitly disables default-on behavior
- `mode` defaults to `peer`, `realm` defaults to `dev`, `connect` defaults to empty, and `instance` may be derived from hostname or a sanitized fallback

This keeps the feature opt-in at compile time while making the runtime behavior zero-config for feature-enabled builds.

**Alternative considered:** require `enabled = true` even in feature-enabled builds. Rejected: the desired user experience is for Stylos to work automatically once the feature is compiled, with config used only for overrides.

### TUI visibility should be lightweight and non-intrusive

When the `stylos` feature is enabled and a session is active, the TUI should expose a compact status indicator in the status bar or another similarly lightweight location.

Expected visibility:

- feature off → no Stylos label at all
- feature on, runtime disabled → `stylos: off`
- feature on, runtime enabled and connected → compact label such as `stylos: peer` or `stylos: on`
- feature on, startup failed → compact error state such as `stylos: error`

The conversation pane may optionally include one startup status line when Stylos starts or fails, but the status bar should remain the primary continuous signal.

This keeps the integration observable without cluttering the main chat flow.

**Alternative considered:** add a dedicated Stylos panel or modal. Rejected: too large for a basic feature whose main purpose is to establish wiring.

### Failure to start Stylos must not block normal Themion usage

If the Stylos feature is compiled but the Stylos session fails to start at runtime, Themion should:

- log or display a concise startup warning
- keep the TUI usable
- continue running without Stylos behavior
- show the degraded state in the status bar if the feature is compiled

This is important because Stylos is optional infrastructure, not a requirement for local coding sessions.

**Alternative considered:** fail the whole app when Stylos startup fails. Rejected: that would make an optional feature too brittle and would punish users for network/socket/config issues unrelated to the core Themion experience.

### Clean shutdown should mirror app lifecycle

When the TUI exits normally, any active Stylos session should be closed cleanly if the feature is enabled.

Expected behavior:

- stop heartbeat/background tasks
- drop queryable registrations
- close the Stylos session best-effort
- avoid blocking app shutdown indefinitely if close fails

This should follow the same general shape already used in Stele's Stylos integration.

**Alternative considered:** rely entirely on process exit without explicit close. Rejected: explicit shutdown is a better fit for long-lived sockets/tasks and keeps the behavior aligned with the sibling Stylos consumer pattern.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/Cargo.toml` | Add an optional `stylos` feature, disabled by default, plus path dependencies to the required Stylos crates from `../stele/apps/stylos/`. |
| `crates/themion-cli/src/config.rs` | Add feature-gated Stylos settings parsing and defaults for a Themion-local config block used only when the `stylos` feature is compiled. |
| `crates/themion-cli/src/main.rs` | Wire feature-gated Stylos startup inputs into TUI startup without affecting non-Stylos builds. |
| `crates/themion-cli/src/tui.rs` | Hold Stylos runtime state in the app, surface compact status text in the TUI, emit optional startup/failure narration, and shut the Stylos session down on exit when active. |
| `crates/themion-cli/src/stylos.rs` (new) | Implement the feature-gated Stylos session wrapper for Themion: config translation, instance derivation, heartbeat/info lifecycle, status snapshot, and shutdown helper. |
| `docs/architecture.md` | Document that `themion-cli` has optional Stylos support behind a disabled-by-default cargo feature and describe its high-level lifecycle placement. |
| `docs/engine-runtime.md` | Clarify that Stylos integration, when enabled, lives in CLI-local runtime wiring rather than the core harness loop. |
| `docs/README.md` | Add this PRD to the PRD index with proposed status and scope. |

## Edge Cases

- feature off, config contains a Stylos section → verifyable behavior should remain ordinary startup; the config is ignored or not deserialized into active behavior because Stylos is not compiled in.
- feature on, Stylos config absent → Themion should start normally with Stylos active using built-in defaults.
- feature on, Stylos config present but `enabled = false` → no session opens; TUI shows `stylos: off`.
- feature on, Stylos startup fails because ports are unavailable or config is invalid → Themion remains usable and shows a compact error state rather than aborting.
- multiple Themion processes run on the same host with Stylos enabled → Stylos port-walk and per-instance identity should avoid immediate crashes, but duplicated instance names should be treated as a configuration issue.
- Stylos feature build is attempted in an environment where sibling `../stele/apps/stylos/` path deps are unavailable → the feature build fails clearly at compile time; non-feature builds remain unaffected.
- user runs print/non-TUI mode with the Stylos feature compiled → the initial implementation may either skip Stylos entirely in print mode or use the same startup wiring, but the behavior must be documented explicitly and remain minimal. **Alternative considered:** require Stylos in every binary path when compiled. Rejected: print mode does not need the extra lifecycle unless there is a clear use case.

## Migration

This is an additive, opt-in feature.

There is no migration for existing users because:

- default builds remain unchanged
- default runtime behavior remains unchanged for non-feature builds
- no database schema changes are required
- no provider or workflow behavior changes are required

Users who want the feature must opt in explicitly:

1. build `themion-cli` with `--features stylos`
2. optionally provide Stylos config only when overriding defaults or disabling it

If future releases add richer Stylos-backed behavior, this PRD's feature-gated basic integration becomes the compatibility foundation for those later additions.

## Testing

- build `themion-cli` without the feature → verify: the crate compiles and runs exactly as before, with no Stylos UI and no path dependency requirement on the sibling Stylos workspace.
- build `themion-cli` with `--features stylos` → verify: the crate compiles successfully when the sibling Stylos workspace is present.
- run feature-enabled Themion with no Stylos config → verify: a Stylos session opens with built-in defaults and the TUI shows a compact active state.
- run feature-enabled Themion with `enabled = false` → verify: the TUI starts normally and shows `stylos: off` or equivalent compact disabled state.
- run feature-enabled Themion with valid Stylos override config → verify: a Stylos session opens using the overridden values, heartbeat/info tasks start, and the TUI shows a compact active state.
- subscribe from the sibling Stylos CLI to `stylos/<realm>/themion/*/heartbeat` → verify: the feature-enabled Themion process publishes observable heartbeat samples.
- query `stylos/<realm>/themion/*/info` from the sibling Stylos CLI → verify: Themion responds with a small JSON info payload containing at least version, instance, active profile, and model.
- force Stylos startup failure in a feature-enabled build, such as invalid config or unavailable transport binding → verify: Themion still launches and remains usable, while the UI surfaces a concise Stylos error state.
- exit a feature-enabled Themion session cleanly → verify: the Stylos background tasks stop and the session closes without hanging TUI shutdown.
- run `cargo check -p themion-cli` and `cargo check -p themion-cli --features stylos` after implementation → verify: both non-feature and feature-enabled builds compile cleanly.
