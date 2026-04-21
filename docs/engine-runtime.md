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
9. This repeats until the model returns a normal assistant response with no more tool calls, or the loop limit is reached.
10. The turn is finalized in SQLite with message, workflow, and token metadata.

## Agent identity boundary

`themion-core::Agent` owns per-agent harness state such as session ID, project directory, workflow state, messages, and model/backend integration. `themion-cli` owns process-local descriptors such as `agent_id`, `label`, and `roles` that describe how a given core agent is used within one Themion process.

This keeps reusable harness behavior in core while allowing the CLI to publish process-level multi-agent status for Stylos.

## Stylos remote-request bridge

Stylos request handling stays CLI-local even when it ultimately causes an agent turn.

In the current implementation:

- Stylos queryables are registered in `crates/themion-cli/src/stylos.rs`
- query handlers read the current exported process snapshot from a snapshot provider set by the TUI runtime
- accepted `talk` and `tasks/request` queries are converted into `StylosRemotePromptRequest` values and sent over an in-process channel
- the TUI event loop receives those requests as `AppEvent::StylosRemotePrompt`
- the TUI either rejects the request immediately if the current local execution path is already busy, or submits the prompt through the same local turn path used for normal input

This means Stylos does not bypass the harness loop, call providers directly, or move history/tool execution into the transport layer. It only injects new work into the existing local input path.

## Snapshot-driven request decisions

The query layer makes best-effort decisions from the current local snapshot rather than from a separate scheduler.

That includes:

- `free` discovery using exported `activity_status`
- `talk` acceptance requiring the requested agent to be present and currently `idle` or `nap`
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

## CLI-local transcript review boundary

Long-session transcript navigation in the TUI is a CLI-local display concern and does not change the core harness loop.

That means:

- follow-tail versus browsed-history state lives in `crates/themion-cli/src/tui.rs`
- the read-only transcript review overlay uses the current in-memory `Entry` list already held by the TUI for the active session
- browsing old content does not alter `themion-core::Agent` message history, turn boundaries, SQLite persistence, tool semantics, or prompt assembly
- streamed assistant chunks continue to arrive through normal `AgentEvent` handling; the CLI only changes whether the current viewport follows the latest content automatically
- persistent history tools such as `history_recall` and `history_search` remain the mechanism for model-visible access to older stored history outside the current prompt window

This feature improves user-facing review behavior without changing runtime semantics in `themion-core`.
