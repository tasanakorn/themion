# PRD-048: Remove Long Navigation Shortcut Hints from the TUI Statusline

- **Status:** Implemented
- **Version:** v0.29.3
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-24

## Summary

- The TUI statusline currently includes a long navigation hint segment: `| PgUp/PgDn page | Alt-g latest | Alt-t review`.
- Remove that shortcut-hint segment from the always-visible statusline.
- Keep the actual navigation shortcuts unchanged.
- Preserve the core statusline metrics such as provider/model, token counts, context usage, activity state, and navigation state.
- Treat this as a small patch-level UI cleanup, not a navigation redesign.

## Goals

- Remove the literal statusline segment `| PgUp/PgDn page | Alt-g latest | Alt-t review` from the TUI.
- Reduce horizontal noise in the statusline so runtime state and token/context information remain easier to scan.
- Preserve the existing keyboard behavior for `PageUp`, `PageDown`, `Alt-g`, and `Alt-t`.
- Preserve existing long-session navigation modes and transcript review behavior.
- Keep the change narrowly scoped to statusline display text and any directly related docs.

## Non-goals

- No removal or remapping of `PageUp`, `PageDown`, `Alt-g`, or `Alt-t` shortcuts.
- No redesign of long-session transcript navigation.
- No new command palette, help overlay, or keybinding discovery UI in this PRD.
- No change to statusline token accounting, context-window display, provider/model display, or activity-state display.
- No broad TUI layout refactor.
- No change to persistent history, transcript storage, or harness runtime behavior.

## Background & Motivation

### Current state

The TUI statusline currently renders runtime and navigation information together with a persistent shortcut hint segment shaped like:

```text
| PgUp/PgDn page | Alt-g latest | Alt-t review
```

The shortcuts are useful, but the always-visible reminder consumes horizontal space on every frame. On narrower terminals this makes the statusline feel busy and can push more important state information into a visually dense line.

### Why remove the always-visible hint

The statusline is most useful when it highlights current state: model/runtime activity, token usage, context-window information, and navigation mode. The long shortcut hint is static instructional text. Once users learn the shortcuts, repeating it in the statusline adds more noise than value.

Removing the hint improves the statusline by:

- shortening the line
- reducing visual clutter
- keeping attention on live runtime state
- avoiding a cramped appearance on narrower terminals

**Alternative considered:** keep the hint because it helps discoverability. Rejected: this PRD prioritizes a cleaner statusline; discoverability can be handled by docs or a future help overlay without keeping this long static hint always visible.

### Why shortcuts should remain unchanged

The request is specifically to remove the statusline text, not to remove navigation capabilities. Existing long-session navigation remains useful and should keep working exactly as before.

**Alternative considered:** remove the shortcuts and the text together. Rejected: that would turn a display cleanup into a behavior change and would regress existing navigation workflows.

## Design

### Remove only the long shortcut hint segment

The TUI statusline format should stop appending the exact segment:

```text
| PgUp/PgDn page | Alt-g latest | Alt-t review
```

Normative direction:

- remove this text from the statusline builder in `crates/themion-cli/src/tui.rs`
- keep the surrounding statusline fields and separators coherent after removal
- do not leave a trailing separator or awkward double spacing where the hint used to be
- do not remove other useful statusline state such as token/context information or navigation mode

**Alternative considered:** hide the hint only when terminal width is small. Rejected: the user requested removal, and conditional display would preserve the same clutter in wider terminals.

### Preserve key handling and navigation state

This PRD is display-only.

Normative direction:

- keep `PageUp` and `PageDown` page navigation behavior unchanged
- keep `Alt-g` returning to latest content unchanged
- keep `Alt-t` transcript review behavior unchanged
- keep any statusline navigation state such as `tail`, `browse`, or `review` if it is already present separately from the shortcut hint

**Alternative considered:** replace the removed hint with shorter key labels. Rejected: the requested outcome is to remove the hint segment, not to abbreviate it.

### Keep documentation aligned with the new statusline

If docs describe the statusline as always showing those shortcut hints, update that wording. Documentation may still mention the shortcuts as available controls, but should not claim the statusline displays the removed segment.

Normative direction:

- update statusline/TUI docs only where they reference the removed always-visible hint
- keep shortcut behavior documented where relevant
- avoid broad documentation rewrites unrelated to this display cleanup

**Alternative considered:** leave docs untouched because the code change is tiny. Rejected: docs should not teach a statusline layout that no longer exists.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Remove the `| PgUp/PgDn page | Alt-g latest | Alt-t review` segment from the statusline string while preserving other statusline fields. |
| `docs/architecture.md` | Update TUI/statusline wording if it says the navigation shortcut hint is always shown. |
| `docs/engine-runtime.md` | Update runtime/TUI wording if it documents the removed statusline hint. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Edge Cases

- statusline renders on a narrow terminal → verify: removing the hint shortens the line and does not leave dangling separators.
- statusline renders while browsing history → verify: navigation state still indicates the current mode if that state was already shown separately.
- user presses `PageUp` or `PageDown` after the hint is removed → verify: page navigation still works.
- user presses `Alt-g` after the hint is removed → verify: the view still returns to latest content.
- user presses `Alt-t` after the hint is removed → verify: transcript review still opens and closes as before.
- docs mention shortcut availability → verify: they describe behavior without claiming the removed hint remains on the statusline.

## Migration

This is a display-only patch with no config, schema, or data migration.

Expected rollout shape:

- remove the static shortcut hint from statusline rendering
- keep existing keybindings and navigation behavior unchanged
- update docs only where they refer to the removed always-visible hint
- no user action is required after upgrade

## Testing

- start Themion and inspect the statusline → verify: it no longer contains `PgUp/PgDn page`, `Alt-g latest`, or `Alt-t review`.
- inspect the rendered statusline after removal → verify: there is no trailing separator or awkward spacing where the hint was removed.
- press `PageUp` and `PageDown` in a long transcript → verify: page navigation behavior is unchanged.
- press `Alt-g` while browsing history → verify: the view returns to latest content.
- press `Alt-t` → verify: transcript review still toggles as before.
- run `cargo check -p themion-cli` after implementation → verify: the default CLI build compiles cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the Stylos-enabled CLI build compiles cleanly.

## Implementation checklist

- [x] remove the exact shortcut hint segment from the statusline format in `crates/themion-cli/src/tui.rs`
- [x] ensure the remaining statusline formatting has clean separators and spacing
- [x] confirm `PageUp`, `PageDown`, `Alt-g`, and `Alt-t` handling is untouched
- [x] update docs if they mention the removed always-visible statusline hint
- [x] update `docs/README.md` with this PRD entry
- [x] run `cargo check -p themion-cli`
- [x] run `cargo check -p themion-cli --features stylos`


## Implementation notes

Implemented as a patch-level TUI cleanup:

- `crates/themion-cli/src/tui.rs` now renders the bottom statusline as rate-limit, token, cached-token, and context information only, without appending `| PgUp/PgDn page | Alt-g latest | Alt-t review`.
- The key handling for `PageUp`, `PageDown`, `Alt-g`, and `Alt-t` was left unchanged.
- No architecture/runtime docs mentioned the removed always-visible hint, so no behavioral docs needed updates beyond the PRD index/status.
- Repository crate versions were bumped to `0.29.3`; `Cargo.lock` was checked and updated for the crate package versions.

Validation run for the implemented slice:

- `cargo check -p themion-cli` → passed
- `cargo check -p themion-cli --features stylos` → passed
