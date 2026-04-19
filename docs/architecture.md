# Architecture

Themion is a Rust AI agent with a Ratatui TUI, streaming token output, persistent SQLite history, and a tool-calling loop compatible with multiple OpenAI-style backends.

For a focused walkthrough of the harness/runtime itself, including system prompt handling, `AGENTS.md` injection, context building, tool execution, and session storage, see [engine-runtime.md](engine-runtime.md).

## Workspace Layout

- `crates/themion-core/`
  - shared harness/runtime logic, provider backends, tool handling, and SQLite-backed history
  - look here first for prompt assembly, streaming, backend-specific request/response translation, and tool execution
- `crates/themion-cli/`
  - terminal UI, config loading, login flows, startup wiring, and other user-facing local behavior
  - look here first for file IO, TUI event handling, app/session orchestration, and local profile/auth flows
  - optional Stylos support lives here behind the `stylos` cargo feature; when that feature is compiled, Stylos starts by default unless config overrides disable it
- `docs/`
  - project docs and behavior notes; keep this aligned with real behavior when flows or semantics change

This separation is intentional: reusable harness/runtime and provider behavior belongs in `themion-core`, while terminal/UI/config/filesystem-driven user flows belong in `themion-cli`.

## Design Philosophy

- **No framework dependencies** — the harness loop, HTTP client, tool dispatch, and TUI are all hand-rolled.
- **Stateful conversation, context-windowed** — `Agent` owns the full in-memory history but sends only the last N turns to the API. Older turns persist in SQLite and are reachable via tools.
- **OpenAI-style tool calling** — tools are described as JSON function schemas; compatible providers can invoke them and return structured tool calls.
- **Event-driven TUI** — `Agent` emits `AgentEvent` variants over an `mpsc` channel; the TUI renders each event as it arrives, giving streaming token display without blocking the input loop.
- **Provider abstraction** — the core harness speaks through a `ChatBackend` trait so different transports and wire formats can be swapped at runtime.
- **Separated prompt inputs** — the base system prompt, predefined coding guardrails, a predefined Codex CLI web-search instruction, and contextual instruction files such as `AGENTS.md` are treated as distinct prompt inputs rather than merged into a single message.
- **Built-in coding guardrails stay minimal** — the predefined guardrail layer covers default coding behavior such as assumption transparency, simple solutions, targeted edits, narrow validation, and brief specific commit messages naming the actual change when the user explicitly asks for a commit.

## Component Map

```text
main.rs
  └─ loads Config, resolves project_dir, opens DbHandle
       ├─ print mode  ──► Agent::new_with_db → run_loop(prompt) → print → exit
       └─ TUI mode    ──► tui::run(cfg, dir_override)

tui::run  (tui.rs)
  ├─ opens DbHandle at $XDG_DATA_HOME/themion/system.db
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
  │    ├─ fs_read_file, fs_write_file, fs_list_directory, shell_run_command  (ignore ctx)
  │    ├─ history_recall  ──► ctx.db.recall(RecallArgs)
  │    ├─ history_search  ──► ctx.db.search(SearchArgs)
  │    └─ workflow_*  ──► workflow state inspection / transitions
  └─ ToolCtx { db: Arc<DbHandle>, session_id, project_dir }

DbHandle  (db.rs)
  ├─ Arc<Mutex<rusqlite::Connection>>  (WAL mode, busy_timeout 5s)
  ├─ schema: agent_sessions, agent_turns, agent_messages + FTS5 vtable
  └─ insert_session / begin_turn / append_message / finalize_turn / recall / search
```

## Harness Loop (agent.rs)

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
[predefined coding guardrails]
[predefined Codex CLI web-search instruction]
[injected contextual instructions such as AGENTS.md, when available]
[workflow context + phase instructions]
[recall hint — only when turn_boundaries.len() > window_turns]
[messages from turn (current − window_turns) … now]
```

The recall hint is a synthetic `role="system"` message:

> "Note: N earlier turn(s) (seq 1–N) are stored in history. Use history_recall to load a range or history_search to find a keyword."

The full `messages` Vec is never trimmed — the in-memory copy is always complete. Windowing only affects what is sent over the wire.

## Streaming

### Chat Completions backends (`client.rs`)

`chat_completion_stream` sends `"stream": true` and reads the response body chunk-by-chunk via `Response::chunk()`. SSE lines are split on the `0x0A` byte (safe for UTF-8 multi-byte sequences) and decoded per line. Each `data:` line is parsed as a `StreamChunkData`; `delta.content` fragments are forwarded to the `on_chunk` callback immediately. Tool call argument fragments are accumulated by `index` and assembled after `[DONE]`.

`"stream_options": {"include_usage": true}` is sent so the last chunk carries token counts.

### Codex Responses backend (`client_codex.rs`)

Codex uses the OpenAI Responses API rather than Chat Completions. Its stream consists of named SSE events such as `response.output_text.delta` and `response.completed`. The parser accumulates `event:` and `data:` lines until a blank-line frame boundary, then updates the in-flight response state. Text deltas stream to the UI immediately; usage is taken from the completion event.

## Tools (tools.rs)

All tools receive a `&ToolCtx` carrying the DB handle and session identity. Filesystem tools ignore it; history tools and workflow tools use it. Tool call display labels are truncated to 60 chars to keep TUI lines readable.

| Tool                        | Underlying call                       | Returns                           |
| --------------------------- | ------------------------------------- | --------------------------------- |
| `fs_read_file`              | `fs::read_to_string`                  | file contents                     |
| `fs_write_file`             | `fs::write`                           | confirmation line                 |
| `fs_list_directory`         | `fs::read_dir`                        | newline-joined names              |
| `shell_run_command`         | `tokio::process::Command` via `sh -c` | stdout + stderr                   |
| `history_recall`            | `DbHandle::recall`                    | JSON array of past messages       |
| `history_search`            | `DbHandle::search` (FTS5)             | JSON array of snippets + turn_seq |
| `workflow_get_state`        | workflow snapshot assembly            | JSON workflow state               |
| `workflow_set_active`       | workflow activation logic             | JSON updated workflow state       |
| `workflow_set_phase`        | workflow transition validation        | JSON updated workflow state       |
| `workflow_set_phase_result` | workflow phase-result update          | JSON updated workflow state       |
| `workflow_complete`         | workflow completion/failure logic     | JSON updated workflow state       |

Tool errors are caught in `call_tool` and returned as `"Error: <message>"` strings — the model sees the error as a tool result and can react.

## Persistent History (db.rs)

Database path: `$XDG_DATA_HOME/themion/system.db` (default `~/.local/share/themion/system.db`). Created on first run; WAL mode enabled on every open for safe multi-process access.

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

Token counts in the status bar use compact human-friendly units such as `2k` and `1m`. `ctx` shows `tokens_in` from the last completed turn (actual context sent to the API).

Key bindings:

| Key                   | Action                    |
| --------------------- | ------------------------- |
| `Enter`               | Submit message            |
| `↑` / `↓`             | Navigate input history    |
| `Alt+↑` / `Alt+↓`     | Scroll conversation pane  |
| `PageUp` / `PageDown` | Scroll conversation pane  |
| Mouse scroll          | Scroll conversation pane  |
| `Esc`                 | Interrupt current turn    |
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
| `!<command>`                     | Run a local shell command in the project directory and show output in the pane |

The `!<command>` shortcut is handled entirely in `themion-cli`. It does not go through the model tool loop, is not sent as a prompt, and is intended as a direct user convenience for local terminal work.
