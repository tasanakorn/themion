# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

- `cargo build` — compile (dev profile: no debug symbols, incremental on)
- `cargo run` — REPL mode (requires `OPENROUTER_API_KEY`)
- `cargo run -- "your prompt"` — print mode: single turn, then exit
- `cargo build --release` — optimized binary with LTO + strip

## Environment Variables

| Var                  | Required | Default                    |
| -------------------- | -------- | -------------------------- |
| `OPENROUTER_API_KEY` | yes      | —                          |
| `OPENROUTER_MODEL`   | no       | `minimax/minimax-m2.7`     |
| `SYSTEM_PROMPT`      | no       | generic assistant prompt   |

## Architecture

Four source files, each with a single responsibility:

- **`src/client.rs`** — `OpenRouterClient`: wraps `reqwest`, sends `POST /chat/completions` to OpenRouter, owns all serde types (`Message`, `ToolCall`, `FunctionCall`, `ChatResponse`). The `Message` type is shared across agent history and the wire format.

- **`src/tools.rs`** — Two public functions: `tool_definitions()` returns the OpenAI-format JSON array sent to the LLM on every request; `call_tool(name, args_json)` dispatches by name to one of four tool implementations (`read_file`, `write_file`, `list_directory`, `bash`). Tool errors are caught and returned as strings so the model sees them rather than crashing the loop.

- **`src/agent.rs`** — `Agent` struct holds the conversation history (`Vec<Message>`) and owns the client + model config. `run_loop(user_input)` is the core loop: push user message → call LLM → if tool calls present, execute all and push role="tool" results → repeat up to 10 iterations → return final text content. System prompt is prepended each call (not stored in history).

- **`src/main.rs`** — Reads env vars, constructs `OpenRouterClient` and `Agent`, then either runs a single turn (print mode, when CLI args present) or a stdin readline loop (REPL mode, history persists across turns).

## Adding a New Tool

1. Add the OpenAI function schema to the `json!([...])` array in `tool_definitions()` (`src/tools.rs`).
2. Add a match arm in `execute_tool()` in the same file.
3. No other files need to change — `agent.rs` passes `tool_definitions()` on every call automatically.

## Known Limitations (TODOs)

- `bash` tool has no timeout
- No path sandboxing on any filesystem tool
- Max agent loop iterations hardcoded to 10
- No context truncation if history exceeds model context window
