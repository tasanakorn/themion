# PRD-090: Agent Role Defaults and Role Instruction Guidance

- **Status:** Implemented
- **Version:** v0.58.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-03


## Implementation status

Landed in `v0.58.0` as dynamic local-agent role defaults and prompt guidance. Dynamically created local agents now resolve omitted or empty role lists to `executor`, and every active agent turn receives a compact role-context instruction block built from that agent's own runtime descriptor. The block includes the active agent id, label, resolved roles, a short known-role glossary, detailed action guidance only for the active agent's own roles, and direct non-interactive reporting guidance when the agent lacks `interactive`.

Implementation notes:

- The predefined `master` agent keeps `master` + `interactive` and does not donate those roles to dynamically created agents.
- Additional agents with omitted or empty roles receive only `executor` unless a different explicit role list is provided and accepted by validation.
- Role-context instructions are inserted as a separate prompt section rather than merged into the base system prompt.

## Summary

- Themion now has local multi-agent membership, but role semantics are still too implicit and dynamically created agents may have no default role when creation input omits `roles`.
- Define a compact built-in role guide for the canonical local roles: `master`, `interactive`, `executor`, `reviewer`, and `architect`.
- Preserve the initial `master` + `interactive` agent as the team leader and human-facing agent, while defaulting additional created agents to the generic `executor` role when no role is specified.
- Inject each active agent's own role set and role-specific guidance into that agent's instructions so the model knows which team member it is acting as on every turn.
- Non-interactive agents should keep chat output short, useful, and routed back to the original task owner or an interactive agent.
- Keep this PRD focused on role defaults and instructions, not on building a full delegation planner or changing board/talk collaboration primitives.

## Goals

- Make local agent roles understandable as durable team responsibilities rather than opaque routing tags.
- Provide concise built-in descriptions for the canonical roles Themion should recognize and communicate to agents.
- Ensure newly created additional agents receive `executor` when the create request omits an explicit role set.
- Preserve the first predefined agent as `master` + `interactive` by default.
- Guide non-interactive agents to reduce direct human-facing chat noise and prefer concise progress/activity reporting.
- Keep human-facing conversation responsibility centered on the `interactive` role unless the user or runtime explicitly targets another agent.
- Make each agent's own `agent_id`, label, role list, and role guidance visible in prompt/instruction construction so future multi-agent behavior can build on the same semantics.

## Non-goals

- No full automatic delegation planner, task decomposition engine, or role-based scheduler in this PRD.
- No redesign of board notes, Stylos talk, task request routing, or watchdog scheduling.
- No requirement to persist full per-agent custom prompts or model/profile overrides.
- No requirement to introduce arbitrary user-defined role taxonomies beyond preserving support for role strings where the runtime already allows them.
- No change to the one-`master` and at-most-one-`interactive` invariants unless implementation review shows current validation differs and docs must be corrected.
- No change to the meaning of `agent_id` or generated `smith-N` ids except the default role assigned to created agents.

## Background & Motivation

### Current state

PRD-081 established a single-instance team model where `themion-cli` owns process-local agent descriptors such as `agent_id`, `label`, and `roles`, while each `themion-core::Agent` owns its harness/session/workflow state.

The current documented behavior is intentionally narrow:

- the built-in `master` agent remains the predefined leader
- the initial interactive agent carries the `master` and `interactive` roles
- `local_agent_create` accepts optional `agent_id`, optional `label`, and optional `roles`
- omitted `agent_id` values allocate the next free `smith-N` worker id
- current validation rejects duplicate ids, another `master`, and another `interactive` role

That creates a usable roster, but not yet a complete role experience. The initial agent has predefined roles, while additionally created agents can be created with no default role if the caller omits `roles`. Role names such as worker, reviewer, or planner-like responsibilities are also not documented as a concise shared guide that agents can use to shape behavior.

### Why this matters now

Themion already exposes local-agent creation and multi-agent status. As soon as multiple agents exist, role clarity affects:

- how the master should delegate or coordinate work
- how a worker should report progress without spamming the human-facing transcript
- how reviewers should avoid making changes when the intended responsibility is validation
- how architecture-focused agents should explore, clarify, refine, and plan without being treated as generic executors
- how future routing and board-note assignment can reason about team responsibilities without inventing new semantics ad hoc

A small canonical role guide gives the current team model enough meaning to be useful while preserving the existing lightweight local roster design.

## Design

### 1. Define canonical built-in role meanings

Themion should document and inject a concise built-in guide for the canonical local roles.

Required role meanings:

- `master` — team master and team leader; coordinates the local team and remains the predefined leader role.
- `interactive` — responsible for human-facing interactive conversation and direct responses to the user.
- `executor` — generic work agent for general implementation, investigation, and task execution.
- `reviewer` — focuses on review, audit, validation, and quality checks; should not be targeted to make changes by default.
- `architect` — focuses on broad system design coverage, including exploration, clarification, refinement, and planning.

Required behavior:

- these descriptions should be short enough to fit naturally into model instructions and status/docs surfaces
- the guide should explain role intent without turning role names into hard scheduling policy by itself
- an agent with multiple roles should combine the responsibilities coherently, with more specific role guidance constraining generic behavior where applicable

**Alternative considered:** leave role meaning entirely to user-provided labels or ad hoc prompts. Rejected: the repository now has built-in local team membership, so the canonical built-in roles need stable baseline semantics.

### 2. Default additional created agents to `executor` when roles are omitted

Creating a new additional local agent with no explicit `roles` should no longer produce an unroled worker.

Required behavior:

- the built-in initial agent keeps its predefined role set, currently `master` + `interactive`
- `local_agent_create` with an omitted or empty role list for an additional agent should assign `executor`
- explicit role lists should remain honored when valid
- generated `smith-N` ids remain unchanged; only the omitted-role default changes
- status, runtime inspection, and Stylos agent snapshots should show the defaulted `executor` role like any explicit role

This makes the common “create another worker” action immediately useful without requiring the caller to remember a role argument.

**Alternative considered:** require callers to always specify a role. Rejected: this creates unnecessary friction for the common generic worker case and is inconsistent with the existing fallback id behavior.

### 3. Add role instruction guidance to prompt construction

Every model turn should tell the active agent which local team member it is and which roles it currently holds. The guidance should be derived from the runtime-owned local descriptor, not inferred from the agent name or conversation history.

Required behavior:

- the instruction source should be separate from the base system prompt, consistent with Themion's existing prompt-layering expectations
- each agent turn should include the active local agent's own `agent_id`, label, and resolved role list
- role guidance should be included dynamically for the active local agent's own role list only
- prompt context may include a short glossary of other known roles for awareness, but detailed action guidance should only be emitted for the active agent's resolved roles
- dynamically created agents must not inherit `master` or `interactive` roles from the predefined agent
- dynamically created agents should receive their own role-instruction block using their resolved role set; when roles are omitted or empty, that resolved role set is `executor`
- if an additional agent's roles were defaulted to `executor`, the injected instruction should show only `executor` unless another role was explicitly requested and accepted
- role guidance should be regenerated from the current runtime descriptor when an agent is created, recreated, or otherwise has its roles changed
- the active-role guidance should explain how the active agent should act for each of its own roles
- other-role awareness should use short descriptions only, so agents know what other roles mean without receiving instructions to act as those roles

The guidance should be compact and systematic so it does not materially bloat prompt overhead. It should make the model's current role context explicit enough that two local agents with different roles receive different instructions even when they share the same base provider/model configuration.

The injected instruction is dynamic. It should include: identity, resolved roles, a short known-role glossary for team awareness, and detailed action guidance only for the active agent's own roles.

Canonical role glossary, kept short for token efficiency:

- `master`: team leader.
- `interactive`: human-facing responder.
- `executor`: general task worker.
- `reviewer`: review and validation.
- `architect`: system design and planning.

The injected instruction should use this product shape, with values filled from the active runtime descriptor and only the matching active-role bullets included:

