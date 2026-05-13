# PRD-127: Preserve Live Session Continuity Across Profile and Config Changes

- **Status:** Implemented
- **Version:** v0.78.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-13

## Summary

- Changing profile or session-local runtime settings inside a live Themion session should not create a new session identity.
- Keep the same `session_id`, live transcript, in-memory conversation history, workflow/runtime state, and current project context when the user changes profile, model, effort, or other supported runtime config.
- Future turns should use the new effective runtime settings without making the agent appear to start from a fresh session.
- Persistent `/config` changes and session-only `/session` changes should differ in save semantics only, not in whether they reset session continuity.
- The agent should continue working seamlessly after the change, with prior context still available for later turns, history tools, and status surfaces.

## Problem

Themion currently supports live profile and session-setting changes such as:

- `/config profile use <name>`
- `/config profile set ...`
- `/session profile use <name>`
- `/session profile set model=<value>`
- `/session profile set effort=<value>`
- `/session profile reset`

These flows rebuild the live interactive agent. Today that rebuild can behave like a fresh session boundary instead of a runtime-setting change inside one ongoing session.

When that happens, the user sees a continuity break:

- `session_id` changes
- live state or transcript continuity is lost or split
- later history lookups no longer see one continuous session story
- the agent appears to forget in-session work even though the user only changed runtime settings

This is the wrong product behavior. A runtime profile/config change is not the same as starting a new conversation.

## Scope

In scope:

- live runtime changes initiated by existing `/config profile ...` and `/session profile ...` commands
- continuity of `session_id`, transcript/history, in-memory conversation state, workflow/runtime state, and project/tool context after those changes
- clear rules for what must stay attached to the current live session versus what may be recomputed safely
- docs and command/help text updates where current wording implies a fresh replacement session

Out of scope:

- starting a brand-new explicit user session
- migrating old persisted rows between unrelated historical sessions
- retroactively rewriting older turn metadata such as provider/model used by already completed turns
- changing the meaning of persistent `/config` versus session-only `/session`
- inventing a new multi-session merge feature

## Current behavior

Documented behavior already distinguishes persistent config changes from session-only overrides. `docs/architecture.md` says `/config profile use <name>` persists the profile choice, while `/session profile use <name>` and `/session profile set ...` rebuild the live interactive agent for the current session only.

Older PRDs also assumed that mid-session profile/model changes are valid product behavior. PRD-076 explicitly allowed temporary switching within one live session, and PRD-123 extended the same model to effort.

The remaining product gap is continuity ownership. Rebuilding the live agent should rebind runtime settings, but it should not silently create a new logical session identity or discard ongoing context.

## Expected behavior

When the user changes supported runtime profile/config values during a live session, Themion must keep one continuous session unless the user explicitly starts a new session.

Required outcomes:

- keep the same `session_id`
- keep the same current project directory and history/search scope that depends on the active session
- keep prior in-memory conversation messages available for future prompt replay under the normal context-budget rules
- keep the live transcript continuous instead of presenting the change as a fresh conversation
- keep workflow state, board-note context, watchdog/runtime ownership, and agent-local session metadata attached to the same ongoing session when they are not inherently provider-specific
- apply the new effective profile/model/effort/config only to future model requests after the change
- preserve already-recorded history as history; do not rewrite earlier turns to pretend they used the new provider/model/config

User expectation rule:

- changing runtime settings should feel like “continue this work with new settings”, not “start over”.

### Explicit preserve / do-not-reset list

For covered live profile/config/session changes, these session-owned items must be preserved and must not be reset just because runtime settings changed:

- the active interactive `session_id`
- the current project directory and project-scoped history/search defaults tied to that session
- in-memory conversation `messages` or equivalent replay source
- turn-boundary history used for replay/windowing
- persisted session history linkage so later messages continue under the same session record
- live transcript entries already shown to the user
- workflow state and retry-state attached to the live session
- agent-local queued follow-up prompts already accepted for that same session/agent
- runtime-owned board-note, inbox, watchdog, and routing ownership state that belongs to the same live session
- per-session activity/status tracking that surfaces the active interactive session in TUI/Web/headless views
- current temporary session overrides that are still semantically active after the new command is resolved

