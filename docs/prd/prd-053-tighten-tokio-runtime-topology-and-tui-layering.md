# PRD-053: Tighten Tokio Runtime Topology Semantics and Remove Remaining TUI-Orchestration Leakage

- **Status:** Implemented
- **Version:** v0.34.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-24

## Summary

- Themion already has explicit runtime domains, but the current implementation still leaves some topology semantics and layering boundaries unclear.
- Align the documented TUI runtime shape with the actual code by documenting and keeping it accurately as a one-worker multi-thread runtime.
- Keep the explicit `tui`, `core`, `network`, and `background` domains, but reduce remaining ambient blocking and orchestration leakage inside `tui.rs`.
- Move non-visual runtime and Stylos coordination out of `tui.rs` into shared CLI application/runtime or controller code.
- Document the real process/thread model more explicitly, including the dedicated terminal input thread and blocking database work in core.
- Preserve the current single-process architecture and current user-visible modes rather than redesigning Themion into multiple processes or a new async stack.

## Goals

- Make runtime-topology documentation match the code that actually ships.
- Clarify that the `tui` runtime domain is semantically a one-worker multi-thread runtime, and keep code and docs aligned with that decision.
- Reduce architecture ambiguity by moving non-visual orchestration responsibilities out of `crates/themion-cli/src/tui.rs`.
- Make domain ownership easier to audit by reducing ambient `Handle::current()` and `block_in_place` patterns in TUI-owned code where a clearer layer-owned helper can be used instead.
- Keep the existing top-level layering intact: reusable harness behavior in `themion-core`, runtime/process wiring in `themion-cli`, and TUI presentation in `tui.rs`.
- Improve observability and debug understanding by documenting the actual relationship between Tokio runtime domains, dedicated OS threads, and `spawn_blocking` work.

## Non-goals

- No multi-process redesign.
- No rewrite away from Tokio.
- No redesign of `themion-core` harness semantics.
- No requirement to remove all blocking calls everywhere in one slice.
- No requirement to eliminate the dedicated terminal input thread if it remains the pragmatic choice for Crossterm event intake.
- No requirement to redesign the TUI widget tree or unrelated rendering behavior.
- No requirement to collapse runtime-domain ownership into `themion-core`.

## Background & Motivation

### Current state

Themion already shipped a meaningful runtime-domain split under PRD-050 and a shared CLI application-runtime direction under PRD-051.

Current documented architecture says:

- `themion-cli` owns runtime construction and mode-specific startup
- named runtime domains are `tui`, `core`, `network`, and `background`
- `tui.rs` should increasingly behave like presentation/input code rather than the application core

That direction is broadly sound, but the current repository still shows three remaining problems.

First, runtime-shape documentation is not fully aligned with code. `docs/architecture.md` and `docs/engine-runtime.md` describe the TUI runtime as a Tokio `current_thread` runtime, but `crates/themion-cli/src/runtime_domains.rs` currently builds the `tui` domain with `Builder::new_multi_thread().worker_threads(1)`. Those are not identical semantics, even if both often behave like one active worker in practice.

Second, `crates/themion-cli/src/tui.rs` still owns more non-visual orchestration than the intended architecture suggests. It does not only render and translate input; it also still coordinates parts of Stylos snapshot publication, runtime follow-up behavior, and other app-level transitions.

Third, the repository's effective thread model is more nuanced than the current docs imply. In addition to the named Tokio runtime domains, the TUI input path still uses a dedicated OS thread, and `themion-core` continues to use `spawn_blocking` for DB work. That is not inherently wrong, but it should be documented clearly so runtime debugging and architectural reasoning do not rely on an oversimplified mental model.

**Alternative considered:** treat the remaining mismatches as minor implementation details and leave the docs broad. Rejected: the current topology is important enough that small semantic mismatches now create avoidable confusion.

### Why this deserves its own PRD after PRD-050 and PRD-051

PRD-050 established explicit runtime-domain ownership. PRD-051 established that TUI presentation should not remain the de facto application core and introduced a shared CLI runtime layer.

This PRD is the follow-through step that tightens those two architectural moves without reopening their scope:

- it is not a replacement for PRD-050's runtime split
- it is not a replacement for PRD-051's shared-runtime direction
- it is the cleanup pass that makes their semantics and boundaries more honest and easier to maintain