```text
Local agent role context:
- You are agent `<agent_id>` (alias: `<label>`). Omit the alias text when it is identical to `agent_id`.
- Your roles are: `<role1>`, `<role2>`, ...
- Known roles: master=team leader; interactive=human responder; executor=general worker; reviewer=review/validation; architect=design/planning.
- Act only from your listed roles; do not assume unlisted roles.

Your role instructions:
<include only bullets for the active agent's resolved roles>
- master: Lead the team; for non-trivial work, consider creating or delegating to another local agent instead of handling everything yourself. Use board notes or local-agent tools when useful. Simple direct Q&A may be answered directly.
- interactive: Own human-facing conversation; respond directly to the user when active/targeted.
- executor: Do general implementation, investigation, and task execution; report concise results to the task originator.
- reviewer: Review, audit, and validate; do not change files unless explicitly asked.
- architect: Cover system design; explore, clarify, refine requirements, plan, and identify tradeoffs.

Keep direct chat very short and activity-oriented; report final results to the requester, board note, or coordinating master/interactive agent.
```

For a dynamically created agent with omitted roles, the resolved instruction should therefore say the agent's own id/label and `Your roles are: executor`. It may include the short known-role glossary, but its `Your role instructions` section should include only the `executor` bullet and must not include `master` or `interactive` action guidance unless those roles were explicitly requested and accepted by validation.

**Alternative considered:** store these instructions only in docs and rely on humans to repeat them. Rejected: role behavior affects model output, so the active agent needs the relevant guidance in its prompt context.

### 4. Non-interactive roles should reduce direct chat noise

Roles that are not responsible for human-facing interactive conversation should keep direct chat responses short and activity-oriented.

Required behavior:

- non-interactive agents should avoid long human-facing explanations unless the task explicitly asks for a full report
- non-interactive progress messages should be very short and useful for tracking activity
- final results should be reported to the original task requester, board note, or coordinating `interactive`/`master` agent rather than treated as open-ended chat with the human by default
- non-interactive agents should prefer durable board-note updates for asynchronous work when appropriate
- this guidance should not prevent a directly targeted non-interactive agent from answering when the user explicitly addresses it; it should shape the response length and reporting route

This keeps multi-agent activity visible without making the main transcript noisy.

### 5. Keep role ownership in CLI/runtime state, not the TUI

Role defaults and prompt guidance must respect the existing ownership boundaries.

Required behavior:

- role defaulting belongs in the local-agent creation/runtime path, not in TUI rendering code
- role-aware prompt context should be derived from runtime-owned agent descriptors
- TUI changes, if any, should only display role information or role-derived events already owned by runtime/app-state
- Stylos status/query paths should publish the same runtime-owned roles that local inspection uses

