# PRD-060: Replace `tui-textarea` by Following the `codex-rs` Local Textarea + Composer Pattern

- **Status:** Implemented
- **Version:** v0.38.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Themion currently uses `tui-textarea` directly inside `crates/themion-cli/src/tui.rs`, but the project already layers significant custom behavior around it for redraw correctness, paste bursts, multiline cursor tracking, and history interactions.
- `../codex/codex-rs` avoids that split ownership by implementing its own local `TextArea` plus `TextAreaState`, then routing higher-level input policy through a `ChatComposer` that owns paste-burst and submit behavior.
- Themion should now follow that same implementation pattern directly: add a local textarea module, add a local composer/controller module above it, migrate `tui.rs` to delegate input behavior into that layer, and then remove `tui-textarea`.
- The first Themion slice should preserve current Themion behavior rather than importing all codex-rs product features.
- This PRD is implementation-ready for a behavior-preserving architectural migration in `themion-cli`.

## Goals

- Remove the `tui-textarea` dependency from Themion's TUI input path.
- Follow the `codex-rs` implementation pattern closely enough that Themion's input architecture has the same major ownership boundaries:
  - a local textarea/editor primitive
  - a local state object for scroll/render state
  - a higher-level composer/controller that owns input policy
- Preserve current Themion user-visible behavior while migrating to that architecture.
- Move non-ASCII handling, paste-burst handling, wrapped cursor tracking, and future input extensions into Themion-owned code rather than behavior layered on top of a third-party widget.
- Make future input work easier to borrow from `codex-rs` by aligning the module boundaries now.

## Non-goals

- No migration away from Ratatui or Crossterm.
- No full port of codex-rs product features such as attachment placeholders, file/skill popups, remote image rows, or the broader slash-command UX.
- No requirement to adopt every codex-rs editing feature in this slice, such as kill/yank support or text elements, unless Themion explicitly decides it needs them for parity.
- No rewrite of Themion's conversation pane, runtime topology, or agent event model.
- No requirement that module or function names match codex-rs exactly, as long as the architecture and behavior match the intended pattern.

## Background & Motivation

### Current state

Themion's documented TUI architecture remains Ratatui-based and event-driven, with:

- `crates/themion-cli/src/tui_runner.rs` handling terminal orchestration
- `crates/themion-cli/src/tui.rs` handling the main app event loop and presentation

The editable input currently lives directly inside `App` as:

- `input: TextArea<'a>` from `tui-textarea`

Themion already wraps that widget with project-specific logic and helpers such as:

- `commit_input_change(...)`
- `commit_pasted_input(...)`
- `handle_non_ascii_char(...)`
- `set_input_text(...)`
- `set_input_text_and_cursor(...)`
- `input_text_and_cursor_byte(...)`
- `insert_pasted_text(...)`

Recent TUI PRDs confirm that input behavior is already a Themion-local concern:

- PRD-044 fixed multiline newline and wrapped-cursor tracking
- PRD-055 fixed redraw correctness for non-ASCII typing and paste-burst flushes

So the current implementation already behaves like a custom composer layered on top of a third-party textarea. The remaining mismatch is ownership: the backing editor model is still external.

### Research: what `codex-rs` actually implements

Local research in `../codex/codex-rs` shows a more explicit architecture than just "custom textarea instead of `tui-textarea`".

From `../codex/codex-rs/tui/Cargo.toml`:

- the TUI depends on `ratatui` and `crossterm`
- it does **not** depend on `tui-textarea`

From `../codex/codex-rs/tui/src/bottom_pane/textarea.rs`:

- codex-rs defines a local `TextArea`
- it also defines a local `TextAreaState`
- the local editor owns core input/editor state such as:
  - UTF-8 text storage
  - byte-offset cursor position
  - wrap cache
  - preferred column tracking
  - edit operations like `insert_str`, `replace_range`, `set_cursor`, and `input(KeyEvent)`
  - render-facing integration via Ratatui `WidgetRef` and `StatefulWidgetRef`
  - cursor positioning helpers such as `cursor_pos_with_state(...)`
  - height calculation such as `desired_height(...)`

