# PRD-051: Separate Shared Application Runtime from TUI Presentation and Introduce Headless Mode

- **Status:** Implemented
- **Version:** v0.32.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-24

## Summary

- Themion's current CLI startup path still mixes presentation concerns with too much application/bootstrap logic.
- If headless operation is a real target, `tui.rs` must become an optional presentation/input adapter rather than the application core.
- Introduce one shared non-UI CLI application-runtime layer that both TUI mode and headless mode use.
- Keep `main.rs` thin, keep `tui_runner.rs` as the terminal-mode orchestrator, add a headless runner, and move non-UI session/agent/bootstrap logic out of `tui.rs` and duplicated mode-specific paths.
- Treat the current `app_runtime.rs` as overlapping scaffolding to remove or rewrite into the actual shared architecture layer.
- Use a minimal real explicit `--headless` mode as the architectural proof that TUI presentation is optional rather than foundational, distinct from the non-interactive one-shot prompt path.
- This PRD succeeds PRD-050 for app-layer architecture; PRD-050 remains focused on Tokio runtime-domain ownership.

## Goals

- Make `tui.rs` responsible only for terminal input/output and presentation concerns.
- Introduce a minimal real explicit `--headless` mode as part of this PRD so the shared application-runtime boundary is proven in practice.
- Ensure TUI mode and headless mode both use the same shared CLI-local application-runtime layer for project resolution, session setup, agent construction, and turn orchestration.
- Keep `main.rs` thin and mode-selecting rather than mode-implementing.
- Keep `tui_runner.rs` as the terminal-mode bridge between shared app/runtime logic and TUI presentation.
- Add a dedicated headless runner that uses the same shared app/runtime layer without depending on TUI-specific types.
- Remove duplicated bootstrap responsibilities from mode-specific entry paths.
- Remove or replace overlapping bootstrap scaffolding that duplicates the active path.
- Preserve existing Themion behavior while making alternate frontends and automation-friendly execution easier.

## Non-goals

- No rewrite of `themion-core` harness semantics.
- No multi-process architecture change.
- No rewrite away from Ratatui for the existing TUI.
- No requirement to design every future frontend in this PRD.
- No requirement to eliminate Tokio runtime domains introduced by PRD-050.
- No requirement to ship a fully featured alternative UX for headless mode beyond proving shared non-TUI runtime operation.
- No requirement to refactor unrelated UI rendering details.
- No requirement to redesign all current non-interactive CLI output formatting in the same slice.

## Background & Motivation

### Current state

The current active startup path is split in ways that blur architecture boundaries:

- `crates/themion-cli/src/main.rs` chooses print mode versus TUI mode
- `crates/themion-cli/src/tui_runner.rs` is the active TUI bootstrap/orchestration entrypoint
- `crates/themion-cli/src/tui.rs` still owns substantial non-UI application logic and state
- `crates/themion-cli/src/app_runtime.rs` exists, but it is not the active bootstrap path and overlaps with existing startup code

The current print path in `main.rs` also duplicates bootstrap responsibilities directly: project-dir resolution, DB opening, session creation, and agent construction are still done in that mode-specific path instead of through one shared CLI application layer.

The repository has already separated Tokio runtime-domain ownership under PRD-050, but it has not yet fully separated application logic from presentation or unified CLI bootstrap across modes.

### Why the current layering is a problem

If TUI is optional, then `tui.rs` should not remain the effective home for shared app/runtime behavior. Otherwise:

- a headless runner must either depend on TUI-shaped logic or duplicate it
- application bootstrap and agent setup remain coupled to terminal-specific types and flows
- print mode and TUI mode continue drifting into separate setup paths
- overlapping files such as `app_runtime.rs` continue to drift without becoming the real architecture
- future alternate frontends or non-interactive runners become harder to build cleanly

The current repository already shows this tension: `tui_runner.rs` is the real bootstrap path for TUI mode, but `tui.rs` still holds too much app-level responsibility, while print mode still performs its own setup inline in `main.rs`.

A minimal real headless mode belongs in this PRD because it provides a concrete success condition for the split. If TUI mode and headless mode can both run through the same shared runtime layer, then the architecture boundary is real rather than aspirational.

**Alternative considered:** keep `tui.rs` as the app layer and accept duplication for headless mode later. Rejected: this would make TUI the accidental architecture center and increase long-term coupling.

### Relationship to PRD-050

PRD-050 should remain about Tokio runtime-domain ownership only. This PRD takes over the next architecture question: how to structure the CLI so presentation is optional and shared application/runtime logic is reusable across TUI mode and headless mode.

