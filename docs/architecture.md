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
- **Project Memory knowledge base** — distilled reusable project knowledge is stored as SQLite-backed graph nodes and edges, with hashtags as the lightweight organization layer; the explicit `[GLOBAL]` context is Global Knowledge for reusable cross-project facts.
- **Event-driven TUI** — `Agent` emits `AgentEvent` variants over an `mpsc` channel; the TUI renders each event as it arrives, giving streaming token display without blocking the input loop.
- **Provider abstraction** — the core harness speaks through a `ChatBackend` trait so different transports and wire formats can be swapped at runtime.
- **Separated prompt inputs** — the base system prompt, predefined coding guardrails, a predefined Codex CLI web-search instruction, and contextual instruction files such as `AGENTS.md` are treated as distinct prompt inputs rather than merged into a single message.
- **Built-in coding guardrails stay minimal** — the predefined guardrail layer covers default coding behavior such as assumption transparency, simple solutions, targeted edits, narrow validation, and brief specific commit messages naming the actual change when the user explicitly asks for a commit.
- **One process can describe multiple agents** — the CLI runtime stores agent descriptors in a vector, and Stylos status publishes process-level metadata plus an `agents` list rather than flattening one effective agent into top-level fields.

## Component Map

```text
main.rs
  └─ loads Config, parses mode/args, and builds shared CLI app runtime
       ├─ non-interactive prompt args ──► headless_runner::run_non_interactive(app_runtime, prompt)
       ├─ --headless               ──► headless_runner::run(app_runtime)
       └─ TUI mode                 ──► tui_runner::run(app_runtime)

AppState  (app_state.rs)
  ├─ resolves project_dir and opens DbHandle
  ├─ inserts agent_sessions row and builds Session
  ├─ owns shared CLI-local runtime/bootstrap state
  └─ builds core Agent instances for TUI, headless, and non-interactive runners

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
  │    ├─ system_inspect_local ──► local runtime/tool/provider readiness snapshot
  │    ├─ board_*  ──► local durable notes board operations
  │    ├─ memory_* ──► SQLite Project Memory KB nodes, hashtags, and edges
  │    └─ workflow_*  ──► workflow state inspection / transitions
  └─ ToolCtx { db: Arc<DbHandle>, session_id, project_dir, workflow_state, system_inspection }

History tools are always scoped to the caller's current project directory. Callers cannot pass a `project_dir` override; omitted `session_id` means the active session, `session_id="*"` means all sessions in the current project, and explicit UUIDs only match sessions within that same current project.
```

## Process, runtime, task, and thread hierarchy

Themion normally runs as a single OS process.

`crates/themion-cli/src/main.rs` constructs explicit Tokio runtime domains through a CLI-local runtime-topology helper rather than relying on `#[tokio::main]`.

Use this hierarchy when reasoning about the implementation:

```text
themion process
├─ bootstrap / entrypoint
│  └─ crates/themion-cli/src/main.rs
│     └─ builds shared AppState
│        ├─ non-interactive prompt mode → headless_runner::run_non_interactive(...)
│        ├─ --headless mode            → headless_runner::run(...)
│        └─ TUI mode                   → tui_runner::run(...)
├─ shared CLI app runtime
│  └─ crates/themion-cli/src/app_state.rs
│     ├─ resolves project_dir
│     ├─ opens DbHandle
│     ├─ creates Session / agent session row
│     └─ builds core Agent instances
├─ Tokio runtime domain: tui          (TUI mode only, one-worker multi-thread)
│  ├─ Tokio tasks
│  │  ├─ TUI event intake / bridge tasks
│  │  ├─ periodic tick task
│  │  └─ frame / redraw scheduling tasks
│  └─ non-Tokio OS thread
│     └─ dedicated terminal-input thread for Crossterm polling
├─ Tokio runtime domain: core         (multi-thread)
│  └─ Tokio tasks
│     ├─ startup coordination
│     ├─ headless / non-interactive execution paths
│     ├─ agent-turn execution
│     └─ core harness orchestration
├─ Tokio runtime domain: network      (multi-thread)
│  └─ Tokio tasks
│     ├─ Stylos status publisher
│     ├─ Stylos query handlers
│     ├─ Stylos command subscriber
│     └─ Stylos bridge tasks into the local app flow
├─ Tokio runtime domain: background   (multi-thread, reserved in phase 1)
│  └─ Tokio tasks
│     └─ lower-priority maintenance work
└─ supporting worker threads
   └─ spawn_blocking worker threads for DB-sensitive work in themion-core
```

