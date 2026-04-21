# PRD-025: Long-Session Chat History Navigation in the TUI

- **Status:** Implemented
- **Version:** v0.14.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-21

## Summary

- Keep the normal chat pane as the default live view, but stop treating long-session review as just tiny repeated scrolling.
- Add an explicit distinction between following the latest output and browsing older history.
- When the user scrolls up, keep them in history-browsing mode instead of snapping back to the bottom when new output arrives.
- Add stronger navigation controls such as real page movement and a clear jump-to-latest action.
- Add a read-only transcript-review mode so long conversations are easier to inspect than in one endlessly growing wrapped text block.
- Keep the first implementation in `themion-cli` using the current in-memory session transcript.
- Do not change persistent history, prompt assembly, provider behavior, or other `themion-core` semantics.

## Goals

- Make long chat sessions remain practical to review inside the TUI after the conversation grows far beyond the visible viewport.
- Improve scroll behavior so reviewing earlier assistant output, tool traces, and user prompts does not become increasingly sluggish or disorienting as `entries` grows.
- Add an explicit transcript-review path for historical conversation navigation instead of relying only on incremental viewport scrolling.
- Preserve the current harness, persistence, and prompt-window behavior while improving only the user-facing chat browsing experience in `themion-cli`.
- Document the navigation model clearly enough that future TUI work has a stable target for long-session behavior.

## Non-goals

- No redesign of persistent history storage, context-windowing, or `history_recall` / `history_search` semantics in `themion-core`.
- No migration of chat rendering to a fully virtualized widget framework outside the current Ratatui-based TUI.
- No requirement to expose a full multi-pane session browser, search UI, or cross-session history manager in this PRD.
- No change to agent turn semantics, workflow state handling, or tool execution order.
- No requirement to match Codex's UI architecture exactly; Codex is a comparison input, not a compatibility target.

## Background & Motivation

### Current state

The current TUI keeps rendered conversation state in `App.entries` and displays it as one wrapped `Paragraph` over the conversation pane. Vertical review uses one `scroll_offset` counter adjusted by small fixed increments for mouse wheel, `PageUp`, `PageDown`, and `Alt` + arrow input.

Today that behavior is implemented as a single reverse-offset measured from the bottom of the rendered conversation:

- `App.scroll_offset` stores how far the user has moved away from the latest content
- `scroll_up()` increases that offset by a fixed `3`
- `scroll_down()` decreases it by a fixed `3`
- submitting input resets `scroll_offset` back to `0`
- rendering computes `Paragraph::line_count(width)` for the full wrapped transcript and derives the visible scroll position from that total

That simple design works for short and medium sessions, but the user asked specifically about very long chats. In the current implementation, long sessions have a few practical drawbacks:

- the viewport scrolls in small fixed steps rather than in larger semantic jumps
- the user has no explicit "jump to latest" or transcript-review mode
- the rendered conversation is treated as one continuously wrapped text block, so navigation precision degrades as the number of wrapped visual lines grows
- the same state is trying to serve both live-tail following and historical browsing
- alternate-screen TUI usage means terminal-native scrollback is not a reliable fallback in all environments

The architecture docs already describe persistent history and TUI behavior, but they do not yet describe a dedicated long-session review model.

### Why terminal scrollback is not sufficient

A fullscreen TUI cannot assume terminal scrollback will reliably preserve and expose the conversation, especially when alternate screen mode is in use. This is a known terminal constraint rather than a Themion-specific bug.

That means long-session usability must come from the TUI itself, not from hoping the surrounding terminal or multiplexer will provide convenient review behavior.

**Alternative considered:** rely on the terminal's own scrollback and keep the in-app behavior unchanged. Rejected: alternate screen mode and multiplexer behavior make that inconsistent, and the request is specifically about improving in-app handling for long chats.

### What Codex suggests without being a direct template

Reviewing the nearby `../codex` repository shows a relevant design direction rather than a drop-in implementation:

- Codex documents that alternate-screen mode can prevent normal terminal scrollback in environments such as Zellij.
- Codex treats this as a TUI UX problem and offers mode-level mitigation plus a separate transcript pager for reviewing conversation history.

That is useful for Themion because it reinforces two design conclusions:

- long-session review should not depend exclusively on terminal scrollback
- a dedicated transcript-review interaction can be cleaner than endlessly stretching the main live chat viewport

Themion does not need to copy Codex's exact controls or architecture, but the PRD should adopt the same underlying lesson: long-history review deserves a first-class UI path.

