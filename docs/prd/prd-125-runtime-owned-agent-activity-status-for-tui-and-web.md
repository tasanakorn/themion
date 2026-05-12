# PRD-125: Runtime-Owned Agent Activity Status for TUI and Web

- **Status:** Implemented
- **Version:** v0.77.1
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-12

## Summary

- Themion sometimes keeps showing `⠦ preparing request…` in the TUI even after the agent is already idle.
- The status line can also keep showing `agent: preparing` from stale presentation state.
- This PRD makes per-agent activity status runtime-owned in app-state and treats UI-local state as render-only.
- TUI and Web UI must read the same current activity truth from app-state snapshots or notifications.
- When the shared runtime state says an agent is idle, preparing animation and preparing labels must stop immediately.

## Problem

The TUI sometimes keeps animating `⠦ preparing request…` even when the target agent is already idle.

The same class of stale state can appear in the status line as `agent: preparing`.

This is a bug because the I/O layer is showing activity that no longer matches the real agent state.

## Scope

In scope:

- TUI preparing animation logic
- status-line agent activity text such as `agent: preparing`
- runtime/app-state ownership of current agent activity status
- status propagation to TUI and Web UI from the same runtime-owned source of truth
- docs that describe the ownership rule

Out of scope:

- redesigning the full transcript UI
- changing model/provider request semantics
- adding new activity kinds unless needed to preserve current meaning
- changing unrelated busy/queue behavior beyond what is needed to fix stale status ownership

## Current behavior

Current symptoms:

- the TUI may keep showing `⠦ preparing request…` after the agent has already returned to an idle state
- the status line may still show `agent: preparing`
- presentation can stay stale because the animation/display logic is not fully governed by the same runtime-owned agent status that should describe real activity

This conflicts with the repository ownership rules already documented in `AGENTS.md` and `docs/engine-runtime.md`:

- TUI is for input and display, not runtime truth
- app-state/runtime owns agent registry, scheduling, and shared status snapshots
- multiple surfaces should read the same current runtime state

## Expected behavior

Required behavior:

- the source of truth for current per-agent activity state must live in runtime/core or app-state, not in TUI-local animation, transcript, or input/output flags
- if the runtime-owned status for an agent is `idle`, TUI must not keep rendering `preparing request…` for that agent
- if the runtime-owned status for an agent is `idle`, status-line text such as `agent: preparing` must clear or change to the correct idle representation in the same refresh path
- TUI and Web UI must consume the same current agent-activity truth from app-state snapshots or app-state-published notifications
- one UI surface must not invent, cache, or preserve a local activity state that disagrees with runtime truth
- preparing visibility must be derived from explicit shared activity state, not from a heuristic like recent submit, pending input, incomplete transcript output, or missing final render cleanup
- the fix must preserve current active-state rendering when the agent is genuinely preparing, streaming, waiting on tools, or otherwise active

## Fix approach

### 1. Canonical activity owner

Implementation must use one canonical per-agent activity state owned by runtime/app-state.

Required behavior:

- `themion-core` may emit lifecycle or activity events, but it must not be the direct UI state store
- `themion-cli` app-state must own the current per-agent activity snapshot that UI surfaces read
- the canonical shared state must be keyed by local agent identity, not by one global preparing flag
- preparing, streaming, tool-wait, finishing, and idle states must all come from that same shared source when those states are shown in UI

This PRD does not require inventing a new status model if an existing runtime/app-state activity enum or snapshot already exists. It does require making that existing shared state authoritative.

### 2. Idle transition must clear preparing truth

The stale-spinner bug means the current flow does not reliably clear preparing state when the agent becomes idle.

Required behavior:

- every runtime path that transitions an agent back to ready/idle must update the canonical shared activity state for that same agent
- after that update, the shared state for that agent must no longer report `preparing`
- no TUI-local cleanup step may be required to make the idle state true
- if runtime emits a `ready`, `idle`, `turn_done`, or equivalent completion signal, app-state must translate that signal into the canonical non-preparing state before UI rendering reads it

### 3. TUI must render from shared activity state only

TUI must not keep its own independent preparing truth.

Required behavior:

- the `⠦ preparing request…` animation must render only when the shared runtime/app-state activity state for the displayed agent is a preparing state
- the status-line text such as `agent: preparing` must be derived from that same shared state
- submit-time local flags, transcript progress, pending output expectations, or animation timers may control presentation details only; they must not decide whether the agent is preparing
- any existing TUI-local preparing flag or fallback that can outlive runtime truth must be removed or reduced to a pure derived presentation helper fed only by shared state

### 4. Web UI must use the same source of truth

Web UI activity display must stay aligned with the same shared status ownership model.

Required behavior:

- if Web UI already renders per-agent activity from app-state snapshots, preserve that path and keep it authoritative
- if any Web-specific fallback or reconstruction logic exists for preparing state, it must not override shared runtime truth
- TUI and Web UI must be able to show the same agent as idle at the same time from the same underlying runtime snapshot

### 5. Surface-neutral propagation

The status pipeline must stay UI-neutral.

Required behavior:

