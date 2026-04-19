# PRD-006: Workflow and Phase Model for the Harness Engine

- **Status:** Implemented
- **Version:** v0.4.0
- **Scope:** `themion-core` (harness loop, turn lifecycle, persisted turn metadata, runtime state); `themion-cli` (status display and session wiring); docs
- **Author:** Tasanakorn (design) + Claude Code (PRD authoring)
- **Date:** 2026-04-19

> **Implementation note:** The landed implementation introduces explicit workflow runtime state and SQLite persistence for workflow/session/turn metadata, but it currently implements only the built-in default workflow path. The default workflow is `NORMAL`, with built-in phases `IDLE` and `EXECUTE`. Sessions begin in `IDLE`, switch to `EXECUTE` while a turn is actively running, and return to `IDLE` when that workflow completes for the turn. The TUI status line now renders `<profile> | <model> | <leaf_project_dir> | flow: <workflow> | phase: <phase> | agent: <agent_state>`, where the `agent:` field shows live runtime activity such as `idle`, `waiting-model`, or `running-tool` rather than a stable agent name. Treat the code and current docs as the source of truth where they differ from the original proposed wording.

## Goals

- Extend the current single-turn harness loop so the engine can support named workflows composed of one or more phases.
- Preserve the current behavior as the default `NORMAL` workflow, where one user input runs a single `EXECUTE` phase and the turn ends when the model finishes without further tool calls.
- Allow future workflows to advance from phase to phase within one logical turn until the workflow reaches an end phase.
- Allow workflow state to remain active across turns when the selected workflow requires multi-turn progression.
- Make workflow state and phase progression explicit runtime state rather than implicit loop behavior.
- Persist current workflow state to SQLite while the workflow is running so runtime and history views can reconstruct the active execution state.
- Keep reusable workflow execution logic in `themion-core`, with `themion-cli` limited to display and user-facing session wiring.
- Surface workflow-related runtime identity clearly in the TUI status line.

## Non-goals

- No commitment in this PRD to the exact set of non-`NORMAL` workflows beyond establishing the engine model needed to support them.
- No redesign of provider/backend request translation beyond what is required for the harness to issue repeated model calls across phases.
- No change to the current tool schema purely for workflow support.
- No full visual workflow editor or complex TUI controls for selecting arbitrary phases in the first version.
- No replacement of the existing turn/tool loop persistence model with a completely separate execution store unless required by implementation.

## Background & Motivation

### Current state

Themion currently treats a user submission as a single turn. Inside that turn, the harness may perform multiple model/tool round-trips, but the overall lifecycle is still single-phase:

1. user submits input
2. model streams a response
3. tools may run and the model may continue
4. the turn ends when the model returns a normal assistant response with no more tool calls

This behavior is documented in `docs/architecture.md` and `docs/core-ai-engine-loop.md`. The current loop has one implicit completion condition: the model is done responding for that user turn.

That is sufficient for straightforward assistant exchanges, but it makes it difficult to express structured multi-step behavior where the engine should move through explicit stages before the turn is considered complete. Examples include workflows where the first phase gathers context, the second phase performs execution, and the final phase produces a user-facing result.

Today, such structure can only be approximated by prompt wording inside one undifferentiated loop. That makes state transitions fragile, hides intent from the runtime, and gives the TUI and persistence layer no first-class way to show where the engine is in a longer-running logical turn.

### Need for persistent runtime workflow state

A workflow is not always bounded to one turn. Some workflows may remain active after one turn completes and continue on the next user turn from the current workflow state. That means the engine needs a persistent notion of current workflow state at the session level, not only a per-turn annotation after the fact.

Without persisted runtime workflow state:

- the engine cannot reliably resume the active workflow/phase on the next turn
- the TUI cannot show the current workflow/phase from durable state
- interrupted sessions lose the execution state that explains what the agent was doing
- later history inspection can see what happened in old turns but not necessarily what workflow state remained active afterward

The data model therefore needs both:

- per-turn workflow/phase records describing what happened inside a turn
- session-level runtime workflow state describing what is active now and what remains active across turns

### Status line motivation