From `../codex/codex-rs/tui/src/bottom_pane/chat_composer.rs`:

- a higher-level `ChatComposer` owns:
  - `textarea: TextArea`
  - `textarea_state: RefCell<TextAreaState>`
  - paste-burst state
  - history behavior
  - submit/newline policy
  - popup and command policy
- the composer explicitly routes key handling through dedicated methods such as:
  - `handle_input_basic(...)`
  - `handle_non_ascii_char(...)`
  - `flush_paste_burst_if_due(...)`
  - `handle_paste_burst_flush(...)`
- full-buffer resets go through local editor APIs such as:
  - `set_text_clearing_elements(...)`
  - `set_text_with_elements(...)`

That means codex-rs is not only using a local editor widget. It is also separating low-level editing from higher-level composer policy in a deliberate way.

**Alternative considered:** interpret codex-rs only as evidence that Themion should write a local text buffer. Rejected: that misses the more important lesson that codex-rs also gives that editor a dedicated local controller/composer layer instead of keeping all policy tangled in the root TUI app object.

### Why Themion should follow that pattern now

Themion already has the same kinds of product-specific concerns that codex-rs isolates in its composer:

- paste-burst timing and flush decisions
- non-ASCII / IME-sensitive input handling
- history navigation
- submit versus newline behavior
- redraw-aware input commits

Today, those concerns are mostly embedded directly in `App` inside `crates/themion-cli/src/tui.rs`. That works, but it keeps editor state, input policy, and broader app concerns too tightly coupled.

Following codex-rs more closely would improve this by separating responsibilities into two local layers:

- a local textarea/editor implementation
- a local composer/controller that owns input policy and editor orchestration

That is the architectural change this PRD now makes implementation-ready.

**Alternative considered:** keep all current policy in `App` and only replace the backing widget. Rejected: it would remove a dependency but would not actually align Themion with the codex-rs implementation pattern the user asked to follow.

## Design

### Design principles

- Keep the terminal stack the same: Ratatui for rendering and Crossterm for input.
- Follow the codex-rs ownership split, not just the codex-rs dependency choice.
- Keep the first slice behavior-preserving for Themion users.
- Start with a narrower feature set than codex-rs if needed, but use the same structural pattern.

### 1. Proposed Themion module layout

Themion should add the following modules in `crates/themion-cli/src/`:

- `textarea.rs`
  - owns the local `TextArea` and `TextAreaState`
  - owns text editing, cursor movement, wrap calculations, and Ratatui-facing render support
- `chat_composer.rs`
  - owns Themion input policy around the local textarea
  - owns paste-burst integration, history navigation behavior, submit/newline policy, and full-buffer resets used by the TUI

The current `tui.rs` should remain the main app/presentation layer, but it should stop owning low-level input mechanics directly.

This naming intentionally mirrors codex-rs closely enough that future cross-reference is easy.

**Alternative considered:** hide the new logic inside `tui.rs` submodules or give it unrelated names. Rejected: the codex-rs-aligned names make the architecture clearer and make future comparison and borrowing easier.

### 2. `textarea.rs` responsibilities

The new `crates/themion-cli/src/textarea.rs` should define:

- `pub(crate) struct TextArea`
- `pub(crate) struct TextAreaState`

Required responsibilities:

- own the editable UTF-8 text buffer
- own byte-offset cursor tracking
- clamp cursor movement and edits to valid char boundaries
- support insertion, deletion, replacement, and newline operations
- support left/right and multiline up/down cursor movement
- support wrap-aware height calculation
- support wrap-aware cursor coordinate calculation for the TUI
- provide a local `input(KeyEvent)`-style editing entry point for simple editing cases
- provide Ratatui-facing rendering support comparable in role to codex-rs `WidgetRef` / `StatefulWidgetRef` integration, either by implementing those traits directly or by exposing an equivalent local rendering path that keeps editor rendering semantics owned here

