# PRD-061: Session-Level API Call Logging Toggle with Per-Round JSON Capture

- **Status:** Implemented
- **Version:** v0.39.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-27

## Summary

- Developers currently have no built-in, session-scoped way to inspect the exact API request/response payload flow for one Themion session without adding ad hoc logging or affecting other sessions.
- Themion should add a session-level enable/disable toggle for API call logging so one active session can capture provider traffic while ordinary sessions remain unchanged.
- The control surface for this feature should be a TUI `/debug ...` slash command so a developer can turn logging on or off during a live session without restarting.
- When enabled, each model round in a turn should write one JSON artifact under the system temp root, typically `/tmp/themion/<session_id>/<turn>/round_<n>.json` on Unix-like systems, so debugging has a predictable file layout.
- The captured artifact should focus on provider-call debugging data: request payload shape, response payload shape, timing/status metadata, and enough session/turn/round attribution to relate logs back to runtime behavior.

## Goals

- Let a developer enable or disable provider/API call logging for one session without turning on global debug logging.
- Use a TUI `/debug ...` slash command as the explicit control surface for that enable/disable behavior.
- Persist one JSON artifact per model round in a stable path layout under the system temp root, typically `/tmp/themion/<session_id>/<turn>/round_<n>.json` on Unix-like systems.
- Make each artifact useful for debugging provider payload translation, tool-call loops, and backend-specific request/response differences.
- Keep ordinary sessions unchanged when logging is off.

## Non-goals

- No permanent database-backed archive of raw provider traffic.
- No requirement to log every internal event, TUI redraw, or tool execution payload outside the provider-call boundary.
- No cross-session global toggle that silently affects other active sessions.
- No promise that the captured JSON is redaction-safe for sharing outside the local machine without review.
- No requirement in this slice to expose a full log browser inside the TUI.
- No requirement in this slice to support non-slash-command control paths such as config-file-only or env-var-only toggles.

## Background & Motivation

### Current state

Themion already persists turn-level metadata in SQLite and streams assistant output through the harness loop, but there is no first-class session-scoped artifact that shows exactly what was sent to and received from the provider for each round of a turn.

That makes several debugging tasks harder than they should be:

- comparing what different backends actually receive after prompt assembly and provider translation
- inspecting why a tool-call round behaved differently from expectation
- checking whether a request included the expected model, tool schema, or message window
- inspecting raw provider error payloads or unexpected response structure

Today, a developer who needs this level of visibility typically has to add temporary logging, use a debugger, or widen logging more broadly than intended.

### Why session-level control matters

Provider-call logging is valuable during debugging, but it can also be noisy, expensive in disk churn, and potentially sensitive because payloads may contain user prompts, tool arguments, and model outputs.

The safest default is therefore:

- off by default
- enabled intentionally
- scoped to one session
- stored locally in a predictable debug path
- controlled from the live TUI session with an explicit `/debug ...` slash command

This keeps the feature useful for targeted debugging without turning every session into a verbose trace run.

**Alternative considered:** make API call logging a global process-level debug switch only. Rejected: it is too broad for a multi-session or multi-agent tool where a developer may want detailed logs from one session while keeping the rest of the runtime normal.

## Design

### Design principles

- Keep logging opt-in and session-scoped.
- Use an explicit `/debug ...` slash command so the control path matches other developer-facing TUI controls.
- Capture one artifact per provider round so tool-call loops remain easy to inspect.
- Prefer structured JSON over ad hoc text logs.
- Keep the artifact path human-browsable and easy to clean up.
- Preserve current runtime behavior when logging is disabled.

### 1. Session-level logging state

Themion should track whether API call logging is enabled for the current harness session.

Required product behavior:

- each session starts with API call logging disabled
- a developer can enable it for the active session from the TUI
- a developer can disable it again later in the same session from the TUI
- enabling or disabling one session must not silently affect other sessions
- the current logging state should remain available to the local session runtime for every provider round in that session

This session-level state may be held in CLI-local app/session runtime wiring, in core session state, or both, as long as the effective behavior is stable across the full turn loop for that session.

**Alternative considered:** store the toggle only as a process-global mutable flag. Rejected: it breaks the requested session-level isolation and makes concurrent debugging less predictable.

### 2. Slash-command control surface

The control surface for this feature should be a TUI `/debug ...` slash command.

Required behavior:

- the TUI should accept a `/debug ...` command that enables API call logging for the active session
- the TUI should accept a `/debug ...` command that disables API call logging for the active session
- the command should produce a clear local acknowledgement showing whether logging is now enabled or disabled
- the acknowledgement should make clear that the scope is the current session