**Alternative considered:** keep this work as a later section of PRD-050. Rejected: executor domains and app-layer ownership are separate design concerns.

## Design

### Keep `main.rs` thin and mode-selecting

`crates/themion-cli/src/main.rs` should remain a thin entrypoint.

Normative direction:

- parse args and config
- resolve the intended mode
- construct top-level mode dependencies such as runtime domains
- call a mode-specific runner
- avoid letting `main.rs` grow into the home for shared bootstrap logic

This keeps startup readable and prevents mode-specific logic from spreading across the entrypoint.

**Alternative considered:** move more logic back into `main.rs` because it already selects the mode. Rejected: that would recreate the same layering problem at a different file boundary.

### Introduce one real shared CLI application-runtime layer

`themion-cli` should gain one shared non-UI application-runtime layer used by both TUI mode and headless mode.

This layer should own behavior such as:

- project directory resolution
- DB opening and persistence setup
- session preparation and session-row insertion
- agent construction
- mode-independent command/turn orchestration helpers
- shared app state transitions that are not presentation-specific
- Stylos startup/wiring that is not inherently tied to Ratatui rendering

This layer should not own terminal drawing, keyboard handling, output formatting, or Ratatui-specific view state.

Naming direction:

- replace the current overlapping `app_runtime.rs` with one clearly active module
- prefer a name that implies shared CLI application/runtime ownership rather than TUI ownership
- `app_runtime.rs` may keep the final name only if it is rewritten into the actual shared layer; otherwise it should be replaced with a clearer module such as `app_core.rs`, `session_runtime.rs`, or `app_controller.rs`

The important requirement is one real shared layer, not another overlapping half-bootstrap file.

**Alternative considered:** keep `app_runtime.rs` as-is and gradually wire more code into it. Rejected: the current file is overlapping scaffolding, not yet a clean shared architecture contract.

### Define a concrete responsibility split

The implementation should aim for this boundary:

- `main.rs` — mode selection and top-level startup
- shared CLI app-runtime layer — project/session/bootstrap/orchestration not tied to presentation
- `tui_runner.rs` — terminal-mode lifecycle and bridging
- `headless_runner.rs` — headless lifecycle and non-TUI I/O bridging
- `tui.rs` — terminal presentation, input translation, and view-local state

This PRD is successful only if code ownership becomes more obvious after the split and both runners are visibly layered over the same shared runtime.

**Alternative considered:** rely on convention without writing down a concrete boundary. Rejected: the current overlap already shows that an unwritten boundary drifts.

### TUI becomes a presentation adapter

`crates/themion-cli/src/tui.rs` should become presentation-oriented.

Normative direction:

- keep terminal rendering in `tui.rs`
- keep terminal input translation in `tui.rs`
- keep view-specific state in `tui.rs`
- move non-UI bootstrap and orchestration out of `tui.rs`
- avoid making TUI-specific types the owner of behavior that headless mode also needs

Examples of TUI-local concerns:

- drawing widgets and layout
- terminal event translation
- scroll/focus/editing state
- presentation-only overlays and view state

Examples of behavior that should move out when practical:

- shared session/bootstrap helpers
- mode-independent agent construction
- shared Stylos startup/setup helpers
- non-visual app lifecycle coordination that does not require terminal state

**Alternative considered:** leave app state in `tui.rs` but expose some helper APIs for headless mode. Rejected: that still makes TUI the real application center.

### `tui_runner.rs` remains the terminal-mode orchestrator

`crates/themion-cli/src/tui_runner.rs` should remain the active terminal-mode runner, but its role should become clearer.

Normative direction:

- own terminal lifecycle and runner wiring
- bridge terminal events into shared app/runtime actions
- bridge shared app/runtime state/events into rendered TUI output
- avoid becoming a second full application core

This keeps one clear boundary: runner/orchestrator outside, presentation adapter in `tui.rs`, shared app/runtime below both.

**Alternative considered:** fold `tui_runner.rs` back into `tui.rs`. Rejected: that would blur the runner/presentation boundary again.

### Add a real headless runner over the same shared layer

This PRD should introduce a minimal real headless mode, not just mention it as a future possibility.

Normative direction:

- add a dedicated `headless_runner.rs` or equivalent runner module
- route headless mode through the same shared CLI application-runtime layer used by TUI mode
- avoid routing headless mode through TUI-only state or APIs
- keep the first headless slice minimal, but real enough to prove shared bootstrap/orchestration boundaries

Acceptable first-slice headless behavior may be limited to:

- starting a session
- constructing shared runtime state without TUI state
- starting shared networking/runtime services such as Stylos
- remaining available as a long-running non-TUI process

