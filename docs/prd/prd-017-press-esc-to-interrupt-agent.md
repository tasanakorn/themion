# PRD-017: Press `Esc` to Interrupt an In-Progress Agent Turn

- **Status:** Proposed
- **Version:** v0.9.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Add a fast keyboard path in the TUI that lets the user press `Esc` to interrupt the currently running agent turn.
- Make interruption user-visible and explicit rather than forcing the user to quit the whole application with `Ctrl+C` when only the current turn should stop.
- End the active turn cleanly enough that the session, conversation pane, workflow state, and persistent history remain inspectable after the interruption.
- Preserve the existing submit-and-wait interaction model while adding a targeted way to regain control during long model generations, long tool runs, or accidental prompt submissions.
- Surface interrupted state consistently in runtime status events, workflow state, and documentation.

## Non-goals

- No redesign of the full TUI keybinding system beyond adding the `Esc` interrupt behavior.
- No promise that every underlying subprocess or provider request can be forcibly killed instantly in all cases; this PRD defines product behavior and implementation expectations, not OS-level guarantees.
- No requirement to make `Esc` interrupt background agents or future parallel workers beyond the current interactive agent turn.
- No change to `Ctrl+C` as the quit shortcut for the entire TUI session.
- No introduction of a general pause/resume execution framework.

## Background & Motivation

### Current state

The current TUI documents these key bindings:

- `Enter` to submit a message
- history and scroll navigation keys
- `Ctrl+C` to quit

There is no documented in-session interrupt key for a running turn.

That creates a UX gap when the user wants to stop only the current work rather than exit Themion entirely. Common cases include:

- the agent is taking the wrong approach and the user wants to stop it early
- a tool call or shell command is taking too long
- a model response is clearly unhelpful and continuing to stream
- the user submitted something by mistake and wants control back immediately

Themion already has workflow status support for `interrupted`, and its docs describe interruption as a meaningful runtime outcome. However, the user-facing TUI currently exposes only whole-app exit through `Ctrl+C`, not turn interruption.

This means the runtime has a concept that the primary interactive surface does not yet expose as a first-class user action.

## Design

### `Esc` interrupts the active interactive turn

When the interactive agent is busy processing a turn, pressing `Esc` in the TUI should request interruption of that in-progress turn.

Normative behavior:

- if no agent turn is currently running, `Esc` should keep its existing no-op behavior unless some future focused UI element needs it
- if a turn is running, `Esc` should target only that running interactive turn
- the TUI should return to an idle input-ready state after the interruption completes
- the current application session should remain open

This gives users a low-latency escape hatch without conflating turn cancellation with application exit.

**Alternative considered:** require `/interrupt` as a typed command instead of a keybinding. Rejected: interruption is most useful when it is immediate and available even while the user cannot conveniently type into a busy TUI flow.

### Interruption semantics across the active turn

An interruption request should stop the current turn as soon as practical, regardless of whether the agent is:

- waiting for the model to begin responding
- streaming assistant text
- between model/tool loop iterations
- running a tool
- waiting on a provider response after a tool call

The interruption should be best-effort but explicit.

Implementation should prefer cooperative cancellation points already present in the async runtime and request chain, while ensuring the UI does not falsely report success if cancellation is delayed or partial.

At minimum, the product behavior should guarantee:

- no further assistant chunks from the interrupted turn are rendered after cancellation completes
- the turn is finalized as interrupted rather than as a successful completion
- the agent becomes available for the next user input without restarting the app

**Alternative considered:** support interruption only during model streaming and not during tool execution. Rejected: from the user's perspective the need is to stop the active turn, not only one transport phase of that turn.

### TUI feedback and event narration

The conversation pane should narrate the interruption as a neutral runtime event rather than as a successful completion.

Expected user-facing feedback includes:

- a status/event row indicating that interruption was requested
- a final status/event row indicating that the turn was interrupted
- removal of any pending spinner or busy indicator once the interrupt completes

If partial assistant output was already streamed before interruption, that already-rendered text may remain visible as partial output for historical accuracy, but the UI should make it clear that the turn did not finish normally.

The statusline should return from a busy activity label to the normal idle state after the interruption.

**Alternative considered:** silently stop the turn and just re-enable input. Rejected: interruption is an important runtime outcome and should be observable for user confidence and debugging.

### Workflow-state interaction

If a workflow-aware turn is interrupted by the user, the runtime should mark the workflow status as `interrupted` for that turn outcome rather than pretending the phase passed or failed normally.

For workflow visibility:

- workflow state snapshots and turn-finalization metadata should show that the turn ended due to interruption
- retry counters should not be incremented merely because the user manually interrupted the turn
- interruption should not be collapsed into `failed`, because a manual stop is semantically different from autonomous phase failure

The next user turn may either resume by explicit workflow action or continue according to the runtime's normal interrupted-session policy, but the interrupted outcome itself should remain visible in history.

**Alternative considered:** map user interruption to ordinary workflow failure. Rejected: that would consume failure semantics and retry behavior for an intentional human stop action.