**Alternative considered:** reopen PRD-050 or PRD-051 instead of creating a successor PRD. Rejected: the remaining work is concrete enough to deserve a focused follow-up PRD rather than broadening already-settled documents.

## Design

### Make the TUI runtime contract explicit and true

The repository should explicitly define the `tui` domain as a one-worker multi-thread runtime and align code, docs, and debug language with that decision.

The key requirement is that the runtime contract be exact rather than approximate. Docs should not call this runtime `current_thread`; they should describe the shipped shape precisely.

**Alternative considered:** switch the `tui` domain to a real Tokio `current_thread` runtime. Rejected for this slice: that would be a runtime-semantics change rather than a documentation-and-layering cleanup, and the safer path here is to document and preserve the existing one-worker multi-thread behavior.

### Keep runtime ownership CLI-local and domain names stable

The current `RuntimeDomain`, `DomainHandle`, and `RuntimeDomains` contract should remain the shared runtime-topology vocabulary inside `themion-cli`.

Normative direction:

- keep runtime construction in CLI startup code
- keep named domains explicit at the point of startup and long-lived task wiring
- keep domain handles cheap to pass through runner and controller boundaries
- avoid introducing ad hoc local runtime wrappers in unrelated modules

This keeps PRD-050's core design intact while tightening its semantics.

**Alternative considered:** move runtime topology into `themion-core` so all subsystems share one lower-level runtime layer. Rejected: runtime/process ownership is still a CLI concern.

### Continue reducing non-visual orchestration inside `tui.rs`

`crates/themion-cli/src/tui.rs` should keep moving toward a presentation-and-input boundary.

Normative direction:

- keep rendering, terminal event translation, and view-local state in `tui.rs`
- move Stylos snapshot-provider wiring and similar non-visual coordination into shared CLI runtime/controller code where practical
- move task-completion follow-up or similar app-policy behavior out of presentation-owned code where practical
- keep `tui_runner.rs` as terminal-mode orchestration and use shared runtime/controller helpers for non-visual state transitions that headless or future frontends could also own

The goal is not to make `tui.rs` tiny; the goal is to stop making it the easiest place for unrelated app orchestration to accumulate.

**Alternative considered:** leave the current ownership in `tui.rs` because the TUI is still the main frontend. Rejected: that keeps the architecture drifting back toward TUI-centralized control.

### Prefer explicit layer-owned helper paths over ambient blocking patterns

Some current TUI-owned code still reaches for ambient runtime state with patterns such as `tokio::runtime::Handle::current().block_on(...)` wrapped inside `block_in_place`.

Normative direction:

- when non-visual coordination needs async bridging, prefer helper APIs owned by the appropriate controller/runtime layer
- keep the callsite's domain ownership visible rather than depending on whatever runtime happens to be current
- treat remaining unavoidable blocking bridges as exceptions that should be documented and easy to locate

This does not require eliminating all `block_in_place` usage immediately. It does require making such crossings more intentional and less architecture-defining.

**Alternative considered:** ban `block_in_place` usage repository-wide. Rejected: too broad and not necessary for this architecture-focused slice.

### Document the real process and thread model, not just the domain names

The architecture docs should describe the runtime model as it actually exists today.

That means documenting at least:

- one Themion process
- named Tokio runtime domains
- that the TUI domain is a one-worker multi-thread runtime
- the dedicated terminal input OS thread used for Crossterm event polling
- `spawn_blocking` use for DB-sensitive work in `themion-core`
- how these pieces interact in `/debug runtime` reasoning

This makes the docs more useful for debugging and for future contributors trying to understand responsiveness or thread-level CPU behavior.

