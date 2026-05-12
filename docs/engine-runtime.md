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
Because current-session runtime state can now change through explicit session-only overrides as well as persistent config changes, those turn-level `profile`/`provider`/`model` values continue to reflect the effective runtime state active when each turn began rather than only the persisted config defaults.
4. The harness builds the model input from:
   - the base system prompt
   - predefined built-in coding guardrails, including guidance to prefer the smallest clear answer shape, prefer plain direct prose for simple answers, answer in 1–2 sentences when that is enough, add bullets/headings/tables only when that extra structure materially helps scanning or comparison, otherwise organize replies into about 4±1 meaningful chunks by default, allow expansion toward about 7±2 chunks only for user-requested fuller explanation, and preserve important tool-learned findings in ordinary assistant chat text with concise 1–2 sentence summaries by default when that information matters
   - predefined Codex CLI web-search instruction
   - injected contextual instructions such as `AGENTS.md`
   - workflow context and phase instructions
   - an optional history recall hint
   - a budget-aware replay of recent conversation history rather than only a fixed recent-turn window
5. The active backend streams the assistant response.
6. If the model requests tools, the harness executes them and appends tool results to the conversation.
7. The harness calls the model again with the updated conversation.
8. Workflow tools may also inspect or mutate the current workflow state between model calls. The active workflow tool surface is now `workflow_get_state` for inspection and `workflow_set` for mutation. `workflow_set` supports only a narrow patch-style shape: workflow activation, phase-result update, phase change, terminal workflow-status update, plus the supported combined cases `phase_result + phase` and `phase_result + workflow_status`.
9. This repeats until the model returns a normal assistant response with no more tool calls, or another existing runtime stop condition ends the turn.
10. The turn is finalized in SQLite with message, workflow, token, and turn-level runtime metadata.

The predefined guardrail layer is also where Themion now tells the model how to shape ordinary human-facing responses by default: prefer the smallest clear answer shape, prefer plain direct prose for simple answers, use 1–2 sentences when that fully answers the user, add bullets, headings, or tables only when that extra structure materially improves scanning or comparison, otherwise organize the reply into about 4±1 meaningful chunks, count each major section or comparison unit as part of that chunk budget, and expand toward about 7±2 chunks only when the user explicitly asks for a fuller explanation that does not fit the smaller structure. When the user mainly needs a recommendation or next action, the answer should lead with that answer first and keep supporting analysis secondary. These are readability heuristics rather than exact quotas, so correctness and user intent still win.

That same guardrail layer also tells the model to preserve user-useful information learned from tools in normal assistant chat text rather than relying only on raw tool results. That guidance is intentionally concise: the default preservation summary is 1–2 sentences, with longer explanation reserved for findings that are materially important or complex. Routine mechanical acknowledgements usually do not need separate narration.

Prompt replay now uses a narrowed budget-aware policy instead of relying only on a strict fixed-turn window. `themion-core` now prefers tokenizer-backed local token estimation through `tiktoken-rs` when the active model resolves through an exact upstream model mapping, otherwise falls back through a short explicit trusted tokenizer mapping for selected known aliases, and finally degrades to the rough `chars / 4` estimator when no tokenizer path is trusted. The replay policy keeps the active turn (`T0`) as the highest-priority replay unit, never replays turns older than `T-7`, degrades `T-1` through `T-5` into assistant-style pure-message replay when `T0` alone exceeds the normal 170K target, and omits prior allowed turns when `T0` alone exceeds the 250K spike ceiling or when older-turn inclusion within the `T-7` band would exceed that ceiling. Calibration, `CompactSummary`, and a broader compaction ladder are intentionally out of scope for this policy slice.

That same core prompt-analysis path now also powers the TUI-local `/context` command. `themion-core` constructs a structured prompt-context report describing prompt sections, tokenizer-backed or fallback token estimates, estimate mode, tokenizer path when available, turn replay modes, and omission boundaries, and `themion-cli` formats that report for transcript display. The visible `history turns:` listing is intentionally bounded to `T0` through `T-10` for readability even when the underlying structured report tracks older omitted turns. This keeps the user-facing inspection path aligned with the real next-round prompt assembly logic rather than relying on a separate TUI-only estimator.

## Codex profile-scoped login state

Codex login state is now resolved per saved profile rather than through one shared global `auth.json` blob for all `openai-codex` usage.

