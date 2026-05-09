# PRD-111: Remove the Stylos Task Request System

- **Status:** Implemented
- **Version:** v0.69.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-09

## Summary

- Remove only the Stylos task request/status/result API because it overlaps with durable board notes and volatile inbox messages.
- Keep board notes as the durable delegated-work path.
- Keep `stylos_send_message` as the lightweight volatile coordination path.
- Remove the process-local `TaskRegistry` and its direct lifecycle hooks only when they are used solely by the removed Stylos task API.
- Treat this as a breaking cleanup because public model-facing tools and Stylos query topics disappear.

## Goals

- Reduce coordination-channel overlap for models and users.
- Make the channel model easier to explain: board notes track work; inbox messages coordinate; discovery/status tools find peers.
- Remove `stylos_request_task`, `stylos_query_task_status`, and `stylos_query_task_result` from the model-facing tool surface.
- Remove the `tasks/request`, `tasks/status`, and `tasks/result` Stylos queryables.
- Remove task lifecycle state that is only process-local, non-durable, and tied to those queryables.
- Update prompt guidance and docs so agents do not choose the removed task path.

## Non-goals

- Do not remove board notes, done mentions, board-note routing, board-note result text, or board-note workflow behavior.
- Do not remove `stylos_send_message`, the volatile inbox, or message queue/drain behavior.
- Do not remove Stylos discovery/status tools such as alive/free/git/status.
- Do not remove local-agent creation/deletion, agent role metadata, or remote agent discovery.
- Do not remove generic Rust async tasks, Tokio runtime tasks, background workers, watchdog tasks, or unrelated code just because it uses the word `task`.
- Do not remove workflow state, workflow tools, Project Memory task-type nodes, or documentation that uses "task" in the ordinary English sense.
- Do not add a replacement job queue in this PRD.
- Do not make inbox messages durable as a substitute for task requests.
- Do not move coordination policy into `tui.rs`.

## Background & Motivation

Themion currently has three work-like remote coordination paths:

- board notes for durable work requests and result text
- inbox messages for short volatile coordination
- Stylos task requests for a remote task id, status lookup, and result polling

The task system overlaps with board notes. It also has weaker durability than board notes: `TaskRegistry` is process-local, in-memory, and lost on restart. This makes it a poor fit for work that must be completed or resumed later. The status/result polling shape looks more reliable than it is.

Recent guidance in PRD-110 tried to separate these channels, but the task path still adds another choice for models. Removing it makes the product simpler. Durable work goes through board notes. Short coordination goes through inbox messages. If a caller needs a durable result, the board note result field or done mention is the source of truth.

## Design

### Scope boundary

This PRD removes a narrow public API family, not every concept named "task".

In scope:

- model-facing tool names exactly `stylos_request_task`, `stylos_query_task_status`, and `stylos_query_task_result`
- Stylos query topic suffixes exactly `query/tasks/request`, `query/tasks/status`, and `query/tasks/result`
- request/reply structs, registry entries, bridge fields, and tests whose only purpose is those tools/topics
- prompt/docs guidance that tells agents to use Stylos task requests

Out of scope and must be preserved:

- board note tools and board runtime behavior
- inbox message tools and inbox runtime behavior
- Stylos discovery and status queryables
- generic runtime tasks, Tokio task spawning, background maintenance tasks, watchdog tasks, and web/server tasks
- human-facing wording such as "task" when it means ordinary work rather than the removed Stylos task API
- historical PRDs and archive docs, except optional cross-reference notes after implementation

Implementation should use exact-name searches before deletion. Broad deletion patterns such as removing every `task` symbol are not acceptable.

### Remove public task tools

Remove these model-facing tools from `themion-core`:

- `stylos_request_task`
- `stylos_query_task_status`
- `stylos_query_task_result`

Required behavior:

- the tools must not appear in normal tool schemas
- `call_tool` must no longer route these names as active Stylos bridge tools
- prompt guidance must not tell agents to use Stylos task requests
- user-facing tool documentation must point to board notes for durable delegated work