The existing prompt-argument path may remain as a separate non-interactive one-shot compatibility path routed through the same shared runtime layer.

This is the core reason for the split: the shared layer should define the app, while runners and adapters define how users interact with it.

**Alternative considered:** continue using the current print mode as an ad hoc special case and postpone headless architecture entirely. Rejected: this PRD exists to make the non-TUI path a real architectural peer instead of an accident.

### Current print mode should remain a separate non-interactive path, while headless mode is explicit

The existing print mode should not remain a separate bootstrap island, but it is distinct from explicit headless mode.

Normative direction:

- keep an explicit `--headless` mode for long-running non-TUI operation
- keep the current prompt-argument CLI surface as a separate non-interactive compatibility path
- route both through the same shared CLI app-runtime layer used by TUI mode
- do not require headless mode to depend on terminal-specific state or APIs

This turns the shared layer into a real architecture boundary rather than a TUI convenience wrapper, while keeping non-interactive scripting behavior distinct from long-running headless operation.

**Alternative considered:** leave print mode as a separate special case indefinitely. Rejected: that preserves exactly the duplication this PRD is meant to reduce.

### Stylos wiring should become mode-independent where practical

Stylos is a CLI-local concern, but it is not inherently a Ratatui concern.

Normative direction:

- move shared Stylos bootstrap/setup responsibilities into the shared CLI app-runtime layer where practical
- keep TUI-specific event bridging or presentation in the TUI runner/presentation layer
- allow headless mode to reuse shared Stylos setup without depending on TUI-local types

This matters because Stylos is one of the biggest examples of logic that currently sits near TUI mode but conceptually belongs to the broader CLI runtime.

**Alternative considered:** leave Stylos setup entirely attached to the TUI path. Rejected: that would make headless reuse awkward and would blur the shared-runtime boundary.

### Replace overlapping bootstrap scaffolding intentionally

`crates/themion-cli/src/app_runtime.rs` should not become permanent dead overlap.

Normative direction:

- either remove `app_runtime.rs` during implementation
- or rewrite it into the actual shared application-runtime layer with clear ownership
- do not keep two competing bootstrap concepts long-term
- document the chosen replacement clearly in docs and PRD notes

This keeps the architecture understandable and avoids preserving misleading scaffolding.

**Alternative considered:** leave `app_runtime.rs` in place indefinitely because it is harmless if unused. Rejected: unused overlapping architecture scaffolding increases confusion.

### Keep the split incremental and implementation-shaped

This architecture shift should land incrementally.

#### Phase 1: shared bootstrap foundation

Required outcomes:

- define the shared CLI application-runtime boundary
- move obvious shared bootstrap helpers out of `tui.rs` and `main.rs`
- stop duplicating project-dir, DB, session, and agent setup across TUI and print/headless paths where practical
- keep current user-visible behavior unchanged where possible

Likely early extraction targets:

- project-dir resolution helpers
- DB opening/session setup helpers
- session preparation helpers
- shared agent-construction helpers

#### Phase 2: headless runner and presentation cleanup

Required outcomes:

- introduce a minimal real headless runner over the shared layer
- reduce remaining app-level ownership inside `tui.rs`
- clarify `tui_runner.rs` as the terminal-mode bridge over the shared layer
- move shared Stylos startup/setup out of TUI-shaped ownership where practical
- remove or rewrite `app_runtime.rs` as the real shared layer

#### Phase 3: mode hardening

Candidate follow-up outcomes:

- make both runners rely primarily on the same shared layer
- reduce any remaining mode-specific bootstrap duplication
- extend headless behavior beyond the minimal proof-of-boundary slice if needed

This phasing keeps the architectural direction concrete without forcing a risky one-shot rewrite.