Normative command shape:

- `/debug api-log enable`
- `/debug api-log disable`

This keeps the command grouped with other developer/runtime inspection controls instead of introducing a separate top-level slash-command family.

**Alternative considered:** use `/debug api-log on` and `/debug api-log off`. Rejected: `enable` and `disable` read more like existing imperative slash-command verbs such as `index`, and they make the intended state transition slightly more explicit.

### 3. Log artifact layout

When API call logging is enabled, Themion should write one JSON file per provider round using this path shape:

- `<system-temp>/themion/<session_id>/<turn>/round_<n>.json>`

Path semantics:

- `<session_id>` is the exact session UUID already used for the active harness session
- `<turn>` is the turn identifier used consistently within the session; the implementation may use the human-facing turn sequence number or another stable turn-specific directory name, but the naming choice must be documented and remain consistent
- `<n>` is the 1-based provider round number within that turn

Normative behavior:

- the parent directories should be created on demand when logging is enabled and a round is written
- one completed provider round should produce at most one `round_<n>.json` file for that turn/round combination
- repeated rounds in the same turn should create sibling files such as `round_1.json`, `round_2.json`, and so on
- when logging is disabled, no such round artifact should be written

The initial implementation should treat the system temp directory as the developer debug root rather than a durable application-data path.

**Alternative considered:** store these artifacts under XDG app data beside `system.db`. Rejected for this slice: these are temporary debugging artifacts and fit better under the system temp directory than under durable app data.

### 4. Artifact JSON shape

Each `round_<n>.json` artifact should be structured JSON intended for debugging rather than replay-grade protocol preservation.

Required content areas:

- session attribution
  - `session_id`
  - project or working-directory context when readily available
- turn attribution
  - stable turn identifier and round number
- backend attribution
  - provider/backend name
  - model name
  - request endpoint or API mode when relevant, such as chat completions vs responses
- request data
  - the translated request payload that Themion sends to the backend, or a close structured equivalent
- response data
  - the final structured response payload or accumulated result shape Themion receives from the backend
- outcome metadata
  - HTTP status when available
  - timing or duration metadata when available
  - whether the round completed normally or failed
  - error payload/details when the backend returns an error

Recommended top-level shape:

```json
{
  "session_id": "...",
  "turn": 12,
  "round": 2,
  "provider": "openai",
  "backend": "chat_completions",
  "model": "gpt-...",
  "request": { ... },
  "response": { ... },
  "meta": {
    "started_at_ms": 0,
    "finished_at_ms": 0,
    "duration_ms": 0,
    "http_status": 200,
    "outcome": "ok"
  }
}
```

Exact field names may differ if the surrounding code has a more natural shape, but the artifact should remain obviously inspectable and stable enough for developer use.

**Alternative considered:** log only the serialized raw HTTP body text. Rejected: raw text alone is harder to compare across backends and gives weaker attribution for turn/round/session debugging.

### 5. Round boundary behavior

Themion's harness loop may call the model multiple times in one user turn because tool calls append tool results and then trigger another provider round.

This PRD defines the logging boundary as one provider round, not one user turn.

That means:

- the first model call in a turn writes `round_1.json`
- if the assistant requests tools and the harness calls the provider again in the same turn, that next call writes `round_2.json`
- the sequence continues until the turn finishes or fails

This makes tool-call loops inspectable without forcing a large mixed artifact for the whole turn.

**Alternative considered:** store one large JSON file per turn containing all rounds. Rejected: a per-round split is easier to inspect incrementally and matches the requested `round_<n>.json` layout.

### 6. Error and partial-failure handling

If a provider round fails after Themion has enough information to produce a useful debug artifact, Themion should still write the `round_<n>.json` file when logging is enabled.

Required behavior:

- backend or HTTP errors should be captured in the artifact with explicit failure metadata
- if request construction succeeded but the response failed, the request section should still be preserved
- if no meaningful request artifact can be formed, the failure should still be represented with whatever attribution is available rather than silently skipping the file
- logging failures themselves should not crash the session; they should degrade gracefully and, when practical, surface a concise local warning

This keeps the feature useful for the exact class of provider failures that usually motivate debug logging.