Current behavior:

- `/login codex <profile>` explicitly authenticates the named profile
- `/login codex` targets the current active profile when it already uses `openai-codex`; otherwise it falls back to the literal `codex` profile
- successful Codex login persists auth under the targeted profile and switches the live session to that profile for immediate use
- token refresh writeback for `CodexClient` persists back to that same active profile-scoped auth store
- when the active profile uses `openai-codex`, provider readiness now depends on whether that specific profile has auth available
- if no auth is available for the active Codex profile, runtime startup/build paths report a profile-specific recovery hint such as `run /login codex <profile>`
- legacy `~/.config/themion/auth.json` is treated only as a narrow migration source for obvious single-profile upgrades; once a profile-scoped auth file exists, that profile-scoped auth is authoritative

This keeps Codex aligned with Themion's existing profile-centric session/config model while preserving the same device-code login flow.

## Agent identity boundary

`themion-core::Agent` owns per-agent harness state such as session ID, project directory, workflow state, messages, and model/backend integration. `themion-cli` owns process-local descriptors such as `agent_id`, `label`, and `roles` that describe how a given core agent is used within one Themion process.

This keeps reusable harness behavior in core while allowing the CLI to publish process-level multi-agent status for Stylos. The first PRD-081 implementation slice extends that boundary: `themion-core` now exposes `local_agent_create` and `local_agent_delete` as tools, but actual roster mutation remains CLI-local because `themion-cli` owns the in-process `Vec<AgentHandle>` plus local agent construction and removal.

Each core `Agent` also receives a CLI-provided local role context derived from its runtime descriptor. The prompt includes a separate compact role-context section with the active `agent_id`, optional alias, resolved role list, a short known-role glossary, and detailed action guidance only for the active agent's own roles. Dynamically created agents with omitted or empty roles resolve to `executor`; they do not inherit `master` or `interactive` from the predefined agent.

## Agent coordination channel guidance

Themion now injects compact guidance that teaches agents to choose the lightest channel that is durable enough for the work:

1. answer directly for simple requests that do not need tracking
2. create a self-note when the current agent needs durable tracking for non-trivial or branching work
3. do not involve another local agent unless the user explicitly asks for delegation, parallel agent work, or another agent's help or review
4. after delegation is explicitly authorized, `master` may create or choose a local worker agent when extra capacity or role separation helps
5. use durable board notes for delegated work another agent must complete, resume, or report later
6. use `stylos_send_message` only for short volatile coordination, clarification, participant-facing state updates, urgent nudges, or final wrap-up with no durable result

Requests for depth, thoroughness, research, investigation, review, or large scope do not count as delegation permission by themselves. Role guidance helps choose how to delegate after authorization exists; it does not authorize delegation by itself. Themion keeps "create local agent" and "team member" wording rather than adopting "spawn agent" or a permanent subagent hierarchy.

Delegated board notes should state the task, expected output, constraints, ownership, and return path. If the result must be durable, the note should ask the worker to update the note result or create a done mention through the board workflow. Inbox messages are not a durable task queue and should not be the only record for work that needs completion tracking.

For authorized multi-agent activity, the guidance tells the coordinator to own authoritative state, use stable activity/turn/note identifiers, state participants and response channels, separate state updates from discussion, broadcast only meaningful state transitions, define completion/timeout/late-input rules up front, and end with a clear final outcome.

## CLI-local runtime domains

`themion-cli` now owns explicit Tokio runtime construction through a CLI-local runtime topology helper.

Current runtime domains:

- `tui` — one-worker multi-thread runtime for TUI event intake, tick scheduling, frame scheduling, and TUI-side bridge tasks
- `core` — multi-thread runtime for startup coordination, print-mode execution, and core harness orchestration paths
- `network` — multi-thread runtime for long-lived Stylos networking tasks
- `background` — multi-thread runtime domain for lower-priority maintenance work such as Project Memory semantic index generation, append-triggered pending unified-search follow-up work, and CLI/TUI-triggered semantic reindex jobs

Mode differences:

- TUI mode constructs `tui`, `core`, `network`, and `background` and runs through a shared CLI app-state/runtime plus `tui_runner`
- explicit `--headless` mode constructs the reduced non-TUI runtime set currently needed by that path, which is `core` and `network`, runs through the same shared CLI app-state/runtime plus `headless_runner`, and emits structured NDJSON lifecycle logs on stdout
- non-interactive prompt-argument mode also reuses that shared CLI app-state/runtime, but remains a one-shot stdout/stderr execution path rather than the long-running headless NDJSON-log mode

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