Important nesting:

- process contains bootstrap and runtime domains
- each Tokio runtime domain contains Tokio tasks
- dedicated terminal-input polling is an OS thread, not a Tokio task
- `spawn_blocking` work uses worker threads alongside the runtime domains

A strict Tokio-centric view is:

```text
process
├─ bootstrap
├─ Tokio runtimes
│  ├─ tui runtime
│  │  └─ Tokio tasks
│  ├─ core runtime
│  │  └─ Tokio tasks
│  ├─ network runtime
│  │  └─ Tokio tasks
│  └─ background runtime
│     └─ Tokio tasks
└─ non-Tokio threads
   ├─ terminal input thread
   └─ spawn_blocking worker threads
```

For debugging, the practical thread model is now:

- one Themion process
- explicit Tokio runtime domains owned by `themion-cli`
- one one-worker multi-thread TUI runtime in TUI mode
- separate multi-thread core and network runtimes
- a reserved background runtime domain for lower-priority work
- one dedicated terminal-input OS thread for Crossterm polling in TUI mode
- `spawn_blocking` worker usage in `themion-core` for DB-sensitive work
- multiple async tasks communicating through unbounded `mpsc` channels

### TUI mode structure

In the current CLI architecture, `crates/themion-cli/src/app_state.rs` owns shared non-UI bootstrap for TUI mode, explicit `--headless` mode, and the non-interactive one-shot prompt path. `crates/themion-cli/src/tui_runner.rs` owns terminal-mode orchestration, `crates/themion-cli/src/headless_runner.rs` owns both the long-running headless NDJSON-log entrypoint and the non-interactive one-shot path, and `crates/themion-cli/src/tui.rs` remains the terminal presentation and event-handling layer.

In `crates/themion-cli/src/tui.rs`, the app creates a central `AppEvent` channel and then spawns a few long-lived background tasks around one main UI loop.

The main UI loop now works as a request-driven redraw loop:

1. perform the initial terminal draw
2. wait for the next `AppEvent` or a scheduled redraw notification
3. handle the event and mark visible UI regions dirty when needed
4. redraw only when a visible change or full invalidation is pending

The CLI keeps redraw requests separate from draw execution. Internal event paths request future frames, redraw requests may be coalesced, and idle wakeups such as ticks do not automatically imply a draw. Ratatui still handles terminal buffer diffing for actual frame updates.

Around that loop, the current implementation starts these long-lived tasks:

- a dedicated terminal-input OS thread using Crossterm polling to forward keyboard, mouse, and paste events into the app channel
- a periodic tick task using `tokio::time::interval(Duration::from_millis(150))` to send `AppEvent::Tick` on the TUI runtime domain
- bridge tasks that forward agent events or Stylos events into the same app event channel on the TUI runtime domain

That makes the TUI architecture event-driven rather than thread-per-subsystem.

### Agent execution model

When the user submits work, Themion does not create a separate process for the agent. Instead, the current process spawns async work on the Tokio runtime.

In the current code, this usually means:

- one spawned task runs the agent turn or shell-command work
- one spawned task forwards resulting events back to the TUI event channel when needed
- the TUI loop remains responsive because it consumes summarized events rather than blocking on provider IO directly

So the user-visible app behaves like one interactive process coordinating background async tasks, not like several child worker processes.

### Stylos-enabled background tasks

When built with the `stylos` cargo feature and enabled in config, `crates/themion-cli/src/stylos.rs` adds a few more long-lived networking tasks inside the same process. In the current phase-1 implementation, these long-lived Stylos tasks run on the explicit `network` runtime domain.

Current examples include:

- a status publisher task with a 5-second interval
- a queryable-serving task that waits on multiple Stylos query surfaces
- a command subscriber task that receives remote prompt requests
- TUI-side bridge tasks that forward Stylos command, prompt, and event channels into the main app loop

These are still process-local async tasks. Stylos does not introduce a separate Themion worker process for this runtime shape.

### What this model is good for

This mental model helps explain common debugging symptoms:

- high CPU with one hot thread often means one busy runtime worker or one event source is spinning
- steady wakeups during otherwise idle TUI use can come from the 150 ms tick task and redraw loop
- Stylos-enabled builds have more always-on background activity than non-Stylos builds
- task-level profiling is often more informative than assuming one named thread per feature

If you need to inspect the live process at the OS level, thread-oriented tools such as `top -H`, `ps -T`, `pidstat -t`, `perf`, or `gdb` can still be used, but the source code is organized first around async tasks and channels.

## Harness Loop (agent.rs)

Each call to `run_loop(user_input)`:

1. Record turn boundary (`turn_boundaries.push(messages.len())`); open a DB turn row via `begin_turn`.
2. Push `role="user"` message to history; persist to `agent_messages`.
3. Build windowed context (see §Context Windowing) and call `chat_completion_stream` on the active backend.
4. Stream tokens to TUI via `AgentEvent::AssistantChunk`; accumulate full response.
5. Push `role="assistant"` response to history; persist to `agent_messages`.
6. If response has no `tool_calls` → break.
7. For each tool call: emit `ToolStart` with raw tool name plus arguments JSON for the frontend to format, execute via `call_tool`, push `role="tool"` result; persist each.
8. Repeat from step 3 until the assistant returns with no more tool calls or another existing runtime stop condition ends the turn.
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

The recall hint is a synthetic `role="system"` message that reminds the model that omitted `session_id` stays in the current session and `session_id="*"` explicitly widens recall or search to all sessions in the current project.

The full `messages` Vec is never trimmed — the in-memory copy is always complete. Windowing only affects what is sent over the wire.

## Streaming

### Chat Completions backends (`client.rs`)

`chat_completion_stream` sends `"stream": true` and reads the response body chunk-by-chunk via `Response::chunk()`. SSE lines are split on the `0x0A` byte (safe for UTF-8 multi-byte sequences) and decoded per line. Each `data:` line is parsed as a `StreamChunkData`; `delta.content` fragments are forwarded to the `on_chunk` callback immediately. Tool call argument fragments are accumulated by `index` and assembled after `[DONE]`.

`"stream_options": {"include_usage": true}` is sent so the last chunk carries token counts.

### Codex Responses backend (`client_codex.rs`)

Codex uses the OpenAI Responses API rather than Chat Completions. Its stream consists of named SSE events such as `response.output_text.delta` and `response.completed`. The parser accumulates `event:` and `data:` lines until a blank-line frame boundary, then updates the in-flight response state. Text deltas stream to the UI immediately; usage is taken from the completion event.

## Tools (tools.rs)

All tools receive a `&ToolCtx` carrying the DB handle and session identity. Filesystem tools ignore it; history tools and workflow tools use it. Tool call display labels are center-trimmed to about 60 chars with `󱑼` when needed so the TUI can preserve both the beginning and end of long values.

Themion also exposes `system_inspect_local`, a read-only aggregate local inspection tool for runtime, tool-surface, and provider-readiness diagnosis. The tool is scoped to the current local Themion process and active agent context, returns structured machine-usable JSON, and is designed to be no-surprise: it does not run shell commands, mutate workflow/config/history, refresh auth, or perform hidden repair actions. In TUI mode, the runtime section includes `runtime.debug_runtime_lines`, which reuses the same local `/debug runtime` snapshot path so model-visible inspection stays aligned with the existing human-facing command. In non-TUI paths, the tool falls back to a bounded partial snapshot and marks unavailable runtime detail explicitly rather than fabricating live metrics.

`time_sleep` is a built-in bounded wait helper for short pauses. It accepts `ms`, rejects values above 30,000, and lets the agent express lightweight waiting without shelling out to `sleep`.

Durable note operations are exposed as `board_*` tools. These board tools manipulate local SQLite-backed note state for create/list/read/move/update-result operations. Stylos remains the transport and intake layer for remote note delivery and related mesh behavior.

Tool contracts now distinguish reads from writes more explicitly: detailed inspection stays on read/query tools, while several mutation tools now return compact structured acknowledgements by default. In particular, `board_create_note`, `board_move_note`, `board_update_note_result`, `memory_create_node`, `memory_update_node`, `memory_link_nodes`, and `fs_write_file` no longer use full-record or plain-text success returns for their normal mutation path.

## Persistent History (db.rs)