These items may change because of the command, but they must not reset session continuity:

- effective profile/provider/model/endpoint/api-key/effort values for future turns
- visible configured-versus-effective status text
- the provider-specific runtime/client object rebuilt for future turns

These items must stay historical and must not be rewritten:

- already completed turn rows and their stored provider/profile/model/effort attribution
- already emitted transcript/history events from earlier turns

## Fix approach

### 1. Treat live profile/config changes as runtime reconfiguration, not session recreation

The runtime should distinguish between these two operations:

- reconfigure the live session's effective runtime settings
- create a new logical session

Existing profile/config/session commands in this PRD's scope must use the first operation.

Implementation direction:

- keep one authoritative live session record and session UUID
- replace or rebuild only the provider/runtime wiring that truly depends on the changed settings
- reattach that rebuilt execution path to the same ongoing session state instead of allocating a fresh session identity

### 2. Preserve conversation and tool-facing continuity

The replacement path must keep the same conversation/history state that the agent has already accumulated in the current session.

Required continuity:

- in-memory `messages` or equivalent conversation state remains attached to the ongoing session
- turn-boundary and replay state remains valid for future turns in the same session
- history/recall/search tools that default to the current session continue to resolve to the same session after the change
- tool context that depends on session identity keeps using the same session unless a tool explicitly scopes differently

If some provider-specific cached object cannot be reused directly, the runtime may rebuild that object, but it must carry forward the same session-owned state.

### 3. Preserve runtime-owned non-provider state

Not all live state should be tied to provider/model wiring.

The runtime must preserve, when applicable:

- workflow state
- board/inbox/watchdog ownership and routing context
- agent activity/session metadata that belongs to the ongoing session
- transcript continuity and visible session surfaces
- queued same-agent follow-up owned by runtime/app-state
- session-scoped activity/status snapshots consumed by TUI, Web, and headless status surfaces

If a state fragment truly depends on the old backend and cannot remain valid, the implementation should reset only that narrow fragment and keep the rest of the session intact.

### 4. Keep old-turn attribution unchanged and future-turn attribution updated

Continuity does not mean pretending past turns used the new settings.

Required behavior:

- already completed turns keep their original provider/profile/model/effort metadata
- turns created after the change use the new effective runtime settings
- if a config/profile change is accepted while an active turn is already running, that in-flight turn keeps its original settings
- status and inspection surfaces should clearly report the current effective settings without implying that earlier history changed

This keeps continuity and auditability at the same time.

### 5. Define exact busy-turn behavior

A live config/profile change while an agent is already busy needs explicit semantics.

This PRD makes that behavior explicit and implementation-ready.

Required behavior:

- do not cancel, restart, or tear down an in-flight turn just because profile/config/session settings changed
- accept supported `/config profile ...` and `/session profile ...` mutations while the target interactive agent is busy if the command is otherwise valid
- save persistent config changes immediately when the command is a `/config ...` path, keeping today's persistence semantics
- update session-owned effective-setting state immediately so inspection/status surfaces can show the pending next-turn settings clearly
- mark the rebuilt runtime wiring as pending while a turn is busy
- apply the pending runtime reconfiguration after the current in-flight turn fully completes and before the next new turn starts for that same agent
- if the same agent already has queued follow-up prompts, apply the pending runtime reconfiguration before auto-starting the next queued prompt
- do not try to swap provider/model wiring in the middle of one active turn's continuation loop

User-visible acknowledgement rule:

- when idle, acknowledge that the current session now uses the new setting immediately
- when busy, acknowledge that the setting was accepted for the current session and will take effect on the next turn after the active work finishes

State rule:

- the current session keeps the same `session_id` before, during, and after deferred apply
- the pending reconfiguration belongs to the same session and must not create a shadow or replacement session

Conflict rule:

- if multiple supported setting-change commands arrive while the same agent is busy, last accepted value wins for each field before the deferred rebuild happens
- deferred apply must rebuild once from the final resolved effective settings rather than rebuilding once per intermediate command

