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
- `crates/themion-cli/src/` for session startup, process-local agent descriptors, and UI integration

## High-level flow

A single user turn follows this shape:

1. The CLI starts or resumes a harness session.
2. The user submits input.
3. The harness records a new turn and persists the user message.
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
10. The turn is finalized in SQLite with message, workflow, and token metadata.

## Agent identity boundary

`themion-core::Agent` owns per-agent harness state such as session ID, project directory, workflow state, messages, and model/backend integration. `themion-cli` owns process-local descriptors such as `agent_id`, `label`, and `roles` that describe how a given core agent is used within one Themion process.

This keeps reusable harness behavior in core while allowing the CLI to publish process-level multi-agent status for Stylos.

## Stylos remote-request bridge

Stylos request handling stays CLI-local even when it ultimately causes an agent turn.

In the current implementation:

- Stylos queryables are registered in `crates/themion-cli/src/stylos.rs`
- query handlers read the current exported process snapshot from a snapshot provider set by the TUI runtime
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
- sender `from` and `from_agent_id` are forwarded from the calling runtime context into Stylos note delivery so receiver-side logs and stored note metadata reflect the actual calling agent
- successful receiver-side note insertion emits `created board note in db note_id=<uuid> ...` before the note is queued for later injection
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
