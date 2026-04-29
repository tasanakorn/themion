# PRD-068: Keep the TUI Chat Composer Usable When Input Exceeds the Visible Height

- **Status:** Implemented
- **Version:** v0.44.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-29

## Implementation status

Landed in `v0.44.0` as a focused `themion-cli` composer usability fix. The shipped behavior adds textarea-owned viewport scroll state, viewport-aware local rendering, cursor-following multiline visibility inside the existing bounded composer height, and lightweight `↑` / `↓` overflow cues when lines are hidden above or below the visible slice.

## Summary

- Themion's TUI chat composer previously stopped showing the active editing line once a multiline draft grew past the small visible input box.
- The landed change keeps the composer compact, but now treats it as a scrollable viewport that follows the cursor through longer wrapped or multiline drafts.
- `themion-cli` now uses local textarea viewport state and local rendering instead of drawing the full draft through a plain `Paragraph`.
- The first landing also adds lightweight overflow arrows so users can tell when content is hidden above or below the visible slice.
- Existing submit, paste-burst, history recall, and multiline editing behavior stay intact while long-draft visibility is fixed.

## Goals

- Keep the TUI input composer usable when the draft exceeds the visible composer height.
- Preserve a bounded bottom-pane composer instead of letting long drafts consume the whole terminal.
- Ensure the on-screen viewport automatically follows the cursor so newly typed lines remain visible.
- Define a Themion-local scroll model inside the existing local textarea/composer ownership split.
- Borrow the implementation pattern from `codex-rs` closely enough that future composer improvements remain easy to compare and reuse.
- Add light user-visible affordances that indicate when part of the draft is currently scrolled out of view.

## Non-goals

- No full-screen editor mode in this PRD.
- No horizontal scrolling; wrapped multiline display should remain the default.
- No new complex text-editing feature set beyond what is needed for bounded vertical viewport scrolling.
- No transcript scrolling redesign or changes to conversation navigation semantics.
- No slash-command, history, or paste-burst redesign except where those paths must preserve the new composer viewport invariants.

## Background & Motivation

### Current state

Themion already replaced `tui-textarea` with a Themion-owned `TextArea` and `ChatComposer` in PRD-060. That gave the project local control over multiline editing, wrapping, and cursor tracking.

However, the current draw path in `crates/themion-cli/src/tui.rs` still renders the composer body as a plain `Paragraph` over the entire input text:

- it computes `input_height` as `(desired_height + 2).clamp(3, 8)`
- it renders `Paragraph::new(display_input).wrap(Wrap { trim: false }).block(input_block)`
- it places the cursor using the full wrapped-row position from `cursor_pos_with_state(...)`

That means the visible composer is capped at about 6 interior text rows, but the rendered content itself is not viewport-scrolled. Once the wrapped cursor row exceeds the visible area, the newest lines fall below the input box and effectively become invisible while editing.

The current local `crates/themion-cli/src/textarea.rs` also confirms the gap:

- `TextAreaState` is an empty marker type
- `cursor_pos_with_state(width, state)` ignores any viewport state and returns the absolute wrapped cursor row
- rendering is delegated to a plain `Paragraph` rather than a stateful textarea renderer that can clip to a visible window

This is a usability bug rather than merely a missing enhancement: long prompts can still be typed, but the user loses sight of where they are editing.

### Comparison with `../codex`

Local docs-first/source-confirmed comparison against `../codex/codex-rs` shows that Codex uses the pattern Themion is currently missing:

- `tui/src/bottom_pane/textarea.rs` defines `TextAreaState { scroll: u16 }`
- `effective_scroll(...)` keeps the cursor within the visible wrapped-line window
- the textarea implements stateful rendering and updates `state.scroll` during render
- the composer keeps a bounded height while relying on the textarea viewport rather than trying to show the whole buffer at once

So the useful lesson from `codex` is not "make the composer taller." It is "keep a bounded composer, but make the local textarea own vertical viewport scrolling and cursor visibility."

**Alternative considered:** remove the height clamp and let the composer grow indefinitely. Rejected: that avoids hidden lines temporarily, but it steals terminal space from the transcript and does not provide a stable long-draft editing model.

## Design

### 1. Add vertical viewport state to the local textarea

`crates/themion-cli/src/textarea.rs` should stop treating `TextAreaState` as an empty marker and instead make it the home for composer viewport state.

