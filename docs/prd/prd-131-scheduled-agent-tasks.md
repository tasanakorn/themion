# PRD-131: Scheduled Agent Tasks

- **Status:** Implemented
- **Version:** v0.80.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-23

## Summary

- Add a schedule feature so an AI model can create and manage future agent work.
- Add compact model-facing tools for creating, reading, updating, deleting, and listing scheduled tasks.
- Add a runtime-owned schedule dispatcher, similar in spirit to the watchdog, that wakes at due times and injects task work into local agents.
- Default scheduled work goes to a dedicated scheduler agent that is created explicitly by schedule-runtime policy, not by hidden or model-initiated agent creation.
- Scheduled work may also target a specific agent id and define what happens when that target is busy.
- Support one-shot schedules at a specified time and repeating schedules expressed as cron jobs.
- Use UTC for stored times and cron evaluation in the first implementation.

## Goals

- Let the model schedule future work through a small, durable tool surface.
- Persist scheduled tasks so they survive process restart when the default Themion database is available.
- Dispatch due tasks through runtime-owned scheduling logic, not TUI-owned logic.
- Allow each scheduled task to route to either a specific local `agent_id` or the dedicated scheduler agent.
- Make busy-target behavior explicit per schedule or via config defaults.
- Support one-shot tasks and repeating cron-style tasks.
- Provide enough status and history to know whether a task was dispatched, delayed, delegated, skipped, or failed.

## Non-goals

- Do not add a general OS-level cron service or depend on system cron.
- Do not make the TUI own schedule timing, agent selection, or retry policy.
- Do not make scheduled tasks a replacement for board notes, watchdog follow-up, or manual delegation.
- Do not require remote Stylos delivery for local scheduled work in the first slice.
- Do not silently reroute explicitly targeted work unless the task policy allows it.
- Do not add complex calendar UI in the first slice.
- Do not implement full missed-cron backlog replay in the first slice.

## Background & Motivation

Themion already has runtime concepts that are close to scheduled work:

- board notes store durable work
- the watchdog wakes in the background and injects eligible pending note work
- local agents can be addressed by `agent_id`
- additional local agents can be created through runtime-owned tools and policy

Missing behavior:

- the model cannot ask Themion to run a task later at a specific time
- there is no durable recurrence rule for recurring agent work
- there is no clear policy for what to do when a scheduled task targets a busy agent
- recurring automation must be simulated manually through notes or prompts

A dedicated schedule feature gives the model a direct, durable way to plan future work while keeping dispatch policy in the runtime layer.

## Design

### 1. Add scheduled task records

A scheduled task is a durable runtime object.

Required fields:

- `schedule_id`: stable UUID string
- `title`: human-readable summary
- `body`: prompt text to inject when due
- `enabled`: boolean
- `schedule_kind`: `one_shot` or `cron`
- `due_at_ms`: required for `one_shot`, stored as UTC epoch milliseconds
- `cron_expr`: required for `cron`, stored unchanged
- `timezone`: must be `UTC` in the first implementation
- `next_due_at_ms`: next occurrence time, stored as UTC epoch milliseconds
- `target_agent_id`: optional concrete local agent id; `NULL` means use the dedicated scheduler agent
- `owner_instance`: concrete local instance id that created and owns dispatch for the schedule
- `busy_policy`: `delay`, `delegate_to_free_agent`, `create_new_agent`, or `miss`
- `retry_after_ms`: optional UTC epoch milliseconds used by delayed occurrences
- `last_due_at_ms`: due time of the last handled occurrence, stored as UTC epoch milliseconds
- `last_run_at_ms`: time the last occurrence was injected or skipped, stored as UTC epoch milliseconds
- `last_status`: `never_run`, `claimed`, `dispatched`, `delayed`, `delegated`, `missed`, or `failed`
- `last_status_detail`: optional short text for a clear failure or routing reason
- `last_handling_agent_id`: concrete local agent that handled the last dispatched occurrence when available
- `created_at_ms` and `updated_at_ms`: UTC epoch milliseconds

The task body is the prompt that the selected local agent receives. It should include enough context for the agent to run without relying on the schedule tool caller still being active.

The database should store enough state to recompute the next due time after restart and to avoid duplicate dispatch for the same due occurrence.