If an old model or caller asks for a task tool after the removal, the normal unknown-tool behavior is required. Do not keep hidden aliases, compatibility tool names, compatibility dispatch branches, or one-release shims.

### Remove Stylos task queryables

Remove these receiver-side query topics from `themion-cli` Stylos wiring:

```text
stylos/<realm>/themion/instances/<instance>/query/tasks/request
stylos/<realm>/themion/instances/<instance>/query/tasks/status
stylos/<realm>/themion/instances/<instance>/query/tasks/result
```

Required behavior:

- do not register task queryables for new builds
- do not keep compatibility responders for old `query/tasks/*` topics
- remove sender-side bridge handling for those topics
- remove task request/reply payload types that have no remaining use
- keep discovery/status/message/note queryables unchanged
- keep task-topic removal out of TUI presentation code except for deleting obsolete display labels if exact-name references exist

### Remove only direct task lifecycle tracking

Remove the in-memory task registry and update hooks only where they are direct support for task status/result polling.

Expected direct removals include:

- `TaskRegistry`
- `TaskRequestPayload`
- `TaskRequestReply`
- `TaskLookupRequest`
- `TaskResultRequest`
- `TaskLookupReply`
- `publish_stylos_task_running`
- `publish_stylos_task_completed`
- `publish_stylos_task_failed`
- `task_id` bridge fields that exist only to update the removed registry

Before deleting any field or function not listed here, check its call sites. If a value also supports board notes, inbox messages, runtime status, transcript display, local-agent routing, or ordinary turn execution, preserve it and remove only the task-specific branch.

The local runtime should still report ordinary turn outcomes through existing transcript/runtime events. Board-note handoff and result handling must remain intact.

### Keep board notes as the durable replacement

Board notes become the only model-facing remote work-tracking primitive.

Required behavior:

- delegated work that needs completion tracking should use `board_create_note`
- the board note should state the expected result and return path
- result text and done mentions remain the durable completion mechanism
- docs should explain that removed task polling is intentionally replaced by board-note state/result inspection

This does not require a new board feature. Existing board-note list/read/move/update-result tools are enough for the removal slice.

**Alternative considered:** keep task requests as a thin alias that creates board notes. Rejected for this PRD because it preserves two model-facing ways to ask for the same durable work and keeps the channel-choice ambiguity.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Remove exact task request/status/result tool definitions and exact-name call routing only. Preserve all other Stylos, board, memory, workflow, filesystem, and shell tools. |
| `crates/themion-core/src/agent.rs` | Remove prompt guidance that recommends Stylos task requests. Preserve board-note, inbox-message, local-agent, and multi-agent coordination guidance. |
| `crates/themion-cli/src/stylos.rs` | Remove exact task query topics, request bridge handling, payload/reply structs, `TaskRegistry`, and tests that cover task lifecycle polling. Preserve discovery, status, message, note, and node-query behavior. |
| `crates/themion-cli/src/app_runtime.rs` | Remove only task-registry publish/update helpers. Preserve local-agent create/delete, incoming prompt admission, watchdog state, and board runtime integration. |
| `crates/themion-cli/src/app_state.rs` | Remove only `task_id` lifecycle update calls and task-specific bridge fields. Preserve normal agent-turn result handling, board-note completion follow-up, inbox draining, and transcript/runtime events. |
| `docs/engine-runtime.md` | Replace task-system descriptions with the simplified board-note/inbox channel model. Preserve unrelated mentions of generic runtime tasks. |
| `docs/architecture.md` | Update coordination-channel guidance to remove Stylos task requests. Preserve process/runtime task hierarchy docs. |
| `docs/README.md` | List this PRD and update status/version/scope when implementation lands. |
| Historical PRDs | Leave implemented historical PRDs unchanged unless adding a short cross-reference note is useful after implementation. |

## Edge Cases

