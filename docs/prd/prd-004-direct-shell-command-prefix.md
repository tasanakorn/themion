# PRD-004: Direct Shell Command Prefix in the TUI

- **Status:** Implemented
- **Version:** v0.3.0
- **Scope:** `themion-cli` (input handling, TUI command execution, output rendering); docs
- **Author:** Tasanakorn (design) + Claude Code (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Let the user run a shell command directly from the TUI by starting the input with `!`.
- Execute the remainder of the line as a shell command without sending it to the model.
- Show the command and its output in the conversation pane so the interaction stays visible in session context.
- Keep the behavior lightweight and local to the CLI/TUI layer rather than routing through the model tool loop.

## Non-goals

- No interactive shell session, PTY, or full-screen subprocess support.
- No streaming subprocess output into the pane for the first version; the command completes, then output is rendered.
- No shell history expansion beyond the `!` prefix itself.
- No change to the model-facing `bash` tool; the assistant can still invoke tools independently.
- No sandboxing, timeout, or privilege restrictions beyond the current process environment.

## Background & Motivation

### Current state

Themion already exposes a `bash` tool in `themion-core`, but that path is model-mediated: the user asks in natural language, the model decides whether to invoke the tool, and the result is fed back into the agent loop. The TUI itself does not currently offer a direct "run this shell command now" shortcut.

For common local actions such as `!ls`, `!pwd`, or `!cargo test -p themion-cli`, routing through the model adds latency and unnecessary ambiguity. The user already knows the exact command they want to run. A direct prefix keeps the interaction inside the terminal workflow and avoids spending model tokens on something the local machine can do immediately.

This behavior belongs in `themion-cli`, not `themion-core`, because it is a user-input affordance specific to the interactive TUI. It is not reusable runtime logic and should not be represented as an assistant turn or tool-call round-trip.

## Design

### `!` prefix semantics

When the user submits input from the TUI:

- if the first character is `!`, the input is treated as a direct shell command
- the command string is the remainder of the line after the leading `!`
- the command is executed in the current project directory
- the input is not sent to `Agent::run_loop`
- the command output is appended to the conversation pane as a local UI event

Whitespace immediately after `!` is ignored for execution purposes, so `!ls`, `! ls`, and `!   ls -la` all execute the expected command string.

An input consisting only of `!` or `!` plus whitespace does not execute anything. The TUI appends a short assistant-style feedback line such as `empty shell command` and returns to idle.

**Alternative considered:** introduce a slash command such as `/sh <command>`. Rejected: the `!` prefix is faster to type, matches common terminal affordances, and keeps direct shell execution visually distinct from themion control commands such as `/config`.

### Execution model

The command runs through the same shell mechanism already documented for the `bash` tool: `sh -c` on Unix-like systems. The working directory is `App.project_dir`, matching the existing project-local behavior the TUI already presents.

The implementation lives in `themion-cli/src/tui.rs` and follows the existing async event pattern used for agent execution:

1. detect the `!` prefix during submit
2. append a local entry showing the command being executed
3. set the busy flag so the user cannot overlap a second operation accidentally
4. spawn an async task that runs the command
5. send an `AppEvent` back with the completed stdout/stderr payload and exit status
6. render the result in the conversation pane and clear the busy flag

This keeps the UI responsive and preserves the current one-operation-at-a-time model used by the TUI.

**Alternative considered:** execute synchronously inside the input handler. Rejected: a slow command would block event handling, freeze repainting, and make the TUI feel hung.

### Output rendering

The conversation pane should make direct shell execution clearly local and user-initiated.

The minimum rendering shape is:

- one entry showing the command, for example `! cargo check -p themion-cli`
- one entry showing the combined stdout/stderr text after the command completes
- optionally one short status line when the exit code is non-zero

Output is shown verbatim except for the same practical display constraints the TUI already has for long wrapped text. The first version should not invent a separate scrollable subprocess widget; the normal conversation scroll behavior is sufficient.

If both stdout and stderr are empty, the TUI should still render a short confirmation such as `(no output)` so the user can tell the command ran.

Non-zero exit status is not treated as a TUI-level failure. The command result is still shown, and the user remains in control. This mirrors normal terminal behavior more closely than surfacing it as an exceptional crash path.

**Alternative considered:** hide stderr unless the command fails. Rejected: users expect shell output to include both streams, and many commands use stderr for warnings or progress.

### Event and state handling

The TUI needs a separate event path for local shell execution so that it does not pretend to be an assistant or tool call.

A minimal shape is:

```rust
AppEvent::ShellComplete {
    command: String,
    output: String,
    exit_code: Option<i32>,
}
```

The existing `agent_busy` gate is reused. While a direct shell command is running:

- additional chat submits are blocked
- additional `!` commands are blocked
- slash commands that are currently guarded by the same busy state remain guarded

This keeps behavior consistent with the rest of the app and avoids needing separate concurrency rules for local commands versus model turns.

### Relationship to history and the model

Direct `!` execution is a TUI-local affordance. It is visible in the conversation pane for the user, but it does not become part of the persisted agent conversation unless the implementation explicitly chooses to persist local UI entries in a future PRD.

For this PRD, the simplest behavior is:

- do not send the `!` command through the model
- do not convert it into a `tool` message
- do not add it to `Agent.messages`
- do not store it in SQLite history

This preserves the architectural boundary: model history remains model turns; TUI-local shell shortcuts remain a local interface convenience.

**Alternative considered:** inject the command output into the agent history automatically so the assistant can immediately reference it. Rejected: that changes conversation semantics and persistence unexpectedly. If desired later, that should be a separate explicit feature with clear UX around what becomes model-visible.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Detect `!`-prefixed input during submit; route it to a local async shell execution path instead of `Agent::run_loop`; add an `AppEvent::ShellComplete` variant; append command/output entries to the conversation pane; reuse busy-state handling. |
| `docs/architecture.md` | Document the new TUI-local `!` shortcut as distinct from model-mediated tool calls. |
| `README.md` | Add a brief mention of `!<command>` in the TUI usage or key workflow documentation if the shortcut is considered user-facing enough for the top-level README. |
| `docs/README.md` | Add the PRD-004 row to the PRD table. |

## Edge Cases

- Input is exactly `!` or only whitespace after it → do not execute; show `empty shell command`.
- Command exits with non-zero status → show output and exit code; do not crash the TUI.
- Command writes only to stderr → show that text normally.
- Command writes nothing to either stream → show `(no output)`.
- Command is long-running → TUI remains responsive because execution happens on a spawned task, but the app stays busy until completion.
- Command cannot be spawned at all (`sh` missing, OS error, invalid environment) → show a concise failure message in the pane and clear busy state.
- User enters `/config ...` or normal chat text without `!` prefix → existing behavior remains unchanged.
- User wants the model to decide whether to run shell commands → they still use normal natural-language prompts and the existing `bash` tool path.

## Migration

This feature is additive and requires no config migration.

Existing users can start using `!<command>` immediately in the TUI after upgrade. Workflows that do not use the prefix are unchanged.

Because the feature is CLI-local, print mode behavior remains unchanged unless extended by a future PRD.

## Testing

- start the TUI and enter `!pwd` → verify: the command is not sent to the model, the pane shows the command and the current project directory output.
- enter `! ls` → verify: leading whitespace after `!` is ignored and directory output is shown.
- enter `!` → verify: no subprocess is launched and the pane shows `empty shell command`.
- enter `!false` → verify: the pane shows completion with a non-zero exit indication and the TUI remains usable.
- enter a command that prints to stderr, such as `!sh -c 'echo err 1>&2'` → verify: stderr text is visible in the pane.
- enter a command with no output, such as `!true` → verify: the pane shows `(no output)` or equivalent confirmation.
- start a long-running command, then try to submit another message while it is running → verify: the existing busy guard blocks the second action consistently.
- after a successful `!echo hello`, send a normal chat message → verify: normal agent execution still works and the two flows do not interfere.
- restart themion after running `!echo hello` and inspect persisted history behavior → verify: the direct shell command was not stored as an agent turn in SQLite.
- run `cargo check -p themion-cli` after implementation → verify: the new event path and submit handling compile cleanly.