Rejection rule:

- reject only normal validation failures such as unknown profile, invalid key, invalid effort value, or missing auth/config needed to build the requested next-turn runtime
- do not reject solely because the agent is currently busy if deferred apply is possible

This keeps turn semantics simple: one turn uses one resolved runtime configuration, and the next turn may use a new one.

### 6. Handle side effects explicitly

The implementation must avoid continuity-preserving changes that accidentally introduce new inconsistencies.

Required side-effect rules:

- do not create a second DB session row for a covered live reconfiguration path
- do not leave any runtime maps keyed by the old session UUID after the reconfiguration path finishes
- do not drop queued prompts, watchdog cooldown state, board-claim ownership, or per-session activity timestamps just because provider/runtime wiring was rebuilt
- do not duplicate transcript events or replay older transcript/history entries as if they were newly produced
- do not make history tools, web projections, or status views temporarily point at a replacement session UUID
- do not let deferred busy-time reconfiguration apply to the wrong agent when multiple local agents exist
- do not mix one active turn's old-setting attribution with next-turn new-setting attribution inside the same persisted turn
- do not clear temporary session-only overrides unless the command semantics already require clearing them, such as `/session profile reset` or a documented profile-switch rule

Required implementation follow-through:

- if current code uses `new_session_id` or replacement-handle logic during rebuild, replace that path with continuity-preserving rebinding instead of UUID replacement
- update all producer/consumer pairs that key status or activity by `session_id`, including runtime state, event routing, and web status projection
- if any narrow provider-specific cache must be dropped, document that narrow reset and verify it does not affect preserved session-owned state

User-visible side-effect rule:

- the user may see that future turns use a new provider/model/profile, but should not see the app behave as if a new conversation, new history scope, or new interactive agent identity was created

### 7. Update user-facing wording and docs

Command acknowledgements and docs should avoid language that implies a brand-new session unless one is actually created.

Examples:

- prefer wording like “updated the current session to use profile X” over wording that implies a fresh session start
- session/profile show surfaces should continue to describe configured versus effective runtime settings for one ongoing session
- architecture/runtime docs should state that profile/config changes reuse the same live session identity and history unless the user explicitly starts a new session

### 8. Define the covered command set exactly

This PRD applies to the existing live setting-change commands that reconfigure the main interactive session runtime.

Covered commands:

- `/config profile use <name>`
- `/config profile set provider=<value>`
- `/config profile set model=<value>`
- `/config profile set endpoint=<value>`
- `/config profile set api_key=<value>`
- `/config profile set effort=<value>`
- `/session profile use <name>`
- `/session profile set model=<value>`
- `/session profile set effort=<value>`
- `/session profile reset`

If a later command is added that rebuilds the live interactive runtime in the same way, it should follow the same continuity rules unless a later PRD states otherwise.

## Risks / edge cases

- user changes profile from one provider/backend to another mid-session → verify: the same session continues and future turns use the new backend while old history remains visible.
- user changes only model or effort mid-session → verify: this is treated as a lightweight continuity-preserving reconfiguration, not a new session.
- user runs a persistent `/config profile set ...` command during a live session → verify: config saves still happen, but the current session remains the same session.
- user runs `/session profile reset` after temporary overrides → verify: the session returns to configured settings without changing `session_id` or losing history.
- user changes settings while the agent is busy → verify: the product uses one explicit safe rule for deferred apply or rejection, and does not lose session continuity.
- user later searches or recalls current-session history → verify: omitted `session_id` still refers to the same ongoing session across the setting change.
- provider-specific cached state cannot be reused → verify: only the minimal backend-specific object is rebuilt, while session-owned state remains attached.

## Migration

No database schema migration is required.

This change is runtime/session behavior and documentation only. Existing sessions and existing persisted history rows remain valid. The implementation should preserve the current turn-level attribution model while stopping future live setting changes from creating new logical session identities.

## Validation

