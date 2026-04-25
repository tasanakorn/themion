# PRD-055: Fix TUI Input Dirty Detection for Non-ASCII Typing and Paste-Burst Flushes

- **Status:** Implemented
- **Version:** v0.34.2
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-25

## Summary

- Themion's current dirty-gated TUI redraw path has a correctness gap for some input updates.
- Non-ASCII typing currently inserts text into the input widget but can skip `mark_dirty_input()` and `request_draw()`, so the text may not appear until a later ASCII key causes a redraw.
- Paste-burst flushing can similarly insert buffered text without immediately marking the input dirty, so pasted text may remain invisible until a later normal key event.
- Fix this as a narrow TUI correctness bug: every visible input mutation must mark the input region dirty and request a redraw through the normal scheduler.
- Keep the existing dirty-region model and paste-burst optimization; the goal is to close redraw holes, not to remove coalescing.

## Goals

- Make non-ASCII typing appear immediately in the TUI input area, just like normal ASCII typing.
- Make explicit paste events and paste-burst flushes appear immediately after the text is inserted.
- Preserve the current dirty-region redraw architecture from PRD-042 rather than bypassing it.
- Keep paste-burst coalescing and Enter-suppression behavior unless a touched path must change to restore redraw correctness.
- Make input-mutation redraw rules easier to audit so future input paths do not silently skip draw requests.

## Non-goals

- No redesign of the input widget or migration away from `tui_textarea`.
- No removal of the existing paste-burst detection heuristic.
- No broader TUI rendering refactor beyond the input dirty-detection bug.
- No change to transcript persistence, command parsing, or model-turn submission semantics.
- No attempt in this PRD to solve unrelated IME/platform-specific input issues beyond the visible redraw gap described here.

## Background & Motivation

### Current state

`docs/architecture.md` describes the TUI as a request-driven redraw loop with dirty-region tracking, and PRD-042 established that visible changes should mark the relevant region dirty and request a frame through the redraw scheduler.

That general model is implemented in `crates/themion-cli/src/tui.rs` with `UiDirty`, `request_draw(...)`, and `handle_draw_event(...)`. For normal ASCII typing in the default key-input path, Themion already does the right thing:

- mutate the `TextArea`
- call `mark_dirty_input()`
- call `request_draw(...)`

However, not every input mutation currently follows that contract.

Two visible problem reports now point to the same class of bug:

- typing non-English / non-ASCII characters does not visibly redraw until a later ASCII or English keystroke happens
- pasted text can be buffered or flushed into the input state but not appear until a later normal keypress

Source inspection shows why these failures are plausible in the current implementation:

- `handle_non_ascii_char(...)` inserts the key into the input widget, but it does not mark the input dirty or request a redraw itself
- `handle_key_event(...)` returns early after calling `handle_non_ascii_char(...)`, so the later generic input path that would normally mark dirty is skipped
- `handle_paste(...)` inserts pasted text and clears paste-burst state, but it does not itself mark dirty or request a redraw
- `handle_key_event(...)` may flush buffered paste text through `FlushResult::Paste(text) => handle_paste(self, text)` before deciding whether to request a draw, so some paste-burst flushes can mutate the input state without immediately scheduling a frame

This means the redraw architecture is correct in principle but incomplete in specific input paths.

**Alternative considered:** treat this as an unavoidable terminal or IME limitation. Rejected: the current code already shows direct state mutation paths that simply fail to mark the UI dirty consistently.

### Why this should be treated as a dirty-detection bug, not a paste feature rewrite

The user-visible symptom is "text does not appear until later typing," but the underlying problem is narrower: some input mutations bypass the redraw contract introduced by PRD-042.

The paste-burst heuristic may still be acceptable as a redraw-reduction strategy. The bug is that when buffered text is finally committed into the input widget, Themion must treat that as a visible input change and request a frame.

Similarly, non-ASCII typing should not need a special rendering path; it should use the same dirty-marking and frame-request rules as ASCII typing.

**Alternative considered:** remove paste-burst buffering entirely so every input event redraws immediately. Rejected: unnecessary scope expansion when the correctness gap can be fixed by restoring consistent dirty marking at commit points.

## Design

### Make every visible input mutation mark the input region dirty