## Automatic chat-message unified-search registration and idle-only background embedding

New transcript writes now feed the generalized unified-search pipeline in two stages.

Current behavior:

- `agent_messages` remains the durable source-of-truth transcript store
- when `DbHandle::append_message(...)` persists a new indexable `chat_message`, `themion-core` now performs a lightweight best-effort follow-up registration into `unified_search_documents`
- that append-time registration reuses the same normalized unified-search identity as rebuilds and leaves the document in durable `embedding_state = "pending"` rather than generating embeddings inline
- chat-message append-time registration currently keeps the existing indexability rules used by generalized unified search: non-empty `user` or eligible `assistant` content is indexable, while `tool` rows and assistant rows carrying `tool_calls_json` are excluded from `chat_message` indexing
- chunk generation and embedding are deferred to runtime-owned background work rather than blocking transcript persistence
- `themion-cli` starts a background worker on the CLI-owned `background` runtime domain that observes the hub-owned `AppSnapshot` stream
- that worker drains pending `source_kind="chat_message"` unified-search documents only when all local agents are idle
- if any local agent is busy, the pending backlog remains durable and waits for a later all-idle window
- on successful background indexing, the pending row becomes `ready` and chunk rows are written
- if background indexing fails, the document keeps durable error visibility through `last_error` but remains in retryable `pending` state so a later idle-time background pass can try again automatically; manual rebuild remains the explicit repair/backfill path
- manual `unified_search_rebuild` remains the repair and historical backfill path; the automatic append-time path is for newly appended transcript data

This keeps the ownership split explicit:

- `themion-core` owns chat-message indexability, append-time pending registration, durable pending/failed state, and final chunk/embedding writes
- `themion-cli` owns only the scheduling decision of when background pending work may run
- the TUI does not own indexing policy or worker lifecycle

## Per-agent queued local user prompts while busy

Themion now uses a runtime-owned per-agent queue for normal local user prompts that target a busy local agent.

Current behavior:

- normal local user prompt submission still shows the user text in the live transcript immediately
- if the target local agent is idle, the turn still starts immediately
- if the target local agent is already busy, the prompt is queued on that same `AgentHandle` instead of trying to start a second immediate turn
- queued prompt ownership is strictly per local agent; one local agent does not drain or auto-start another local agent's queue
- queued prompts are not appended to core `Agent.messages` or persisted to session history until the owning agent actually consumes them
- while an active turn continues after tool work, `themion-core::Agent::run_loop_with_cancellation(...)` now calls a CLI-wired same-agent drain hook before the next continued model round and appends all currently queued prompts in FIFO order as ordinary `user` messages for that same turn
- when a full turn ends and the `Agent` object returns to `themion-cli`, runtime ready handling checks that same agent's queue and auto-starts exactly one next queued prompt as the next turn when any remain
- if more queued prompts remain after that post-turn launch, they stay queued for later continuation-drain points or later turns
- interrupting the active turn does not silently drop queued prompts; they remain owned by that same agent for later handling under the current policy
- queue admission, continuation drain wiring, and post-turn auto-continue are runtime-owned behavior in `themion-cli` and `themion-core`; `tui.rs` only renders the resulting busy/pending state

This queueing slice currently covers normal local user prompt submission only. Slash commands, shell commands, login commands, indexing commands, and remote inbox or board-note intake paths keep their existing admission behavior unless another PRD changes them.

## Local agent membership tools

Themion now exposes two local team-membership tools through the normal tool surface:

- `local_agent_create`
- `local_agent_delete`

Current behavior:

- `local_agent_create` accepts optional `agent_id`, optional `label`, and optional `roles`
- when `agent_id` is omitted, the CLI runtime allocates the next free `smith-N` worker id in the current local roster
- when `roles` is omitted or empty for an additional created agent, the CLI runtime assigns `executor`
- explicit valid role lists are preserved without adding `executor` implicitly
- `master` remains reserved for the predefined leader and cannot be recreated through the tool
- the current implementation rejects duplicate ids, another `master`, and another `interactive` role
- `local_agent_delete` accepts a target `agent_id` for a non-leader local agent
- deleting `master` is rejected
- deleting while the local runtime is busy is currently rejected explicitly rather than deferred
- successful create/delete operations mutate the active in-memory roster immediately, so local targeting and runtime inspection stay aligned with the changed roster

