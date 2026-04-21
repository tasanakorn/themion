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
  - the CLI runtime owns process-local agent descriptors such as `agent_id`, `label`, and `roles`, while each core `Agent` continues to own its own session/workflow state
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
- **One process can describe multiple agents** — the CLI runtime stores agent descriptors in a vector, and Stylos status publishes process-level metadata plus an `agents` list rather than flattening one effective agent into top-level fields.

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
  │    ├─ time_sleep  ──► bounded non-shell wait for short sleeps
  │    ├─ history_recall  ──► ctx.db.recall(RecallArgs)
  │    ├─ history_search  ──► ctx.db.search(SearchArgs)
  │    └─ workflow_*  ──► workflow state inspection / transitions
  └─ ToolCtx { db: Arc<DbHandle>, session_id, project_dir }
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

`time_sleep` is a built-in bounded wait helper for short pauses. It accepts `ms`, rejects values above 30,000, and lets the agent express lightweight waiting without shelling out to `sleep`.

## Persistent History (db.rs)

Database path: `$XDG_DATA_HOME/themion/system.db` (default `~/.local/share/themion/system.db`). Created on first run; WAL mode enabled on every open for safe multi-process access.

## Stylos status

When the `stylos` feature is enabled, Themion still opens one Stylos session per process. Status now publishes shared process metadata plus an `agents` array. The top-level `startup_project_dir` records where the Themion process started and is informational provenance; consumers must not assume it matches every agent `project_dir`.

Each agent snapshot includes its `agent_id`, `label`, `roles`, `session_id`, workflow/activity state, provider/model/profile, and per-agent git metadata.

## Stylos query surface

Feature-enabled `themion-cli` also exposes additive Stylos queryables under the existing Themion namespace.

Discovery queryables are mesh-wide and are not addressed to a single instance:

- `stylos/<realm>/themion/query/agents/alive`
- `stylos/<realm>/themion/query/agents/free`
- `stylos/<realm>/themion/query/agents/git`

Per-instance queryables target one Themion process:

- `stylos/<realm>/themion/instances/<instance>/query/status`
- `stylos/<realm>/themion/instances/<instance>/query/talk`
- `stylos/<realm>/themion/instances/<instance>/query/tasks/request`
- `stylos/<realm>/themion/instances/<instance>/query/tasks/status`
- `stylos/<realm>/themion/instances/<instance>/query/tasks/result`

The direct instance identifier is transport-safe `<hostname>:<pid>`, not a slash-delimited path.

All of these queryables live in `crates/themion-cli/src/stylos.rs` and are registered only when the `stylos` cargo feature is enabled.

Matching injected Stylos tools in `themion-core` now include:

- `stylos_query_agents_alive`
- `stylos_query_agents_free`
- `stylos_query_agents_git`
- `stylos_query_nodes`
- `stylos_query_status`
- `stylos_request_talk`
- `stylos_request_task`
- `stylos_query_task_status`
- `stylos_query_task_result`

`stylos_query_nodes` is a Zenoh-session-level check, not a Themion mesh discovery queryable. It inspects the current local Zenoh session via `session.info()` and returns the local session ZID plus currently known peer and router ZIDs.

### Discovery behavior

- `alive` returns one reply per responding instance with that instance identity, session ID, and its current agent list.
- `free` uses the current exported activity state and returns only agents whose `activity_status` is `idle` or `nap`.
- `git` returns only agents whose `project_dir_is_git_repo` is true. When the request includes a `remote`, the handler matches against normalized repo keys when possible and falls back to exact raw-remote comparison for unsupported forms.
- injected discovery tools accept `exclude_self`; when omitted, it defaults to `true` and filters out replies from the current instance.
- `stylos_query_nodes` returns a local network snapshot with `self_zid`, `peer_zids`, and `router_zids` gathered from the active Zenoh session.

The reply payloads include per-agent fields already present in the status snapshot plus normalized `git_repo_keys` derived from exported `git_remotes`.

### Git normalization rules

Git discovery matching is designed around comparable repository identity rather than raw remote-string equality.

Current normalization behavior:

- supports common SSH and HTTP(S) forms for `github.com`, `gitlab.com`, and `bitbucket.org`
- lowercases the host for comparison
- trims surrounding `/`
- ignores a trailing `.git`
- emits canonical keys in the form `<host>/<owner>/<repo>`
- accepts query selectors either as raw remotes or as direct comparable identities such as `github.com/example/themion`
- requester-side normalization is preferred when the selector depends on caller-local context such as SSH aliases, mirror rewrites, or other shorthand the responder cannot safely reconstruct
- returns no normalized key for unsupported hosts, so matching falls back to exact raw remote comparison instead of guessing

Examples:

- `git@github.com:example/themion.git` → `github.com/example/themion`
- `https://github.com/example/themion` → `github.com/example/themion`
- `ssh://git@gitlab.com/group/proj.git` → `gitlab.com/group/proj`
- `git@bitbucket.org:team/repo.git` → `bitbucket.org/team/repo`

### Request and task query behavior

- `status` returns the current process snapshot and supports optional `agent_id` and `role` filtering. Filters can be used independently or together. Unknown filters return `not_found`.
- `talk` accepts mandatory target `instance`, optional `to_agent_id`, and optional `wait_for_idle_timeout_ms`; sender identity is resolved automatically by the local runtime.
- accepted `talk` requests are injected into the local agent turn as a peer-message wrapper with exact `from=<hostname>:<pid>` and `to=<hostname>:<pid>` identifiers, plus reply guidance using `***QRU***`.
- when an inbound Stylos talk is received, the receiver-side TUI emits one `Stylos hear from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id>` line using the inbound payload fields directly.
- when an outbound `stylos_request_talk` call is accepted, the sender-side TUI emits `Stylos talk to=<hostname>:<pid> from=<hostname>:<pid>` using the same exact identifier format.
- receiver-side inbound `hear` logging and sender-side outbound `talk` logging remain distinct; one inbound delivery must not also surface as a receiver-side `Stylos talk ...` line.
- `talk` keeps acknowledgement-oriented semantics: it reports delivery acceptance or rejection and does not wait for the remote agent’s final natural-language answer.
- when the target agent is busy and `wait_for_idle_timeout_ms` is positive, the CLI query layer polls the exported snapshot until the peer becomes `idle` or `nap` or the timeout expires; timeout produces `timed_out_waiting_for_idle`.
- `tasks/request` filters local candidates using the current snapshot, optional `preferred_agent_id`, optional `required_roles`, and optional `require_git_repo`, then chooses deterministically by sorted `agent_id`.
- `tasks/status` returns the current in-memory lifecycle state for a previously accepted task.
- `tasks/result` returns immediately for terminal tasks, returns current non-terminal state when `wait_timeout_ms` is omitted or zero, and otherwise waits up to the requested timeout clamped to 60,000 ms.
- injected per-instance Stylos tools issue direct Zenoh queries to the addressed instance key, enforce single-reply expectations, and distinguish transport no-reply from responder-side `not_found` payloads.

Task lifecycle tracking is process-local and in-memory. Accepted tasks start as `queued`, move to `running` when the selected local agent turn begins, and then to `completed` or `failed`. Lifecycle records currently expire after 30 minutes.

## TUI (tui.rs)

The TUI still boots with one main interactive agent in the first shipped step, but its runtime agent descriptor now uses explicit roles rather than relying only on an `is_interactive` boolean. The initial main agent carries `roles = ["main", "interactive"]`.

### Long-session chat navigation

Long-session transcript review is now a CLI-local navigation feature in `crates/themion-cli/src/tui.rs`.

Current behavior:

- the main conversation pane starts in follow-tail mode and stays pinned to the latest content while the user has not browsed upward
- scrolling upward or paging upward moves the UI into browsed-history mode instead of relying only on the implicit `scroll_offset == 0` convention
- while in browsed-history mode, new streamed output continues to append but does not forcibly snap the viewport back to the bottom
- `Alt-g` returns to the latest content and restores follow-tail mode
- `PageUp` and `PageDown` perform page-sized navigation rather than the old tiny fixed-step behavior
- `Alt-t` opens a read-only transcript review overlay for the current in-memory session transcript; `Esc`, `Enter`, or `Alt-t` closes it
- transcript review uses the current `Entry` list as its source transcript and keeps review navigation local to the CLI rather than changing persistence or harness semantics
- the status bar exposes the current navigation state as `tail`, `browse`, or `review`

This keeps long-history usability in the TUI itself rather than depending on terminal scrollback, which remains unreliable in alternate-screen environments.
