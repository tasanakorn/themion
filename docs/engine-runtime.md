# Engine Runtime

This document explains how Themion's core harness/runtime works: how prompt inputs are assembled, how context is built, how tool calls are executed, how workflow state progresses, and how session history is stored.

## Scope

Most of the logic described here lives in `crates/themion-core/`. The CLI crate (`crates/themion-cli/`) is responsible for starting sessions, wiring the TUI, loading config, and passing the active project/session context into the core runtime.

When the optional `stylos` cargo feature is enabled for `themion-cli`, Stylos session startup and shutdown remain CLI-local runtime wiring rather than part of the core harness loop. In feature-enabled builds, Stylos starts by default unless config overrides disable it.

Relevant areas:

- `crates/themion-core/src/agent.rs`
- `crates/themion-core/src/client.rs`
- `crates/themion-core/src/client_codex.rs`
- `crates/themion-core/src/tools.rs`
- `crates/themion-core/src/db.rs`
- `crates/themion-core/src/workflow.rs`
- `crates/themion-core/src/memory.rs`
- `crates/themion-cli/src/` for session startup, process-local agent descriptors, and UI integration

## High-level flow

A single user turn follows this shape:

1. The CLI starts or resumes a harness session.
2. The user submits input.
3. The harness records a new turn and persists the user message. New `agent_turns` rows also capture optional turn-level runtime attribution in `agent_turns.meta` as compact JSON, currently including `app_version`, `profile`, `provider`, and `model` when available.
4. The harness builds the model input from:
   - the base system prompt
   - predefined built-in coding guardrails
   - predefined Codex CLI web-search instruction
   - injected contextual instructions such as `AGENTS.md`
   - workflow context and phase instructions
   - an optional history recall hint
   - the recent conversation window
5. The active backend streams the assistant response.
6. If the model requests tools, the harness executes them and appends tool results to the conversation.
7. The harness calls the model again with the updated conversation.
8. Workflow tools may also inspect or mutate the current workflow state between model calls.
9. This repeats until the model returns a normal assistant response with no more tool calls, or another existing runtime stop condition ends the turn.
10. The turn is finalized in SQLite with message, workflow, token, and turn-level runtime metadata.

## Agent identity boundary

`themion-core::Agent` owns per-agent harness state such as session ID, project directory, workflow state, messages, and model/backend integration. `themion-cli` owns process-local descriptors such as `agent_id`, `label`, and `roles` that describe how a given core agent is used within one Themion process.

This keeps reusable harness behavior in core while allowing the CLI to publish process-level multi-agent status for Stylos.

## CLI-local runtime domains

`themion-cli` now owns explicit Tokio runtime construction through a CLI-local runtime topology helper.

Current phase-1 runtime domains:

- `tui` — one-worker multi-thread runtime for TUI event intake, tick scheduling, frame scheduling, and TUI-side bridge tasks
- `core` — multi-thread runtime for startup coordination, print-mode execution, and core harness orchestration paths
- `network` — multi-thread runtime for long-lived Stylos networking tasks
- `background` — reserved multi-thread runtime domain for lower-priority maintenance work in this phase

Mode differences:

- TUI mode constructs `tui`, `core`, `network`, and `background` and runs through a shared CLI app-runtime plus `tui_runner`
- explicit `--headless` mode constructs the reduced non-TUI runtime set currently needed by that path, which is `core` and `network`, runs through the same shared CLI app-runtime plus `headless_runner`, and emits structured NDJSON lifecycle logs on stdout
- non-interactive prompt-argument mode also reuses that shared CLI app-runtime, but remains a one-shot stdout/stderr execution path rather than the long-running headless NDJSON-log mode

This preserves the single-process architecture while making runtime ownership explicit in startup code. In the current implementation, the `tui` domain remains a Tokio multi-thread runtime configured with one worker thread, while `core`, `network`, and `background` remain multi-thread runtimes. The full thread model is slightly broader than the domain list alone: TUI mode also uses one dedicated terminal-input OS thread for Crossterm polling, and `themion-core` uses `spawn_blocking` for DB-sensitive work.