This preserves the PRD-084/087 architecture rule that the TUI is a surface and the hub/app-state/runtime layer owns agent roster truth.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/app_state.rs` / local agent construction path | Preserve initial `master` + `interactive` roles and apply `executor` as the default role for additional created agents when roles are omitted or empty. |
| `crates/themion-cli/src/app_runtime.rs` / local roster mutation helpers | Ensure runtime-owned snapshots, inspection, and Stylos publication observe the defaulted role set from the same source of truth. |
| `crates/themion-core/src/tools.rs` | Update `local_agent_create` schema/argument wording if needed so omitted roles clearly mean the default `executor` role for additional agents. |
| `crates/themion-core/src/agent.rs` / prompt assembly path | Add the dynamic role-context instruction block for every active local agent turn, including `agent_id`, label, resolved roles, short known-role glossary, matching active-role bullets only, and non-interactive reporting guidance, without merging it into the base system prompt. |
| `docs/architecture.md` | Document canonical role meanings and the default additional-agent role behavior. |
| `docs/engine-runtime.md` | Document how role descriptors influence prompt guidance and non-interactive reporting behavior. |
| `docs/README.md` | Add this PRD and later reflect implementation status when the behavior lands. |

## Edge Cases

- create an additional agent with omitted `roles` → verify: the new agent has `executor` in status/inspection snapshots.
- create an additional agent with `roles=[]` → verify: the behavior matches omitted roles and defaults to `executor`, unless the implementation deliberately rejects empty arrays with a clear error and the docs/tool schema say so.
- create an additional agent with `roles=["reviewer"]` → verify: the explicit role is preserved and `executor` is not added implicitly.
- create an additional agent with `roles=["master"]` → verify: creation is rejected because the leader role remains unique.
- create an additional agent with `roles=["interactive"]` while the initial interactive agent exists → verify: creation is rejected under the current at-most-one-`interactive` rule.
- create an agent with multiple valid non-exclusive roles such as `executor` + `architect` → verify: prompt guidance includes the active agent identity and both responsibilities without conflicting with leader/interactivity invariants.
- a `reviewer` agent is assigned validation work → verify: it reviews and reports findings without editing files unless explicitly asked.
- a non-interactive `executor` completes a board-note task → verify: it reports concise progress/result information back to the note/request originator rather than producing noisy human-facing chat.

## Migration

This is a behavior and instruction update with no data migration required.

Required rollout behavior:

- existing sessions and existing in-memory agents keep their current role lists until recreated or explicitly changed
- newly created additional agents after the change default to `executor` when roles are omitted or empty, and do not inherit roles from the creating/current agent
- documentation should state the new default clearly so callers know how to request a roleless agent is no longer the default outcome
- if any stored future roster state exists by implementation time, do not rewrite explicit stored roles automatically unless a migration requirement is added separately

## Testing

- call `local_agent_create` without `roles` → verify: the returned and published local agent has `executor`.
- call `local_agent_create` with an empty roles array → verify: the result follows the documented omitted-role behavior.
- call `local_agent_create` with `roles=["reviewer"]` → verify: the result contains exactly the explicit valid role set and no unwanted `executor` default.
- call `local_agent_create` with duplicate or forbidden leader/interactivity roles → verify: existing role invariants still reject invalid rosters.
- inspect local runtime status after creating defaulted and explicit-role agents → verify: `roles` are consistent across local inspection, Stylos status, and any TUI display.
- start a turn for the predefined `master` agent → verify: the prompt context includes that agent's own `agent_id`, label, `master` + `interactive` role list, and matching role guidance as a separate contextual instruction source.
- start a turn for a dynamically created agent with omitted roles → verify: the prompt context includes the dynamic role-context instruction block with that agent's own identity, `Your roles are: executor`, the short known-role glossary, executor action guidance, non-interactive reporting guidance, and no inherited `master` or `interactive` action guidance.
- start a turn for an explicitly assigned `reviewer` or `architect` agent → verify: the prompt context includes that agent's own identity, resolved role list, and matching role guidance as a separate contextual instruction source.
- run `cargo check -p themion-cli` after implementation → verify: default CLI build compiles with the new role default path.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: Stylos-enabled role publication still compiles.
- run `cargo check -p themion-cli --all-features` after implementation → verify: all feature combinations for touched CLI role/status code compile.
- run `cargo check -p themion-core` and `cargo check --all-features -p themion-core` if prompt/tool schema code is touched → verify: core prompt/tool changes compile in default and all-feature builds.

## Implementation checklist

- [x] define canonical role descriptions for `master`, `interactive`, `executor`, `reviewer`, and `architect`
- [x] update local-agent creation so omitted or empty roles on additional agents default to `executor`
- [x] preserve the initial built-in `master` + `interactive` role set
- [x] preserve duplicate-id, one-`master`, and at-most-one-`interactive` validation
- [x] add the dynamic role-context instruction block for every active local agent turn, including agent identity, optional alias, resolved roles, short role glossary, matching active-role bullets only, and no inherited roles from another agent
- [x] add non-interactive role guidance for short activity reporting and originator/interactive-agent reporting routes
- [x] update tool schema wording for `local_agent_create`
- [x] update architecture/runtime docs with role meanings and defaulting behavior
- [x] keep role defaulting and prompt guidance out of TUI-owned policy code
- [x] validate default and relevant feature-enabled builds for touched crates
