# PRD-074: Require Double `Ctrl+C` to Exit the TUI

- **Status:** Implemented
- **Version:** v0.48.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-30

## Implementation status

Landed in `v0.48.0` as a focused `themion-cli` TUI safety improvement. The shipped behavior keeps `/exit` and `/quit` as explicit single-step command exits, preserves `Esc` interrupt behavior, and changes keyboard `Ctrl+C` so the first press shows a local notice while a second `Ctrl+C` within 3 seconds exits the TUI.

## Summary

- The TUI currently exits too easily when the user presses `Ctrl+C` by accident.
- This PRD keeps `Ctrl+C` as the exit shortcut, but changes it to require a second `Ctrl+C` within 3 seconds before the app actually quits.
- After the first `Ctrl+C`, the TUI should notify the user that one more `Ctrl+C` is required to exit and that the confirmation window will expire shortly.
- If the second `Ctrl+C` does not arrive within 3 seconds, the confirmation state should clear automatically and normal TUI use should continue.
- Slash-command exits such as `/exit` and `/quit`, plus existing `Esc` interrupt behavior, stay unchanged.

## Goals

- Reduce accidental TUI exits caused by an unintended single `Ctrl+C` press.
- Preserve a fast keyboard-only quit path for users who intentionally want to leave the TUI.
- Make the new behavior obvious by showing a user-visible confirmation hint after the first `Ctrl+C`.
- Keep the change local to `themion-cli` TUI event handling and status messaging.
- Avoid introducing modal confirmation prompts or blocking dialogs for a lightweight safety improvement.

## Non-goals

- No change to non-TUI modes such as headless or one-shot non-interactive execution.
- No redesign of slash-command exits such as `/exit` or `/quit`.
- No persistent config option in this PRD for customizing the timeout or disabling the confirmation behavior.
- No change to `Esc` interrupt behavior for in-progress agent turns.
- No requirement to add a full-screen confirmation dialog or extra confirmation step beyond the second `Ctrl+C`.

## Background & Motivation

### Current state

Themion's TUI is keyboard-driven, and `Ctrl+C` is a natural terminal habit for users. In practice, that also makes it easy to hit by mistake while editing, interrupting, or moving quickly through the interface.

A direct single-press exit is efficient, but it is unforgiving when triggered accidentally. Because TUI sessions often contain active context, in-progress drafting, or pending review of agent output, an unintended immediate exit is a frustrating failure mode even when durable history exists.

Current implementation shape confirmed from `themion-cli`:

- `Ctrl+C` is mapped to `InputAction::Quit` in `crates/themion-cli/src/chat_composer.rs`
- `crates/themion-cli/src/tui.rs` currently exits immediately on that action by setting `running = false`
- `/exit` and `/quit` are separate slash-command exits and do not need to change for this PRD

The desired product behavior is a small safety rail rather than a heavier quit workflow:

- first `Ctrl+C` warns
- second `Ctrl+C` within a short window exits
- waiting past the short window resets the warning state

This keeps the shortcut familiar while making accidental exits less likely.

## Design

### 1. Treat the first `Ctrl+C` as an exit arming step

In TUI mode, the first `Ctrl+C` press should not exit immediately. Instead, it should arm a short-lived exit confirmation state.

Required behavior:

- on the first `Ctrl+C`, keep the TUI running
- record that exit confirmation is armed
- record when the armed state expires, using a 3-second timeout
- a second `Ctrl+C` received before expiry should exit the TUI normally
- if the confirmation is not completed before expiry, the armed state should clear automatically

This preserves the familiar shortcut while adding a lightweight guard against accidental exits.

**Alternative considered:** require a dedicated quit key or slash command instead of `Ctrl+C`. Rejected: users already expect `Ctrl+C`, and the goal is to make it safer, not replace it.

### 2. Show a clear user-visible notification after the first press

After the first `Ctrl+C`, the TUI should immediately tell the user what changed.

Required behavior:

- show a visible local message such as `Press Ctrl+C again within 3s to exit`
- the message should appear through the existing local status-entry surface used for lightweight TUI notices, so it becomes visible in the transcript area without adding a new modal UI layer
- the message should make both requirements explicit: one more press and a short timeout window
- the message should disappear naturally when the armed state expires or when the app exits

The message is part of the product behavior, not only a debug aid. Users need feedback so the first `Ctrl+C` does not feel ignored.

**Alternative considered:** silently arm the exit and rely on user intuition for the second press. Rejected: without visible feedback, the first press would look broken rather than protective.

### 3. Reset the arming state conservatively

The confirmation window should be short, predictable, and easy to reason about.

Required behavior:

- the arming window lasts 3 seconds from the first `Ctrl+C`
- once expired, a later `Ctrl+C` should behave like a new first press rather than immediately exiting
- if the user continues normal interaction during the 3-second window, the TUI should remain usable
- the implementation may clear the armed state lazily on the next tick as long as the visible behavior matches a 3-second timeout closely enough for users
- the first implementation does not need a separate "confirmation expired" message; silent reset is acceptable once the original notice is no longer current