### Provider, tool, and shell cancellation expectations

The implementation should add a single turn-level cancellation path that the TUI can trigger and the running agent loop can observe.

That cancellation path should cover:

- active provider streaming requests
- the model/tool loop between iterations
- long-running tool execution where practical
- direct shell convenience commands if they are later wired into the same busy-state interrupt path

For tool behavior, the product requirement is best-effort interruption with accurate reporting:

- async work that supports cancellation should stop promptly
- work that cannot be interrupted immediately should stop the turn from progressing further once control returns
- the UI should avoid claiming that a subprocess was killed if the implementation only stopped awaiting its result

**Alternative considered:** add separate interrupt implementations for provider streaming, tool execution, and shell commands with unrelated user-facing semantics. Rejected: users need one clear concept of “stop the current turn,” not multiple partially overlapping interrupt models.

### Keybinding documentation

The documented keybinding table should be updated so `Esc` is listed as the interrupt shortcut for the active agent turn.

This should live alongside the existing `Ctrl+C` quit binding to make the distinction clear:

- `Esc` interrupts the current turn
- `Ctrl+C` exits Themion

This distinction should also be reflected in architecture/runtime docs where TUI behavior is summarized.

**Alternative considered:** document `Esc` informally in release notes only. Rejected: interruption is a core interactive control and belongs in the canonical docs.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Handle `Esc` as an interrupt request when the interactive agent is busy, emit appropriate status entries, clear pending busy UI state when cancellation completes, and keep the app session running. |
| `crates/themion-core/src/agent.rs` | Introduce or extend turn-level cancellation handling so the running loop can observe an interrupt request during model streaming, tool execution, and loop boundaries, then finalize the turn as interrupted. |
| `crates/themion-core/src/client.rs` and `crates/themion-core/src/client_codex.rs` | Ensure streaming provider calls can participate in cooperative cancellation without leaving the harness stuck waiting for a full completion path. |
| `crates/themion-core/src/tools.rs` and any long-running tool helpers | Propagate cancellation where practical so interrupted turns stop requesting additional work and report interruption accurately. |
| `crates/themion-core/src/workflow.rs` and turn-finalization/persistence code | Record interruption as a distinct turn/workflow outcome, preserving status visibility without incrementing retry counters as ordinary failure. |
| `docs/architecture.md` | Update the TUI keybinding table and related runtime description to include `Esc` turn interruption and its distinction from `Ctrl+C`. |
| `docs/engine-runtime.md` | Document turn-level interruption behavior, workflow interaction, and expected runtime/status-event visibility. |
| `docs/README.md` | Add this PRD to the PRD index with proposed status and scope. |

## Edge Cases

- the user presses `Esc` while no turn is running → verifyable behavior should remain a no-op rather than producing a confusing interruption message.
- the user presses `Esc` multiple times during the same busy turn → the runtime should coalesce repeated requests and avoid duplicate interruption finalization.
- partial assistant text already streamed before interruption → the partial text may remain visible, but the turn must still be marked interrupted rather than complete.
- a tool or subprocess ignores cancellation immediately → the UI should continue to reflect that interruption was requested and should finalize as interrupted once the runtime regains control, without falsely claiming an immediate hard kill.
- interruption happens during a workflow retry-eligible phase → retry counters should remain unchanged because the stop was user-driven, not an autonomous failure.
- interruption happens while the runtime is between tool iterations or finalizing usage stats → the turn should still end in an interrupted state if the cancel request arrived before normal completion.
- future background agents exist alongside one interactive agent → `Esc` should only target the active interactive turn unless and until a separate UX for multi-agent interruption is defined.

## Migration

This is an additive user-interaction feature.

Existing sessions, history rows, and config files remain valid. After upgrade, interrupted turns may begin to appear more explicitly in persisted turn/workflow metadata and UI event streams.

If persisted turn-end reason or workflow-state serialization currently treats interruption as an exceptional or unreachable path, that storage should be extended in a backward-compatible way so older rows still read cleanly and new rows can record user-triggered interruption explicitly.

No config migration is required.

## Testing

- start a long-running model turn and press `Esc` → verify: the turn stops without quitting the TUI and input becomes available again.
- start a turn that is actively streaming assistant text and press `Esc` → verify: streaming stops promptly, partial output remains visible if already rendered, and the turn is marked interrupted.
- start a turn that is waiting on a tool and press `Esc` → verify: the runtime stops the turn as soon as practical and does not continue into additional tool/model iterations.
- press `Esc` while no agent turn is running → verify: the app does not quit and does not emit a misleading interruption completion event.
- interrupt a workflow-aware turn during a retry-eligible phase → verify: workflow status/history record interruption distinctly and retry counters do not increment.
- inspect runtime status events for an interrupted turn → verify: the event stream includes clear interruption narration rather than a normal success completion message.
- press `Ctrl+C` during idle and busy states → verify: whole-app quit behavior remains unchanged and distinct from `Esc` interruption.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: TUI, runtime cancellation, and workflow/documentation changes compile cleanly.