Database path: `$XDG_DATA_HOME/themion/system.db` (default `~/.local/share/themion/system.db`). Created on first run; WAL mode enabled on every open for safe multi-process access.

History and note persistence currently use these main tables:

- `agent_sessions` — one row per harness session, including project directory, interactive flag, and persisted workflow state
- `agent_turns` — one row per user turn with token counts, LLM round counts, tool-call counts, workflow phase-at-start/end metadata, and optional turn-level runtime attribution JSON in `meta` (for example `app_version`, `profile`, `provider`, and `model`)
- `agent_messages` — one row per persisted message; assistant tool-call payloads are stored in `tool_calls_json`, and tool result rows link back through `tool_call_id`
- `agent_workflow_transitions` — workflow/phase transition audit trail
- `board_notes` — durable note board rows keyed by canonical UUID `note_id` with unique human-friendly `note_slug`
- `memory_nodes` — Project Memory knowledge-base nodes for concepts, components, files, tasks, decisions, facts, observations, troubleshooting records, people, and occasional narrative memory records; each row includes a `project_dir` context
- `memory_node_hashtags` — normalized hashtag labels for memory nodes
- `memory_edges` — typed directed links between knowledge-base nodes

Machine-consumed note timestamps use explicit milliseconds fields such as `created_at_ms`, `updated_at_ms`, `injected_at_ms`, `completion_notified_at_ms`, and `blocked_until_ms`.

## Project Memory knowledge base

Themion includes Project Memory: a lightweight SQLite-backed long-term durable knowledge base for distilled knowledge that should outlive one conversation. The knowledge base complements persistent transcript history and board notes rather than replacing either of them. History remains the log of conversation turns, board notes remain task-coordination records, and Project Memory stores intentional reusable facts, decisions, concepts, file contracts, troubleshooting records, and relationships.

Storage is graph-backed and unified: concepts, components, files, tasks, decisions, facts, observations, troubleshooting records, people, and occasional narrative memory records are all rows in `memory_nodes`, and any two nodes can be connected through typed rows in `memory_edges`. Each memory node stores a `project_dir` context. Omitted project selection in memory tools defaults to the current session project directory only. The exact selector `[GLOBAL]` selects Global Knowledge, the virtual cross-project context inside Project Memory; it is for reusable cross-project facts, preferences, conventions, provider/tool behavior, and troubleshooting patterns, and is not resolved as a filesystem path. When unsure, agents should keep knowledge project-local and promote it to Global Knowledge later only when cross-project usefulness is clear. Project-specific searches do not silently include `[GLOBAL]` rows. Agents should prefer specific KB node types such as `fact`, `observation`, `decision`, `concept`, `component`, or `file`; `memory` is reserved for genuinely narrative capture when no more specific type is known. Hashtags are stored in `memory_node_hashtags` and act as flat retrieval labels such as `#rust`, `#provider`, or `#todo`; there is no separate memory scope hierarchy. Hashtags are normalized to lowercase, leading-`#` form, with hyphens converted to underscores.

The model-visible tool family is:

- `memory_create_node` — create a Project Memory KB node with title, optional content, type, hashtags, metadata, and optional `project_dir`; omitted `node_type` defaults to `observation`, omitted `project_dir` defaults to the current project, and `[GLOBAL]` writes to Global Knowledge
- `memory_update_node` — update node fields and replace hashtags when provided
- `memory_link_nodes` / `memory_unlink_nodes` — add or remove typed directed graph edges
- `memory_get_node` — read one node with immediate incoming and outgoing links
- `memory_search` — search by title/content FTS query, hashtags, node type, optional `project_dir`, and optional relation filters
- `memory_open_graph` — return a bounded local neighborhood around one or more anchor nodes
- `memory_delete_node` — delete a node and its hashtag/edge rows
- `memory_list_hashtags` — inspect frequently used labels in the selected Project Memory context

