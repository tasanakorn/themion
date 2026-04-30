# PRD-078: Idle Watchdog for Background Agent Follow-Up and Pending Board-Note Injection

- **Status:** Implemented
- **Version:** v0.50.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-30

## Implementation status

Landed in `v0.50.0` as a watchdog-driven follow-up refinement for Stylos-enabled local board-note workflows. The shipped behavior moves pending board-note trigger decisions off the TUI tick path into a background watchdog task, tracks shared busy/idle watchdog state, preserves `type=stylos_note` prompt recognition, adds clearer watchdog-authored prompt wording for watchdog-triggered note turns, and makes watchdog-triggered handling visible in the TUI transcript/status path.

## Summary

- Themion currently picks up pending board notes from a TUI tick path. That works for basic note intake, but it keeps the trigger tied to terminal polling and makes future automatic follow-up behavior harder to grow cleanly.
- This PRD adds an idle watchdog: a background runtime service that notices when the interactive agent has stayed idle long enough and can ask it to resume deferred work.
- The first watchdog action is pending board-note injection after a configurable idle delay, with a default of 2000 ms.
- When watchdog work is pending or injected, the TUI should make that reason visible so the user can distinguish plain idleness from automatic follow-up.
- The injected prompt should read like a clear local instruction, explicitly telling the agent that a pending note was found and showing the note below while preserving the metadata needed by existing board workflow logic.

## Goals

- Move idle-triggered follow-up orchestration out of the TUI tick loop and into a self-contained background runtime task.
- Introduce a reusable watchdog mechanism that can grow beyond board-note injection without requiring each future automation to live in UI-specific code.
- Automatically inject pending board-note work when the interactive agent has remained idle past a configurable threshold.
- Make watchdog-triggered work visible in the TUI so the user can understand why an idle agent resumed work.
- Preserve current board ordering, note-state transitions, and done-mention behavior while changing only how pending work gets resumed.

## Non-goals

- No redesign of the board-note data model, board columns, or done-mention semantics.
- No requirement to implement a broad plugin framework for every future watchdog action before the first board-note use case lands.
- No requirement to auto-run follow-up work while the agent is actively busy, waiting on model output, or processing a current turn.
- No requirement in this PRD to add a full user-facing settings surface for the idle delay if a narrow config/runtime default is sufficient for the first version.
- No requirement to change one-shot non-interactive execution paths unless later implementation review finds that they already share the same runtime hooks cleanly.

## Background & Motivation

### Current state

Themion's current pending board-note pickup path is TUI-driven.

`docs/architecture.md` and `docs/engine-runtime.md` already describe that in Stylos-enabled builds, board-note coordination lives in `crates/themion-cli/src/board_runtime.rs`, while `tui.rs` decides when to invoke that helper and submits the returned prompt into the local agent flow. The trigger today is a periodic TUI tick: once the UI sees that the agent is not busy, it asks for the next pending note and submits it immediately.

Current implementation details confirmed in code:

- `App::handle_tick_event(...)` in `crates/themion-cli/src/tui.rs` calls `maybe_inject_pending_board_note(...)`
- that path stops immediately if `self.agent_busy` is true
- idle timing currently exists only as TUI-local state through `idle_since: Option<Instant>` and `agent_activity: Option<AgentActivity>`
- incoming remote or note-driven work already enters the local turn path through `AppEvent::IncomingPrompt(IncomingPromptRequest)`
- note-driven completion follow-up currently recognizes injected note turns by checking whether `IncomingPromptRequest.prompt` starts with `type=stylos_note `
- note prompt construction currently happens in `build_board_note_prompt(...)` in `crates/themion-cli/src/stylos.rs`

That implementation is sufficient for the first board workflow, but it has four practical limits:

- the trigger is anchored to terminal polling instead of a reusable runtime background service
- the only current automation is pending-note pickup, even though the same pattern is likely useful for other self-follow-up behaviors
- the idleness signal needed for this behavior is trapped in TUI-local state rather than exposed as a runtime-owned coordination surface
- the injected prompt is metadata-first and transport-shaped rather than a clear "I found pending work for you" instruction

