# PRD-044: Fix Multiline Input Newline and Wrapped-Cursor Tracking

- **Status:** Implemented
- **Version:** v0.26.1
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-23

## Summary

- The TUI input composer should treat `Shift-Enter` as a newline insertion, not as submit.
- The current code already attempts that, but the rendered cursor does not move correctly, so the UI looks like the newline did not land.
- Long input lines that wrap at the input box edge also show the cursor in the wrong place or make it disappear from the visible input region.
- Fix the input box so visual cursor placement is computed from the rendered wrapped layout, not from the raw textarea row/column alone.
- Keep the current submit shortcuts and multiline behavior model; this is a patch-level correctness fix, not a composer redesign.
- Use `../codex/codex-rs` as implementation guidance for separating text-buffer state from rendered cursor placement and for testing wrapped input behavior.

## Goals

- Make `Shift-Enter` visibly insert a newline in the input composer without submitting the prompt.
- Keep plain `Enter` submit behavior unchanged except for existing paste-burst safeguards.
- Ensure the input cursor moves to the next rendered line immediately after explicit newline insertion.
- Ensure long input that soft-wraps at the input box edge keeps the visible cursor aligned with the rendered text.
- Prevent the input cursor from disappearing or drifting horizontally/vertically when the composer wraps.
- Keep the input height calculation, redraw model, and current shortcut hints consistent with the actual behavior.
- Add regression coverage for explicit newlines and soft-wrapped cursor placement.

## Non-goals

- No redesign of the overall TUI layout.
- No replacement of `tui-textarea` with a different editor widget in this patch.
- No change to submit shortcuts such as `Ctrl-S` or `Ctrl-J` beyond preserving their documented behavior.
- No new rich composer features such as attachments, syntax highlighting, or markdown preview.
- No broad refactor of unrelated redraw scheduling or transcript rendering code.
- No change to agent-side prompt handling or message persistence semantics.

## Background & Motivation

### Current state

The current TUI input handling in `crates/themion-cli/src/tui.rs` already routes `Shift-Enter` and `Ctrl-J` to `app.input.insert_newline()` rather than to `submit_input()`. The placeholder text also advertises `Shift-Enter/Ctrl-J newline`.

However, the visible behavior is still wrong in two related cases:

- after pressing `Shift-Enter`, the prompt appears not to update because the cursor remains at the old on-screen position rather than moving to the next line
- when typing a long message that soft-wraps at the input box width, the cursor position no longer tracks the rendered wrapped text and may drift or disappear from the visible box

This points to a rendering-level mismatch, not just a keybinding bug. The input text is rendered through a `Paragraph` with wrapping enabled, while the cursor is positioned separately from `app.input.cursor()` as if the raw textarea row/column directly matched the on-screen wrapped layout.

### Why this is a patch-level bug fix

The user-visible contract already exists:

- `Enter` sends
- `Shift-Enter` and `Ctrl-J` insert newline
- wrapped multiline input should keep the cursor visible and accurate

The code and the placeholder already claim this behavior, so the issue is that the shipped UI rendering is inconsistent with the intended behavior. That makes this a patch-level correctness fix rather than a new feature.

**Alternative considered:** document that multiline input is only partially supported. Rejected: the code already exposes multiline editing affordances, so the right fix is to make the UI truthful.

### Research note from `../codex/codex-rs`

`../codex/codex-rs/tui` uses a more explicit separation between composer text state and render-time layout state. Its bottom-pane composer code keeps dedicated textarea state and includes focused tests around wrapping behavior in the TUI layer.

The relevant lesson for Themion is not to copy the entire Codex composer, but to follow the same principle:

- compute visible layout from the actual rendered width
- derive cursor placement from that rendered layout
- add regression tests for wrapping and cursor behavior rather than relying only on manual terminal testing

**Alternative considered:** keep using the current ad hoc cursor math and only special-case `Shift-Enter`. Rejected: the wrapped-line bug shows the root problem is broader than one keybinding branch.

## Design

### Keep current input editing shortcuts, but make rendered cursor placement layout-aware

The current shortcut model should remain:

- `Enter` submits when review mode is closed and paste-burst heuristics do not force newline insertion
- `Shift-Enter` inserts a literal newline
- `Ctrl-J` inserts a literal newline

The fix should focus on how the input region is rendered and how the terminal cursor position is computed.

Normative direction:

- keep `app.input` as the editing source of truth for text content and logical cursor position
- when rendering the input box, compute the cursor's visible row/column from the actual rendered text layout within the input inner width
- treat explicit `\n` as hard line breaks and width overflow as soft wraps that advance the visible row
- place the terminal cursor from this rendered visual position rather than directly from the raw textarea row/column tuple

This preserves the current editing model while fixing the visible mismatch.

**Alternative considered:** change the widget to stop soft-wrapping and require horizontal scrolling instead. Rejected: it would avoid the bug by reducing functionality and would be a bigger UX change than needed.

### Derive input height and cursor placement from the same wrapping logic

The current code computes `input_height` from a manual visual-line count, then renders the text via `Paragraph::wrap(Wrap { trim: false })`, and separately places the cursor from `app.input.cursor()`.

Those three pieces should use one consistent wrapping model.

Normative direction:

- use the same width and wrapping assumptions for input height calculation and cursor placement
- count wrapped rows using display-width-aware logic that matches the rendered paragraph as closely as practical
- avoid one code path for height and another unrelated code path for cursor placement
- when the input region reaches its maximum height clamp, keep the cursor anchored to the visible bottom portion rather than letting it point outside the rendered box silently

This reduces the chance of future drift between layout sizing and cursor placement.

**Alternative considered:** fix only cursor placement and leave input-height estimation unchanged. Rejected: both behaviors come from the same render-layout assumptions and should stay aligned.

### Make newline insertion visibly dirty the input region immediately

The current `Shift-Enter` branch inserts a newline, but the user report indicates the screen does not visibly update correctly at that moment.

Normative direction:

- ensure explicit newline insertion marks the input region dirty and schedules a redraw through the normal redraw path
- preserve the current redraw scheduler introduced by PRD-042 rather than bypassing it with ad hoc direct draws
- treat soft-wrap-affecting edits similarly, because cursor visibility depends on prompt rerendering after input mutations

This keeps the redraw architecture intact while ensuring the corrected layout becomes visible immediately.

**Alternative considered:** force a synchronous direct draw only for `Shift-Enter`. Rejected: that would work around symptoms while bypassing the request-driven redraw model.

### Prefer render-oriented helper functions over duplicated cursor math in the draw path

The draw path should gain a small helper for computing input visual layout metadata such as wrapped row count and visible cursor offset.

Normative direction:

- add a focused helper near the input rendering code that accepts the current input text, logical cursor position, and inner width
- return the computed visual line count and the cursor's rendered row/column within the wrapped input area
- use this helper both for dynamic input-height decisions and for terminal cursor placement
- keep the helper local to `crates/themion-cli/src/tui.rs` unless later reuse clearly justifies extracting a module

This keeps the patch targeted and testable.

**Alternative considered:** scatter wrap math inline in the `draw()` function. Rejected: harder to test and easier to regress.

### Add regression tests modeled after the Codex approach

This bug is highly visual and easy to reintroduce if only validated manually.

Normative direction:

- add targeted tests in `themion-cli` covering explicit newline insertion and long soft-wrapped input
- verify that rendered cursor coordinates advance after `Shift-Enter`
- verify that long input near and beyond the wrap edge keeps the cursor within the visible input box and on the expected wrapped row
- where practical, use Ratatui test backends or focused helper-level tests rather than requiring a full interactive terminal harness

This follows the useful testing direction visible in `../codex/codex-rs/tui`, where wrapped-layout behavior is treated as something worth testing explicitly.