The current architecture doc describes a status bar that emphasizes project, profile, model, and token/context counters. For workflow-aware execution, the most important live identity is no longer only which model is active, but also which workflow, phase, and agent are currently driving the turn.

A workflow/phase engine should therefore make that state directly visible in the TUI. This is especially important once one logical user turn may advance across several internal phases before completion, or a workflow remains active between turns.

### Why workflow/phase state belongs in the engine

Workflow and phase progression are runtime semantics, not just prompt text. They determine when a turn continues, when another model call should happen, whether a workflow remains active after the turn ends, and when the overall workflow is complete. That makes them a `themion-core` concern.

The CLI may display the active workflow and phase, but it should not own the progression logic.

## Design

### Workflow as a named execution policy

The harness should introduce an explicit workflow concept. A workflow defines how execution progresses and when it finishes.

Each turn executes within the current session workflow context. The initial implementation should support:

- `NORMAL` — the existing behavior, modeled explicitly as a workflow with a single execution phase and an explicit idle phase surrounding active work

A workflow should define:

- its stable identifier/name
- its ordered or rule-based phase progression
- its start phase
- its end condition
- whether the workflow is expected to end inside the current turn or may remain active across turns
- whether the next model request should continue automatically into the next phase or wait for a future user turn

This makes the current loop a special case of a more general execution model instead of a one-off path.

**Alternative considered:** encode workflow behavior only in prompt text and keep the runtime loop unchanged. Rejected: the runtime still needs to know whether to continue, end the turn, keep the workflow active for the next turn, or mark the workflow complete, and hidden prompt-only state would be brittle and hard to observe.

### Phase as explicit runtime state within and across turns

A phase is a named substage inside a workflow. During a turn, the harness should track the current phase explicitly. The current phase also belongs to the session's runtime workflow state, because some workflows may stay active across turns.

At minimum, runtime state should include:

- active workflow name
- current phase name
- workflow status such as `running`, `waiting_user`, `completed`, or `failed`
- whether phase advancement was caused by model completion, tool results, user input, or engine rules
- the turn sequence in which the current workflow state was last updated

For the built-in `NORMAL` workflow, the landed implementation currently uses:

- workflow: `NORMAL`
- built-in phases: `IDLE` and `EXECUTE`
- session/runtime state starts in `IDLE`
- workflow switches to `EXECUTE` when a turn begins
- workflow returns to `IDLE` when the `EXECUTE` phase completes with no pending tool calls
- no workflow state remains active for the next turn beyond the default `NORMAL` workflow returning to `IDLE`

This preserves existing semantics while giving the engine a uniform representation.

For future multi-phase workflows, the engine should be able to:

1. start in the workflow's start phase
2. execute the normal model/tool loop for that phase
3. evaluate whether the workflow should advance to another phase in the same turn
4. if another phase exists and should continue immediately, issue the next model request in the same logical turn
5. if the workflow should wait for future user input, keep the workflow active with its current phase/state in session runtime state
6. end the workflow only when it reaches a terminal phase and that phase completes

**Alternative considered:** treat every model/tool iteration as its own phase automatically. Rejected: tool-call iterations are already an implementation detail of a single conversational step; workflow phases should express higher-level engine states, not mirror every low-level loop cycle.

### Turn completion semantics

Turn completion should no longer be defined only as “the model stopped responding.” Instead, completion should depend on workflow state.

Proposed rules:

- a phase may complete when the model returns a non-tool-calling assistant response for that phase
- after phase completion, the workflow policy decides whether to end the turn, transition to another phase in the same turn, or stay active and wait for the next turn
- the turn ends when the current turn's work is done, even if the workflow remains active for a future turn
- the workflow ends only when the workflow reaches a terminal phase and that phase completes

For the landed `NORMAL` behavior, this reduces to the current single active execution phase surrounded by idle state:

- `IDLE` before work begins
- `EXECUTE` during the active turn
- `IDLE` again when work is complete

For a future multi-phase workflow, phase completion may either advance the engine immediately within the same turn or leave the workflow active across turns without resetting the workflow state.

This distinction is important:

- model completion is not always turn completion
- turn completion is not always workflow completion