This keeps the feature simple and prevents surprising delayed exits.

**Alternative considered:** keep the armed state until any non-`Ctrl+C` key is pressed. Rejected: time-based reset is simpler, more predictable, and matches the requested interaction.

### 4. Keep quit-safety logic inside the TUI event loop

The double-press behavior should be handled where TUI keyboard input already flows today.

Required behavior:

- keep the arming state in `themion-cli` TUI-local app state
- continue mapping `Ctrl+C` to `InputAction::Quit` in `chat_composer.rs`, and interpret that action in `tui.rs` as either "arm exit" or "complete exit" depending on the local timeout state
- use existing tick/redraw/event infrastructure to expire the arming state and refresh the UI if needed
- keep `themion-core` unchanged because this is a local TUI interaction policy, not a core runtime concern

This matches the repository's CLI/core separation.

**Alternative considered:** push `Ctrl+C` confirmation into shared runtime state in `themion-core`. Rejected: the behavior is TUI-local and should not widen the core surface unnecessarily.

### 5. Preserve other established interruption semantics

This PRD should narrowly change TUI exit behavior without broadening its scope into a general interruption redesign.

Required behavior:

- existing `Esc` interrupt semantics from PRD-017 should remain unchanged
- agent-turn interruption and TUI-process exit should remain distinct behaviors
- only the TUI keyboard `Ctrl+C` exit path should gain the double-press confirmation
- `/exit` and `/quit` may remain single-step exits because they are explicit commands rather than accidental keypresses

This keeps the fix focused on accidental exits rather than reopening the broader keyboard model.

**Alternative considered:** apply the same double-confirmation policy to every quit path immediately. Rejected: that expands scope beyond the specific accidental `Ctrl+C` problem the PRD is trying to solve.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Add TUI-local state for armed `Ctrl+C` exit confirmation, handle double-press timing in the keyboard event path, expire the arming state on tick, and surface the temporary user notification through local status entries. |
| `crates/themion-cli/src/chat_composer.rs` | Keep mapping keyboard `Ctrl+C` to `InputAction::Quit`; no broader input-model redesign is required. |
| `docs/architecture.md` | Document that TUI `Ctrl+C` exit now requires a confirming second press within 3 seconds and emits a local notice after the first press. |
| `docs/README.md` | Track this PRD as implemented once the work lands. |
| `docs/prd/prd-074-require-double-ctrl-c-to-exit-the-tui.md` | Update status and implementation notes to reflect the shipped behavior. |

## Edge Cases

- first `Ctrl+C` is pressed accidentally during ordinary idle TUI use → verify: the app stays open and shows the confirmation hint.
- second `Ctrl+C` arrives within 3 seconds → verify: the TUI exits normally.
- more than 3 seconds pass after the first `Ctrl+C` → verify: the confirmation state resets and the next `Ctrl+C` is treated as a new first press.
- the user presses other keys between the first and second `Ctrl+C` → verify: normal interaction continues and exit still requires the confirming second press before timeout.
- the app is redrawn or receives tick events while the confirmation window is active → verify: the hint remains visible until expiry or exit.
- an agent turn is in progress when the first `Ctrl+C` is pressed → verify: the TUI does not exit on the first press and the behavior remains distinct from `Esc` interruption semantics.

## Migration

This feature requires no database or config migration.

Rollout guidance:

- keep `Ctrl+C` as the quit shortcut so the interaction remains familiar
- add only a small confirmation timeout and local notice rather than a larger quit dialog
- update docs where TUI keyboard behavior is described so the new exit semantics are discoverable

## Testing

- press `Ctrl+C` once in the TUI while idle → verify: the app stays open and a message indicates that `Ctrl+C` must be pressed again within 3 seconds to exit.
- press `Ctrl+C` twice within 3 seconds → verify: the second press exits the TUI.
- press `Ctrl+C` once, wait more than 3 seconds, then press `Ctrl+C` again → verify: the second later press acts like a new first press and shows the warning again instead of exiting.
- press `Ctrl+C`, then continue typing or navigating before the timeout expires → verify: the TUI remains usable and no unexpected immediate exit occurs.
- trigger an in-progress agent turn and press `Esc` → verify: existing interrupt behavior remains unchanged by the new `Ctrl+C` exit confirmation logic.
- run `cargo check -p themion-cli` after implementation → verify: the touched crate builds cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the touched crate still builds with the feature enabled.
- run `cargo check -p themion-cli --all-features` after implementation → verify: the touched crate still builds cleanly across feature combinations.

## Implementation checklist

- [x] add TUI-local state for first-press `Ctrl+C` exit confirmation and 3-second expiry tracking
- [x] change the TUI keyboard handler so one `Ctrl+C` arms exit and a second `Ctrl+C` within the timeout exits
- [x] show a concise visible notice after the first `Ctrl+C` and clear the arming state when the timeout expires
- [x] keep `Esc` interrupt behavior, `/exit`, `/quit`, and non-TUI modes unchanged
- [x] update docs and mark the PRD status/version when implementation lands