Themion should enforce one simple correctness rule in the TUI input path:

- if a code path changes visible input text or cursor position, it must mark the input region dirty
- if that mutation can leave the app otherwise clean, it must also request a frame through the normal scheduler

Normative direction:

- treat direct `TextArea` mutations as redraw-relevant unless they are immediately followed by another helper that already guarantees dirty marking and draw scheduling
- prefer helper-level enforcement so call sites do not each have to remember redraw policy manually
- keep the dirty target narrow as `input` rather than escalating to full invalidation

This keeps PRD-042's redraw contract intact while making the input behavior correct.

**Alternative considered:** rely on the next tick or unrelated key event to make the inserted text visible. Rejected: visible input feedback must be immediate and should not depend on later unrelated events.

### Fix the non-ASCII key path to follow the same redraw contract as ASCII typing

The non-ASCII branch in `handle_key_event(...)` should no longer be a redraw hole.

Normative direction:

- after `handle_non_ascii_char(...)` commits text into the input widget, mark the input region dirty and request a draw before returning, or move that responsibility into a helper used by the branch
- preserve existing paste-buffer flushing behavior before the non-ASCII insertion when needed
- do not route non-ASCII input through ASCII-specific paste-burst heuristics unless there is a documented reason to do so

The important requirement is behavioral parity: non-ASCII text should appear as promptly as ASCII text.

**Alternative considered:** send non-ASCII keys through the generic `_ => self.input.input(key)` branch. Rejected for now: possible, but the narrower and less risky fix is simply to restore dirty marking and redraw scheduling in the existing special path.

### Treat paste-burst flush commit points as visible input updates

Buffered paste text is not visible until it is committed into the `TextArea`, so that commit point must be redraw-aware.

Normative direction:

- when `FlushResult::Paste(text)` is applied, mark the input region dirty before the event handler continues or returns
- when `flush_before_modified_input()` returns pasted text that gets committed through `handle_paste(...)`, mark the input region dirty in the same event path
- explicit `AppEvent::Paste` handling should continue to mark dirty and request a frame as it already does, but the shared helper path should also be correct so flush-based commits cannot skip redraw scheduling
- avoid duplicating redraw requests unnecessarily when the caller already intends to request a draw in the same branch; correctness comes first, coalescing can still handle duplicate requests safely

This ensures that buffered paste optimizations do not accidentally suppress visible updates.

**Alternative considered:** keep `handle_paste(...)` as a pure text-insertion helper and require every caller to remember the redraw policy. Rejected: that makes the bug easy to reintroduce and spreads correctness responsibility too widely.

### Centralize input-commit behavior where practical

The current input path has several places that mutate the `TextArea`:

- explicit paste events
- paste-burst flushes
- non-ASCII typing
- normal typing and history navigation
- newline insertion

Normative direction:

- where practical, introduce a small helper or helper pattern for "input changed visibly" so direct `TextArea` mutations are easier to audit
- prefer keeping the helper local to `crates/themion-cli/src/tui.rs` rather than creating a broader abstraction
- use the helper for at least the bug-prone non-ASCII and paste-flush paths in this slice

The goal is not abstraction for its own sake. The goal is to make redraw correctness harder to forget in input-handling code.

**Alternative considered:** patch only the two known branches and leave the rest ad hoc. Rejected: acceptable as a temporary hotfix, but the PRD should still direct implementation toward a slightly more auditable helper shape.

### Keep redraw scheduling request-driven and coalesced

This bug should be fixed within the redraw system introduced by PRD-042, not around it.

Normative direction:

- keep using `request_draw(...)` and `FrameRequester` rather than calling `terminal.draw(...)` directly from input code
- allow coalescing to collapse multiple near-simultaneous input redraw requests
- do not weaken the dirty-gating behavior in `handle_draw_event(...)`
- continue using `draw_skip_clean_count` and existing draw counters for observability

This preserves the existing architecture while closing a correctness gap.

