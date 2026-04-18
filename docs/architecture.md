# Architecture

Themion is a Rust AI agent with a Ratatui TUI, streaming token output, persistent SQLite history, and a tool-calling loop compatible with multiple OpenAI-style backends.

## Design Philosophy

- **No framework dependencies** — the agent loop, HTTP client, tool dispatch, and TUI are all hand-rolled.
- **Stateful conversation, context-windowed** — `Agent` owns the full in-memory history but sends only the last N turns to the API. Older turns persist in SQLite and are reachable via tools.
- **OpenAI-style tool calling** — tools are described as JSON function schemas; compatible providers can invoke them and return structured tool calls.
- **Event-driven TUI** — `Agent` emits `AgentEvent` variants over an `mpsc` channel; the TUI renders each event as it arrives, giving streaming token display without blocking the input loop.
- **Provider abstraction** — the core agent speaks through a `ChatBackend` trait so different transports and wire formats can be swapped at runtime.

## Component Map

```text
main.rs
  └─ loads Config, resolves project_dir, opens DbHandle
       ├─ print mode  ──► Agent::new_with_db → run_loop(prompt) → print → exit
       └─ TUI mode    ──► tui::run(cfg, dir_override)

tui::run  (tui.rs)
  ├─ opens DbHandle at $XDG_DATA_HOME/themion/history.db
  ├─ generates session_id (UUID v4), inserts agent_sessions row
  ├─ builds App { agents: Vec<AgentHandle>, db, project_dir, session_tokens, … }
  └─ event loop: keyboard / mouse / AgentEvent / AgentReady / Tick (150 ms)

Agent  (agent.rs)
  ├─ owns: Vec<Message> (full in-memory history)
  ├─ owns: Arc<DbHandle>, session_id, project_dir, turn_boundaries
  ├─ calls: client.chat_completion_stream() via ChatBackend
  ├─ calls: tools::call_tool(name, args, &ToolCtx)
  └─ emits: AgentEvent over mpsc channel

ChatBackend  (trait in client.rs)
  └─ async fn chat_completion_stream(model, messages, tools, on_chunk)

ChatClient  (client.rs)
  ├─ POST /chat/completions with stream=true
  ├─ parses Chat Completions SSE line-by-line (byte-safe UTF-8 splitting)
  └─ assembles ResponseMessage + Usage from stream chunks

CodexClient  (client_codex.rs)
  ├─ POST /responses with stream=true
  ├─ parses named-event Responses API SSE frames
  ├─ refreshes OAuth tokens when needed
  └─ assembles ResponseMessage + Usage from Responses events

tools.rs
  ├─ tool_definitions() → JSON schema array (sent every request)
  ├─ call_tool(name, args, ctx: &ToolCtx) → String
  │    ├─ read_file, write_file, list_directory, bash  (ignore ctx)
  │    ├─ recall_history  ──► ctx.db.recall(RecallArgs)
  │    └─ search_history  ──► ctx.db.search(SearchArgs)
  └─ ToolCtx { db: Arc<DbHandle>, session_id, project_dir }

DbHandle  (db.rs)
  ├─ Arc<Mutex<rusqlite::Connection>>  (WAL mode, busy_timeout 5s)
  ├─ schema: agent_sessions, agent_turns, agent_messages + FTS5 vtable
  └─ insert_session / begin_turn / append_message / finalize_turn / recall / search
```

## Agent Loop (agent.rs)

Each call to `run_loop(user_input)`:

1. Record turn boundary (`turn_boundaries.push(messages.len())`); open a DB turn row via `begin_turn`.
2. Push `role="user"` message to history; persist to `agent_messages`.
3. Build windowed context (see §Context Windowing) and call `chat_completion_stream` on the active backend.
4. Stream tokens to TUI via `AgentEvent::AssistantChunk`; accumulate full response.
5. Push `role="assistant"` response to history; persist to `agent_messages`.
6. If response has no `tool_calls` → break.
7. For each tool call: emit `ToolStart` (detail truncated to 60 chars), execute via `call_tool`, push `role="tool"` result; persist each.
8. Repeat from step 3, up to 10 iterations.
9. Finalize the DB turn row with token stats; emit `TurnDone`.

## Context Windowing

`Agent.window_turns` (default 5) controls how much history is sent to the API. On each LLM round:

```text
[system_prompt]
[recall hint — only when turn_boundaries.len() > window_turns]
[messages from turn (current − window_turns) … now]
```

The recall hint is a synthetic `role="system"` message:

> "Note: N earlier turn(s) (seq 1–N) are stored in history. Use recall_history to load a range or search_history to find a keyword."

The full `messages` Vec is never trimmed — the in-memory copy is always complete. Windowing only affects what is sent over the wire.

## Streaming

### Chat Completions backends (`client.rs`)