Themion does not need codex-rs text elements or kill-buffer behavior in this slice, but the type should be shaped so those can be added later without redesigning the module boundary.

**Alternative considered:** implement only text storage and leave wrap/cursor rendering math in `tui.rs`. Rejected: that would keep editor invariants split across modules and would not really follow the codex-rs pattern.

### 3. `chat_composer.rs` responsibilities

The new `crates/themion-cli/src/chat_composer.rs` should define a Themion-local composer/controller object that owns input policy above the textarea.

Recommended shape:

- `pub(crate) struct ChatComposer`
  - owns `textarea: TextArea`
  - owns `textarea_state: TextAreaState` or `RefCell<TextAreaState>`
  - owns or references current paste-burst state
  - owns input-history draft/position behavior currently managed directly in `App`

Required responsibilities:

- route normal key input into the textarea
- own non-ASCII / IME-sensitive handling currently implemented in `tui.rs`
- own paste-burst flush and buffering integration currently coordinated from `tui.rs`
- own full-buffer replace/reset behavior used by history navigation and submission
- own submit/newline interpretation decisions and return a small result enum to `tui.rs`
- expose helper methods for current text, cursor position, input emptiness, and rendered cursor coordinates

This should be narrower than codex-rs `ChatComposer`, because Themion does not currently need popup, attachment, or command-surface features from that module. But it should be close in role and ownership.

**Alternative considered:** move only paste-burst handling into `chat_composer.rs` and leave the rest in `App`. Rejected: that would not produce a clear codex-rs-like ownership boundary.

### 4. `tui.rs` responsibilities after migration

After migration, `crates/themion-cli/src/tui.rs` should:

- own a `ChatComposer` instead of a raw `tui_textarea::TextArea<'a>`
- delegate input editing decisions to the composer
- continue owning broader app concerns such as:
  - transcript state
  - review mode
  - agent lifecycle state
  - draw scheduling
  - shell completion events
  - session orchestration

Normative direction:

- `App` should stop directly mutating the low-level text buffer except through the composer API
- helper functions like `set_input_text(...)`, `set_input_text_and_cursor(...)`, `input_text_and_cursor_byte(...)`, and `insert_pasted_text(...)` should either move into the new modules or disappear into clearer composer/textarea methods

This keeps Themion's broader TUI structure intact while moving input concerns into codex-rs-like local ownership.

**Alternative considered:** let `tui.rs` keep direct access to the textarea and use the composer only for a few policy paths. Rejected: that would blur the boundary the migration is intended to create.

### 5. Behavior-preserving requirements

Following codex-rs structurally does **not** mean adopting codex-rs UX wholesale.

Required parity targets:

- plain typing inserts immediately
- non-ASCII typing remains visible immediately
- multiline editing and wrapped cursor placement continue to work
- `Enter`, `Shift+Enter`, and `Ctrl+J` keep their current Themion meanings
- history up/down behavior remains unchanged
- paste-burst buffering and flush behavior remain unchanged from the user's perspective
- redraw behavior remains on Themion's request-driven dirty-region path from PRD-042 and PRD-055

The migration should therefore copy codex-rs's structure, not silently import codex-rs-specific UX.

**Alternative considered:** adopt codex-rs key semantics directly wherever they differ. Rejected: the requested goal is to follow the implementation pattern, not to silently change Themion UX.

### 6. Migration slices

This migration should land in explicit slices.

#### Slice 1: local textarea primitive

Add `crates/themion-cli/src/textarea.rs` with:

- `TextArea`
- `TextAreaState`
- wrap-aware cursor and height logic
- local editing operations
- minimal tests around text editing, wrapping, and cursor placement

Outcome:

- Themion has a local editor primitive comparable in role to codex-rs `textarea.rs`
- Landed in `crates/themion-cli/src/textarea.rs`

#### Slice 2: local composer/controller

Add `crates/themion-cli/src/chat_composer.rs` with:

- `ChatComposer`
- input result type(s) for submit/newline/no-op handling
- migrated paste-burst integration
- migrated non-ASCII handling
- migrated history navigation and full-buffer reset helpers

Outcome:

- input policy now has a codex-rs-like home outside `App`
- Landed in `crates/themion-cli/src/chat_composer.rs`

#### Slice 3: wire `tui.rs` to the composer

Update `crates/themion-cli/src/tui.rs` so:

- `App` owns the composer instead of `tui_textarea::TextArea<'a>`
- existing input handling delegates into the composer
- direct textarea helper functions are removed or relocated

Outcome:

- the TUI app loop now treats the composer as the input subsystem boundary
- `App` now owns `ChatComposer` and uses local textarea rendering/cursor APIs

#### Slice 4: dependency removal and docs cleanup

After parity is proven:

- remove `tui-textarea` from `crates/themion-cli/Cargo.toml`
- update `docs/architecture.md` to describe the local textarea + composer ownership split
- keep `docs/README.md` and PRD status aligned if the implementation lands

Outcome:

- Themion fully follows the codex-rs-style local ownership pattern in this area
- `tui-textarea` has been removed from workspace and CLI manifests

**Alternative considered:** implement all slices in one large edit. Rejected: the input path is behavior-sensitive, and smaller slices make regressions easier to isolate.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/textarea.rs` | Add a Themion-owned local `TextArea` plus companion state type, following codex-rs's `textarea.rs` pattern for editing, wrap, cursor, and Ratatui-facing render support. |
| `crates/themion-cli/src/chat_composer.rs` | Add a Themion-owned composer/controller layer that owns input policy around the local textarea, following codex-rs's `chat_composer.rs` role at a narrower initial scope. |
| `crates/themion-cli/src/tui.rs` | Replace direct `tui_textarea::TextArea<'a>` ownership and inline input-policy logic with delegation into the new composer/controller. |
| `crates/themion-cli/src/paste_burst.rs` | Preserve existing paste-burst behavior, but integrate it through the new composer/controller layer rather than through `App`-local textarea handling. |
| `crates/themion-cli/src/tui_runner.rs` | No major orchestration change expected; touch only if composer cursor/render APIs require integration changes. |
| `crates/themion-cli/Cargo.toml` | Remove the `tui-textarea` dependency after the local textarea and composer migration is complete. |
| `docs/architecture.md` | Update TUI/input documentation to describe the new local textarea + composer ownership model. |
| `docs/README.md` | Keep the PRD index and later implementation status aligned with landed work. |

## Edge Cases

- type non-ASCII characters rapidly through the current IME-sensitive path → verify: visible input remains immediate and no character is lost during burst detection.
- type enough text to wrap across multiple visual lines → verify: cursor placement, desired input height, and on-screen cursor coordinates remain correct.
- edit in the middle of a multiline draft → verify: insertion, replacement, backspace, and cursor movement remain stable at UTF-8 boundaries.
- trigger paste-burst buffering and then interrupt it with navigation or submit keys → verify: buffered text still flushes or clears under the same rules as today.
- navigate history after editing a draft with wrapped lines → verify: restored text and cursor placement match current behavior.
- resize the terminal while a multiline draft is present → verify: wrapped-line metrics recompute correctly and the cursor remains on-screen.
- clear or replace the whole draft through a command path → verify: the local composer/controller and local textarea stay synchronized after full-buffer resets.
- open transcript review or other non-input UI states and then return to editing → verify: the local editor/composer state persists exactly as before.

## Migration

This is an internal TUI architecture migration with no data-format or config migration.

Rollout requirements:

- land the local textarea module first
- land the local composer/controller layer next
- delegate `App` input behavior into that layer
- remove `tui-textarea` only after parity is verified
- update docs so Themion's TUI description reflects the new local ownership boundaries