**Alternative considered:** bypass dirty gating for input and force a direct immediate draw call. Rejected: inconsistent with the current TUI redraw design and unnecessary for this fix.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Fix non-ASCII input and paste-flush paths so any committed visible input mutation marks `input` dirty and schedules a redraw through the normal frame requester. |
| `crates/themion-cli/src/tui.rs` | Optionally add a small local helper for redraw-aware input commits so direct `TextArea` mutations are easier to audit. |
| `crates/themion-cli/src/paste_burst.rs` | No heuristic redesign required, but touched behavior should remain compatible with redraw-correct paste-burst flush handling. |
| `docs/architecture.md` | No major rewrite required; only update if implementation semantics around redraw-aware input commits need clarification. |
| `docs/README.md` | Add this PRD to the PRD index table. |

## Edge Cases

- type one or more non-ASCII characters into an otherwise idle input box → verify: each visible input update appears without waiting for a later ASCII keypress.
- type a mix of ASCII and non-ASCII characters rapidly → verify: input remains visible and ordered correctly, and redraw requests still coalesce normally.
- receive an explicit terminal paste event containing Unicode text → verify: the pasted text appears immediately after insertion.
- trigger paste-burst buffering from rapid character input that later flushes as pasted text → verify: the committed buffered text appears as soon as the flush happens, not only after a later unrelated key.
- flush buffered paste text and then press Enter quickly → verify: the input contents visible to the user match what will be submitted.
- use history navigation or newline insertion after a buffered paste flush → verify: no pasted text disappears or remains visually delayed.
- keep the app otherwise idle while only the input changes → verify: the redraw path still uses `input` dirty marking rather than requiring unrelated status/conversation updates.
- type non-ASCII text while transcript review is closed → verify: the main input region redraws correctly and no overlay-specific logic is required.

## Migration

This is an internal TUI correctness fix with no data, config, or protocol migration.

Expected rollout shape:

- keep the existing dirty-region and frame-request scheduler design
- patch redraw holes in input-mutation paths
- preserve current paste-burst heuristics unless a touched branch must change to restore visible correctness

## Testing

- type Thai, Japanese, or other non-ASCII characters into the input box → verify: each character becomes visible immediately without needing a later ASCII key.
- paste multiline text through the terminal paste event path → verify: the inserted text appears immediately in the input area.
- trigger rapid plain-character input that enters the paste-burst path and then idles long enough to flush → verify: the flushed text appears as soon as the flush is committed.
- type ASCII, then non-ASCII, then ASCII again in one line → verify: the full input remains visible in order and no segment is delayed until the next redraw-causing key.
- press Enter shortly after a paste-burst flush or explicit paste → verify: the submitted content matches the text already visible in the input box.
- run `cargo check -p themion-cli` after the fix → verify: the TUI input-path changes compile cleanly in the default configuration.
- run `cargo check -p themion-cli --features stylos` after the fix if touched code is shared across feature builds → verify: the TUI input-path changes remain feature-safe.

## Implementation checklist

- [x] audit all direct visible `TextArea` mutation paths in `crates/themion-cli/src/tui.rs`
- [x] fix the non-ASCII typing branch so it marks `input` dirty and schedules a redraw
- [x] fix paste-burst flush commit paths so committed text marks `input` dirty and becomes visible immediately
- [x] keep redraw scheduling on the existing `FrameRequester` path rather than adding direct draw calls
- [x] add or tighten tests around redraw-aware input mutation paths where practical
- [x] update `docs/README.md` with the new PRD entry

## Implementation notes

Implemented in v0.34.2.

What landed:

- `crates/themion-cli/src/tui.rs` now routes redraw-relevant input commits through small local helpers so visible input mutations consistently call `mark_dirty_input()` and `request_draw(...)`
- non-ASCII typing now flushes any pending paste-burst text through the redraw-aware helper and then commits the non-ASCII key through the same redraw-aware input path
- paste-burst flushes from `flush_if_due(...)` and `flush_before_modified_input()` now use the redraw-aware paste commit helper instead of mutating input silently
- explicit `AppEvent::Paste` handling now also uses the same helper, so explicit paste and buffered paste-flush paths share one visible-input commit contract
- workspace and crate versions were bumped to `0.34.2`, and `Cargo.lock` updated accordingly

Known limitation:

- this slice validates compile-time correctness and code-path consistency, but it does not yet add a dedicated automated TUI interaction test harness for IME or paste redraw behavior