**Alternative considered:** end every phase as a separate persisted turn and stitch them together in the UI. Rejected: the user still perceives one logical request in many cases, and splitting it mechanically into separate turns would fragment history and complicate recall semantics.

### Phase-aware prompt assembly

Workflow and phase should be available to prompt assembly as separate contextual inputs, not hidden inside unrelated prompt text.

The harness should be able to inject workflow/phase context alongside the existing prompt components:

- base system prompt
- injected contextual instruction files such as `AGENTS.md`
- workflow/phase context
- optional recall hint
- recent conversation window

The phase context may include lightweight engine-generated guidance such as:

- current workflow name
- current phase name
- workflow status
- any rules about whether the assistant should produce a user-facing answer yet
- whether the workflow is expected to continue in this turn or wait for another turn

This keeps workflow semantics explicit and compatible with the repository's existing prompt assembly rule that separate instruction sources remain separate prompt inputs.

**Alternative considered:** merge workflow/phase instructions directly into the base system prompt string. Rejected: that would blur runtime state with static configuration and break the repository's separation of prompt layers.

### Workflow definition shape

The first implementation should keep workflow definitions simple and code-owned.

A practical initial shape is a static Rust representation in `themion-core`, for example:

- `WorkflowKind` enum for known workflows
- `WorkflowDefinition` struct describing phases and transitions
- `PhaseDefinition` struct describing one phase's metadata and end behavior
- `WorkflowStatus` enum for runtime state such as running, waiting, completed, or failed

That allows the runtime to evolve without introducing config-file complexity too early.

Later, themion may support selecting workflows from config, slash commands, or higher-level orchestration, but this PRD only requires that the runtime model support more than one phase and that workflow state may persist across turns.

**Alternative considered:** make workflows fully user-configurable from TOML in the first version. Rejected: the engine semantics should stabilize in code first before exposing a user-defined workflow DSL.

### Data model and SQLite persistence

The workflow feature needs two related persistence layers:

1. session-level runtime workflow state
2. per-turn workflow history

#### Session-level runtime workflow state

Each interactive session should persist its current workflow runtime state in SQLite so the system can reconstruct what workflow is active right now, not only what happened in past turns.

At minimum, the session runtime state should record:

- `session_id`
- current workflow name
- current phase name
- workflow status (`running`, `waiting_user`, `completed`, `failed`, or similar)
- current agent label or identifier when relevant
- last updated turn sequence
- timestamps for last state update

This state should be updated whenever workflow state changes, including:

- workflow start
- phase start
- phase transition
- turn completion with workflow still active
- workflow completion
- workflow failure or interruption

A practical implementation may extend `agent_sessions` with current workflow fields or add a dedicated runtime-state table keyed by `session_id`.

The important requirement is behavioral, not table shape: SQLite must always reflect the latest known runtime workflow state for the session while the workflow is active.

**Alternative considered:** store only finished turn history and derive current workflow state by scanning the latest turn. Rejected: workflows may remain active across turns, and deriving runtime state indirectly is fragile and expensive.

#### Concrete schema proposal

The first implementation should prefer extending existing tables where the meaning is clearly session-level or turn-level, and add one focused transition table for workflow history.

##### `agent_sessions` additions

Proposed new nullable columns on `agent_sessions`:

- `current_workflow TEXT`
- `current_phase TEXT`
- `workflow_status TEXT`
- `current_agent TEXT`
- `workflow_last_updated_turn_seq INTEGER`
- `workflow_started_at INTEGER`
- `workflow_updated_at INTEGER`
- `workflow_completed_at INTEGER`

Behavioral expectations:

- these columns represent the latest known runtime workflow state for the session
- `NORMAL` is implicit at startup but explicit once workflow state is first written
- built-in phase defaults should allow startup/steady state to be represented as `IDLE`
- `workflow_status` should be constrained by application logic to a small known set such as `running`, `waiting_user`, `completed`, `failed`, `interrupted`

This keeps statusline reads and session resume logic simple because the current workflow state is directly available from the session row.

##### `agent_turns` additions

Proposed new nullable columns on `agent_turns`:

- `workflow_name TEXT`
- `phase_start TEXT`
- `phase_end TEXT`
- `workflow_status_at_start TEXT`
- `workflow_status_at_end TEXT`
- `workflow_continues_after_turn INTEGER`
- `turn_end_reason TEXT`

Behavioral expectations:

- each turn records the workflow context it executed under
- `phase_start` and `phase_end` allow quick reconstruction without scanning transition rows for common cases
- `workflow_continues_after_turn` distinguishes a completed turn from a completed workflow
- `turn_end_reason` may contain values such as `phase_waiting_user`, `workflow_completed`, `error`, `interrupted`, `tool_loop_limit`, or another small application-defined set

This makes turn history self-describing while still allowing more detailed transition logging.

##### New `agent_workflow_transitions` table

Proposed new table:

- `transition_id INTEGER PRIMARY KEY`
- `session_id TEXT NOT NULL`
- `turn_id INTEGER`
- `turn_seq INTEGER`
- `workflow_name TEXT NOT NULL`
- `from_phase TEXT`
- `to_phase TEXT NOT NULL`
- `workflow_status TEXT NOT NULL`
- `transition_kind TEXT NOT NULL`
- `trigger_source TEXT`
- `message_id INTEGER`
- `created_at INTEGER NOT NULL`

Suggested meanings:

- `turn_id` and `turn_seq` tie the transition to a specific turn when applicable
- `from_phase` may be null for workflow start
- `to_phase` is the newly active phase
- `workflow_status` is the resulting workflow status after the transition
- `transition_kind` may be values such as `workflow_started`, `phase_started`, `phase_advanced`, `waiting_user`, `workflow_completed`, `workflow_failed`, `workflow_interrupted`
- `trigger_source` may capture why the change happened, such as `user_input`, `model_completion`, `tool_result`, `engine_rule`, `startup_recovery`
- `message_id` is optional and may point at the assistant or tool message most closely associated with the transition when available

This table is the canonical event log for workflow progression.

##### Optional `agent_messages` additions

Where practical, add nullable workflow context columns to `agent_messages`:

- `workflow_name TEXT`
- `phase_name TEXT`

If adding message columns is too invasive for the first version, the implementation may instead rely on `agent_workflow_transitions.message_id` and turn-level fields. However, direct message annotations are preferred when they fit the existing write path cleanly.

##### Index expectations

At minimum, add indexes that support:

- session runtime state lookup by `session_id`
- transition history lookup by `session_id, created_at`
- transition lookup by `turn_id`
- optional message-to-phase lookup by `message_id`

The exact index names are implementation detail, but workflow resume and history inspection should not require full-table scans.

#### Per-turn workflow history

In addition to current runtime state, each turn should persist the workflow/phase context under which it ran.

At minimum, persisted turn history should allow the system to answer:

- which workflow a turn ran under
- which phase was active at turn start
- which phase was active at turn end
- which phase transitions happened during the turn
- whether the workflow remained active after the turn completed
- why the turn ended, when practical

This may be implemented by extending `agent_turns` with workflow fields and adding a focused phase-transition table, or by another schema that preserves equivalent information.

The key design requirement is that one can inspect a turn and understand both:

- what happened during that turn
- what workflow state the session was left in afterward

#### Section-level or message-level state tagging

Where practical, assistant/tool segments written during a turn should be attributable to the workflow phase that produced them. This can be done through direct message annotations, a phase-transition log that can be joined back to messages, or another focused representation.

The intent is that each persisted section of execution has enough workflow context to support debugging and future history-aware tooling.

**Alternative considered:** record only one workflow and phase value for the entire turn. Rejected: multi-phase turns would lose important detail about where transitions occurred and which phase produced which outputs.

### Status line presentation

The TUI status line should be updated so workflow-aware runtime identity is visible at a glance. The landed format is:

- `<profile> | <model> | <leaf_project_dir> | flow: <workflow> | phase: <phase> | agent: <agent_state>`

Where:

- `<profile>` is the active profile name
- `<model>` is the active model name
- `<leaf_project_dir>` is the basename/leaf directory name of the current project path rather than the full path
- `<workflow>` is the active workflow identifier for the current turn or session
- `<phase>` is the currently active phase identifier from persisted/runtime workflow state
- `<agent_state>` is the live runtime activity label shown by the UI, such as `idle`, `waiting-model`, `streaming ...`, or `running-tool`

For the default single-agent, single-workflow case, typical displays now look like:

- `default | gpt-5.4 | themion | flow: NORMAL | phase: IDLE | agent: idle`
- `default | gpt-5.4 | themion | flow: NORMAL | phase: EXECUTE | agent: waiting-model`

During future multi-phase execution, the `phase:` segment should update live as the engine advances or waits across turns.

**Alternative considered:** reserve the `agent:` field for a stable agent name such as `main`. Rejected: in the current single-agent UI, the more useful value is the live runtime state, while stable agent identity can remain in persisted workflow state for future multi-agent use.

### Persistence and observability

Because workflow and phase affect execution semantics, themion should persist enough metadata to reconstruct both current state and historical transitions.

At minimum, persisted metadata should allow the system to answer:

- which workflow is currently active for a session
- which phase is currently active for a session
- which workflow a turn used
- which phases ran during the turn
- in what order phase transitions occurred
- whether the workflow remained active after that turn
- which phase was active for each assistant/tool segment when practical

The TUI should be able to surface at least the active workflow and current phase for in-flight turns and between turns when a workflow remains active. A compact display in the status line is sufficient for the first version.