This is still a local-runtime management slice rather than a separate multi-process scheduler, but the current TUI/runtime path now supports overlapping active turns across multiple local agents in one process. Turn admission is checked per local agent handle, explicit target-specific busy outcomes are preserved, and any remaining process-level busy field is only an aggregate observability summary rather than the scheduler's source of truth.

The TUI transcript layer now also carries explicit local-agent attribution for visible runtime entries. When the CLI knows which local `agent_id` produced or owned a transcript item, it stores that attribution in the TUI entry model and renders a compact highlighted `[agent_id]` prefix using a small deterministic roster-order color palette. In the current implementation, assistant replies, tool lines, status lines, remote intake/event lines, and turn-complete lines may be agent-tagged; ordinary local user-input lines remain untagged because they represent the shared operator. For status and remote-event lines that have no specific local owner, the TUI now uses a separate structured non-agent source classification and renders a compact labeled prefix such as `BOARD`, `STYLOS`, `RUNTIME`, or `WATCHDOG` with stable category color instead of reclassifying those lines as agent-owned.

## Stylos remote-request bridge

Shared CLI bootstrap now lives in `crates/themion-cli/src/app_state.rs`, which resolves the project directory, opens the local DB, inserts the session row, builds `Session`, and exposes shared agent-construction helpers used by TUI mode, explicit `--headless` mode, and non-interactive prompt execution. `main.rs` stays thin and only selects which runner to invoke.

Stylos request handling stays CLI-local even when it ultimately causes an agent turn.

In the current implementation:

- Stylos queryables are registered in `crates/themion-cli/src/stylos.rs` and their long-lived serving/publishing/subscription tasks now run on the CLI-owned `network` runtime domain
- query handlers read the current exported process snapshot from a snapshot provider owned by the Stylos runtime path rather than by `tui.rs`
- accepted volatile message and durable `notes/request` queries are converted into inbox items or persisted note records and sent over an in-process/local-runtime path
- CLI-local incoming-prompt admission and rejection policy belongs in `crates/themion-cli/src/app_runtime.rs`, not in `tui.rs`
- CLI-local board-note coordination for pending note injection and note-completion follow-up belongs in `crates/themion-cli/src/board_runtime.rs`, not in `tui.rs`
- TUI should not be a Stylos/watchdog policy endpoint; it should receive runtime/app-state outcomes and render them, while human-originated input flows the other direction as intents
- accepted remote work should enter the same runtime-owned local turn path used for normal human input, with TUI only observing/rendering that path rather than controlling it

This means Stylos does not bypass the harness loop, call providers directly, or move history/tool execution into the transport layer. It only injects new work into the existing local input path.
For durable board notes in TUI mode, `board_runtime.rs` is now the CLI-local coordination boundary for selecting the next pending note, claiming one note locally before handoff, mutating injected/completion state only after successful handoff, releasing local claims when a selected target loses the handoff race, and resolving post-turn follow-up into typed actions that the TUI displays or submits. This keeps the watchdog scheduler independent while preventing duplicate in-process injection of the same note across overlapping local-agent activity.

Sender-side Stylos transport event derivation is also no longer a TUI transcript-inference path. The current implementation derives outbound message and board-note transport events through explicit helper logic in `crates/themion-cli/src/stylos.rs`, and the TUI only renders the returned event line when present.

## Snapshot-driven request decisions

The query layer makes best-effort decisions from the current local snapshot rather than from a separate scheduler.

That includes:

- `free` discovery using exported `activity_status`
- `stylos_send_message` acceptance requiring the requested agent to be present; valid messages queue in the receiver's volatile inbox unless that target inbox is full
- message inbox delivery happens through the runtime/watchdog drain path; the send-message query handler does not inject peer-message prompts directly
- Zenoh-level `stylos_query_nodes` using `session.info()` from the active local Stylos session rather than Themion mesh queryables

Because those checks are snapshot-based, they can race with local activity changes. The runtime reports the chosen agent honestly and leaves accepted peer messages in the inbox until the watchdog/runtime drain path can hand them to the local agent.

