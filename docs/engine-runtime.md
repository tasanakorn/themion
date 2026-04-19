# Engine Runtime

This document explains how Themion's core harness/runtime works: how prompt inputs are assembled, how context is built, how tool calls are executed, and how session history is stored.

## Scope

Most of the logic described here lives in `crates/themion-core/`. The CLI crate (`crates/themion-cli/`) is responsible for starting sessions, wiring the TUI, loading config, and passing the active project/session context into the core runtime.

Relevant areas:

- `crates/themion-core/src/agent.rs`
- `crates/themion-core/src/client.rs`
- `crates/themion-core/src/client_codex.rs`
- `crates/themion-core/src/tools.rs`
- `crates/themion-core/src/db.rs`
- `crates/themion-cli/src/` for session startup and UI integration

## High-level flow

A single user turn follows this shape:

1. The CLI starts or resumes a harness session.
2. The user submits input.
3. The harness records a new turn and persists the user message.
4. The harness builds the model input from:
   - the base system prompt
   - injected contextual instructions such as `AGENTS.md`
   - an optional history recall hint
   - the recent conversation window
5. The active backend streams the assistant response.
6. If the model requests tools, the harness executes them and appends tool results to the conversation.
7. The harness calls the model again with the updated conversation.
8. This repeats until the model returns a normal assistant response with no more tool calls, or the loop limit is reached.
9. The turn is finalized in SQLite with message and token metadata.

## Prompt inputs

Themion keeps different instruction sources separate instead of flattening them into one blob.

### 1. Base system prompt

The base system prompt comes from configuration. It establishes the assistant's default behavior and is always part of the prompt sent to the model.

This is the top-level instruction layer.

### 2. Contextual instruction files

Repository or workspace instructions such as `AGENTS.md` are treated as separate injected prompt inputs, not as text concatenated into the base system prompt.

That separation matters because:

- it preserves the distinction between global assistant behavior and repository-local instructions
- it matches the repository's prompt assembly expectations
- it keeps compatibility with both chat-completions-style backends and the Codex Responses backend

In practice, the model sees both the base system prompt and the injected contextual instructions, but they remain separate prompt components.

### 3. Recall hint for trimmed history

When the in-memory conversation is longer than the configured context window, the harness adds a synthetic system message explaining that earlier turns are still available in persistent history.

Example shape:

> Note: N earlier turn(s) (seq 1–N) are stored in history. Use `history_recall` to load a range or `history_search` to find a keyword.

This gives the model a way to recover older context without sending the full conversation every time.

## Context building

The harness keeps the full conversation in memory, but only sends a bounded recent window to the model.

### Full in-memory history

`Agent` owns a complete `Vec<Message>` for the active session. Messages are not trimmed out of memory during the session.

This full history includes:

- user messages
- assistant messages
- tool results

### Windowed model context

For each model request, the harness constructs a smaller prompt window. Conceptually it looks like this:

```text
[system prompt]
[injected contextual instructions, e.g. AGENTS.md]
[recall hint, if older turns were omitted]
[recent turns only]
```

`Agent.window_turns` controls how many recent turns are included. Older turns remain in memory and in SQLite, but are not sent unless recovered through history tools.

This design gives a few benefits:

- lower token usage on long sessions
- stable prompt size
- recoverability of old context through explicit tool use

## Harness loop behavior

Each `run_loop(user_input)` call handles one user-submitted turn, including any tool round-trips triggered during that turn.

### Step-by-step

1. Record a turn boundary in memory.
2. Open a new turn row in SQLite.
3. Append the user message to the in-memory conversation.
4. Persist the user message to the database.
5. Build the current model context window.
6. Call the active `ChatBackend` with:
   - model name
   - prompt messages
   - tool definitions
   - streaming callback
7. Stream assistant text chunks to the UI while accumulating the full assistant response.
8. Persist the assistant response.
9. If there are no tool calls, the turn is complete.
10. If there are tool calls:
    - execute each requested tool
    - append tool results as `role="tool"` messages
    - persist those results
    - call the model again with the updated conversation
11. Repeat until no more tool calls are returned, up to the hardcoded loop limit.
12. Finalize the turn with token statistics.

Themion currently caps this inner tool loop at 10 iterations per turn.