### Runtime hierarchy

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

## Stylos remote-request bridge

Shared CLI bootstrap now lives in `crates/themion-cli/src/app_state.rs`, which resolves the project directory, opens the local DB, inserts the session row, builds `Session`, and exposes shared agent-construction helpers used by TUI mode, explicit `--headless` mode, and non-interactive prompt execution. `main.rs` stays thin and only selects which runner to invoke.

Stylos request handling stays CLI-local even when it ultimately causes an agent turn.

In the current implementation:

- Stylos queryables are registered in `crates/themion-cli/src/stylos.rs` and their long-lived serving/publishing/subscription tasks now run on the CLI-owned `network` runtime domain
- query handlers read the current exported process snapshot from a snapshot provider published from the CLI-local app/TUI state path
- accepted `talk`, durable `notes/request`, and `tasks/request` queries are converted into `IncomingPromptRequest` values or persisted note records and sent over an in-process/local-runtime path
- the TUI event loop receives those requests as `AppEvent::IncomingPrompt`
- the TUI either rejects the request immediately if the current local execution path is already busy, or submits the prompt through the same local turn path used for normal input

This means Stylos does not bypass the harness loop, call providers directly, or move history/tool execution into the transport layer. It only injects new work into the existing local input path.

## Snapshot-driven request decisions

The query layer makes best-effort decisions from the current local snapshot rather than from a separate scheduler.

That includes:

- `free` discovery using exported `activity_status`
- `talk` acceptance requiring the requested agent to be present and currently `idle` or `nap`, unless the caller provides positive `wait_for_idle_timeout_ms`
- `talk` busy-peer waiting polling the exported snapshot until the target becomes available or the timeout expires
- `tasks/request` candidate selection using the exported agent list, role metadata, git-repo metadata, and current activity state
- Zenoh-level `stylos_query_nodes` using `session.info()` from the active local Stylos session rather than Themion mesh queryables

Because those checks are snapshot-based, they can race with local activity changes. The runtime reports the chosen agent honestly and may still fail later with `agent_busy` if the selected local execution path is no longer available by the time the request reaches the event loop.

## In-memory task lifecycle tracking

`crates/themion-cli/src/stylos.rs` maintains an in-memory `TaskRegistry` for accepted remote tasks.

Current lifecycle behavior:

- `tasks/request` allocates a stable `task_id` and inserts a `queued` entry
- when the bridged local turn actually begins, the registry updates that task to `running`
- when the turn finishes, the TUI stores the last observed assistant text as the terminal result and marks the task `completed`
- if the request cannot be delivered or arrives while the selected local execution path is already busy, the registry marks the task `failed` with a machine-readable reason such as `agent_busy`
- `tasks/status` reads the current registry entry without blocking
- `tasks/result` can wait for a terminal state up to a bounded timeout, then returns either the finished result or the current non-terminal state with `timed_out = true`

The registry is intentionally process-local and non-durable in this first release. Process restart drops pending remote work and prior task records.

## Remote execution targeting in the current slice

The current Stylos bridge validates requested `agent_id` values against the exported snapshot, records the requested agent in the remote request payload, and routes execution to that matching local agent when present.

That means:

- strict local `agent_id` execution targeting has landed for the current process-local agent set
- the query layer still relies on snapshot-based selection and a CLI-local in-process bridge rather than a durable scheduler
- this preserves the current harness architecture while still making the query and task surface useful for discovery, request submission, and best-effort status lookup

## Sender-aware talk prompt injection

Stylos `talk` now resolves sender identity automatically and carries exact instance identifiers through the CLI-local bridge:

- sender-side local instance `from` resolved automatically as exact `<hostname>:<pid>`
- mandatory target `to` in exact `<hostname>:<pid>` form
- optional `to_agent_id` on the request input, defaulting to `main`
- optional `request_id`
- optional `wait_for_idle_timeout_ms`

When a `talk` request is accepted, the CLI does not inject the raw message directly. Instead it wraps the message in a peer-message prompt that tells the receiving agent:

- who sent the message
- which local agent received it
- that it should reply only when a materially useful response is needed
- that `***QRU***` means no further reply is normally needed
- that empty acknowledgements and thank-you-only replies should be avoided

This keeps sender identity and reply guidance visible to the model in the harness prompt path rather than hidden only in transport metadata.

When the local agent invokes `stylos_request_talk` and the request is accepted, the TUI also emits a sender-side chat-panel event line in exact identifier form:

- `Stylos talk to=<hostname>:<pid> from=<hostname>:<pid>`

This sender-side log is distinct from generic tool-call text and is intended to make outbound peer messaging visible in the chat transcript.

Tool-call chat labels remain compact, but long tool detail values are now center-trimmed to about 60 characters with `󱑼` so the display can preserve both the beginning and the end of paths, commands, and other long identifiers.

## Local system inspection tool

`ToolCtx` now also carries current workflow state and an optional local system-inspection snapshot so tool execution can answer runtime-diagnostic requests without reaching back through TUI-only slash-command handling.

`system_inspect_local` is the current aggregate local inspection tool. It is intentionally read-only and bounded: it reports local runtime state, available tool surface, and provider/model readiness without mutating workflow/config/history, writing board or memory data, invoking shell commands, or performing implicit expensive probes.

Current behavior:

- returns structured top-level sections including `overall_status`, `summary`, `runtime`, `tools`, `provider`, `warnings`, and `issues`
- in TUI mode, the runtime section includes `runtime.debug_runtime_lines`, reusing the same `/debug runtime` snapshot text assembly path used by the human-facing command
- in non-TUI or fallback paths, the tool still returns a bounded local snapshot but reports unavailable runtime details explicitly
- provider readiness is based on already-known local state such as active profile/provider/model, auth presence, base URL presence, and recent rate-limit metadata when available
- tool-surface reporting is based on the locally defined tool registry for the current build/runtime shape

This keeps the model-visible diagnostic surface aligned with the human `/debug runtime` command while preserving a stable structured tool contract.

## File and shell tool bounds

`themion-core` now uses more explicit and bounded contracts for the main local filesystem and shell tools.

Current behavior:

- `fs_read_file` accepts `path` plus optional `mode`, `offset`, and `limit`
- `fs_read_file` defaults to `mode=base64`, `offset=0`, and `limit=131072` bytes
- `fs_read_file` rejects `limit` values above `2097152` bytes
- `fs_read_file` returns the selected byte range together with range metadata such as returned byte count, file size, and EOF state
- `fs_read_file` only allows `mode=raw` when the selected byte slice is valid UTF-8; otherwise it returns an error directing the caller to `base64`
- `fs_write_file` accepts optional `mode` and defaults to `base64`, decoding bytes before writing in that default mode
- `fs_write_file` still supports direct text writes through `mode=raw`
- `shell_run_command` accepts optional `result_limit` and `timeout_ms`, defaulting to `16384` bytes and `300000` ms
- `shell_run_command` truncates oversized returned output with an explicit truncation notice
- `shell_run_command` returns a clear timeout result when the command exceeds the configured timeout

These defaults make binary-safe file transfer and bounded shell usage the normal path rather than a caller-side convention.

## Project Memory knowledge-base tools

Themion exposes one `memory_*` tool family backed by SQLite in `themion-core`. The tools build Project Memory: an intentional long-term durable knowledge base, not a transcript log or task board. Concepts, components, files, tasks, decisions, facts, observations, troubleshooting records, people, and occasional narrative memory records are all `memory_nodes`, while `memory_edges` records typed directed links between any two nodes. Each node stores a `project_dir` memory context. Hashtags are flat normalized labels in `memory_node_hashtags`; they replace a separate memory scope concept for this feature.