## Removed Stylos task request system

PRD-111 removed the Stylos task request/status/result API. Themion no longer exposes `stylos_request_task`, `stylos_query_task_status`, or `stylos_query_task_result`, and new builds no longer register `query/tasks/request`, `query/tasks/status`, or `query/tasks/result` queryables. There is no compatibility responder for those old topics.

Durable delegated work should use board notes. Short volatile coordination should use `stylos_send_message`. Generic Tokio/runtime tasks are unrelated to this removed Stylos API and remain part of the runtime model.


## Remote execution targeting in the current slice

The current Stylos bridge validates requested `agent_id` values against the exported snapshot, records the requested agent in the remote request payload, and routes execution to that matching local agent when present.

That means:

- strict local `agent_id` execution targeting has landed for the current process-local agent set, including dynamically created non-leader local agents in the active runtime
- the query layer still relies on snapshot-based selection and a CLI-local in-process bridge rather than a durable scheduler
- this preserves the current harness architecture while still making the query and task surface useful for discovery, request submission, and best-effort status lookup

## Sender-aware message prompt injection

Stylos `message` delivery resolves sender identity automatically and carries exact instance identifiers through the CLI-local bridge:

- sender-side local instance `from` resolved automatically as exact `<hostname>:<pid>`
- mandatory target `to` in exact `<hostname>:<pid>` form
- optional `to_agent_id` on the request input, defaulting to `master`
- optional `request_id`

When a message request is accepted, the CLI does not inject the raw message directly. Instead it wraps the message in a peer-message prompt that tells the receiving agent:

- who sent the message
- which local agent received it
- that it should reply only when a materially useful response is needed
- that `***QRU***` means no further reply is normally needed
- that empty acknowledgements and thank-you-only replies should be avoided

This keeps sender identity and reply guidance visible to the model in the harness prompt path rather than hidden only in transport metadata.

When the local agent invokes `stylos_send_message` and the request is accepted, the sender-side chat-panel event line is now derived by explicit helper logic in `crates/themion-cli/src/stylos.rs` rather than by TUI transcript backtracking:

- `Stylos message to=<hostname>:<pid> from=<hostname>:<pid>`

This sender-side log remains distinct from generic tool-call text and is intended to make outbound peer messaging visible in the chat transcript.

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
- `fs_patch` accepts unified-diff patch text plus optional `reason` and is the preferred tool for small edits to existing UTF-8 text files
- `fs_patch` supports raw unified diffs and markdown ```diff fences, validates workspace-relative header paths, rejects create/delete/rename/conflict-marker input, and applies the whole request atomically in strict exact mode
- `fs_patch` returns compact `file_patch` results with `ok`, `changed_paths`, `rejected_paths`, and a short `message`
- `fs_write_file` accepts optional `mode` and defaults to `base64`, decoding bytes before writing in that default mode
- `fs_write_file` still supports direct text writes through `mode=raw` and remains the tool for file creation or intentional whole-file replacement
- `shell_run_command` accepts optional `result_limit` and `timeout_ms`, defaulting to `16384` bytes and `300000` ms
- `shell_run_command` truncates oversized returned output with an explicit truncation notice
- `shell_run_command` returns a clear timeout result when the command exceeds the configured timeout

These defaults make binary-safe file transfer and bounded shell usage the normal path rather than a caller-side convention.

## Project Memory knowledge-base tools

Themion exposes one `memory_*` tool family backed by SQLite in `themion-core`. The tools build Project Memory: an intentional long-term durable knowledge base, not a transcript log or task board. Concepts, components, files, tasks, decisions, facts, observations, troubleshooting records, people, and occasional narrative memory records are all `memory_nodes`, while `memory_edges` records typed directed links between any two nodes. Each node stores a `project_dir` memory context. Hashtags are flat normalized labels in `memory_node_hashtags`; they replace a separate memory scope concept for this feature.

For model-facing tool calls, the canonical current-project form is still to omit `project_dir`. Explicit absolute paths remain the canonical way to name a specific project, and exact `project_dir="[GLOBAL]"` remains the explicit Global Knowledge selector where supported. As a compatibility fallback added in PRD-094, the targeted project-scoped memory tools also treat exact `project_dir="."` as the current project scope at the tool-resolution layer, but `"."` is not part of the advertised schema contract.