- start a session, send several prompts, then run `/session profile set model=<value>` → verify: `session_id` stays the same, prior transcript remains visible, and future turns still see prior context.
- start a session, send several prompts, then run `/session profile use <name>` → verify: the runtime switches effective profile without creating a new logical session.
- start a session, send several prompts, then run `/config profile use <name>` → verify: the profile choice persists and the live session still keeps the same `session_id` and history.
- start a session, change effort with `/session profile set effort=<value>` or `/config profile set effort=<value>` → verify: only future requests use the new effort and current session continuity remains intact.
- run a covered setting-change command while the agent is idle → verify: the live session keeps the same `session_id` and the new settings apply on the very next turn.
- run a covered setting-change command while the agent is busy → verify: the command succeeds, the current turn keeps old settings, and the next turn uses the deferred new settings without creating a new session.
- queue one or more same-agent follow-up prompts, then change profile while the agent is busy → verify: deferred reconfiguration applies before the next queued prompt auto-starts.
- issue several supported setting changes while the agent is busy → verify: the final next-turn runtime uses the last accepted value for each changed field and rebuilds only once.
- inspect runtime/session maps before and after a covered change → verify: no replacement session UUID was allocated for the live interactive path and no stale old-session keyed status entries remain.
- inspect watchdog, board-note, and queued-prompt behavior across a covered change → verify: accepted queued work and watchdog/board ownership remain attached to the same session.
- inspect Web or other session-projection surfaces across a covered change → verify: they continue to show one stable session identity and do not flicker to a fresh session.
- change `/config profile set api_key=...` or endpoint/provider fields in a live session → verify: future turns use the new runtime configuration while prior history, session scope, and transcript remain intact.
- inspect history/recall behavior before and after a live profile/config change → verify: default current-session scope still points to the same session.
- inspect persisted turn metadata across the change → verify: turns before the change keep old attribution and turns after the change record the new effective settings.
- exercise the busy-agent case for each supported command shape → verify: the product follows one documented rule without resetting session state or losing queued/live work.

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-127-preserve-live-session-continuity-across-profile-and-config-changes.md` | Add the durable PRD for continuity-preserving live runtime reconfiguration. |
| `crates/themion-cli` runtime/session handling | Preserve one ongoing session identity and session-owned state when profile/config/session commands rebuild effective runtime settings. |
| `crates/themion-core` agent/session context | Keep conversation/history/session-owned context reusable across live runtime reconfiguration boundaries. |
| `docs/architecture.md` and related runtime docs | Clarify that live profile/config changes reuse the same session unless the user explicitly creates a new one. |
| `docs/README.md` | Index the new PRD with its status and scope. |

## Implementation checklist

- [x] preserve the existing live `session_id` across all covered profile/config/session runtime rebuild paths
- [x] separate session-owned state from provider-specific runtime wiring so rebuilds do not discard history, transcript continuity, or workflow/runtime ownership
- [x] add one deferred-reconfiguration path for busy interactive agents that applies after the current turn completes and before the next queued or manual turn starts
- [x] preserve queued prompts, watchdog/board ownership state, and session-scoped activity/status maps across live runtime reconfiguration
- [x] update command acknowledgements and session/profile inspection surfaces to reflect immediate-apply versus deferred-next-turn apply clearly
- [x] preserve old-turn attribution while recording new effective settings only for later turns
- [x] remove replacement-session side effects from runtime, event-routing, and Web/status projection paths that currently key off a newly allocated session UUID
- [x] update runtime/docs wording that currently implies live profile/config changes create a fresh session
- [x] validate idle, busy, queued-follow-up, and multi-change-last-wins behavior for the covered commands

## Implementation notes

Implemented in v0.78.0 by updating `crates/themion-core/src/agent.rs`, `crates/themion-cli/src/app_runtime.rs`, `crates/themion-cli/src/app_state.rs`, and `docs/architecture.md`. The landed behavior preserves the live interactive `session_id`, carries forward session-owned agent state across runtime reconfiguration, preserves queued follow-up work, and defers busy-time reconfiguration until the next turn instead of creating a replacement session.