- old model calls `stylos_request_task` → verify: the tool is unavailable through the normal unknown-tool path; no compatibility alias handles it.
- old peer sends to `query/tasks/request` → verify: new builds do not register the topic and do not return a compatibility removal response; the peer must use board notes or messages.
- source search finds generic `task` references → verify: references are not removed unless they are exact Stylos task API support.
- remote work needs durable tracking → verify: agents create a board note with expected output and return path.
- caller wants progress/result polling → verify: caller can inspect board note column/result instead of task status/result.
- active local turn completes after task code removal → verify: no missing task-registry update path affects ordinary transcript, board, or inbox behavior.
- Stylos feature disabled build → verify: removing task code does not create unguarded references to feature-gated types.

## Implementation Notes

Implemented in v0.69.0. The landed slice removes `stylos_request_task`, `stylos_query_task_status`, and `stylos_query_task_result` from the model-facing tool surface; removes `query/tasks/request`, `query/tasks/status`, and `query/tasks/result` registration with no compatibility responders; deletes the in-memory `TaskRegistry` and direct lifecycle hooks; removes `task_id` bridge plumbing; and updates prompt/runtime docs so durable delegated work uses board notes while volatile coordination uses inbox messages. Generic Tokio/runtime tasks, watchdog tasks, board notes, inbox messages, discovery/status queryables, workflows, and Project Memory task wording remain in scope and were preserved.


## Migration

This is a breaking protocol and tool-surface cleanup. There is no backward-compatible transition window.

Callers must migrate:

- from `stylos_request_task` to `board_create_note` for work that needs tracking
- from `stylos_query_task_status` to `board_list_notes` or `board_read_note`
- from `stylos_query_task_result` to `board_read_note` result text or done mentions
- from quick task-like nudges to `stylos_send_message` only when no durable work record is needed

No database migration is required because task lifecycle records are in memory only. Existing board notes and inbox messages are unaffected.

The version target is `v0.69.0`. This is a minor-release cleanup by product decision, even though the removed task tools and topics are intentionally not backward-compatible.

## Testing

- inspect generated tool schemas → verify: `stylos_request_task`, `stylos_query_task_status`, and `stylos_query_task_result` are absent, while board, message, discovery/status, workflow, memory, filesystem, and shell tools remain as expected.
- search source for exact task tool names and `query/tasks` → verify: remaining references are historical PRDs, archive docs, or explicit migration notes.
- search touched source for broad `task` deletions → verify: generic Tokio/background/watchdog/runtime task references remain intact.
- run `cargo check -p themion-core` → verify: default core build compiles without task tools.
- run `cargo check -p themion-core --all-features` → verify: all-feature core build compiles.
- run `cargo check -p themion-cli` → verify: default CLI build compiles.
- run `cargo check -p themion-cli --features stylos` → verify: Stylos-enabled CLI build compiles without task queryables.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build compiles.
- create a board note for delegated work → verify: durable result and return-path behavior still works.
- send a `stylos_send_message` to a known target → verify: volatile inbox behavior still works.
- query Stylos alive/free/git/status when available → verify: non-task Stylos queryables still work.

## Implementation checklist

- [x] run exact-name dependency searches for `stylos_request_task`, `stylos_query_task_status`, `stylos_query_task_result`, `query/tasks`, `TaskRegistry`, and `task_id`
- [x] classify each hit as direct Stylos task API support, generic runtime task behavior, historical docs, or unrelated wording
- [x] remove task tools from the core tool schema
- [x] remove task tool bridge routing
- [x] remove task request/status/result queryable registration with no compatibility responders
- [x] remove task payload/reply structs and registry code
- [x] remove app-runtime/app-state task lifecycle update hooks without changing ordinary turn, board, or inbox behavior
- [x] update prompt guidance to remove task requests from the channel-choice ladder
- [x] update runtime and architecture docs without deleting generic process/runtime task descriptions
- [x] update docs/README.md status when implementation lands
- [x] run default, Stylos-feature, and all-feature validation for touched crates