`chat_completion_stream` sends `"stream": true` and reads the response body chunk-by-chunk via `Response::chunk()`. SSE lines are split on the `0x0A` byte (safe for UTF-8 multi-byte sequences) and decoded per line. Each `data:` line is parsed as a `StreamChunkData`; `delta.content` fragments are forwarded to the `on_chunk` callback immediately. Tool call argument fragments are accumulated by `index` and assembled after `[DONE]`.

`"stream_options": {"include_usage": true}` is sent so the last chunk carries token counts.

### Codex Responses backend (`client_codex.rs`)

Codex uses the OpenAI Responses API rather than Chat Completions. Its stream consists of named SSE events such as `response.output_text.delta` and `response.completed`. The parser accumulates `event:` and `data:` lines until a blank-line frame boundary, then updates the in-flight response state. Text deltas stream to the UI immediately; usage is taken from the completion event.

## Tools (tools.rs)

All tools receive a `&ToolCtx` carrying the DB handle and session identity. Filesystem tools ignore it; history tools use it. Tool call display labels are truncated to 60 chars to keep TUI lines readable.

| Tool             | Underlying call                       | Returns                           |
| ---------------- | ------------------------------------- | --------------------------------- |
| `read_file`      | `fs::read_to_string`                  | file contents                     |
| `write_file`     | `fs::write`                           | confirmation line                 |
| `list_directory` | `fs::read_dir`                        | newline-joined names              |
| `bash`           | `tokio::process::Command` via `sh -c` | stdout + stderr                   |
| `recall_history` | `DbHandle::recall`                    | JSON array of past messages       |
| `search_history` | `DbHandle::search` (FTS5)             | JSON array of snippets + turn_seq |

Tool errors are caught in `call_tool` and returned as `"Error: <message>"` strings — the model sees the error as a tool result and can react.

## Persistent History (db.rs)

Database path: `$XDG_DATA_HOME/themion/history.db` (default `~/.local/share/themion/history.db`). Created on first run; WAL mode enabled on every open for safe multi-process access.

Schema:

| Table                | Key columns                                                    |
| -------------------- | -------------------------------------------------------------- |
| `agent_sessions`     | `session_id` (UUID), `project_dir`, `is_interactive`           |
| `agent_turns`        | `turn_id`, `session_id`, `turn_seq`, token stats               |
| `agent_messages`     | `message_id`, `turn_id`, `role`, `content`, `tool_calls_json`  |
| `agent_messages_fts` | FTS5 virtual table over `agent_messages.content`               |

Every process start generates a new `session_id`. Multiple concurrent processes share the same file; each writes to its own session rows.

## TUI (tui.rs)

The TUI uses Ratatui + Crossterm and runs in the alternate screen buffer.

Layout (top to bottom):

```text
┌─ conversation pane (Min 1) ───────────────────────────────────────┐
│  startup banner (block ASCII art + version / profile / model)     │
│  word-wrapped entries; bottom-pinned via ratatui line_count()     │
│  pending line: braille spinner ⠋⠙⠹… animated at 150 ms tick      │
├─ input box (3 rows) ──────────────────────────────────────────────┤
│  ▸ ────────────────────────────────────────────────────────────── │
│    <typed text>                                                    │
├─ status bar (1 row) ──────────────────────────────────────────────┤
│  <project>  |  <profile>  |  <model>  |  in:N out:N cached:N  |  ctx:N
└───────────────────────────────────────────────────────────────────┘
```

Token counts in the status bar are formatted with thousands separators. `ctx` shows `tokens_in` from the last completed turn (actual context sent to the API).

Key bindings:

| Key                   | Action                    |
| --------------------- | ------------------------- |
| `Enter`               | Submit message            |
| `↑` / `↓`             | Navigate input history    |
| `Alt+↑` / `Alt+↓`     | Scroll conversation pane  |
| `PageUp` / `PageDown` | Scroll conversation pane  |
| Mouse scroll          | Scroll conversation pane  |
| `Ctrl+C`              | Quit                      |

### Entry types

| Variant        | Colour  | Description                              |
| -------------- | ------- | ---------------------------------------- |
| `User`         | bold    | User message with `▸` prefix            |
| `Assistant`    | default | Model response, word-wrapped             |
| `Banner`       | cyan    | Startup ASCII art and info lines         |
| `ToolCall`     | yellow  | `↳ <tool>: <args>` (args ≤ 60 chars)    |
| `ToolDone`     | green   | Appends ` ✓` to the matching ToolCall   |
| `Stats`        | dim     | Turn summary (tokens, time)              |
| `Blank`        | —       | Vertical spacing                         |

### Commands