Required behavior:

- add a scroll field that tracks the first visible wrapped line
- compute wrapped lines once per width as today, then derive an effective scroll for the current viewport height
- when content fits within the visible height, scroll stays at `0`
- when content exceeds the visible height, adjust scroll so the cursor remains visible
- preserve the current wrapped-display behavior and UTF-8-safe cursor logic

The essential invariant should match the codex pattern:

- cursor is always on screen
- no scrolling when content fits
- scrolling is expressed in wrapped visual lines, not only explicit newline rows

**Alternative considered:** keep scroll state in `App` or `ChatComposer` and treat `TextArea` as a passive string wrapper. Rejected: viewport math depends on wrapped-line knowledge already owned by `TextArea`, so splitting the state would make correctness harder.

### 2. Render the composer through the local textarea, not a plain paragraph

The current TUI draw path should stop rendering the input body via `Paragraph::new(display_input)` and instead let the local textarea render the visible slice directly.

Required behavior:

- reserve the same bordered composer box shape unless a small visual tweak is justified
- compute the inner textarea rectangle from the block padding/borders
- render only the visible wrapped lines inside that rectangle
- update viewport scroll state during render so cursor placement and visible content stay in sync
- place the cursor using viewport-relative coordinates rather than absolute wrapped-row coordinates

This is the critical product fix: the visible lines and the cursor must agree about which slice of the draft is currently on screen.

**Alternative considered:** keep paragraph rendering and manually trim the displayed string before passing it to the widget. Rejected: that duplicates wrapping logic, invites cursor drift bugs, and works against the local textarea abstraction introduced in PRD-060.

### 3. Preserve the compact composer height, but define it as a viewport height

Themion should continue using a compact composer rather than letting long drafts take over the terminal. The current clamp is acceptable as a starting point, but the product contract should describe it as a viewport cap rather than as the total editable capacity.

Required behavior:

- keep a bounded composer height with a small minimum and a current-style maximum near the existing 7-line visible target
- treat that height as the visible viewport for a longer draft, not as a hard draft-size limit
- keep `desired_height` useful for short drafts so the composer still grows naturally before scrolling is needed

The PRD does not require the exact current `clamp(3, 8)` numbers to remain forever, but the initial implementation should preserve today's layout unless testing shows a small adjustment materially improves usability.

**Alternative considered:** immediately raise the max visible height substantially. Rejected: it may be reasonable later, but it is not required to solve the current invisibility bug and would change layout behavior more than necessary.

### 4. Add light overflow cues for hidden content

When the draft is taller than the visible composer viewport, the UI should provide a small cue that some content is hidden above or below the visible slice.

Acceptable first-step cues include one of:

- a tiny up/down indicator inside the composer border or padding area
- a short status hint such as `lines 5-10 of 14`
- a subtle ellipsis/marker on the first or last visible line region

Requirements:

- cues should be lightweight and should not noticeably reduce editing width
- cues should appear only when overflow exists
- cues should distinguish hidden-above from hidden-below when practical

This is intentionally secondary to cursor visibility. If implementation simplicity requires splitting the work, automatic scrolling is phase-one-critical and overflow cues are a small follow-on within the same PRD.

**Alternative considered:** no overflow cues at all. Rejected: automatic scrolling fixes the worst failure, but users still benefit from knowing the draft extends beyond the current visible window.

### 5. Keep existing editing and history behavior compatible with the new viewport model

Viewport scrolling should be a rendering/state improvement, not a rewrite of input semantics.

The following should continue to work as they do today:

- Enter vs submit/newline behavior
- paste-burst handling
- history recall and draft restoration
- arrow-key cursor movement across logical lines
- wrapped cursor tracking for wide characters and UTF-8 input

Additional viewport requirements:

- after history recall or draft restoration, the viewport should scroll to the cursor position rather than leaving the cursor off-screen
- after paste of a long multiline block, the viewport should follow the resulting cursor position
- after moving the cursor upward into older wrapped lines, the viewport may scroll upward enough to keep the cursor visible

**Alternative considered:** reset viewport scroll to the top after every synthetic content change. Rejected: that would make recalled or pasted long drafts jump to the wrong end and feel broken.

### 6. Acceptance target for the first implementation

This PRD should be considered implemented when all of the following are true:

- the composer still uses a bounded visible height in the TUI
- typing or pasting beyond the visible multiline limit no longer makes the active editing line disappear below the input box
- moving the cursor through a long wrapped or multiline draft keeps the cursor visible within the composer viewport
- the local textarea owns viewport scroll state and exposes rendering/cursor methods that account for it
- `tui.rs` renders the composer through the local textarea path rather than a plain paragraph of the full text
- at least one light overflow cue is shown when hidden lines exist above or below the visible slice, or any intentionally deferred cue is called out explicitly in the PRD status note if the first landing is phased
- touched docs describe the bounded-scrollable composer behavior accurately

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/textarea.rs` | Add wrapped-line viewport state, effective-scroll calculation, viewport-aware cursor positioning, and stateful rendering of only the visible composer slice. |
| `crates/themion-cli/src/chat_composer.rs` | Own and preserve textarea viewport state across editing, recall, paste, and cursor-movement flows; expose any helper methods needed by the TUI draw path. |
| `crates/themion-cli/src/tui.rs` | Replace plain `Paragraph` input rendering with local textarea rendering, keep the bounded composer layout, and show lightweight overflow cues when the draft exceeds the visible viewport. |
| `docs/architecture.md` | Document that the local composer now uses a bounded vertically scrollable viewport rather than showing only the first visible slice of a long draft. |
| `docs/README.md` | Add and track this PRD in the PRD index table. |
| `docs/prd/prd-068-keep-the-tui-chat-composer-usable-when-input-exceeds-the-visible-height.md` | Record the product requirement and later implementation status for multiline composer scrolling. |

## Edge Cases

- a single very long unbroken line wraps into many visual rows → verify: the viewport scrolls by wrapped rows and keeps the cursor visible.
- a draft contains explicit newlines plus wrapped long lines → verify: scrolling logic handles both without cursor drift.
- the user recalls a long history entry → verify: the restored draft opens with the cursor visible at the expected end or cursor position.
- a large multiline paste lands near the composer limit → verify: the viewport follows the pasted tail instead of leaving it below the visible box.
- the terminal is resized narrower while editing a long draft → verify: wrapped-line recalculation preserves a valid scroll offset and keeps the cursor visible.
- the terminal is resized taller or shorter while the composer is scrolled → verify: scroll is clamped into a valid range and does not leave blank invalid viewport space.
- wide Unicode characters appear near wrap boundaries → verify: viewport-relative cursor placement stays aligned with rendered text.
- the draft shrinks after deletions from a previously long message → verify: scroll snaps back as needed and does not leave the viewport stranded below the remaining content.

## Migration

This is a TUI behavior improvement with no database, config, or protocol migration.

Rollout guidance:

- keep the current composer size behavior as the initial visual baseline
- land viewport scrolling under the existing local textarea/composer architecture
- update TUI/runtime docs only where they describe multiline composer behavior

## Testing

- type a draft that exceeds the current visible multiline height → verify: the newest line remains visible inside the composer instead of disappearing below the box.
- move the cursor upward and downward through a long multiline draft → verify: the viewport scrolls enough to keep the cursor visible in both directions.
- paste a multiline block longer than the visible composer height → verify: the visible slice follows the cursor after paste.
- recall a previously submitted long multiline draft from history → verify: the restored draft remains editable and the cursor is on screen.
- resize the terminal narrower while a long draft is open → verify: wrapped rows reflow, scroll remains valid, and the cursor stays visible.
- resize the terminal taller after the composer had scrolled → verify: extra visible space is used and scroll clamps correctly.
- run `cargo check -p themion-cli` after implementation → verify: the touched crate builds cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the touched crate still builds with the feature enabled.
- run `cargo check -p themion-cli --all-features` after implementation → verify: the touched crate still builds cleanly across feature combinations.

## Implementation checklist

- [x] add vertical scroll state to the local textarea and define cursor-visible viewport invariants
- [x] implement viewport-aware textarea rendering and cursor positioning
- [x] wire `ChatComposer` to preserve textarea viewport state across normal edits and synthetic draft changes
- [x] replace plain paragraph input rendering in `tui.rs` with local textarea rendering
- [x] add lightweight overflow cues for hidden-above / hidden-below content
- [x] update the relevant docs/PRD index entries
- [x] validate multiline typing, paste, history recall, and resize behavior