The repository architecture already reserves a background runtime domain for lower-priority maintenance work. That makes a watchdog-style service a better long-term fit than continuing to grow TUI-owned polling logic.

### Why this should become a watchdog

The requested behavior is broader than board-note injection alone. A background agent helper that notices idle time, evaluates follow-up conditions, and resumes the agent only when appropriate is a reusable product concept. Board-note injection is the first concrete policy that should use it, not the last reason the mechanism exists.

This PRD therefore keeps the product framing broad enough to support future watchdog actions while keeping the first implementation slice narrow and concrete.

## Design

### 1. Introduce a CLI-owned background watchdog service with core-owned policy helpers

The first implementation should run a long-lived watchdog task on the existing CLI background runtime domain, while keeping reusable watchdog policy and prompt-shaping logic in `themion-core` where practical.

Required behavior:

- a long-lived watchdog task must be started for interactive TUI sessions when the relevant runtime support is available
- the task must run on the existing CLI `background` runtime domain rather than on the TUI tick loop
- the watchdog must observe shared runtime activity state instead of inferring idleness from redraw timing
- `themion-core` should own reusable watchdog policy helpers, decision rules, or prompt-shaping support where that can be done without taking a direct dependency on TUI presentation types
- `themion-cli` should own task startup/shutdown, local DB/runtime wiring, and user-facing event delivery

Rationale:

- the current explicit Tokio runtime topology lives in `themion-cli`
- long-lived session tasks such as Stylos bridges are also started from the CLI layer
- `themion-core` does not currently own the process runtime lifecycle or TUI event channels

This boundary keeps implementation realistic for the current codebase while still moving policy out of `tui.rs` and avoiding a TUI-owned watchdog.

**Alternative considered:** require the very first implementation to live entirely inside `themion-core`. Rejected: current runtime/task ownership and event-channel startup are CLI-local, so that requirement would force a larger architectural move than the requested feature needs.

### 2. Add an explicit shared activity snapshot for watchdog decisions

The watchdog must not read `App` fields directly from the TUI loop. It needs a small shared state surface that expresses whether the interactive execution path is idle, when it became idle, and whether a watchdog injection is already queued or active.

Required behavior:

- introduce a shared runtime state object accessible from both the TUI/app path and the background watchdog task
- that shared state must include, at minimum:
  - whether the interactive execution path is currently busy
  - the last activity status or equivalent active/idle phase
  - the timestamp of the most recent transition into idle, in milliseconds
  - whether an injected incoming prompt is already active for the current turn path, or an equivalent signal that prevents duplicate watchdog submission
- the idle-transition timestamp should be updated when the app transitions from active work into idle, not recomputed from polling gaps
- the shared timestamp should use milliseconds so it matches existing repository guidance for machine-consumed status timing
- the existing TUI-local `idle_since: Option<Instant>` may remain for rendering convenience, but watchdog decisions must rely on the shared runtime state rather than on TUI-only fields

Implementation shape is intentionally not over-specified beyond the minimum observable state above. The feature may use an `Arc<Mutex<_>>`, atomics plus a small struct, or another equivalent thread-safe local runtime state surface.

**Alternative considered:** let the watchdog read `App` directly or keep its own inferred idle timer. Rejected: that would either couple the watchdog back to the UI layer or risk divergent activity semantics.

### 3. Use an explicit watchdog request path into the existing prompt intake flow

The watchdog should not call `submit_text(...)` directly from the background task and should not invent a separate execution path. It should hand off a typed request into the same intake flow already used for remote prompt delivery.

Required behavior:

- the watchdog must enqueue work back into the app through an explicit app event or equivalent request bridge
- the preferred first implementation is a new `AppEvent` variant dedicated to watchdog injection or a reused `IncomingPromptRequest` delivery path with explicit watchdog source metadata
- the TUI/app layer must remain the single place that converts that request into `self.active_incoming_prompt = Some(...)` plus `submit_text(...)`
- the app event path must preserve transcript visibility so the UI can show that the watchdog triggered the action
- if the watchdog request arrives after the app became busy again, the app must discard or defer it safely instead of starting a competing turn

This keeps all automatic work entering the existing local turn path instead of creating a shadow execution path.

**Alternative considered:** have the watchdog call directly into the current `maybe_inject_pending_board_note(...)` code path. Rejected: that would preserve the same TUI-coupled control flow under a different caller.

### 4. Trigger pending board-note injection only after sustained idle time

Pending board-note pickup should no longer happen immediately on the first idle tick. Instead, the watchdog should wait until the agent has been idle long enough that background work is appropriate.

Required behavior:

- the watchdog should consider pending board-note injection only when the shared runtime state says the interactive agent is idle
- the default idle threshold must be 2000 ms
- the threshold must be measured from the recorded most recent transition into idle
- while the agent is busy, streaming, or otherwise in an active turn, the watchdog must not inject a pending note
- once a note is injected, normal existing board workflow rules continue to apply
- the implementation must avoid repeatedly reinjecting the same note during the same idle period after the first injection decision
- after a watchdog-injected turn completes, any further pending note should require a fresh idle period before the next automatic injection

This delay makes the behavior feel intentional rather than twitchy and avoids immediately interrupting short quiet gaps between nearby user actions.

**Alternative considered:** inject the next pending note as soon as the agent becomes idle. Rejected: that recreates the current eager behavior and gives the user less visible idle time before automatic follow-up starts.

### 5. Preserve current pending-note selection and note-state mutation rules

The watchdog changes when note work is resumed, not which note is chosen or how note state transitions work after selection.

Required behavior:

- pending-note selection order must remain consistent with current DB policy in `DbHandle::next_board_note_for_injection(...)`
- pending `in_progress` notes must continue to be preferred before pending `todo` notes
- within the same column priority, the oldest matching pending note must continue to win
- selected notes must still be marked injected through the existing note-state mutation path or an equivalent mutation with the same externally visible result
- no new note column semantics are introduced by this PRD

This keeps the product change focused on idle automation and visibility rather than altering board scheduling rules.

**Alternative considered:** revise note ordering while introducing the watchdog. Rejected: that would mix separate product changes and make behavior changes harder to review.

### 6. Keep note-driven completion follow-up detection stable

The existing completion follow-up path depends on recognizing note-driven injected work by prompt shape. The watchdog prompt rewrite must preserve a stable machine-recognizable marker.

Required behavior:

- a watchdog-injected board-note prompt must still begin with the `type=stylos_note` header line so existing prompt recognition continues to work
- the prompt may add clearer watchdog instruction text after that header block, but it must preserve parseable note metadata such as `note_id`, `note_slug`, `note_kind`, and `column`
- `resolve_completed_note_follow_up(...)` and related logic must continue to be able to recognize note-driven work without depending on a human-only transcript event line
- if implementation chooses to add an extra structured marker such as `trigger=watchdog`, that marker must be additive rather than replacing the existing `type=stylos_note` identity

This is the main place where implementation must avoid a hidden assumption break: the prompt can become more readable, but it cannot stop being recognizable as note-driven work.

**Alternative considered:** replace the existing note header with a purely natural-language prompt. Rejected: that would silently break existing completion follow-up logic or force an unnecessary parallel detection mechanism.

### 7. Use a clearer watchdog-authored human instruction block in the injected prompt

The prompt should read like an explicit local instruction from the watchdog, not like a raw transport envelope.

Required behavior:

- after the structured `type=stylos_note ...` header line, the prompt should include a human-readable watchdog instruction block
- that block should start with wording equivalent to: `I found that you have a pending note to handle. Below is that note.`
- the exact sentence may be polished for grammar, but the resulting prompt must explicitly say that pending note work was found and that the note content follows
- the existing note-purpose guidance may remain, but the watchdog instruction must appear before the note body so the reason for the turn is immediately clear
- the note body should remain clearly separated below the instruction so the agent can act on it directly

A compliant example shape is:

```text
type=stylos_note note_id=... note_slug=... note_kind=work_request ... column=todo

I found that you have a pending note to handle. Below is that note.

<existing note-purpose guidance or equivalent workflow guidance>

Note body:
<body>
```

The implementation may refine wording, but it must preserve this overall structure: machine-readable note header first, then watchdog explanation, then actionable note content.

**Alternative considered:** keep the current metadata-first prompt body unchanged. Rejected: it does not clearly explain why the agent resumed work on its own.

### 8. Surface watchdog state explicitly in the TUI

When the watchdog finds follow-up work, the TUI should expose that state clearly rather than making the resulting turn look like unexplained spontaneous activity.

Required behavior:

- the transcript or event area must include a concise visible event when the watchdog injects work
- the status surface should distinguish plain idle from idle-with-watchdog-pending-work when that information is available before injection
- if pre-injection pending visibility is not available in the first shipped slice without adding excessive complexity, the implementation may ship injection-time visibility first, but the transcript event is mandatory
- visible wording must use the term `watchdog` so the source of the action is explicit
- remote Stylos-delivered note events and local watchdog-triggered note events must remain distinguishable in transcript output

Minimum acceptable visible event example:

- `Watchdog injected board note <note_slug> after idle timeout`

This PRD does not require one exact statusline string, but it does require a stable user-visible explanation path.

**Alternative considered:** rely only on existing generic remote-event log lines after injection. Rejected: they do not clearly explain that an idle watchdog noticed pending work and intentionally resumed the agent.

### 9. Keep the first implementation scoped to Stylos-enabled local board-note automation

This PRD should be implementation-ready without pretending to solve every runtime mode at once.

Required behavior:

- the first implementation only needs to activate board-note watchdog behavior in builds where the `stylos` feature is enabled and local board-note support is present
- in builds without `stylos`, the watchdog infrastructure may be absent or may run with no board-note policy, but it must compile cleanly and avoid user-visible noise
- the implementation must guard any feature-gated references consistently so always-on code paths do not depend on Stylos-only types such as `IncomingPromptRequest`

This keeps the first shipping slice coherent with the current location of board-note functionality.

