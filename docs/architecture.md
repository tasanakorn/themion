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
- **Stateful conversation, budget-windowed** — `Agent` owns the full in-memory history, while prompt assembly replays recent history through the current budget-aware policy. Older turns persist in SQLite and are reachable via tools.
- **OpenAI-style tool calling** — tools are described as JSON function schemas; compatible providers can invoke them and return structured tool calls.
- **Project Memory knowledge base** — distilled reusable project knowledge is stored as SQLite-backed graph nodes and edges, with hashtags as the lightweight organization layer; the explicit `[GLOBAL]` context is Global Knowledge for reusable cross-project facts.
- **Event-driven TUI** — `Agent` emits `AgentEvent` variants over an `mpsc` channel; the TUI renders each event as it arrives, giving streaming token display without blocking the input loop.
- **Multi-agent transcript attribution** — the TUI transcript now carries structured optional local-agent attribution for assistant, tool, status, remote-event, and turn-complete entries, rendering compact highlighted `[agent_id]` prefixes with deterministic session-local colors when a specific local agent owns the line.
- **Ownership-first non-agent source labeling** — when no specific local agent owns a status or remote-event transcript line, the TUI now keeps that line on a separate non-agent presentation path and renders a compact source label such as `BOARD`, `STYLOS`, `RUNTIME`, or `WATCHDOG` with stable category color instead of forcing a misleading agent tag.
- **Provider abstraction** — the core harness speaks through a `ChatBackend` trait so different transports and wire formats can be swapped at runtime.
- **Separated prompt inputs** — the base system prompt, predefined coding guardrails, a predefined Codex CLI web-search instruction, and contextual instruction files such as `AGENTS.md` are treated as distinct prompt inputs rather than merged into a single message.
- **Built-in coding guardrails stay minimal** — the predefined guardrail layer covers default coding behavior such as assumption transparency, simple solutions, targeted edits, narrow validation, preferring the smallest clear answer shape with plain prose first, 1–2 sentence replies when enough, bullets/headings/tables only when they materially help, and about 4±1 meaningful chunks by default otherwise, allowing expansion toward about 7±2 chunks only for user-requested fuller explanation, preserving important tool-learned findings in ordinary assistant chat text with concise 1–2 sentence summaries by default, and brief specific commit messages naming the actual change when the user explicitly asks for a commit.
- **One process can describe multiple agents** — the CLI runtime stores agent descriptors in a vector, and Stylos status publishes process-level metadata plus an `agents` list rather than flattening one effective agent into top-level fields.
- **Local team membership is CLI-owned** — the current single-instance team roster is managed in `themion-cli`, where the process-local runtime owns `agent_id`, `label`, `roles`, and runtime construction/removal for local agents, while `themion-core` exposes the tool surface and each core `Agent` keeps its own harness state.
- **TUI is a strict human I/O surface** — `tui.rs` and `tui_runner.rs` should only collect human input, translate that input into runtime/app-state intents, observe runtime/app-state outputs, and render those outputs back to the human. They should not own or interpret watchdog policy, Stylos coordination, board scheduling, incoming-prompt admission, agent scheduling, workflow control, or other runtime decisions.
- **Hub/app-state must own shared runtime truth and system decisions** — per repository guidance, TUI and headless surfaces should observe or project runtime-owned snapshots rather than owning the canonical agent roster, workflow truth, watchdog state, board-routing policy, or Stylos-published status. Stylos status/query handling should consume hub/app-state-owned cheap-clone snapshots instead of reconstructing status from TUI-owned state.
- **If the system decided it, TUI should only display it** — decisions made by watchdog, Stylos transport, board coordination, runtime scheduling, or agent admission belong in non-TUI runtime/orchestrator modules such as `app_state.rs`, `app_runtime.rs`, `board_runtime.rs`, and `stylos.rs`. The TUI may render those outcomes or forward human requests that influence them, but should not contain the policy logic itself.

## Component Map