**Alternative considered:** keep docs focused only on the Tokio domains and ignore extra threads so the story stays simpler. Rejected: that simplicity is currently misleading.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/runtime_domains.rs` | Keep the `tui` runtime as a one-worker multi-thread runtime and make that contract exact in code-adjacent naming and documentation. |
| `crates/themion-cli/src/main.rs` | Keep thin mode selection and ensure startup language/comments continue to reflect the exact runtime contract. |
| `crates/themion-cli/src/tui_runner.rs` | Keep terminal-mode orchestration explicit and prefer pushing non-visual coordination into shared helpers rather than deeper into `tui.rs`. |
| `crates/themion-cli/src/tui.rs` | Remove or reduce remaining non-visual orchestration responsibilities such as Stylos snapshot publication or app-policy follow-up logic that do not belong to presentation/input ownership. |
| `crates/themion-cli/src/app_runtime.rs` | Grow or host shared CLI runtime/controller helpers that absorb non-visual orchestration currently leaking through `tui.rs`. |
| `crates/themion-cli/src/stylos.rs` | Keep network-domain ownership explicit while aligning snapshot-provider and coordination boundaries with the shared CLI runtime/controller layer. |
| `crates/themion-core/src/agent.rs` | No architecture rewrite required, but docs and debug reasoning should continue to account for existing `spawn_blocking` DB work when describing the overall thread model. |
| `docs/architecture.md` | Update the process/thread/runtime section so the TUI runtime shape and dedicated input thread are described accurately. |
| `docs/engine-runtime.md` | Update runtime-domain and thread-model language so it matches actual startup code and thread behavior. |
| `docs/README.md` | Add this PRD to the index table. |

## Edge Cases

- TUI runtime remains one-worker multi-thread → verify: docs and debug wording stop calling it `current_thread` and describe the actual semantics precisely.
- terminal input continues to use a dedicated thread → verify: docs describe it explicitly and shutdown still works cleanly.
- Stylos is enabled → verify: long-lived networking tasks remain on the `network` domain while snapshot publication and app-policy coordination no longer depend on `tui.rs` owning too much non-visual logic.
- Stylos is disabled → verify: runtime layering remains clean and no controller extraction accidentally hard-depends on Stylos-only types in always-on code.
- remaining `block_in_place` bridges are kept in touched code → verify: they are intentional, bounded, and easier to trace to the owning layer/domain.

## Migration

This PRD is an internal architecture-and-documentation cleanup slice.

Expected rollout shape:

- first align the TUI runtime contract between docs and code
- then move a small set of clearly non-visual responsibilities out of `tui.rs`
- then refresh architecture/debug documentation so the real thread model is easy to reason about

There is no database, protocol, or user-config migration required by this PRD.

## Testing

- start Themion in TUI mode after the runtime-contract change → verify: startup still enters the TUI path and the UI remains responsive to input, ticks, and redraw requests.
- run print mode and `--headless` mode after the cleanup → verify: non-TUI paths still execute correctly and do not depend on TUI-only orchestration.
- run with `--features stylos` after moving non-visual coordination out of `tui.rs` → verify: Stylos startup, status publication, incoming request handling, and shutdown still work.
- inspect `docs/architecture.md` and `docs/engine-runtime.md` against `runtime_domains.rs`, `tui_runner.rs`, and the TUI input path → verify: runtime and thread-model descriptions match the code exactly.
- inspect touched TUI-side async bridge paths → verify: domain ownership is clearer and any remaining blocking bridges are explicit and justified.

## Implementation checklist

- [x] decide that the `tui` domain remains a one-worker multi-thread runtime and document it precisely
- [x] align `crates/themion-cli/src/runtime_domains.rs` and related debug terminology with that exact TUI runtime decision
- [x] audit and extract a small first slice of non-visual orchestration from `crates/themion-cli/src/tui.rs`
- [x] add or expand shared CLI runtime/controller helpers in `crates/themion-cli/src/app_runtime.rs` or a successor shared module for those extracted responsibilities
- [x] review touched TUI-side `block_in_place` and ambient runtime crossings for clearer layer/domain ownership
- [x] update `docs/architecture.md` and `docs/engine-runtime.md` to describe the real process/thread/runtime model
- [x] keep `docs/README.md` aligned with the new PRD entry

## Implementation notes

Implemented in v0.34.0.

What landed:

- `crates/themion-cli/src/runtime_domains.rs` keeps the `tui` domain as a one-worker multi-thread runtime while keeping `core`, `network`, and `background` as explicit multi-thread domains
- `crates/themion-cli/src/app_runtime.rs` now hosts shared helpers for done-mention creation, reducing non-visual board-orchestration leakage from `tui.rs`
- `crates/themion-cli/src/tui.rs` now delegates done-mention creation through shared app-runtime helpers instead of embedding the full creation logic inline
- docs now describe the real thread model more explicitly, including the dedicated terminal-input OS thread and `spawn_blocking` DB work in `themion-core`

Follow-through that remains intentionally narrow:

- some TUI-side async/blocking bridge paths still exist and can be reduced further in later slices
- this PRD did not attempt a broad rewrite of all TUI-owned app logic