**Alternative considered:** do a full one-shot rewrite to the final layering. Rejected: too much surface area changes at once.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/main.rs` | Stay thin and mode-selecting; push shared bootstrap helpers into the shared CLI app-runtime layer instead of keeping separate mode-specific setup inline. |
| `crates/themion-cli/src/tui_runner.rs` | Remain the terminal-mode runner and bridge between shared app/runtime logic and TUI presentation. |
| `crates/themion-cli/src/headless_runner.rs` | Add a headless runner over the same shared app/runtime layer used by TUI mode. |
| `crates/themion-cli/src/tui.rs` | Shed non-UI bootstrap/orchestration responsibilities and keep terminal I/O and presentation concerns. |
| `crates/themion-cli/src/app_runtime.rs` | Remove or rewrite into the actual shared non-UI application-runtime layer; do not keep it as overlapping scaffolding. |
| `crates/themion-cli/src/<shared app-runtime module>` | Own project/session/bootstrap helpers, shared agent construction, and other mode-independent CLI runtime wiring. |
| `crates/themion-cli/src/stylos.rs` | Keep transport/runtime functionality, but allow shared startup/setup hooks to move behind the shared CLI app-runtime boundary where practical. |
| `docs/architecture.md` | Clarify the boundary between shared CLI app/runtime logic, runners, TUI presentation, and headless mode after implementation. |
| `docs/engine-runtime.md` | Update CLI-local runtime/app wiring docs so headless reuse and presentation boundaries are explicit. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Edge Cases

- TUI mode starts after the split → verify: terminal setup and rendering still work while shared app/runtime logic remains outside `tui.rs`.
- explicit `--headless` mode starts after the split → verify: it can initialize shared runtime state and long-lived network services without depending on TUI-only state, and it emits structured NDJSON lifecycle logs suitable for machine consumption.
- current print-mode behavior is preserved as a separate non-interactive path through the shared runtime layer → verify: the user-facing non-TUI path still works while bootstrap is no longer duplicated.
- Stylos startup is needed in both TUI and headless flows → verify: shared runtime wiring does not depend on Ratatui-only types.
- extraction moves agent/bootstrap helpers out of `tui.rs` and `main.rs` → verify: behavior stays the same and no duplicated setup path remains for the moved slice.
- overlapping scaffolding is removed or rewritten → verify: the repository no longer has two competing bootstrap concepts for the same CLI runtime layer.

## Migration

This is an internal CLI architecture migration with one externally meaningful result: a real explicit `--headless` mode exists alongside TUI mode and the separate non-interactive prompt path, with NDJSON lifecycle logging for headless operation.

Expected rollout shape:

- keep PRD-050 runtime domains in place
- introduce a shared non-UI CLI application-runtime layer
- route TUI mode through `tui_runner.rs` over that shared layer
- route headless mode through `headless_runner.rs` over that same shared layer
- make explicit `--headless` a long-running machine-oriented path with NDJSON lifecycle logging, while keeping prompt arguments as the separate one-shot non-interactive path
- move TUI-specific presentation concerns into `tui.rs`
- remove or rewrite overlapping bootstrap scaffolding once the shared layer is real

No database or wire-protocol migration is expected from this architectural split by itself.

## Testing

- start Themion in TUI mode after the first extraction slice → verify: TUI behavior still works and terminal rendering remains intact.
- start Themion with `--headless` after the first runnable slice → verify: project resolution, session setup, and long-running non-TUI runtime services work without TUI-specific state, with NDJSON lifecycle events written one per line.
- start Themion with prompt args after the first runnable slice → verify: the separate non-interactive path still executes one prompt through the shared runtime layer.
- inspect `tui.rs` after the refactor slice → verify: it mainly contains input/output and presentation-related logic rather than shared bootstrap/orchestration helpers.
- inspect `main.rs` after the refactor slice → verify: it remains thin and does not keep duplicated mode-specific bootstrap logic that belongs in the shared layer.
- inspect startup wiring after the refactor slice → verify: `tui_runner.rs` and `headless_runner.rs` both rely on the same shared app/runtime logic.
- inspect overlapping bootstrap files after the refactor slice → verify: `app_runtime.rs` is either removed or rewritten into the clearly active shared module.
- run `cargo check -p themion-cli` after implementation → verify: the default CLI build compiles cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the Stylos-enabled CLI build compiles cleanly.

## Implementation checklist

- [x] define the shared non-UI CLI application-runtime boundary in `themion-cli`
- [x] choose the real shared module name and remove or rewrite overlapping scaffolding accordingly
- [x] extract project-dir, DB, session, and agent/bootstrap helpers out of `tui.rs` and duplicated mode-specific paths in `main.rs` where they are not presentation-specific
- [x] keep `tui.rs` focused on terminal I/O, rendering, and view-specific state
- [x] keep `tui_runner.rs` as the terminal-mode orchestrator over the shared layer
- [x] add a minimal real `headless_runner.rs` or equivalent headless runner over the same shared layer
- [x] route the current non-TUI path through the shared bootstrap/runtime logic, either directly as headless mode or through a thin compatibility wrapper
- [x] move shared Stylos startup/setup behind the shared CLI app-runtime boundary where practical
- [x] remove or rewrite `app_runtime.rs` once the shared layer is established
- [x] update `docs/architecture.md` and `docs/engine-runtime.md`
- [x] update `docs/README.md` with this PRD entry
- [x] decide and apply the repository version bump if this PRD is implemented
- [x] check `Cargo.lock` after any version change
- [x] run `cargo check -p themion-cli`
- [x] run `cargo check -p themion-cli --features stylos`