- app-state must publish current per-agent activity in a form that both TUI and Web UI can query or receive by notification
- do not move activity ownership into `tui.rs`, `tui_runner.rs`, or a Web-only render path
- do not make one surface rebuild runtime truth from transcript side effects or provider I/O timing

### 6. Keep current activity meaning where correct

This bug fix is about stale ownership, not about redefining all activity labels.

Required behavior:

- keep the current meaning of preparing, streaming, waiting, and idle unless a code-level fix needs one small naming cleanup to preserve consistency
- do not regress legitimate preparing visibility during real request preparation
- do not hide real active states just to remove the stale spinner

## Risks / edge cases

- an agent transitions quickly from preparing to idle → verify: the UI does not leave a stale spinner behind
- one agent is active while another is idle → verify: one agent's preparing state does not leak into another agent's UI status
- a tool-call continuation changes activity rapidly → verify: preparing/active labels still reflect real runtime state without flicker from stale local state
- TUI and Web UI are both connected → verify: both surfaces converge on the same agent activity state
- feature-gated Stylos builds remain aligned with the same app-state ownership model → verify: status truth does not move into TUI in non-Stylos or Stylos-enabled builds

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/app_state.rs` | Make app-state own the canonical current per-agent activity snapshot or equivalent shared status view consumed by UI surfaces. Ensure idle/ready transitions clear stale preparing state for the correct agent. |
| `crates/themion-cli/src/app_runtime.rs` | Route agent lifecycle/activity transitions through app-state updates so preparing and idle state changes remain coherent across turn start, active execution, and completion. |
| `crates/themion-cli/src/tui.rs` | Remove or narrow any TUI-local preparing-state ownership and render spinner/status-line activity only from shared runtime/app-state status. |
| `crates/themion-cli` web-status/event path | Keep Web UI activity rendering aligned with the same shared per-agent status source and prevent Web-specific preparing fallback from outliving runtime truth. |
| `crates/themion-core/src/agent.rs` or related activity-origin path | Preserve or emit lifecycle/activity transitions needed for app-state to maintain correct per-agent activity truth without making core the UI state owner. |
| `docs/engine-runtime.md` | Document that current per-agent activity state is runtime/app-state owned and that TUI/Web render from shared snapshots or notifications rather than local presentation heuristics. |
| `docs/README.md` | Track this PRD and later update status/version when implemented. |

## Risks / edge cases

- an agent transitions quickly from preparing to idle → verify: the UI does not leave a stale spinner behind
- one agent is active while another is idle → verify: one agent's preparing state does not leak into another agent's UI status
- a tool-call continuation changes activity rapidly → verify: preparing/active labels still reflect real runtime state without flicker from stale local state
- TUI and Web UI are both connected → verify: both surfaces converge on the same agent activity state
- feature-gated Stylos builds remain aligned with the same app-state ownership model → verify: status truth does not move into TUI in non-Stylos or Stylos-enabled builds
- an agent finishes with no final transcript output after preparation → verify: the idle transition still clears preparing state because it depends on runtime status, not output completion heuristics

## Implementation notes

- Landed in the existing `themion-cli` snapshot/status pipeline rather than by adding a separate status model. `AppRuntimeState` remains the runtime owner of live per-session activity, and `publish_runtime_snapshot` now derives the exported `AppSnapshot` activity status and primary activity label directly from current runtime state rather than from the previously published snapshot.
- `crates/themion-cli/src/tui.rs` now renders the top status-line activity text and the pending/spinner line from the shared `AppSnapshot` instead of from local `runtime.pending` / local status heuristics.
- Added a targeted regression test in `crates/themion-cli/src/app_state.rs` that proves snapshot publication clears stale exported `preparing` state when runtime truth is already idle.

## Validation

- reproduce the stale `⠦ preparing request…` case in TUI → verify: after the fix, the spinner stops when the agent becomes idle
- reproduce the stale `agent: preparing` status-line case → verify: after the fix, the label clears or changes when runtime status becomes idle
- exercise a normal active request lifecycle → verify: preparing still appears only while the shared runtime/app-state status says the agent is preparing
- exercise multiple local agents with mixed activity → verify: each surface shows per-agent status from shared runtime truth
- inspect TUI and Web UI behavior for the same session when possible → verify: both reflect the same activity transitions from the same underlying app-state status
- interrupt or complete a request soon after preparation begins → verify: the agent returns to idle state without a stale preparing indicator
- run `cargo check -p themion-cli` → verify: default CLI build compiles
- run `cargo check -p themion-cli --features stylos` → verify: Stylos-enabled CLI build compiles
- run `cargo check -p themion-core -p themion-cli --all-features` → verify: touched crates still build across feature combinations

## Implementation checklist

- [x] identify the canonical runtime/app-state source for current per-agent activity status and document which type/field is authoritative
- [x] ensure every runtime path that marks an agent ready/idle also clears preparing state in the shared status for that same agent
- [x] remove or constrain any TUI-local preparing-state ownership so it cannot outlive runtime truth
- [x] make TUI spinner and status-line activity render from the shared runtime-owned state only
- [x] keep Web UI activity rendering aligned with the same shared runtime-owned state and remove any stale local fallback if present
- [x] update `docs/engine-runtime.md` if implementation changes status flow details or clarifies ownership boundaries
