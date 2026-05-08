# PRD-108: Agent-Attributed Web Transcript Tool Events

- **Status:** Implemented
- **Version:** v0.66.0
- **Scope:** `themion-core`, `themion-cli`, `themion-cli-web-ui`
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-08

## Summary

Backfilled PRD for the implemented web transcript attribution work.

The `themion-cli --web` transcript must preserve the same agent ownership that the TUI transcript shows. Status, remote, and tool rows must make it clear which local agent produced the event. Tool completion must merge into the corresponding tool call by live `tool_call_id`, so the browser shows one completed tool-call row instead of separate generic `tool` and `tool_done` rows.

## Goals

- Preserve agent attribution in the web transcript for assistant, status, remote, turn, and tool events.
- Carry live `tool_call_id` from core tool execution into CLI transcript entries.
- Merge tool completion into the matching web transcript tool call by `tool_call_id`.
- Keep a safe adjacency fallback only when an older or incomplete event lacks a tool id.
- Render completed tool calls as one agent-owned row, such as `[master] TOOL_CALL ✓`.
- Keep TUI behavior intact while making the web projection richer.

## Non-goals

- Do not redesign the TUI transcript layout.
- Do not add a new persistent transcript schema.
- Do not change provider tool-call semantics beyond surfacing the existing tool-call id.
- Do not introduce a new browser architecture outside `themion-cli --web`.
- Do not make the web UI own runtime decisions or reconstruct agent state independently.

## Background & Motivation

The TUI transcript already prefixes runtime events with the producing agent, for example `[master]`, `[smith-1]`, or `[smith-2]`. The web transcript API and SPA had partial support for `agent_id`, but the browser rendered several event kinds with generic labels such as `status`, `remote`, `tool`, and `tool_done`.

This made multi-agent web sessions hard to read. A user could see a tool event or Stylos event but not quickly tell which agent caused it. Tool calls were especially noisy because the call and completion appeared as separate generic rows.

The core runtime already has a real model/provider tool-call id in the tool execution loop. Surfacing that id in live events lets web mode correlate `ToolStart` and `ToolEnd` by identity rather than by display text.

## Design

### Live tool-call ids

`themion-core` live tool events carry the provider/model tool-call id:

- `AgentEvent::ToolStart { tool_call_id, name, arguments_json, display_arguments_json }`
- `AgentEvent::ToolEnd { tool_call_id }`

The agent emits the same `tc.id` for start and end events for a tool execution.

Required behavior:

- the id is optional at the type boundary for compatibility with any future event source that cannot provide one
- normal agent tool execution should provide `Some(id)` for both start and end
- consumers should not invent a stable id when none is available

### CLI transcript entries

`themion-cli` carries the live id into transcript entries:

- `Entry::ToolCall { agent_id, tool_call_id, detail, reason }`
- `Entry::ToolDone { tool_call_id }`

The TUI can keep rendering the same compact visual check mark behavior. The added id is for consumers that need reliable correlation, especially web mode.

### Web transcript projection

The web transcript API includes `tool_call_id` and `completed` on each `WebChatEntry`.

For tool events:

- a `ToolCall` creates a `tool_call` web entry with `agent_id`, `tool_call_id`, detail, and `completed: false`
- a `ToolDone` first searches for a matching `tool_call` entry by `tool_call_id`
- if found, it sets that entry to `completed: true` and does not emit a separate row
- if no id is available, it may fall back to the immediately preceding uncompleted tool call
- if no match exists, it may emit a standalone completed `tool_done` row as a defensive fallback

This keeps the web transcript compact while preserving real correlation when available.

**Alternative considered:** merge only by adjacency. Rejected as the final behavior because adjacency is only a fallback; live `tool_call_id` gives a stronger contract and better future concurrency safety.

### Web UI rendering

The browser transcript separates the owner label from the event kind:

- owner label prefers `agent_id` whenever present
- source labels such as `stylos`, `runtime`, or `board` are used only for non-agent rows
- kind labels render in uppercase, such as `STATUS`, `REMOTE`, `TOOL_CALL`, and `TURN_DONE`
- completed tool calls render as `TOOL_CALL ✓`
- row keys include `tool_call_id` so completion updates target the right row

Expected tool-call display:

```text
[master]
TOOL_CALL ✓
stylos_request_talk ...
```

## Changes by Component

### `themion-core`

- Extend `AgentEvent::ToolStart` and `AgentEvent::ToolEnd` with `tool_call_id`.
- Emit the current tool call id from the agent tool execution loop.

### `themion-cli`

- Thread tool ids through app-state event handling into TUI transcript entries.
- Keep the TUI output behavior stable.
- Add web transcript projection fields for `tool_call_id` and `completed`.
- Merge web tool completion by `tool_call_id` with adjacency fallback.

### `themion-cli-web-ui`

- Prefer `agent_id` for transcript owner labels.
- Render uppercase kind labels.
- Render merged completed tool calls as `TOOL_CALL ✓`.
- Use `tool_call_id` in list keys.
- Rebuild embedded web assets served by `themion-cli --web`.

## Edge Cases

- If `ToolEnd` has no id, web mode falls back to the most recent uncompleted tool call.
- If no matching tool call exists, web mode can still show a standalone completed tool event rather than dropping it.
- Non-agent events keep source labels so rows such as Stylos transport events can still show `STYLOS`-like ownership.
- Existing callers that do not inspect `tool_call_id` continue to work.

## Testing

- `cargo test -p themion-cli-web-ui` → verify web label and kind rendering helpers.
- `cargo test -p themion-cli web::tests::tool_done_merges_into_previous_tool_call` → verify web projection merges completion into the matching tool call.
- `cargo check -p themion-core -p themion-cli` → verify default builds after event shape changes.
- `scripts/build_web_assets.sh` → verify and regenerate embedded web assets.
- `cargo check -p themion-core --all-features` → verify core all-features build.
- `cargo check -p themion-cli --all-features` → verify CLI all-features build.
