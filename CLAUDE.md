# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

- `cargo build` — builds all workspace crates
- `cargo run -p themion-cli` — TUI mode (requires `OPENROUTER_API_KEY`)
- `cargo run -p themion-cli -- "your prompt"` — print mode: single turn, then exit
- `cargo run -p themion-cli -- --dir /path/to/project` — TUI with explicit project directory
- `cargo build --release` — optimized binary with LTO + strip

**Never invoke `rustc` directly.** Always use `cargo build` to verify compilation. Direct `rustc` invocations drop stray `.rlib` artifacts in the working directory.

## Environment Variables

| Var                  | Required | Default                  |
| -------------------- | -------- | ------------------------ |
| `OPENROUTER_API_KEY` | yes      | —                        |
| `OPENROUTER_MODEL`   | no       | `minimax/minimax-m2.7`   |
| `SYSTEM_PROMPT`      | no       | generic assistant prompt |

## Architecture

Cargo workspace with three crates:

- **`crates/themion-core`** — library crate with four modules:
  - **`client.rs`** — `ChatClient` (formerly `OpenRouterClient`): wraps `reqwest`, sends `POST /chat/completions` with SSE streaming, owns all serde types (`Message`, `ToolCall`, `FunctionCall`, `ChatResponse`).
  - **`tools.rs`** — `tool_definitions()` returns the OpenAI-format JSON array sent to the LLM on every request; `call_tool(name, args_json, &ToolCtx)` dispatches to six tool implementations (`read_file`, `write_file`, `list_directory`, `bash`, `recall_history`, `search_history`). Long argument values are truncated to 60 chars in display labels. Tool errors are returned as strings so the model sees them.
  - **`agent.rs`** — `Agent` holds `Vec<Message>` (full history) and owns `ChatClient`, `DbHandle`, `session_id`, `project_dir`, and `turn_boundaries`. `run_loop(user_input)` is the core loop: push user → call LLM (streaming) → execute tool calls → repeat up to 10 iterations → emit `TurnDone`. Windowed context: only the last `window_turns` (default 5) turns are sent to the API; older turns live in SQLite and are accessible via `recall_history`/`search_history` tools.
  - **`db.rs`** — `DbHandle` wraps a SQLite connection (WAL mode). Schema: `agent_sessions`, `agent_turns`, `agent_messages`, and an FTS5 virtual table for full-text search. Path: `~/.local/share/themion/history.db`.

- **`crates/themion-cli`** — binary crate (`themion`):
  - **`main.rs`** — parses CLI args (`--dir`), loads config, dispatches to print mode or TUI mode.
  - **`tui.rs`** — Ratatui TUI. Layout: conversation pane (top, `Min 1`) → input box (3 rows) → status bar (1 row, bottom). Startup shows a block-character ASCII art banner with version/profile/model/project. Thinking indicator animates with a braille spinner at 150 ms ticks. Status bar token counts use thousands separators.
  - **`config.rs`** — XDG config file (`~/.config/themion/config.toml`), multi-profile support.

- **`web/`** — placeholder for a future frontend.

## Adding a New Tool

1. Add the OpenAI function schema to the `json!([...])` array in `tool_definitions()` (`crates/themion-core/src/tools.rs`).
2. Add a match arm in `call_tool()` in the same file.
3. No other files need to change — `agent.rs` passes `tool_definitions()` on every call automatically.

## Known Limitations

- `bash` tool has no timeout — a hung subprocess blocks the agent indefinitely.
- No path sandboxing on any filesystem tool.
- Max agent loop iterations hardcoded to 10 in `agent.rs`.
- `window_turns` (context window size) hardcoded to 5; not user-configurable.