**Alternative considered:** rely only on manual TUI testing because terminals are hard to simulate. Rejected: helper-level layout tests are practical here and would catch the core regression.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Fix input rendering so wrapped-layout cursor placement matches the visible paragraph for explicit newlines and soft-wrapped long lines. |
| `crates/themion-cli/src/tui.rs` | Consolidate input visual-line counting and cursor-position calculation behind a shared helper used by the draw path. |
| `crates/themion-cli/src/tui.rs` or nearby CLI tests | Add regression tests for `Shift-Enter` newline behavior and wrapped-input cursor tracking. |
| `docs/architecture.md` | Update the TUI section if needed so multiline input behavior is described accurately. |
| `docs/engine-runtime.md` | Document the corrected multiline input and wrapped-cursor semantics at the CLI runtime level if current wording is ambiguous. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Edge Cases

- press `Shift-Enter` in an empty input box → verify: one blank first line is created, the cursor moves to the second visible line, and nothing is submitted.
- press `Shift-Enter` in the middle of existing text → verify: a newline is inserted at the logical cursor position and the rendered cursor moves to the next line after the split.
- type a long unbroken line that exceeds the input width → verify: the text soft-wraps and the cursor remains visible at the wrapped visual position.
- type a long multiline message with both explicit newlines and soft wraps → verify: visual cursor tracking stays correct across both hard and soft line boundaries.
- grow input to the maximum composer height clamp → verify: the visible cursor remains inside the input region and follows the active editing line rather than disappearing off-box.
- use `Ctrl-J` instead of `Shift-Enter` → verify: it follows the same corrected newline and cursor behavior.
- trigger paste-burst newline heuristics with plain `Enter` → verify: existing submit-versus-newline behavior is preserved and cursor placement still updates correctly.
- edit text containing non-ASCII characters of varying display width → verify: cursor placement does not regress for wide or multibyte characters in the touched code path.

## Migration

This is a patch-level UI correctness fix with no config, schema, or history migration.

Expected rollout shape:

- keep the current input shortcuts and placeholder guidance
- fix wrapped-layout cursor computation in place
- add regression tests so future input/render work does not silently reintroduce cursor drift
- avoid changing unrelated TUI layout or agent runtime behavior in the same patch

## Testing

- start Themion, type text, press `Shift-Enter`, then continue typing → verify: no submit occurs, a visible newline is inserted, and the cursor moves to the next rendered line.
- start Themion, type a long line past the input edge → verify: the input soft-wraps and the cursor remains visible at the correct wrapped position.
- combine soft-wrapped lines with explicit `Shift-Enter` newlines → verify: cursor placement stays correct across both kinds of line breaks.
- repeat the same checks with `Ctrl-J` → verify: newline insertion behavior matches `Shift-Enter`.
- trigger a normal `Enter` submit on a short prompt → verify: submit behavior is unchanged.
- run targeted `themion-cli` tests covering input layout helpers and cursor placement → verify: explicit newline and wrap regressions are covered automatically.
- run `cargo check -p themion-cli` after implementation → verify: the fix compiles cleanly in the default configuration.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the fix also compiles cleanly in the Stylos-enabled configuration.

## Implementation checklist

- [x] identify the current mismatch between logical textarea cursor state and rendered wrapped input layout
- [x] add a shared helper for input visual-line counting and visible cursor placement
- [x] route input-height calculation and terminal cursor placement through the same layout helper
- [x] ensure newline insertion paths mark the input dirty and request redraw normally
- [x] add regression coverage for explicit newline insertion and long wrapped-input cursor tracking
- [x] update any affected CLI docs if their wording is now incomplete or misleading
- [x] update `docs/README.md` with the new PRD entry

## Implementation notes

The implemented fix landed in `crates/themion-cli/src/tui.rs` with these concrete changes:

- added `InputLayoutMetrics` and `input_layout_metrics(...)` to compute visual wrapped-line count plus rendered cursor row and column from the input text, cursor byte position, and input width
- updated the draw path to use the same layout helper for both input-height calculation and terminal cursor placement
- kept `Shift-Enter` and `Ctrl-J` newline insertion behavior while making the rendered cursor move correctly after explicit newlines and soft wraps
- added regression tests covering explicit newline cursor movement, wrapped long-line cursor tracking, and wide-character wrapping behavior

Validation run for the implemented slice:

- `cargo test -p themion-cli input_layout_tests -- --nocapture` → passed
- `cargo check -p themion-cli` → passed
- `cargo check -p themion-cli --features stylos` → passed