If regressions appear, the split should make review easier because editor mechanics and composer policy can be inspected separately.

## Testing

- implement `textarea.rs` and run `cargo check -p themion-cli` → verify: the new local editor module compiles cleanly in the default CLI build.
- implement `textarea.rs` and run `cargo check -p themion-cli --all-features` → verify: the new local editor module remains feature-safe across CLI builds.
- implement `chat_composer.rs` and run `cargo check -p themion-cli` → verify: the composer/controller layer compiles cleanly against the local textarea.
- wire `tui.rs` to the composer and run `cargo check -p themion-cli --features stylos` → verify: the shared TUI input path remains compatible with the feature-enabled build.
- finish the migration and run `cargo check -p themion-cli --all-features` → verify: all feature-gated CLI paths still compile cleanly after dependency removal.
- exercise plain typing, backspace, cursor movement, and multiline editing in TUI mode → verify: behavior matches current Themion semantics.
- exercise non-ASCII typing and explicit paste in TUI mode → verify: the local composer/editor preserves the redraw and input correctness fixed by PRD-055.
- exercise history up/down, `Enter`, `Shift+Enter`, and `Ctrl+J` in TUI mode → verify: submission and newline behavior remain unchanged.
- exercise terminal resize with a multiline draft in progress → verify: wrap-aware height and cursor positioning remain correct with the local textarea state model.
- review `cargo tree -p themion-cli` or equivalent dependency output after migration → verify: `tui-textarea` is no longer part of the crate's dependency graph.

## Implementation checklist

- [x] add `crates/themion-cli/src/textarea.rs` with a Themion-owned local `TextArea` type and companion `TextAreaState`
- [x] give the local textarea wrap-aware height and cursor-position APIs comparable in role to codex-rs `desired_height(...)` and `cursor_pos_with_state(...)`
- [x] add `crates/themion-cli/src/chat_composer.rs` with a Themion-owned composer/controller layer around the local textarea
- [x] migrate non-ASCII handling, paste-burst handling, history reset/restore behavior, and submit/newline interpretation into the composer/controller layer
- [x] migrate `App` in `crates/themion-cli/src/tui.rs` to delegate input editing and policy into the composer/controller
- [x] remove direct `tui_textarea::TextArea<'a>` usage and delete obsolete helper paths in `tui.rs`
- [x] remove `tui-textarea` from `crates/themion-cli/Cargo.toml`
- [x] update `docs/architecture.md` to describe the local textarea + composer split
- [x] update `docs/README.md` and PRD-060 status/notes when implementation lands

## Appendix: codex-rs implementation evidence

Observed local evidence from `../codex/codex-rs`:

- `tui/Cargo.toml` uses `ratatui` and `crossterm` and does not depend on `tui-textarea`.
- `tui/src/bottom_pane/textarea.rs` defines both `TextArea` and `TextAreaState` locally.
- that local textarea owns editing mechanics and render/cursor behavior, including:
  - `input(KeyEvent)`
  - `replace_range(...)`
  - `desired_height(...)`
  - `cursor_pos_with_state(...)`
  - Ratatui `WidgetRef` and `StatefulWidgetRef` implementations
- `tui/src/bottom_pane/chat_composer.rs` owns higher-level input policy and directly stores:
  - `textarea: TextArea`
  - `textarea_state: RefCell<TextAreaState>`
- that composer routes editing through explicit methods including:
  - `handle_input_basic(...)`
  - `handle_non_ascii_char(...)`
  - `flush_paste_burst_if_due(...)`
  - `handle_paste_burst_flush(...)`
- full-buffer resets and rehydration go through local editor APIs such as:
  - `set_text_clearing_elements(...)`
  - `set_text_with_elements(...)`

This PRD therefore requires Themion to follow not just the absence of `tui-textarea`, but the same broad local-ownership structure.