| Command                          | Action |
| -------------------------------- | ------ |
| `/config`                        | Show active settings |
| `/config profile list`           | List profiles |
| `/config profile show`           | Show active profile |
| `/config profile create <name>`  | Create a profile from current settings |
| `/config profile use <name>`     | Switch profile |
| `/config profile set key=value`  | Update provider/model/base_url/api_key |
| `/login codex`                   | Start Codex auth flow and switch to codex profile |

## Multi-Agent Shape

`App` holds `agents: Vec<AgentHandle>` where each handle owns an `Option<Agent>` (None while a task is running), a `session_id`, and an `is_interactive` flag. Exactly one handle is interactive today; the Vec shape is forward-compatible for background agents without another struct refactor.

On each submit, the interactive agent is moved out of its handle into a spawned task. When `run_loop` returns, the agent is sent back via `AppEvent::AgentReady(Box<Agent>, Uuid)` and restored to the handle.

## Providers

Themion abstracts the LLM backend behind a `ChatBackend` trait (`crates/themion-core/src/client.rs`). Each provider implements `async fn chat_completion_stream(...)`. `Agent.client` is a `Box<dyn ChatBackend + Send + Sync>`, allowing swappable backends at runtime.

| Provider       | Config value       | Auth                 | Endpoint family                  | Wire format                  |
| -------------- | ------------------ | -------------------- | -------------------------------- | ---------------------------- |
| OpenRouter     | `openrouter`       | API key              | `/chat/completions`              | Chat Completions SSE         |
| llama.cpp      | `llamacpp`         | none                 | `/chat/completions`              | Chat Completions SSE         |
| OpenAI Codex   | `openai-codex`     | OAuth / device login | `/responses` via Codex backend   | Responses API named-event SSE |

**Codex authentication** — tokens are persisted to `$XDG_CONFIG_HOME/themion/auth.json` (typically `~/.config/themion/auth.json`) and written with mode `0600` on Unix. Login is initiated from the TUI with `/login codex`. Refresh happens automatically before requests when needed.

**SSE format divergence** — OpenRouter and llama.cpp both use Chat Completions with unnamed `data:` frames. Codex uses named-event frames (`event: response.output_text.delta`, etc.) from the Responses API, so it has a separate parser and request translator.

## Configuration

Configuration is resolved in priority order: **env var > config file > built-in default**.

No environment variables are required. All settings can be managed with `/config` inside the TUI and are saved to `$XDG_CONFIG_HOME/themion/config.toml` (typically `~/.config/themion/config.toml`). A commented template is written on first run. Environment variables remain available as convenience overrides.

Multiple named profiles are stored under `[profile.<name>]` in the config file and switchable at runtime via `/config profile use <name>`.

### Provider: openrouter (default)

Requires an API key from [openrouter.ai](https://openrouter.ai). Supports any model on the OpenRouter catalogue.

| Field      | Env var               | Default                        |
| ---------- | --------------------- | ------------------------------ |
| `api_key`  | `OPENROUTER_API_KEY`  | —                              |
| `model`    | `OPENROUTER_MODEL`    | `minimax/minimax-m2.7`         |
| `base_url` | `OPENROUTER_BASE_URL` | `https://openrouter.ai/api/v1` |

### Provider: llamacpp (local)

No API key needed. Compatible with any OpenAI-format local server such as llama.cpp, Ollama, or LM Studio.

| Field      | Env var             | Default                    |
| ---------- | ------------------- | -------------------------- |
| `base_url` | `LLAMACPP_BASE_URL` | `http://localhost:8080/v1` |
| `model`    | `LLAMACPP_MODEL`    | `local`                    |

### Provider: openai-codex

Uses persisted OAuth credentials rather than an API key.

| Field      | Env var         | Default                               |
| ---------- | --------------- | ------------------------------------- |
| `base_url` | —               | `https://chatgpt.com/backend-api/codex` |
| `model`    | `CODEX_MODEL`   | `gpt-5.4`                             |
| auth file  | —               | `$XDG_CONFIG_HOME/themion/auth.json`  |

### Global

| Field          | Env var           | Default                             |
| -------------- | ----------------- | ----------------------------------- |
| `system_prompt`| `SYSTEM_PROMPT`   | `"You are a helpful AI assistant…"` |
| active profile | `THEMION_PROFILE` | `default`                           |
| provider       | `THEMION_PROVIDER`| from selected profile               |

## Build Profiles

| Profile   | Debug symbols | LTO  | opt-level  | Strip |
| --------- | ------------- | ---- | ---------- | ----- |
| `dev`     | off           | off  | default    | no    |
| `release` | off           | full | `z` (size) | yes   |

## Known Limitations

- **No timeout on `bash`** — a hung subprocess blocks the agent indefinitely.
- **No path sandboxing** — tools accept any absolute or relative path.
- **Max 10 tool-call iterations per turn** — hardcoded in `agent.rs`.
- **No user-configurable `window_turns`** — default of 5 is hardcoded; requires a code change.
