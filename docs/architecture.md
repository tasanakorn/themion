# Architecture

Themion is a minimal AI agent core: a Rust binary that connects an LLM (via OpenRouter) to a set of local tools in a loop, consuming user input from stdin and producing text output.

## Design Philosophy

- **No framework dependencies** — the agent loop, HTTP client, and tools are all hand-rolled. The entire agent core fits in four source files.
- **Stateless tools, stateful conversation** — tools are pure functions; the `Agent` struct owns the only mutable state (conversation history).
- **OpenAI tool-calling protocol** — tools are described as JSON function schemas and called by the LLM using the standard `tool_calls` response field, making the agent compatible with any OpenAI-compatible model.

## Component Map

```
main.rs
  └─ reads env vars, selects mode
       ├─ print mode  ──► Agent::run_loop(prompt) → print → exit
       └─ REPL mode   ──► loop { Agent::run_loop(line) → print }

Agent (agent.rs)
  ├─ owns: Vec<Message> (conversation history)
  ├─ calls: OpenRouterClient::chat_completion()
  └─ calls: tools::call_tool() for each tool_call in response

OpenRouterClient (client.rs)
  └─ POST https://openrouter.ai/api/v1/chat/completions

tools.rs
  ├─ tool_definitions() → JSON schema array (sent every request)
  └─ call_tool(name, args_json) → String
       ├─ read_file
       ├─ write_file
       ├─ list_directory
       └─ bash
```

## Agent Loop (agent.rs)

Each call to `run_loop(user_input)`:

1. Push `role: "user"` message to history.
2. Prepend system prompt (not stored in history) and call `chat_completion`.
3. Push `role: "assistant"` response to history.
4. If response has no `tool_calls` → break, return `content`.
5. For each tool call: execute, push `role: "tool"` result with matching `tool_call_id`.
6. Repeat from step 2, up to 10 iterations.

The loop stops when the LLM returns a plain text response with no tool calls, or after 10 iterations (whichever comes first).

REPL mode keeps the `Agent` alive across turns, so conversation history accumulates. Print mode creates a fresh `Agent` per invocation.

## Message Flow (client.rs)

Every `chat_completion` call sends the full conversation as:

```
[system_prompt, ...history]
```

The system prompt is injected fresh each call. Optional fields (`tool_calls`, `tool_call_id`, `content`) are skipped in serialization when `None` via `#[serde(skip_serializing_if = "Option::is_none")]`.

Response is deserialized into `ResponseMessage { role, content, tool_calls }`. API errors (non-2xx) surface the raw response body in the error.

## Tools (tools.rs)

All four tools are synchronous filesystem/shell operations wrapped in async for compatibility with the tokio runtime:

| Tool             | Underlying call                     | Returns           |
| ---------------- | ----------------------------------- | ----------------- |
| `read_file`      | `fs::read_to_string`                | file contents     |
| `write_file`     | `fs::write`                         | confirmation line |
| `list_directory` | `fs::read_dir`                      | newline-joined names |
| `bash`           | `tokio::process::Command` via `sh -c` | stdout + stderr |

Tool errors are caught in `call_tool` and returned as `"Error: <message>"` strings — the model sees the error as a tool result and can react rather than crashing the agent loop.

## Data Flow Diagram

```
User input
    │
    ▼
Agent::run_loop
    │
    ├──► OpenRouterClient::chat_completion ──► OpenRouter API
    │         │
    │    ResponseMessage
    │         │
    │    tool_calls?
    │    ├── no  ──► return content string
    │    └── yes ──►  tools::call_tool(name, args)
    │                      │
    │               tool result string
    │                      │
    └──────────────── push as role="tool" ──► repeat
```

## Configuration

Configuration is resolved in priority order: **env var > config file > built-in default**.

### Config file

Path: `$XDG_CONFIG_HOME/themion/config.toml` (fallback: `~/.config/themion/config.toml`)

On first run, if the file does not exist, a commented template is written automatically so the user can fill it in.

Example config file:

```toml
# api_key = "sk-or-v1-..."
# model = "minimax/minimax-m2.7"
# system_prompt = "You are a helpful AI assistant with access to tools."
```

### Fields

| Field           | Env var override     | Built-in default                                       |
| --------------- | -------------------- | ------------------------------------------------------ |
| `api_key`       | `OPENROUTER_API_KEY` | — (required; error if absent from both sources)        |
| `model`         | `OPENROUTER_MODEL`   | `minimax/minimax-m2.7`                                 |
| `system_prompt` | `SYSTEM_PROMPT`      | `"You are a helpful AI assistant with access to tools."` |

## Build Profiles

| Profile   | Debug symbols | LTO   | opt-level | Strip  |
| --------- | ------------- | ----- | --------- | ------ |
| `dev`     | off           | off   | default   | no     |
| `release` | off           | thin  | `z` (size)| yes    |

Dev profile disables debug symbols to reduce artifact size during iteration. Release uses `opt-level = "z"` for minimum binary size rather than maximum speed.

## Known Limitations

- **No timeout on `bash`** — a hung subprocess blocks the agent indefinitely.
- **No path sandboxing** — tools accept any absolute or relative path.
- **No context truncation** — if conversation history grows beyond the model's context window, the API call will fail.
- **Max 10 tool-call iterations** — hardcoded in `agent.rs`.

## Persistent History

Chat history is persisted to `$XDG_DATA_HOME/themion/history.db` (default `~/.local/share/themion/history.db`). On each process start, `App::new` in `themion-cli` canonicalizes the working directory as `project_dir`, opens (or creates) the database, and inserts a row into `agent_sessions`.

The database has three tables — `agent_sessions`, `agent_turns`, `agent_messages` — plus an `agent_messages_fts` FTS5 virtual table for full-text search. WAL mode is enabled on open for safe concurrent access across processes.

`Agent` in `themion-core` holds a `window_turns` limit (default 5). On each `run_loop` call, only messages from the last `window_turns` complete turns are included in the API request. When older turns are omitted, a synthetic `role="system"` hint is prepended telling the model to use `recall_history` or `search_history`.

Two tools are registered globally: `recall_history` (retrieves messages by session/project/direction) and `search_history` (FTS5 keyword search returning snippets). Both receive the DB handle via `ToolCtx` threaded through `call_tool`.