**Alternative considered:** write artifacts only for successful rounds. Rejected: that would remove the most useful evidence during provider-debugging scenarios.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/agent.rs` | Thread per-turn and per-round attribution into the provider-call path so each model invocation can emit one debug artifact when session logging is enabled. |
| `crates/themion-core/src/client.rs` | Expose or return enough structured request/response/error metadata for chat-completions-style backends to populate per-round JSON logs. |
| `crates/themion-core/src/client_codex.rs` | Expose or return enough structured request/response/error metadata for the Codex responses backend to populate the same logging shape. |
| `crates/themion-core/src/client.rs` / backend abstraction | Add a backend-agnostic logging handoff or round-trace shape so session-level logging does not become ad hoc per call site. |
| `crates/themion-cli/src/app_state.rs` and/or session wiring | Carry the session-scoped enabled/disabled logging state and make it available to active agent execution. |
| `crates/themion-cli/src/tui.rs` and related local command handling | Add `/debug api-log enable` and `/debug api-log disable` handling for the active session and acknowledge the current state. |
| system temp dir (`/tmp/themion/` on Unix-like systems) | Runtime-created debug artifact root for session-scoped API call traces; not source-controlled. |
| `docs/architecture.md` / `docs/engine-runtime.md` | Document the session-scoped logging behavior, round-based file layout, and `/debug ...` slash-command control surface once implemented. |
| `docs/README.md` | Add and later maintain the PRD entry/status for this feature. |

## Edge Cases

- enable logging midway through a session after earlier turns already ran via `/debug api-log enable` → verify: only later rounds create artifacts, and prior turns remain absent.
- disable logging after several logged rounds via `/debug api-log disable` → verify: new provider rounds stop creating files immediately for that session.
- one turn triggers multiple tool-call rounds → verify: each round gets its own `round_<n>.json` file under the same turn directory.
- a provider call fails with an HTTP or structured API error → verify: the round artifact still captures request attribution plus explicit failure metadata when possible.
- logging output directory creation fails or the filesystem is read-only → verify: the session continues and surfaces a bounded warning instead of crashing.
- enter an invalid slash-command argument such as `/debug api-log maybe` → verify: Themion keeps the current session state unchanged and shows clear usage feedback.
- two sessions run concurrently with logging enabled → verify: artifacts remain separated by `session_id` and do not collide.
- a developer enables logging in one session but not another in the same process lifecycle → verify: only the enabled session writes artifacts.
- streamed responses arrive incrementally → verify: the final round artifact captures the accumulated response in a stable completed shape rather than many partial files.

## Migration

This feature is additive and developer-facing.

- no SQLite schema migration is required if the logging state remains process-local or session-runtime-local
- no existing history rows need backfill
- cleanup of system-temp `themion/` artifacts can remain manual in this slice
- if later work adds persisted session debug settings, that should be treated as a follow-on behavior change rather than implied here

## Testing

- start a session with logging disabled and run one prompt → verify: no system-temp `themion/<session_id>/...` round artifact is created.
- enter `/debug api-log enable` and run one prompt with no tool calls → verify: a system-temp artifact such as `/tmp/themion/<session_id>/<turn>/round_1.json` is created with request, response, and round attribution fields.
- keep logging enabled and run a prompt that triggers tool use plus another model round → verify: the same turn directory contains `round_1.json` and `round_2.json` with increasing round numbers.
- enter `/debug api-log disable` and run another prompt in the same session → verify: later turns stop creating new round artifacts immediately.
- enter an invalid command such as `/debug api-log maybe` → verify: Themion shows usage or validation feedback and does not change the current session logging state.
- trigger a provider/backend failure while logging is enabled → verify: the corresponding `round_<n>.json` file records explicit failure metadata without crashing the session.
- run two sessions concurrently, enabling logging in only one → verify: only the enabled session writes artifacts under its own `session_id` directory.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: the default build compiles cleanly with the new session-scoped logging path.
- run `cargo check --all-features -p themion-core -p themion-cli` after implementation → verify: feature-enabled builds still compile cleanly, including backend-specific logging paths.

## Implementation checklist

- [x] define the session-scoped API call logging state and thread it through active session execution
- [x] add `/debug api-log enable` and `/debug api-log disable` handling for the active session
- [x] provide clear acknowledgement and invalid-usage feedback for the `/debug api-log ...` command without affecting other sessions
- [x] define one backend-agnostic round-trace shape that can capture request, response, status, timing, and failure metadata
- [x] capture one trace artifact per provider round in the harness loop
- [x] write enabled-session artifacts under the system temp root, typically `/tmp/themion/<session_id>/<turn>/round_<n>.json` on Unix-like systems
- [x] ensure failure cases still write useful round artifacts when possible
- [x] keep logging-write failures non-fatal to normal session execution
- [x] document the `/debug api-log ...` command and file layout in runtime/architecture docs when implementation lands
- [x] update `docs/README.md` and this PRD status/notes when implementation lands