Model-facing guidance: use these tools for durable reusable knowledge that should outlive the current session. Project Memory defaults to the current project. Use the exact selector `[GLOBAL]` only for Global Knowledge: the virtual cross-project context inside Project Memory for reusable facts, preferences, conventions, provider/tool behavior, or troubleshooting patterns. `[GLOBAL]` is not a filesystem path and is not resolved or canonicalized. When unsure, keep knowledge project-local and promote later only when cross-project usefulness is clear. Prefer specific `node_type` values such as `concept`, `component`, `file`, `task`, `decision`, `fact`, `observation`, `troubleshooting`, or `person`. Use `memory` only for narrative long-term capture when no more specific type fits. Keep ordinary transcript reconstruction in history tools and coordination work in board notes.

Current tool contracts:

- `memory_create_node` accepts `title`, optional `project_dir`, optional `node_type` (default `observation`), optional `content`, optional `hashtags`, optional UUID `node_id`. Omitted `project_dir` uses the current session project directory; exact `[GLOBAL]` uses Global Knowledge. It stores timestamps in milliseconds as `created_at_ms` and `updated_at_ms`.
- `memory_update_node` accepts `node_id` plus any mutable node fields. When `hashtags` is supplied, it replaces the node's hashtag set. `content` may be set to `null`.
- `memory_link_nodes` accepts `from_node_id`, `to_node_id`, `relation_type`, and optional UUID `edge_id`. Both endpoint nodes must already exist.
- `memory_unlink_nodes` removes an edge by `edge_id`, or by the exact `from_node_id`/`to_node_id`/`relation_type` tuple.
- `memory_get_node` returns the node, including its `project_dir`, plus immediate `outgoing` and `incoming` edge arrays.
- `memory_search` supports FTS5 keyword queries over title/content when FTS5 is available, project context filtering, hashtag filtering with `hashtag_match` of `any` or `all`, node type filtering, and optional relation filters. Omitted `project_dir` searches only the current session project; `[GLOBAL]` searches only Global Knowledge. If FTS5 is unavailable, non-keyword filters still work.
- `memory_open_graph` opens a bounded local neighborhood around `node_id` or `node_ids`; depth is clamped to 3 and node limit to 200.
- `memory_delete_node` deletes one node and its directly owned hashtag and edge rows.
- `memory_list_hashtags` returns hashtag usage counts for the selected project context, optionally by prefix. Omitted `project_dir` uses the current session project; `[GLOBAL]` lists only Global Knowledge tags.

Hashtag normalization is case-insensitive: leading `#` is optional, stored hashtags always include it, and hyphens become underscores. Labels such as node type, relation type, and hashtag bodies accept letters, digits, underscores, and hyphens.

## Lightweight wait tool

`themion-core` now exposes a built-in `time_sleep` tool for short bounded waits.

Current behavior:

- accepts `ms`
- sleeps without invoking the shell
- rejects durations above 30,000 ms
- returns structured JSON with the slept duration

This is intended for lightweight pauses and retry gaps. It is not a general scheduler or background timer system.

## CLI-local transcript review boundary

Long-session transcript navigation in the TUI is a CLI-local display concern and does not change the core harness loop.

That means:

- follow-tail versus browsed-history state lives in `crates/themion-cli/src/tui.rs`
- the read-only transcript review overlay uses the current in-memory `Entry` list already held by the TUI for the active session
- browsing old content does not alter `themion-core::Agent` message history, turn boundaries, SQLite persistence, tool semantics, or prompt assembly
- streamed assistant chunks continue to arrive through normal `AgentEvent` handling; the CLI only changes whether the current viewport follows the latest content automatically
- persistent history tools such as `history_recall` and `history_search` remain the mechanism for model-visible access to older stored history outside the current prompt window
- these history tools are always scoped to the current project directory; omitted `session_id` means the active session, `session_id="*"` means all sessions in the current project, and explicit UUIDs only match sessions within that current project

This feature improves user-facing review behavior without changing runtime semantics in `themion-core`.

## CLI redraw scheduling

`crates/themion-cli/src/tui.rs` now uses a request-driven redraw path rather than unconditionally drawing once per event-loop iteration.

Current behavior:

- the TUI performs an initial draw at startup
- later draws are triggered by scheduled redraw notifications after event handlers mark visible state dirty
- dirty tracking is coarse and UI-shaped: conversation, input, status, overlay, or full invalidation
- multiple redraw requests may be coalesced before one actual `terminal.draw(...)` call
- the 150 ms tick still updates runtime counters and idle-time logic, but it only results in a draw when visible UI state changed
- `/debug runtime` distinguishes draw requests, executed draws, and skipped-clean redraw attempts so redraw churn can be inspected without confusing wakeups with actual draws

This keeps Ratatui's buffer-diff renderer in place while avoiding unnecessary frame rebuilding during idle periods.

## Durable Stylos notes runtime

PRD-029 phase 1 adds a durable board-backed note path.

Current behavior:

- `notes/request` validates the target agent from the current exported snapshot
- when the `stylos` feature is enabled, `board_create_note` always reaches note creation through `notes/request`, including self-targeted delivery to the current local instance
- accepted notes are persisted in SQLite immediately rather than rejected when the agent is busy
- persisted notes start in column `todo` by default, or may start in `blocked` for waiting-first follow-up work
- blocked notes store durable `blocked_until_ms` cooldown metadata and are only eligible for automatic idle-time reinjection after cooldown expires
- idle injection priority is `in_progress`, then `todo`, then cooldown-eligible `blocked`
- `themion-core` exposes `board_*` note tools for create/list/read/move/update-result operations using canonical durable UUID `note_id` values and returns companion `note_slug` metadata for human-readable inspection
- TUI chat-panel/operator-facing note lifecycle events prefer `note_slug` as the visible note identifier, while machine-facing tool results and prompt metadata preserve canonical `note_id`
- sender `from` and `from_agent_id` are forwarded from the calling runtime context into Stylos note delivery so receiver-side logs and stored note metadata reflect the actual calling agent
- successful receiver-side note insertion emits `created board note in db note_slug=<slug> ...` before the note is queued for later injection
- inbound note delivery logging is distinct from talk logging: note delivery uses `Board note intake ...`, while talk delivery uses `Stylos hear ...`
- the TUI checks for pending local notes on tick when no local turn is active
- idle injection prefers pending `in_progress` notes; `todo` is considered only when no pending `in_progress` note exists for that agent
- injected notes use a note-specific prompt wrapper, include core note metadata (`note_id`, `note_slug`, sender/target identities, current column, then body), and are marked injected to avoid duplicate delivery
- prompt-visible board guidance says simple direct Q&A without tools usually should not create a self-note, while tool-using or follow-up-tracked work should consider one
- prompt-visible collaboration guidance prefers durable notes over `talk` for delegated asynchronous work, treating `talk` as a more interrupting realtime path
- injected note prompts distinguish delegated work-request notes from informational done mentions so agents can treat completion notifications as result handoff rather than fresh delegated work
- when a delegated cross-agent note reaches `done`, the CLI-side completion path can emit one requester-directed done mention through the existing note-create flow, including original-note reference metadata and result text when available
- auto-created done mentions are classified so later marking them `done` does not recursively emit another automatic completion notification

This keeps persistence and board state durable while still reusing the normal harness turn path for actual agent work.

## Runtime debug command

`themion-cli` now exposes `/debug runtime` as a CLI-local diagnostic command.

Current behavior:

- prints process identity, current busy/workflow state, and thread snapshot data for the running Themion process
- on Linux, thread details come from `/proc/self/task/*/stat` and are reported as sampled cumulative user/system CPU ticks, not exact percentages
- prints Themion task/activity metrics derived from lightweight counters and handler timing around the TUI loop, input path, tick path, agent event path, shell completion path, and agent-turn lifecycle
- keeps cumulative counters internally but renders the recent-window section from snapshot deltas rather than from raw lifetime totals
- labels separately shown lifetime totals explicitly as lifetime values
- when the `stylos` feature is enabled, also prints lightweight Stylos runtime counters for status publishing, query handling, and bridge event categories

Because Tokio tasks are cooperatively scheduled async tasks rather than kernel threads, the task section is intentionally described as activity/busy-time observability. It must not be read as exact per-Tokio-task CPU accounting.