## Tool calling

Themion uses OpenAI-style tool calling.

### Tool definitions

On each model request, the harness sends a JSON-schema-style description of the available tools. This happens through `tool_definitions()` in `tools.rs`.

### Tool execution

When the model returns tool calls, the harness dispatches them through `call_tool(name, args, &ToolCtx)`.

Available canonical tools include:

- `fs_read_file`
- `fs_write_file`
- `fs_list_directory`
- `shell_run_command`
- `history_recall`
- `history_search`
- `workflow_get_state`
- `workflow_set_active`
- `workflow_set_phase`
- `workflow_set_phase_result`
- `workflow_complete`

Deprecated aliases for the older short names are still accepted internally during the transition period, but only the domain-prefixed names are exposed in tool definitions.

### Tool context

Each tool call receives a `ToolCtx` containing:

- database handle
- session ID
- project directory
- workflow state
- current turn sequence

Filesystem tools mostly ignore this context. History tools use it to query session-aware SQLite data. Workflow tools use it to inspect or update runtime workflow state.

### Tool result handling

Tool output is inserted back into the conversation as a tool message. The model then sees the result and can:

- answer the user directly
- request another tool
- recover older context from history
- revise its prior plan based on tool output

If a tool fails, the runtime returns an error string as the tool result rather than crashing the loop. That lets the model observe the failure and decide what to do next.

## Streaming and backend abstraction

Themion keeps provider-specific transport logic behind a shared backend abstraction.

### `ChatBackend`

The harness talks to a `ChatBackend` trait rather than directly to a provider-specific client. This keeps the core loop reusable while allowing different wire formats.

### Chat Completions backends

Providers such as OpenRouter and local OpenAI-compatible servers use a chat-completions-style streaming API. The client parses `data:` SSE frames and forwards token deltas to the harness.

### Codex Responses backend

Codex uses the OpenAI Responses API with named SSE events. It has separate request/response translation and streaming parsing, but the harness loop above it stays the same.

This separation is important: provider-specific behavior lives in backend modules instead of being spread across ad hoc conditionals in the core loop.

## Session and history storage

Themion stores persistent history in SQLite so older context can be recalled across long sessions.

### Database location

History is stored at:

- `$XDG_DATA_HOME/themion/history.db`
- typically `~/.local/share/themion/history.db`

### Main tables

The database includes:

- `agent_sessions`
  - one row per started session
  - includes session identity and project directory
- `agent_turns`
  - one row per user turn
  - includes turn sequence and token metadata
- `agent_messages`
  - one row per message within a turn
  - stores role, content, and tool-call metadata
- `agent_messages_fts`
  - full-text search index for message content

### Session model

Each process start creates a new `session_id`. Multiple sessions can coexist in the same database file.

This allows:

- persistent history across runs
- session-scoped recall
- full-text search over past messages
- future support for multiple concurrent agents

## Why history tools exist

The model does not automatically receive the full archived conversation. Instead, older messages are discoverable through tools.

This has two effects:

- normal requests stay smaller and cheaper
- the model can explicitly fetch older context only when needed

The two history tools serve different jobs:

- `history_recall` loads known earlier turns or message ranges
- `history_search` finds relevant older content by keyword or text match

Together with the recall hint, these tools form the bridge between a bounded context window and long-lived session memory.

## Relationship to the CLI/TUI

The CLI crate is not the harness loop itself, but it drives the loop.

It is responsible for:

- loading config
- resolving project directory
- opening the database handle
- creating session IDs
- running the TUI or print mode
- rendering streaming `AgentEvent` output

The core crate is responsible for:

- message history management
- prompt/context assembly
- provider calls
- tool execution
- database persistence logic

## Practical mental model

A useful way to think about Themion's engine/runtime is:

- **system prompt** defines default assistant behavior
- **`AGENTS.md` and related instructions** define repository-local behavior
- **recent turns** provide immediate conversational context
- **history tools** recover older context on demand
- **workflow tools** let the model inspect and control workflow runtime state
- **tool calls** let the model inspect and change the workspace
- **SQLite** preserves the long-term memory that no longer fits in the active prompt window

That combination gives Themion a bounded live context with explicit access to deeper session memory.