**Alternative considered:** keep workflow/phase state entirely in memory and never persist it. Rejected: phase-aware debugging, interruption recovery, history inspection, and future multi-agent behavior become much harder if execution structure disappears after the process exits.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/agent.rs` | Tracks explicit workflow runtime state, emits workflow-state updates during execution, injects workflow context into prompt assembly, and records workflow-aware turn completion metadata. The landed implementation currently uses the built-in `NORMAL` workflow with `IDLE` and `EXECUTE` phases. |
| `crates/themion-core/src/workflow.rs` | Defines workflow runtime types, built-in defaults, and status enums for the workflow/phase model. |
| `crates/themion-core/src/db.rs` | Adds schema migrations for `agent_sessions` workflow state columns, `agent_turns` workflow summary columns, `agent_messages` workflow annotations, and `agent_workflow_transitions`. Persists session-level runtime workflow state, per-turn workflow metadata, and workflow transitions. |
| `crates/themion-cli/src/tui.rs` | Renders the statusline as `<profile> | <model> | <leaf_project_dir> | flow: <workflow> | phase: <phase> | agent: <agent_state>` and updates workflow/phase state live from core events during a running turn. |
| `docs/architecture.md` | Should document workflow-aware turn lifecycle, `IDLE` versus `EXECUTE`, workflow persistence, and the revised status line semantics. |
| `docs/core-ai-engine-loop.md` | Should document workflow/phase prompt inputs, session-level runtime workflow state, per-turn phase transitions, and the distinction between phase completion, turn completion, and workflow completion. |
| `docs/README.md` | Updated the PRD-006 row status to Implemented. |

## Edge Cases

- Workflow is `NORMAL` → behavior should match current single-turn semantics, using `IDLE` before and after active work and `EXECUTE` while the turn is running.
- A workflow remains active after one turn and expects more user input → the runtime model and persistence should support this, even though the landed implementation currently returns the built-in `NORMAL` workflow to `IDLE` on completion.
- A non-terminal phase completes but the next phase cannot be resolved → fail the workflow clearly rather than silently ending as if it succeeded.
- A phase triggers repeated tool calls and hits the existing inner loop cap → the turn should fail or stop with explicit phase-aware diagnostics rather than advancing as though the phase completed normally.
- Provider or stream failure happens mid-phase → persisted state should make it clear which workflow/phase was active when the failure occurred, and whether the workflow remains resumable.
- A future workflow requires a final user-facing answer only in the last phase → earlier phases should be allowed to produce internal progress without being mistaken for completed workflows.
- The user interrupts or exits while a workflow is still active → the current runtime workflow state should remain inspectable from SQLite as far as practical.
- History recall should not become ambiguous when one logical turn contains multiple phases or when a workflow spans multiple turns → recall output should still preserve turn order and, when available, phase/workflow metadata.
- TUI busy-state logic must still prevent overlapping submissions while a multi-phase turn is advancing automatically between phases.
- The project directory may be `/` or otherwise have an unusual basename → the statusline should still show a stable leaf-project value without panicking.
- The `agent:` segment currently reflects live runtime state rather than a stable agent identifier → docs and future UI work should treat that as intentional until a separate multi-agent presentation design lands.
- Session startup encounters persisted workflow state marked `running` from a previous interrupted process → the implementation should define whether that becomes resumable, failed, or waiting state rather than ignoring it silently.
- A transition is written for a turn before its final `agent_turns` summary fields are updated → schema and write ordering should still allow consistent reconstruction after crashes.
- Message-level phase columns are absent in older rows → history queries should degrade gracefully by using turn-level and transition-level workflow data.

## Migration

This feature is additive.

Existing sessions, configs, and user workflows should continue to behave as before under the default `NORMAL` workflow. If no workflow is specified, themion should use the built-in `NORMAL` workflow and begin in phase `IDLE`, transitioning to `EXECUTE` only while a turn is actively running.

If schema changes are required for workflow/phase persistence, they should be backward-compatible migrations that preserve existing turn and message history while adding session-level runtime workflow state.

Expected migration shape:

- add nullable workflow-state columns to `agent_sessions`
- add nullable workflow-summary columns to `agent_turns`
- create `agent_workflow_transitions`
- add nullable workflow/phase columns to `agent_messages`
- backfill is optional; older rows may remain null and be interpreted with defaults where appropriate

Older turns created before workflow metadata existed may be treated as:

- workflow: `NORMAL`
- phase: `EXECUTE`

where needed for display or reporting.

Older sessions with no persisted runtime workflow state may be treated as implicitly ready in `NORMAL` / `IDLE`, depending on what best matches current startup behavior.

## Testing

- start a normal session without specifying a workflow → verify: the session begins with `NORMAL` and phase `IDLE`.
- submit a normal prompt under the default workflow → verify: the phase changes from `IDLE` to `EXECUTE` while the turn runs and returns to `IDLE` when the turn completes.
- run an existing tool-using prompt under `NORMAL` → verify: tool round-trips still occur inside the `EXECUTE` phase and the turn ends with the same behavior as before.
- inspect `agent_sessions` during an active workflow → verify: `current_workflow`, `current_phase`, `workflow_status`, `current_agent`, and `workflow_last_updated_turn_seq` reflect the latest engine state.
- inspect `agent_turns` after a workflow-aware turn → verify: `workflow_name`, `phase_start`, `phase_end`, `workflow_status_at_start`, `workflow_status_at_end`, and `workflow_continues_after_turn` describe the turn outcome correctly.
- inspect `agent_workflow_transitions` for a normal run → verify: workflow start and workflow completion are recorded in order for the turn.
- inspect persisted turn data for a workflow-aware turn → verify: workflow name, phase start/end, and phase annotations are recorded in a way that can be reconstructed later.
- inspect persisted session runtime state while a workflow is running → verify: current workflow, current phase, workflow status, and last-updated turn sequence reflect the latest engine state.
- view the TUI during a running turn → verify: the active workflow/phase indicator updates live from `IDLE` to `EXECUTE` and back to `IDLE` on completion.
- start the TUI in a project directory with a normal basename such as `/work/themion` → verify: the statusline shows `themion` in the `<leaf_project_dir>` position rather than the full path.
- start the TUI in the default single-agent case → verify: the statusline format is `<profile> | <model> | <leaf_project_dir> | flow: NORMAL | phase: IDLE | agent: idle` while idle, and the `agent:` segment shows live runtime state rather than a stable agent name.
- query older history that includes workflow-aware turns → verify: turn ordering remains stable and workflow/phase metadata is visible when available.
- run schema migration on an existing history database → verify: older sessions and turns remain readable, new workflow columns/tables exist, and null legacy values are handled safely.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: workflow/phase runtime, persistence, and UI wiring compile cleanly.
