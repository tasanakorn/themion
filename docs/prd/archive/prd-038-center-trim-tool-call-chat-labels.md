# PRD-038: Center Trim Tool Call Chat Labels

- **Status:** Implemented
- **Version:** v0.23.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-22

## Summary

- Current tool-call chat labels keep only about 60 characters from the start and then add an ellipsis.
- That works poorly for long values where the important part is near the end, such as deep paths, long commands, or exact instance identifiers.
- Change the trimming policy to center trim instead of end trim.
- Use the symbol `󱑼` as the visible center-trim marker.
- Keep tool-call labels short and readable in the TUI; this PRD changes the display strategy, not the underlying tool arguments or persistence.

## Goals

- Make tool-call chat labels more informative when long values contain important prefixes and suffixes.
- Replace the current end-only truncation style with center trimming.
- Use `󱑼` consistently as the trim marker for these tool-call labels.
- Preserve compact one-line tool-call display in the TUI.
- Update docs so they describe the new trimming behavior accurately.

## Non-goals

- No change to actual tool-call execution or arguments passed to tools.
- No redesign of the TUI layout for tool-call lines.
- No expansion of tool-call labels into multi-line previews in this slice.
- No broader rewrite of unrelated truncation or wrapping behavior elsewhere in the app.
- No change to history window trimming, transcript storage, or message persistence.

## Background & Motivation

### Current state

Themion emits a short detail string for `AgentEvent::ToolStart` so the chat panel can show readable one-line tool activity such as `read: <path>` or `shell: <command>`. The current implementation in `crates/themion-core/src/agent.rs` trims long string values to about 60 characters and appends an ellipsis.

The docs currently describe this as “detail truncated to 60 chars” and “tool call display labels are truncated to 60 chars to keep TUI lines readable.”

### Why end-only truncation is often not useful

End-only truncation keeps only the start of the value. For some tool calls, that loses the most useful distinguishing information:

- long file paths often differ most at the end
- shell commands may have important trailing flags or filenames
- Stylos instance or agent-targeted strings may need both the beginning and the end to stay recognizable
- note and workflow display strings may contain meaningful suffixes

Center trimming keeps context from both sides while still respecting the compact label budget.

**Alternative considered:** increase the fixed maximum length and keep end-only ellipsis. Rejected: more width helps somewhat, but it still hides the tail that is often the useful part.

## Design

### Replace end trim with center trim for tool-call detail values

The helper that currently truncates long values for `tool_call_detail` should use center trimming instead of keeping only the prefix.

Normative behavior:

- if the display value fits within the configured maximum, show it unchanged
- if it exceeds the maximum, keep a prefix and suffix from the original value
- insert exactly one center marker between those preserved parts
- the final displayed string should stay within the same approximate width budget currently used for tool-call detail values

This keeps one-line labels compact while exposing both ends of long values.

**Alternative considered:** trim from the left for some tools and from the right for others. Rejected: that adds per-tool special cases when a single center-trim rule is easier to understand and maintain.

### Use `󱑼` as the center-trim marker

The visible trim marker for truncated tool-call values should be `󱑼` rather than `...` or `…`.

Normative behavior:

- center-trimmed values use `󱑼` exactly once
- values that do not need trimming do not add the marker
- docs and examples should refer to the same marker so the behavior is explicit

This gives the UI a distinctive symbol that clearly communicates omitted middle content.

**Alternative considered:** use the existing Unicode ellipsis `…` in the middle. Rejected: the requested marker is more visually distinctive and avoids looking like ordinary sentence punctuation.

### Keep the existing compact label budget unless implementation proves a small adjustment is needed

This PRD is about where trimming happens, not about making tool-call labels dramatically longer.

Normative behavior:

- the current approximate width budget of about 60 characters remains the baseline target
- implementation may adjust the exact preserved prefix/suffix split to account for the single marker character cleanly
- any such adjustment should remain visually close to the current label width so TUI readability stays stable

That keeps the UI change focused on informativeness rather than on layout churn.

**Alternative considered:** make center trimming configurable. Rejected: the request is for a straightforward display-policy improvement, not a new settings surface.

### Apply the new behavior where tool-call detail strings are constructed

The center-trim policy should be implemented in the shared helper path used by `tool_call_detail` rather than by adding ad hoc per-tool formatting in the TUI.

This keeps the display logic centralized and consistent across tool types such as shell, filesystem, history, workflow, Stylos, and board detail labels.

**Alternative considered:** post-process already formatted tool-call lines in `themion-cli`. Rejected: the detail value should be produced correctly at the source rather than patched later in presentation code.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/agent.rs` | Replace the current end-truncation helper used by `tool_call_detail` with center trimming that uses `󱑼`. |
| `docs/architecture.md` | Update tool-call display wording from generic 60-char truncation to center trimming with the marker `󱑼`. |
| `docs/engine-runtime.md` | Document that tool-call chat labels now use center trimming for long values while remaining compact. |
| `docs/README.md` | Add this PRD to the PRD index. |

## Edge Cases

- a value shorter than the trim limit → verify: it is shown unchanged with no `󱑼` marker.
- a value exactly at the trim limit → verify: it is shown unchanged.
- a value one character over the limit → verify: it is center-trimmed once and still fits the intended width budget.
- a long path where only the suffix is distinctive → verify: the displayed label still shows the filename or trailing path segment.
- a long shell command with important trailing flags → verify: the displayed label preserves both the command prefix and trailing arguments.
- a very short trim budget in tests or helper-level checks → verify: the helper still returns a valid string without panicking or producing malformed Unicode boundaries.

## Migration

No schema or config migration is required.

This is a display-policy change only:

- tool-call execution and stored tool messages remain unchanged
- chat/TUI tool-call labels become more informative for long values
- documentation should stop describing the behavior as simple end truncation with ellipsis

## Testing

- test the trim helper with a short string → verify: the string is unchanged.
- test the trim helper with a long string → verify: the result keeps the beginning and end with `󱑼` in the center.
- test the trim helper with Unicode content → verify: the result remains valid Unicode and the marker appears only when trimming occurs.
- trigger a long `fs_read_file` or `shell_run_command` tool label in the TUI-facing event path → verify: the displayed detail uses center trimming rather than suffix ellipsis.
- review updated docs for tool-call display behavior → verify: they describe center trimming with `󱑼` rather than generic 60-character truncation.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: the workspace still compiles cleanly.

## Implementation checklist

- [x] replace the current end-truncation helper used for tool-call detail values
- [x] add center-trim behavior that preserves both prefix and suffix content
- [x] use `󱑼` as the center-trim marker
- [x] keep the resulting label width close to the current compact display budget
- [x] add or update tests for helper behavior and representative tool labels
- [x] update `docs/architecture.md` and `docs/engine-runtime.md`
- [x] update `docs/README.md` with the new PRD entry


## Implementation notes

The implemented slice landed with these concrete behaviors:

- `crates/themion-core/src/agent.rs` now uses a shared center-trim helper for tool-call detail values
- long tool-call labels preserve both prefix and suffix content with `󱑼` in the middle
- the compact display budget remains about 60 characters for trimmed values
- helper-level tests now cover short strings, long strings, Unicode-safe trimming, and representative tool-call detail formatting
- `docs/architecture.md` and `docs/engine-runtime.md` now describe center trimming rather than end truncation