**Alternative considered:** require identical watchdog behavior across all builds in the first slice. Rejected: board-note prompt types and note intake wiring are already Stylos-gated, so that would overstate the currently available cross-build surface.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core` watchdog support | Add reusable watchdog policy helpers and any shared prompt-shaping support that does not depend on TUI presentation types. |
| `crates/themion-core` board/prompt support or shared helpers | Preserve the structured note header contract while supporting clearer watchdog instruction text in injected note prompts. |
| `crates/themion-cli/src/app_state.rs` or adjacent runtime wiring | Start and own the long-lived watchdog task on the background runtime, and hold the shared runtime state or event bridge it needs. |
| `crates/themion-cli/src/tui.rs` | Stop using TUI tick polling as the policy trigger for pending-note injection, publish shared activity/idle state, accept watchdog-triggered requests, and render watchdog transcript/status feedback. |
| `crates/themion-cli/src/board_runtime.rs` | Refactor current pending-note selection/mutation logic as needed so it can be invoked by watchdog-driven request building instead of direct TUI tick polling. |
| `crates/themion-cli/src/stylos.rs` | Update `build_board_note_prompt(...)` or adjacent prompt helpers so watchdog-injected note prompts remain `type=stylos_note`-recognizable while adding clearer human instruction text. |
| `docs/architecture.md` | Update runtime/task architecture notes so board-note idle follow-up is described as watchdog/background-runtime behavior rather than TUI polling. |
| `docs/engine-runtime.md` | Document the shared activity-state boundary and watchdog request handoff into the existing prompt intake path. |
| `docs/README.md` | Track this PRD entry and status. |

## Edge Cases

- the user sends a new prompt just before the idle threshold expires → verify: the watchdog does not inject pending note work into the now-active turn.
- multiple pending notes exist while the agent becomes idle → verify: selection order remains consistent with current board policy, with `in_progress` preferred before oldest `todo`.
- a note is injected and the agent finishes quickly while another pending note still exists → verify: the watchdog waits for a fresh idle period before injecting the next note instead of chaining immediately with no idle gap.
- the app is running with Stylos support disabled → verify: watchdog board-note logic remains inactive or cleanly no-ops without noisy errors.
- the TUI is open but the agent is idle with no pending notes → verify: no watchdog-specific pending-work indicator or transcript event appears.
- the agent is idle for a long time and the same note was already injected → verify: the watchdog does not repeatedly inject duplicate work for the already-injected note.
- completion follow-up for an injected note still needs to emit a done mention → verify: the changed prompt wording does not break note completion detection or done-mention generation.
- the user manually resumes work before the watchdog injects → verify: the watchdog abandons that pending idle decision and waits for a later fresh idle period if follow-up work still exists.
- a watchdog request races with a remote incoming prompt → verify: the app accepts at most one turn start and the losing request is safely rejected or retried according to the chosen intake policy.

## Migration

This feature requires no database migration.

Rollout guidance:

- keep current board-note workflow semantics, selection order, and note state transitions intact
- shift the injection trigger from TUI polling to background watchdog orchestration
- land the clearer prompt wording and TUI visibility as part of the same user-facing feature so the automation feels understandable on first use
- document the feature-gated runtime boundary clearly so non-Stylos builds do not appear partially broken

## Testing

- leave one pending `todo` board note for the local interactive agent and keep the agent idle for more than 2000 ms → verify: the watchdog injects the note automatically after the idle delay rather than immediately.
- leave the agent idle with no pending notes for longer than 2000 ms → verify: no injected turn starts and no false pending-work indicator appears.
- leave an `in_progress` note and a newer `todo` note pending, then wait for idle injection → verify: the `in_progress` note is selected first.
- let the agent become idle for less than 2000 ms, then submit a user message → verify: no watchdog injection fires into the user-driven turn.
- trigger watchdog note injection and observe the transcript/event area → verify: a watchdog-specific visible event is emitted.
- if the first implementation includes pre-injection pending indication, leave a pending note while the agent is idle before timeout expiry → verify: the status surface distinguishes plain idle from idle with watchdog-pending work.
- inspect the injected prompt content in logs or transcript tooling → verify: it begins with `type=stylos_note`, includes the watchdog explanation block, and retains required note metadata.
- complete a watchdog-injected work-request note → verify: existing done-mention follow-up behavior still works.
- run the app with Stylos disabled → verify: watchdog board-note logic remains inactive or cleanly no-ops without user-visible noise.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check --all-features -p themion-core` after implementation → verify: the core crate builds cleanly across features.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the CLI crate still builds with Stylos enabled.
- run `cargo check -p themion-cli --all-features` after implementation → verify: the CLI crate still builds cleanly across feature combinations.

## Implementation checklist

- [x] add a long-lived watchdog task on the CLI background runtime for interactive-session idle follow-up work
- [x] add a shared runtime activity-state surface with an idle-transition timestamp in milliseconds for watchdog decisions
- [x] replace TUI tick-owned pending-note injection policy with watchdog-driven request generation
- [x] enforce a default 2000 ms idle delay before automatic pending-note injection
- [x] preserve `type=stylos_note` prompt recognition while adding a clearer watchdog explanation block
- [x] add watchdog-specific transcript visibility and, when practical, status-surface visibility
- [x] confirm completion follow-up and done-mention behavior still works with the new prompt shape
- [x] update architecture/runtime docs and PRD status/index references