```text
main.rs
  └─ loads Config, parses mode/args, constructs Tokio runtime domains, and builds shared AppState
       ├─ non-interactive prompt args ──► headless_runner::run_non_interactive(app_state, prompt)
       ├─ --headless               ──► headless_runner::run(app_state)
       └─ TUI mode                 ──► tui_runner::run(app_state)

AppState  (app_state.rs)
  ├─ resolves project_dir and opens DbHandle
  ├─ inserts agent_sessions row and builds Session
  ├─ owns shared CLI-local runtime/bootstrap state, agent roster, and runtime snapshots
  └─ builds core Agent instances for TUI, headless, and non-interactive runners

tui.rs / tui_runner.rs
  ├─ terminal-mode setup, cleanup, keyboard/mouse/paste intake, and redraw scheduling
  ├─ forwards human input as runtime/app-state intents
  ├─ observes runtime/app-state snapshots and transcript events
  └─ renders terminal presentation without owning DB/session bootstrap or runtime policy

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
  ├─ loads auth through the active `openai-codex` profile rather than one shared global login blob
  └─ assembles ResponseMessage + Usage from Responses events

tools.rs
  ├─ tool_definitions() → JSON schema array (sent every request)
  ├─ call_tool(name, args, ctx: &ToolCtx) → String
  │    ├─ fs_read_file, fs_write_file, fs_list_directory  (ignore ctx)
  │    ├─ shell_run_command  ──► resolves the user shell, prefers Unix login-shell execution (`-lc`), and falls back to `sh` or platform default shell behavior when needed
  │    ├─ time_sleep  ──► bounded non-shell wait for short sleeps
  │    ├─ history_recall  ──► ctx.db.recall(RecallArgs)
  │    ├─ unified_search  ──► generalized indexed retrieval across memory/chat/tool records
  │    ├─ system_inspect_local ──► local runtime/tool/provider readiness snapshot
  │    ├─ local_agent_create / local_agent_delete ──► CLI-owned local team roster mutation within the current instance
  │    ├─ board_*  ──► local durable notes board operations
  │    ├─ memory_* ──► SQLite Project Memory KB nodes, hashtags, and edges
  │    └─ workflow_*  ──► workflow state inspection / transitions
  └─ ToolCtx { db: Arc<DbHandle>, session_id, project_dir, workflow_state, system_inspection }

History tools are always scoped to the caller's current project directory. Callers cannot pass a `project_dir` override; omitted `session_id` means the active session, `session_id="*"` means all sessions in the current project, and explicit UUIDs only match sessions within that same current project.

Current local team-membership behavior is intentionally narrow: the built-in `master` agent remains the predefined leader with `master` + `interactive`, `master` is reserved and cannot be recreated or deleted through the management tools, omitted create requests allocate the next free `smith-N` worker id, omitted or empty role lists for additional created agents default to `executor`, and deletion currently uses a safe-refusal policy while the local runtime is busy. Created agents become available immediately in the active in-memory roster and removed agents stop being targetable in that active runtime. Each active agent turn receives a separate compact role-context instruction from runtime-owned descriptors: active identity, resolved roles, short known-role glossary, matching action guidance only for the agent's own roles, and non-interactive short-reporting guidance when it lacks `interactive`.
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
├─ shared CLI app-state/runtime ownership (application state, not a Tokio executor)
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
├─ Tokio runtime domain: background   (multi-thread)
│  └─ Tokio tasks
│     ├─ lower-priority maintenance work
│     ├─ pending chat-message unified-search embedding
│     └─ semantic reindex / indexing follow-up work
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
- a background runtime domain for lower-priority maintenance, pending chat-message embedding, and semantic reindex work
- one dedicated terminal-input OS thread for Crossterm polling in TUI mode
- `spawn_blocking` worker usage in `themion-core` for DB-sensitive work
- multiple async tasks communicating through unbounded `mpsc` channels

### TUI mode structure

In the current CLI architecture, `crates/themion-cli/src/app_state.rs` owns shared non-UI bootstrap for TUI mode, explicit `--headless` mode, and the non-interactive one-shot prompt path. `crates/themion-cli/src/tui_runner.rs` owns terminal-mode orchestration, `crates/themion-cli/src/headless_runner.rs` owns both the long-running headless NDJSON-log entrypoint and the non-interactive one-shot path, and `crates/themion-cli/src/tui.rs` remains the terminal presentation and event-handling layer.

In Stylos-enabled builds, the board-note injection and note-completion follow-up coordination that feeds the TUI now lives in `crates/themion-cli/src/board_runtime.rs`. Incoming-prompt acceptance/rejection policy and related delivery-side effects now live in `crates/themion-cli/src/app_runtime.rs`, while sender-side Stylos transport-event derivation now lives in `crates/themion-cli/src/stylos.rs`. `tui_runner.rs` owns terminal-mode orchestration and no longer routes the targeted snapshot-refresh/publication hook through `tui.rs`. Together, these boundaries keep `tui.rs` focused on rendering, event translation, and UI-local state instead of request-admission or transport-policy ownership.

Within that TUI layer, input ownership is now split into a Themion-local editor and composer pattern inspired by `codex-rs`: `crates/themion-cli/src/textarea.rs` owns the local UTF-8 text buffer, wrap-aware height and cursor calculations, and render-facing state via `TextArea` plus `TextAreaState`, while `crates/themion-cli/src/chat_composer.rs` owns higher-level input policy such as paste-burst handling, non-ASCII-sensitive input routing, history draft restore, and submit-versus-newline decisions. `App` in `crates/themion-cli/src/tui.rs` delegates input editing to that composer instead of owning a third-party textarea directly.

TUI keyboard exit behavior now keeps `/exit` and `/quit` as explicit single-step slash-command exits, preserves `Esc` as the in-progress turn interrupt key, and requires two `Ctrl+C` presses within 3 seconds to exit from the keyboard path. After the first `Ctrl+C`, the TUI emits a lightweight local notice telling the user to press `Ctrl+C` again within the timeout window.

The TUI now also distinguishes persistent config changes from session-only runtime overrides. `/config profile use <name>` still switches profile and saves that choice to config, while `/session profile use <name>` and `/session model use <model>` rebuild the live interactive agent for the current session only without rewriting config on disk. `/session show` reports configured versus effective runtime state, and `/session reset` clears temporary session-only overrides.

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

For each admitted local agent turn, the runtime starts agent-owned async work rather than blocking the terminal loop. In the common TUI path this means:

- a spawned task runs the selected agent turn or shell-command work
- a relay path forwards resulting events back to runtime/app-state and, when presentation is needed, to the TUI event channel
- the TUI loop remains responsive because it consumes summarized events and snapshots rather than blocking on provider IO directly

So the user-visible app behaves like one interactive process coordinating background async tasks, not like several child worker processes.

Themion now supports overlapping local turns across multiple local agents within one process. The CLI app-state/runtime layer owns the local roster and event fan-in, but turn admission is now per-agent rather than gated by one app-global active-turn lock. Process-level busy summaries remain aggregate observability fields, while targeted execution availability is determined from each local agent handle individually.

### Stylos-enabled background tasks

When built with the `stylos` cargo feature and enabled in config, `crates/themion-cli/src/stylos.rs` adds a few more long-lived networking tasks inside the same process. In the current phase-1 implementation, these long-lived Stylos tasks run on the explicit `network` runtime domain.

Current examples include:

- a status publisher task with a 5-second interval
- a queryable-serving task that waits on multiple Stylos query surfaces
- a command subscriber task that receives remote prompt requests
- runtime/app-state bridge tasks that forward Stylos command, prompt, and event channels into the local app flow, with TUI receiving only renderable outcomes

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
7. For each tool call: emit `ToolStart` with raw tool name plus raw arguments JSON and optional display-enriched arguments JSON for frontend formatting, execute via `call_tool`, push `role="tool"` result; persist each.
8. Repeat from step 3 until the assistant returns with no more tool calls or another existing runtime stop condition ends the turn.
9. Finalize the DB turn row with token stats; emit `TurnDone`.

## Context Windowing

Prompt replay is budget-aware rather than purely fixed by `Agent.window_turns`. The current implementation keeps `window_turns` as a compatibility field, but `themion-core` now prefers tokenizer-backed local token estimation through `tiktoken-rs` when the active model resolves through an exact upstream model mapping, falls back through a short explicit trusted tokenizer mapping for selected known aliases, and finally degrades to the rough `chars / 4` estimator when no tokenizer path is trusted.

The replay policy keeps the active turn (`T0`) as the highest-priority replay unit, never replays turns older than `T-7`, degrades `T-1` through `T-5` into assistant-style pure-message replay when `T0` alone exceeds the normal 170K target, and omits prior allowed turns when `T0` alone exceeds the 250K spike ceiling or when older-turn inclusion within the `T-7` band would exceed that ceiling. Durable history remains stored in SQLite even when replay is reduced.

On each LLM round, prompt assembly uses this broad order:

```text
[system_prompt]
[predefined coding guardrails]
[predefined Codex CLI web-search instruction]
[injected contextual instructions such as AGENTS.md, when available]
[workflow context + phase instructions]
[recall hint, when older session history is omitted]
[budget-aware replay of recent conversation history]
```

The recall hint is a synthetic `role="system"` message that reminds the model that omitted `session_id` stays in the current session and `session_id="*"` explicitly widens recall or search to all sessions in the current project. The full `messages` Vec is never trimmed; the in-memory copy remains complete while only the prompt-visible replay is reduced.

For the most detailed and current prompt-budget behavior, see [engine-runtime.md](engine-runtime.md).