**Alternative considered:** copy Codex's exact transcript pager and alternate-screen policy. Rejected: Themion's TUI structure is different, and the request is about improving chat scrolling behavior first, not cloning another product's full terminal UX.

## Design

### Split live-chat scrolling from transcript review

The TUI should distinguish between two related but different tasks:

- following the live tail of the conversation while the session is active
- reviewing older chat history once the session becomes long

Proposed behavior:

- the main conversation pane remains the default live view
- normal scroll commands continue to work in the main pane for nearby navigation
- a dedicated transcript-review mode can be opened from the main chat view to inspect long history more comfortably
- transcript-review mode is read-only and exits cleanly back to the live chat view

This keeps the common case simple while giving long sessions a purpose-built review path.

**Alternative considered:** keep only one scrolling conversation pane and continue adding more scroll bindings to it. Rejected: it does not solve the core problem that long-history review and live-tail following are different interaction modes.

### Introduce explicit follow-tail versus browsed-history state

The TUI should track whether the user is effectively pinned to the latest conversation output or has intentionally moved into historical review.

Normative behavior:

- new sessions begin in follow-tail mode
- when the user scrolls upward or opens transcript review, the UI enters browsed-history mode
- while in browsed-history mode, newly arriving assistant chunks and entries must not forcibly yank the viewport back to the bottom
- a clear command should return the user to follow-tail mode and jump to the newest content
- submitting a new prompt should return to follow-tail mode for the first implementation

Implementation direction:

- replace the implicit "`scroll_offset == 0` means tail-follow" assumption with explicit navigation state
- keep the state CLI-local in `App` rather than pushing it into `themion-core`
- treat mouse scrolling, page navigation, and transcript-review entry as state transitions into browsed-history mode

This prevents the common long-chat frustration where incoming output disrupts ongoing review.

**Alternative considered:** always snap to the newest content whenever new output arrives. Rejected: it makes historical inspection frustrating precisely when sessions are long and active.

### Add larger and more predictable navigation primitives

The current fixed-step scroll behavior should be expanded into a more complete navigation model.

Minimum expected navigation behaviors:

- line or small-step scrolling for nearby movement
- page-sized scrolling for faster traversal through long content
- jump-to-top
- jump-to-bottom / return-to-latest
- transcript-review open/close shortcuts surfaced in help text or status copy

The exact key bindings can follow surrounding TUI conventions, but the behavior contract should be stable even if key choices evolve during implementation.

Implementation direction:

- preserve existing mouse wheel support
- upgrade `PageUp` and `PageDown` to page-oriented behavior instead of sharing the same tiny step as the current scroll helpers
- keep `Alt` + arrow navigation meaningful, even if final bindings change during implementation
- add one obvious recovery path back to the latest content

**Alternative considered:** increase the fixed scroll step from `3` to a larger constant. Rejected: it improves one symptom slightly but still leaves the TUI without semantic navigation or explicit tail-follow state.

### Treat transcript review as entry-aware rather than one giant wrapped block

The current conversation rendering effectively behaves like one large wrapped text paragraph. For very long sessions, that makes navigation and future enhancements harder because the UI works only in visual-line offsets rather than around meaningful chat boundaries.

Transcript review should instead operate on conversation entries with enough structure to support predictable navigation.

Normative design direction:

- retain existing `Entry` data as the source transcript model for the first implementation where practical
- derive review rendering from entry boundaries instead of treating the whole transcript as one opaque text blob
- make it possible to jump by larger units such as pages while keeping stable position near entry boundaries
- preserve wrapped rendering, but avoid making wrapped total-line count the only navigation primitive

Implementation direction:

- avoid a large rendering rewrite in the first slice
- it is acceptable for the live conversation pane to keep using `Paragraph` initially if transcript-review mode introduces the entry-aware behavior first
- any cached or precomputed measurement added for entry-aware review should remain local to `themion-cli/src/tui.rs` unless later refactoring proves worthwhile

This keeps the initial change incremental while creating room for later additions such as per-entry jumps or transcript search.

**Alternative considered:** immediately replace the entire chat rendering stack with a fully virtualized custom widget. Rejected: too much scope for a first long-session navigation improvement.

### Prefer in-memory transcript review first, with persistent-history extension left open

Themion already persists messages to SQLite, but the main long-session request can be solved first using the current in-memory transcript for the active session.

Initial scope:

- transcript review covers the current live session's visible transcript
- opening transcript review does not require fetching older sessions from SQLite
- persistent-history tools remain the way the model accesses older context

Future-compatible expectation:

- the transcript-review path should not block future expansion into persisted session replay or transcript search