Use one `scheduled_tasks` table for the first implementation. A separate occurrence-history table is not required for v0.80.0; the durable last-status fields are enough for implementation readiness. A later PRD may add full per-occurrence audit history.

The scheduler should claim a due row atomically by changing an eligible row to `last_status = 'claimed'` and recording enough claim owner data to detect in-flight work:

- `claimed_by_instance`
- `claimed_by_agent_id`
- `claimed_due_at_ms`
- `claimed_at_ms`

The atomic claim must update exactly one row only if the schedule is still enabled, still due, and not already claimed. If zero rows are updated, the dispatcher must treat the race as lost and not inject the task.

### 2. Add one canonical schedule tool family

The model-facing tool surface should be compact and explicit.

Required canonical tools:

- `schedule_create`: create one scheduled task
- `schedule_read`: read one scheduled task by `schedule_id`
- `schedule_list`: list scheduled tasks, with filters for enabled state, target, and due status
- `schedule_update`: patch mutable fields such as title, body, schedule rule, target, busy policy, and enabled state
- `schedule_delete`: delete one scheduled task by `schedule_id`

Tool contracts should prefer one canonical parameter shape for schedule rules:

- one-shot schedule: `{ "kind": "one_shot", "due_at_ms": <UTC epoch milliseconds> }`
- cron schedule: `{ "kind": "cron", "cron_expr": "...", "timezone": "UTC" }`

For this PRD, the tool API only accepts UTC epoch milliseconds for one-shot times and `timezone="UTC"` for cron schedules. Human-language date parsing is out of scope.

Default values:

- `enabled`: `true`
- `target_agent_id`: omitted, meaning the dedicated scheduler agent
- `busy_policy`: config default when `target_agent_id` is set, otherwise `delay`

The tool descriptions should state exact timestamp units. Machine-consumed timestamps should use milliseconds unless a documented consumer requires another unit.

### 3. Keep the dispatcher runtime-owned

The schedule dispatcher should follow the watchdog ownership model.

Required behavior:

- run as a runtime-owned background task
- use a bounded poll interval of 5 seconds by default
- sleep until the next known due task when it is sooner than the poll interval, but never hot-loop
- inspect current runtime-owned agent status before dispatch
- claim one due occurrence before injection so concurrent dispatcher checks cannot run it twice
- inject scheduled work into the selected local agent turn path
- record dispatch outcomes durably
- compute the next due time for repeating cron tasks after each handled occurrence
- mark one-shot tasks complete after successful dispatch handling, or missed if policy says to miss
- clear claim fields after the occurrence reaches `dispatched`, `delayed`, `missed`, or `failed`

The dispatcher must not live in `tui.rs` or depend on presentation state.

Dispatch prompt shape:

- prefix schedule-originated prompts with a compact runtime header that includes `schedule_id`, `title`, and `due_at_ms`
- then include the stored task body as the user-visible instruction for the handling agent
- transcript/debug events should label the source as `schedule`, not as a human prompt or board-note watchdog prompt

### 4. Default to a dedicated scheduler agent

If a scheduled task does not specify a concrete target agent, it should use a dedicated scheduler agent.

Required behavior:

- the default target mode is the dedicated scheduler agent
- the scheduler agent has the stable reserved identity `scheduler`
- the runtime may create the dedicated scheduler agent automatically only through explicit schedule-runtime policy
- this creation is not model-requested delegation and must not be described as hidden agent spawning
- if the scheduler agent already exists, reuse it
- if the scheduler agent cannot be created or reused, the dispatch outcome must fail clearly instead of silently using `master`

The dedicated scheduler agent exists to keep automated scheduled work separate from the interactive `master` agent by default.

This PRD reserves the dedicated scheduler identity for the schedule feature. The normal local-agent creation path should reject user-created agents with `agent_id="scheduler"` unless the request comes from the schedule-runtime creation path.

### 5. Support specific target agents and busy policies

A scheduled task may specify a concrete `agent_id`.

If that target is busy when the task is due, the task must follow an explicit policy.

Required busy policies:

- `delay`: keep the occurrence pending and retry later
- `delegate_to_free_agent`: run on another eligible idle local agent if one exists
- `create_new_agent`: create a new local agent through the normal local-agent creation path for this occurrence, then run the task there
- `miss`: mark this occurrence missed and do not run it

Required behavior:

- the default policy for specific targets should come from config
- each schedule may override the config default
- `delay` must avoid hot-looping and should record the next retry time
- `delegate_to_free_agent` must record the actual agent that ran the occurrence
- `create_new_agent` must use the normal local-agent creation path and respect agent-id safety rules; the new agent may use the allocator default id
- `miss` must record that the occurrence was skipped because the target was busy
- explicit target semantics stay honest: rerouting only happens when the policy says so

Config defaults:

- add `schedule.default_busy_policy`, default `delay`
- add `schedule.dispatch_poll_interval_ms`, default `5000`
- add `schedule.delay_retry_ms`, default `60000`

`delegate_to_free_agent` should choose the first idle eligible local agent in stable roster order, excluding the busy target. It must not create an agent.

### 6. Support one-shot and cron schedules

The first schedule kinds are one-shot time and repeating cron.

Required behavior:

- one-shot tasks run once at or after the specified due time
- cron tasks compute the next due occurrence after each dispatch or miss
- cron parsing errors must fail at create/update time with clear validation messages
- timezone behavior must be explicit; only `UTC` is accepted in the first implementation
- missed occurrences after downtime should use a documented catch-up policy
- schedule calculations should store machine times in milliseconds and store the original schedule rule unchanged
- the dispatcher should not run the same cron occurrence more than once after restart

Cron expression rule:

- use a standard five-field cron expression: minute, hour, day-of-month, month, day-of-week
- evaluate the expression in UTC
- if a Rust cron parser dependency is added, justify it in the implementation notes and keep it scoped to the crate that needs it
- if no new dependency is added, implement only the documented five-field subset and test validation failures clearly

Initial catch-up policy:

- one-shot tasks that become overdue while Themion is stopped should run once after startup if still enabled
- cron tasks should run only the latest due occurrence by default, then compute the next future occurrence
- a later PRD may add full backlog replay if users need it

### 7. Observability and task history

Users and models need to understand what happened.

Required behavior:

- inspection/status surfaces should expose whether the dispatcher is running and when the next task is due
- each scheduled task should expose last outcome, last run time, next due time, and last handling agent
- transcript or debug events should label schedule-originated injections clearly
- failures such as invalid target, busy miss, create-agent failure, or prompt-injection failure should be durable enough for later inspection

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-131-scheduled-agent-tasks.md` | Add the durable PRD for scheduled agent tasks. |
| `crates/themion-core/src/tools.rs` | Add `schedule_create`, `schedule_read`, `schedule_list`, `schedule_update`, and `schedule_delete` schemas and compact descriptions. |
| `crates/themion-core/src/db.rs` | Add `scheduled_tasks`, schema migration, claim fields, and helpers for due lookup, atomic claim, claim clear, next-due update, CRUD, and list filtering. |
| `crates/themion-cli/src/app_runtime.rs` and runtime helpers | Add runtime-owned schedule dispatcher state, due-task polling, claim/release handling, target selection, and injection. |
| `crates/themion-cli/src/app_state.rs` | Start and stop the dispatcher with the shared runtime and publish schedule status snapshots. |
| local-agent management path | Reserve `scheduler` from normal user-created agents; support explicit runtime-policy creation/reuse of the dedicated scheduler agent and optional per-occurrence worker creation. |
| config loading docs and code | Add `schedule.default_busy_policy`, `schedule.dispatch_poll_interval_ms`, and `schedule.delay_retry_ms`. |
| `docs/engine-runtime.md` and related docs | Document scheduled task behavior, target policies, timestamp units, and scheduler-agent defaults. |
| `docs/README.md` | Add this PRD entry and later update implementation status. |

## Edge Cases

- Themion starts after a one-shot due time → verify: the task runs once if still enabled.
- Themion starts after several cron occurrences were missed → verify: only the latest due occurrence is handled by default, then the next future occurrence is stored.
- dispatcher sees the same due task twice during a race → verify: durable claim prevents duplicate injection.
- task targets an unknown agent id → verify: dispatch fails clearly or follows only a documented fallback if policy allows it.
- task targets a busy agent with `delay` → verify: task stays pending and retries later without hot-looping.
- task targets a busy agent with `delegate_to_free_agent` → verify: another idle eligible local agent runs it and the actual handler is recorded.
- task targets a busy agent with `create_new_agent` → verify: a new local agent is created through normal validation and receives the occurrence.
- task targets a busy agent with `miss` → verify: the occurrence is marked missed and does not run later.
- default scheduler agent cannot be created → verify: dispatch fails clearly and does not silently run on `master`.
- cron expression is invalid → verify: create/update fails before storing an unusable schedule.
- a schedule is disabled while an occurrence is pending → verify: no new dispatch starts after disable takes effect.

## Migration

This feature needs a database migration for scheduled task storage.

Required migration behavior:

- existing databases open normally and gain empty schedule tables
- no existing board notes, watchdog state, sessions, or local-agent records are rewritten
- disabling the feature or downgrading should leave schedule tables unused but should not block normal database open unless the repository's migration policy requires otherwise

This is a new user-visible feature. The implementation target is `v0.80.0`; implementation follow-through should check whether crate versions and `Cargo.lock` need updates before marking the PRD implemented.

## Testing

- create a one-shot task due soon → verify: dispatcher injects it once into the dedicated scheduler agent and records success.
- create a disabled one-shot task due soon → verify: dispatcher does not run it.
- create a cron task with a short test cadence → verify: next due time advances after each handled occurrence.
- create a cron task with `timezone="UTC"` → verify: create succeeds and stores UTC next due time.
- create a cron task with another timezone → verify: create fails clearly.
- restart with an overdue one-shot task → verify: it runs once after startup.
- restart with an overdue cron task → verify: only the latest due occurrence is handled by default.
- target a busy specific agent with each busy policy → verify: `delay`, `delegate_to_free_agent`, `create_new_agent`, and `miss` outcomes match policy.
- race two dispatcher checks against the same due task → verify: only one claim succeeds and only one prompt is injected.
- call each schedule tool with invalid ids, invalid timestamps, and invalid cron rules → verify: clear validation errors.
- run `cargo check -p themion-core` → verify: schedule tool and DB code compile with default features.
- run `cargo check -p themion-cli` → verify: dispatcher and config code compile with default features.
- run `cargo check -p themion-core --all-features` and `cargo check -p themion-cli --all-features` → verify: feature combinations compile.

## Implementation checklist

- [x] Add schedule storage and migrations.
- [x] Add schedule create/read/list/update/delete tool support.
- [x] Add runtime-owned dispatcher loop and due occurrence claim logic.
- [x] Add dedicated scheduler-agent creation/reuse policy.
- [x] Add specific-target busy policies and config defaults.
- [x] Add UTC one-shot support and a first five-field cron implementation.
- [x] Add schedule dispatch observability through runtime transcript status events.
- [x] Update runtime/config docs and run default and all-feature build validation.
- [x] Check `Cargo.lock` follow-through; the version bump updated the workspace package versions in `Cargo.lock`.
- [x] Add focused automated tests for schedule tools and DB claims.
- [x] Complete release-version bump follow-through for the `v0.80.0` feature target.

## Implementation Notes

The first implementation slice has landed in the working tree.

Implemented behavior:

- `scheduled_tasks` durable storage and migration columns in `themion-core`.
- `schedule_create`, `schedule_read`, `schedule_list`, `schedule_update`, and `schedule_delete` tool definitions and handlers.
- UTC one-shot schedules and a minimal UTC five-field cron evaluator that accepts `*`, `*/N` intervals, or numeric fields.
- runtime-owned schedule ticks in `themion-cli`, due-row claims, and dispatch to local agent turns.
- schedule `owner_instance` binding so only the creating local instance (`host:pid` or `local:pid`) can pick up or claim a due schedule; legacy unowned rows bind on first successful claim.
- default `scheduler` agent routing within the owning instance, with runtime-only creation and normal user creation blocked for `agent_id="scheduler"`.
- specific-target busy handling for `delay`, `delegate_to_free_agent`, `create_new_agent`, and `miss`.
- config defaults for `schedule.default_busy_policy`, `schedule.dispatch_poll_interval_ms`, and `schedule.delay_retry_ms`.
- runtime docs for schedule behavior in `docs/engine-runtime.md`.

Validation completed:

- `cargo check -p themion-core`
- `cargo check -p themion-cli`
- `cargo check -p themion-core --all-features`
- `cargo check -p themion-cli --all-features`
- `cargo test -p themion-core schedule`

Known follow-up:

- add deeper dispatcher-policy tests if a future runtime test harness makes direct `App` scheduling branches cheap to exercise