Machine-consumed memory timestamps use explicit millisecond fields such as `created_at_ms` and `updated_at_ms`. Neighborhood reads are bounded by depth and node limit so a heavily linked node does not dump the whole graph accidentally.

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
- `stylos/<realm>/themion/instances/<instance>/query/notes/request`

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
- `board_create_note`
- `board_list_notes`
- `board_read_note`
- `board_move_note`
- `board_update_note_result`

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
- `notes/request` accepts note-creation delivery for a target instance and agent; sender instance and sender agent identity are resolved automatically by the local runtime.
- when an inbound Stylos note delivery is accepted for local processing, the receiver-side TUI emits `Board note intake note_slug=<slug> from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id> column=todo` rather than reusing `Stylos hear ...` talk logging.
- when a receiver-side note delivery is stored successfully, the runtime emits `created board note in db note_slug=<slug> from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id> column=todo`.
- receiver-side inbound note logging and talk logging remain distinct; one inbound note delivery must not surface as a `Stylos hear ...` talk event.
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

### Durable Stylos notes board

Stylos collaboration now also supports durable notes backed by SQLite.

Current behavior:

- note records live in the main `system.db` SQLite database
- each note stores canonical UUID `note_id`, globally unique human-friendly `note_slug`, optional sender identity, exact target instance `<hostname>:<pid>`, target `agent_id`, body, board column, result text, and millisecond timestamps
- chat-panel/operator-facing note lifecycle events prefer `note_slug` as the visible note identifier, while tools, storage, and prompt metadata keep canonical UUID `note_id` where machine identity is required
- board columns are `todo`, `in_progress`, `blocked`, and `done`
- newly created notes start in `todo` by default, or may start in `blocked` for waiting-first follow-up work
- notes are model-visible through dedicated `board_*` note tools rather than transcript scraping
- when the `stylos` feature is enabled, `board_create_note` always submits through the Stylos `notes/request` path, even when the destination instance is the current local instance
- in Stylos-enabled builds, the receiver-side `notes/request` handler is the canonical create path that validates the target agent, creates the note row in local SQLite, and returns the created `note_id`
- `board_list_notes`, `board_read_note`, `board_move_note`, and `board_update_note_result` remain local board operations against the receiving instance's SQLite state after creation
- blocked notes store cooldown eligibility in `blocked_until_ms` milliseconds; moving a note into `blocked` or marking a blocked note injected refreshes that cooldown
- idle-time delivery prefers the oldest eligible `in_progress` note for an agent, then `todo`, then cooldown-eligible `blocked` notes
- once injected, the note is marked so it is not injected repeatedly by default
- idle-time injected note prompts identify themselves as durable notes and include core metadata such as `note_id`, `note_slug`, source/target identities, current column, and the note body so the model usually does not need an immediate `board_read_note` call for orientation
- collaboration guidance now treats durable notes as the preferred path for delegated asynchronous agent-to-agent work, while `talk` remains the interrupting realtime path for urgent or interactive coordination
- notes now distinguish delegated work requests from informational done mentions so completion notifications do not look like fresh delegated work
- when a delegated cross-agent note is moved to `done`, the completion path can create exactly one requester-directed done mention that includes original-note reference metadata and useful result content
- auto-created done mentions do not recursively generate more done mentions when they are later marked `done`

The receiver-side Stylos query surface now includes `stylos/<realm>/themion/instances/<instance>/query/notes/request` for durable note creation. This supersedes the old idle-only `talk` model for asynchronous work intake, while `talk` remains available as a lightweight realtime path.


## Runtime debug command

Themion now includes a built-in `/debug runtime` command in the TUI for app-local runtime diagnostics.

Current behavior:

- reports process-local identity and current app/workflow busy state
- reports a thread snapshot for the current process only; on Linux this reads `/proc/self/task/*/stat` and shows sampled cumulative thread CPU ticks rather than claiming exact percentages
- reports Themion-owned runtime activity counters for draw requests, executed draws, skipped-clean redraw attempts, ticks, input, agent events, incoming prompts, shell completions, and agent-turn start/completion
- reports recent-window activity counts and rates from snapshot deltas between retained in-app samples
- labels lifetime activity totals separately so they are not confused with recent-window metrics
- reports approximate draw timing from the same lightweight in-app counters
- in `stylos` builds, also reports lightweight Stylos loop counters for status publishing, query handling, and bridge activity
- explicitly treats task metrics as Themion activity signals, not exact per-Tokio-task CPU accounting

The redraw path is now dirty-gated and request-driven rather than unconditionally redrawing at the top of every loop iteration. Tick wakeups still occur, but they only cause a draw when some visible state actually changed.

This command is intended to help connect OS-visible symptoms such as hot threads with Themion's own event-loop and async-task structure.