**Alternative considered:** couple transcript review directly to SQLite-backed multi-session browsing in the first change. Rejected: that turns a focused TUI navigation improvement into a larger history-browser project.

### Keep alternate-screen behavior separate from the scroll fix

Codex's docs are useful because they show that alternate-screen policy and transcript review are related but separable concerns.

For Themion, this PRD should keep scope focused:

- improve in-app long-history navigation regardless of terminal scrollback availability
- do not bundle alternate-screen configuration changes into the same implementation unless they become strictly necessary
- document that TUI-native history review is the primary solution for long-chat usability

This preserves a small, testable feature slice.

**Alternative considered:** solve the problem primarily by disabling alternate screen in more environments. Rejected: that changes terminal integration policy and still does not give the TUI first-class long-history navigation.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Replace the current long-chat navigation assumptions built around one bottom-relative `scroll_offset` and fixed `3`-line scroll helpers with explicit follow-tail versus browsed-history state, larger navigation primitives, and a read-only transcript-review mode or equivalent entry-aware review path. |
| `docs/architecture.md` | Document the TUI's long-session chat navigation model, including live-tail behavior, transcript review, and the fact that terminal scrollback is not the primary review path. |
| `docs/engine-runtime.md` | Clarify that long-session transcript review is a CLI-local display/navigation feature and does not alter core history or prompt assembly semantics. |
| `docs/README.md` | Keep this PRD indexed and update status/version when implementation lands. |
| `docs/prd/prd-025-long-session-chat-history-navigation.md` | Record the proposed UX, implementation constraints, and implementation checklist for the feature. |

## Edge Cases

- the agent is actively streaming while the user is reading older content → new output should accumulate without forcibly snapping the viewport back to the bottom until the user explicitly returns to latest.
- a transcript contains very large tool-output or shell-output entries → page navigation should remain predictable and should not require hundreds of small-step scroll actions.
- the user opens transcript review during an active turn → the review view should stay read-only and must not interfere with interruption, workflow, or live event processing.
- the user exits transcript review after a long pause while more output arrived → the UI should offer a clear return-to-latest behavior rather than leaving the user uncertain about where the live tail is.
- the viewport is resized while reviewing long history → the review position should remain stable as practical and should not reset to the top or bottom unexpectedly.
- a short session never grows beyond one screen → the default live chat experience should remain effectively unchanged.

## Migration

This is an additive TUI-only usability improvement.

Migration expectations:

- no database migration is required
- no provider or workflow behavior changes are required
- existing sessions remain valid
- long sessions gain better in-app review behavior without changing persistent history semantics

If implementation introduces new key bindings or help text, those should be documented in the user-facing docs touched by the final change.

## Testing

- start `themion-cli`, generate a conversation longer than several screens, and use the normal conversation view scroll controls → verify: nearby scrolling remains responsive and predictable after the transcript becomes long.
- scroll upward during active streaming output → verify: the viewport stays in browsed-history mode and does not snap back to the bottom on each new chunk.
- invoke the return-to-latest command after browsing old content → verify: the conversation jumps to the newest visible content and follow-tail behavior resumes.
- open transcript-review mode on a long session → verify: older conversation content can be traversed substantially faster than with repeated small-step scrolling alone.
- resize the terminal while reviewing long history → verify: the review position remains stable enough that the user does not lose their place unexpectedly.
- exercise large tool-output and shell-output entries in the transcript → verify: page navigation remains usable and transcript review does not degrade into line-by-line scrolling.
- run `cargo check -p themion-cli` after implementation → verify: the TUI compiles cleanly with the new navigation state and review behavior.

## Implementation checklist

- [x] add explicit navigation state in `crates/themion-cli/src/tui.rs` for follow-tail versus browsed-history behavior
- [x] stop relying on the implicit assumption that `scroll_offset == 0` fully describes whether the user wants to follow the live tail
- [x] replace or extend the current fixed `3`-line scroll helpers so `PageUp` and `PageDown` perform substantially larger navigation steps
- [x] add a jump-to-latest path that returns the viewport to the newest conversation content and re-enables follow-tail behavior
- [x] ensure incoming assistant chunks and entry appends do not snap the viewport back to the bottom while the user is browsing older content
- [x] add a read-only transcript-review mode or equivalent long-history review path for the current in-memory session transcript
- [x] make transcript review entry-aware enough that very long wrapped transcripts remain navigable without relying only on one giant wrapped paragraph offset
- [x] document the final user-visible navigation behavior in `docs/architecture.md`
- [x] document in `docs/engine-runtime.md` that the feature is CLI-local display state and does not change harness history semantics
- [x] validate with `cargo check -p themion-cli`
