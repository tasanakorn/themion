# PRD-081: Single-Instance Multi-Agent Team Structure and Agent Membership Tools

- **Status:** Draft
- **Version:** v0.53.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-05-01

## Summary

- Themion already has the beginnings of local multi-agent collaboration: one process can describe multiple local agents, Stylos requests can target an `agent_id`, and the board plus talk facilities already support durable and realtime coordination.
- What is still missing is a stable team model inside one Themion instance: who the local agents are, which one is the leader, what roles team members hold, and how agents are added or removed intentionally.
- This PRD defines that product model first, then scopes the first delivery slice narrowly: add tooling to create and delete local agents inside one instance while preserving the existing `master` agent as the predefined leader.
- The initial release should improve structure and runtime readiness for future multi-agent execution without yet promising automatic delegation, agent spawning plans, or parallel orchestration policies.
- Board notes and talk requests remain the main collaboration primitives; this PRD builds the local team roster that those primitives will later operate across more intentionally.

## Goals

- Establish a clear single-instance multi-agent team model for Themion.
- Treat each local agent as a team member with a stable `agent_id`, label, and role set.
- Preserve the first built-in local agent as the predefined leader for the instance.
- Add explicit tooling to create and delete local agents at runtime.
- Keep the first delivery slice focused on team structure and local membership management rather than full autonomous delegation.
- Reuse the existing board and talk facilities as the collaboration primitives for future team behavior instead of inventing a separate communication model.

## Non-goals

- No automatic delegation planner, task decomposition engine, or parallel execution scheduler in this PRD.
- No redesign of the board-note model, talk transport, or Stylos network protocol.
- No cross-instance team management in this first slice; scope is one Themion instance only.
- No requirement to support arbitrary role hierarchies, approval chains, or nested subagent trees yet.
- No requirement in this PRD to persist full per-agent custom prompts, model overrides, or independent config profiles unless later implementation decides a minimal subset is already needed for correct local lifecycle behavior.
- No requirement to allow deleting the predefined leader agent.
- No requirement to ship human-name or wordlist-based automatic agent naming in the first slice.

## Background & Motivation

### Current state

Themion already has several important pieces of local multi-agent infrastructure:

- `themion-cli` owns a process-local `agents: Vec<AgentHandle>` model and already exports multi-agent status snapshots rather than flattening everything to one effective agent.
- The local runtime currently requires exactly one `master` role and at most one `interactive` role.
- Stylos request paths can already target a concrete local `agent_id` for talk, board-note intake, and task requests.
- Board notes and talk requests already provide durable and realtime coordination primitives.

Current documentation confirms that the product is no longer purely single-agent internally, but the actual user-facing agent-team model is still incomplete.

Today the runtime effectively bootstraps one built-in leader agent and may carry additional agent descriptors in memory, but Themion does not yet expose a first-class way to manage that local roster as an intentional team. As a result:

- the product has collaboration primitives but not a stable team-membership surface
- future multi-agent work would otherwise be forced to invent agent lifecycle ad hoc inside other features
- users cannot intentionally shape the local team before asking it to collaborate
- the meaning of roles such as leader versus worker is still implied by code and validation rather than presented as a product concept

### Why start with structure first

The requested outcome is not yet “full autonomous multi-agent Themion.” The immediate need is to make one instance structurally ready for that future.

That means the first step should be a strong local team model with explicit membership operations, not a premature planner. Existing board and talk facilities already give Themion a usable communication substrate. The missing layer is the local roster: a way to say who is on the team, what role they play, and which agent remains the leader.

### Neighboring-project inspiration

The nearby `codex-rs` repository is useful mainly as a framing reference, not as a direct protocol template. Its current subagent-related surfaces emphasize that spawned subagents should carry distinct identity and optional role metadata, and that parent-visible context should acknowledge their existence. That supports a Themion design where local agents are explicit team members rather than hidden background threads.

The review of `../claw-code` did not reveal a stronger local team-management model than that. For this PRD, the most relevant external lesson is simple: agent identity and role should be explicit before higher-level delegation behavior grows around them.

## Design

### 1. Define the local multi-agent model as a team inside one instance

Themion should treat the local agent roster in one process as a team.

Required behavior:

- one Themion process may host multiple local agents
- each local agent is a team member with a stable `agent_id`, user-visible `label`, and role list
- the local roster is instance-local, not a cross-instance mesh concept
- the team model should be visible in local status, runtime inspection, and future user-facing management flows
- the current process-local ownership boundary should remain: `themion-cli` owns team-member descriptors, while each core `Agent` owns its own harness/session/workflow state

This keeps the product model aligned with the current architecture instead of inventing a separate scheduler-owned abstraction.

**Alternative considered:** define multi-agent support first as a remote Stylos network concept. Rejected: the request is specifically to support multi-agent operation inside one instance, and the repository already has the right local ownership boundary for that.

### 2. Preserve the first built-in agent as the predefined leader

The built-in `master` agent should remain the leader of the local team.

Required behavior:

- the first shipped team member must remain the built-in `master` agent
- `master` must be reserved for that predefined leader and must not be auto-generated for other agents
- explicit creation requests that try to reuse `master` as a new `agent_id` must be rejected
- the `master` role should continue to be unique within the local roster
- the leader may also carry the `interactive` role, as it does today, unless later work deliberately moves interactivity elsewhere
- future management flows must treat the leader as the canonical default target when a request omits an explicit `agent_id`
- the first delivery slice must not allow deleting the leader or leaving the roster without a leader

This gives the team model a stable anchor while keeping current default routing behavior intact.

**Alternative considered:** make every local agent a peer immediately with no distinguished leader. Rejected: current runtime defaults, validation, and request targeting already assume one primary local agent, so removing that distinction now would expand scope without solving the immediate product gap.

### 3. Represent roles as team-member responsibilities, not only routing tags

Themion should treat roles as intentional team responsibilities.

Required behavior:

- each agent may carry one or more roles
- `master` remains the unique leader role
- `interactive` remains the role used for the primary direct user-facing agent path unless later work changes that behavior explicitly
- additional non-leader roles should be allowed for created agents so future worker/reviewer/planner patterns can be expressed without another data-model change
- role validation should continue to prevent invalid leader multiplicity and any other contradictory role combinations the implementation already relies on

This turns the existing role list into a product concept that can grow naturally into future multi-agent patterns.

**Alternative considered:** defer role modeling and create anonymous worker agents only. Rejected: the user explicitly wants to treat agents as team members with roles, and future collaboration will need those semantics anyway.

### 4. Add explicit local agent membership tools for create and delete

The first shipped management surface should be tool-based and limited to add/remove operations.

Required behavior:

- Themion should add a tool to create a new local agent team member within the current instance
- Themion should add a tool to delete an existing non-leader local agent from the current instance
- create input should allow specifying an explicit `agent_id`, optional display label, and role set
- when create input omits `agent_id`, the runtime should generate a simple default non-leader worker id in `smith-N` form using the next free positive integer, for example `smith-1`, `smith-2`, or `smith-99`
- `smith-N` generation is only a fallback for omitted `agent_id`; it must not replace explicit user-provided ids
- delete input should identify the target local agent by `agent_id`
- the tools should return compact structured mutation acknowledgements consistent with current repository conventions
- the create/delete operations should mutate the local team roster used by targeted request routing and exported status snapshots

This gives the model and the user a controlled way to shape the local team before more advanced orchestration exists.

**Alternative considered:** start with wordlist-based human-name generation such as adjective-noun aliases. Rejected: `smith-N` is simpler, predictable, collision-resistant enough for the first slice, and avoids unnecessary wordlist and policy complexity before the team-management surface itself is proven.

### 5. Keep the first slice local-runtime scoped and minimally persistent

The first implementation should focus on correct lifecycle behavior for the active instance.

Required behavior:

- created agents must become available to the current in-memory local runtime without requiring process restart
- deleted agents must stop appearing in local targeting, snapshots, and team listings for the current instance
- the PRD does not require durable restart persistence for dynamic agents in the first slice if in-memory lifecycle support is sufficient for a clean initial delivery
- if implementation chooses to persist roster state, that persistence must be clearly documented as instance-local team configuration rather than network state
- any persistence decision must preserve the existing one-leader invariant on startup and restore

This keeps the first delivery slice manageable while still producing real user-visible capability.

**Alternative considered:** require full durable restart persistence in the first implementation. Rejected: the main product need is to establish the team structure and runtime facility; persistence can follow once the local team model is proven.

### 6. Reuse board and talk as the first collaboration primitives across the local team

The new team model should compose with what Themion already has.

Required behavior:

- created local agents should be addressable through the same `agent_id` targeting semantics already used by local routing, talk, and board-note workflows where applicable
- the team model should not introduce a second parallel local messaging concept
- existing board and talk features should remain the recommended coordination primitives for future leader-to-member or member-to-member collaboration inside the instance
- user-facing docs should explain that this PRD adds local team membership structure, not a replacement communication channel

This keeps the architecture coherent and lets future multi-agent execution build on existing primitives.

**Alternative considered:** introduce a brand-new intra-process agent bus at the same time as membership tools. Rejected: that would mix structural work with coordination redesign and make the first slice harder to ship cleanly.

### 7. Validate membership operations against current routing and role rules

Membership changes should preserve the invariants that current runtime code already depends on.

Required behavior:

- create must reject duplicate `agent_id` values within the local roster
- create must reject any explicit or implied `agent_id` collision, including collisions against reserved names such as `master`
- create must reject any role combination that would violate the one-leader rule
- delete must reject attempts to remove the leader agent
- delete must reject attempts to remove an agent that is currently required by an active turn if immediate deletion would leave the runtime inconsistent; the implementation may choose either a safe refusal or a deferred-removal policy, but that choice must be explicit
- status export, request targeting, and local validation must continue to operate correctly after membership changes

This ensures that the structural tools improve the runtime instead of destabilizing it.

**Alternative considered:** accept permissive roster edits first and rely on downstream routing to fail later. Rejected: the current code already has important role and targeting assumptions, so membership validation should happen at the management boundary.

### 8. Expose the team model clearly in status and documentation

The product needs visibility, not just hidden mutability.

Required behavior:

- local runtime/system inspection output should make the local team roster visible in a stable way
- exported status snapshots should continue to list local agents, including dynamically managed members when present
- docs should describe the team model explicitly: one instance, one leader, optional additional members, existing board/talk coordination primitives, and tool-based membership management in the first slice
- docs should also describe the initial naming policy clearly: predefined leader `master`, reserved `master` id, and fallback worker ids generated as `smith-N` only when no explicit `agent_id` is supplied
- the PRD index and related docs should frame this work as the foundation for future single-instance multi-agent execution rather than as already-complete autonomous delegation

This makes the feature understandable and gives later work a documented base to build on.

**Alternative considered:** ship the management tools first and document the team model later. Rejected: without docs and visible status shape, the feature would feel like an internal mechanism rather than a product capability.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Add tool definitions and tool-call handling for local agent membership management, including compact structured create/delete acknowledgements and any shared validation contract needed at the tool boundary. |
| `crates/themion-core/src/agent.rs` or adjacent shared runtime helpers | Add or adapt shared helpers only if the implementation needs a core-visible contract for managing agent-local harness instances safely. |
| `crates/themion-cli/src/tui.rs` | Extend local app/runtime behavior so the in-process roster can add and remove team members safely, preserve current role invariants, reserve `master` for the predefined leader, allocate fallback `smith-N` worker ids when `agent_id` is omitted, and keep targeted routing aligned with the changed roster. |
| `crates/themion-cli/src/app_state.rs` / `app_runtime.rs` | Wire any shared local team-management state needed so newly created agents are constructed consistently with current bootstrap behavior. |
| `crates/themion-cli/src/stylos.rs` | Ensure local request targeting, status export, and role-based selection continue to work with dynamic non-leader team members. |
| `docs/architecture.md` | Document the local team model and the boundary between core agent state and CLI-owned team-member descriptors. |
| `docs/engine-runtime.md` | Document local agent membership management, the one-leader invariant, the reserved `master` id, fallback `smith-N` worker naming, and how board/talk continue to fit the single-instance team model. |
| `docs/README.md` | Add the new PRD entry and later track status changes. |

## Edge Cases

- create a new local agent with an existing `agent_id` → verify: the operation is rejected with a clear duplicate-id error.
- create a new local agent with explicit `agent_id=master` → verify: the operation is rejected because `master` is reserved for the predefined leader.
- create a new local agent without `agent_id` when `smith-1` and `smith-2` already exist → verify: the runtime allocates the next free worker id such as `smith-3`.
- create a new local agent with role `master` when a leader already exists → verify: the operation is rejected.
- delete the built-in leader agent → verify: the operation is rejected.
- delete an unknown `agent_id` → verify: the operation is rejected with a clear not-found error.
- create a non-interactive worker agent and target it with local routing → verify: targeted request selection can resolve that agent by `agent_id`.
- create multiple non-leader agents → verify: status export and runtime inspection list all of them clearly.
- remove a non-leader agent that is idle → verify: it disappears from the local roster and is no longer targetable.
- attempt to remove an agent while it is mid-turn or otherwise busy → verify: the implementation applies its documented refusal or deferred-removal behavior consistently.
- omit a target agent on a flow that still defaults to the leader → verify: the built-in leader remains the default target.

## Migration

This PRD can land without a database migration if the first implementation keeps dynamic team membership instance-local and in-memory.

Rollout guidance:

- preserve the existing bootstrapped `master` leader as the default local agent
- reserve `master` for the predefined leader only
- add team-member management as an additive capability
- use fallback `smith-N` worker ids only when create requests omit `agent_id`
- keep board and talk semantics unchanged while making more local agents available to those workflows
- document any persistence limitations clearly if created agents do not survive restart in the first release

## Testing

- start a normal single-agent instance and inspect local status → verify: the instance still starts with one `master` leader and current default behavior unchanged.
- call the create-agent tool without `agent_id` → verify: the tool succeeds and allocates the next free `smith-N` worker id.
- call the create-agent tool with a new worker `agent_id` and non-leader role → verify: the tool succeeds and the new team member appears in local status or inspection output.
- call the create-agent tool twice with the same `agent_id` → verify: the second call is rejected as a duplicate.
- call the create-agent tool with explicit `agent_id=master` → verify: the request is rejected.
- call the create-agent tool with a second `master` role → verify: the request is rejected.
- target a newly created agent through an existing local routing path that uses `agent_id` selection → verify: the new team member can be selected correctly.
- create multiple non-leader agents and inspect the exported snapshot → verify: all team members are listed with stable `agent_id`, label, and roles.
- call the delete-agent tool for a non-leader idle member → verify: the tool succeeds and the agent disappears from the local roster.
- call the delete-agent tool for `master` → verify: the tool is rejected.
- call the delete-agent tool for an active busy member if busy-removal protection is implemented → verify: the documented refusal or deferred-removal behavior occurs.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check --all-features -p themion-core` after implementation → verify: the core crate builds cleanly across features.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the CLI crate still builds with Stylos enabled.
- run `cargo check -p themion-cli --all-features` after implementation → verify: the CLI crate still builds cleanly across feature combinations.

## Implementation checklist

- [ ] define the single-instance team model in docs and code-facing terminology
- [ ] preserve the unique built-in `master` leader invariant explicitly in membership management
- [ ] reserve `master` for the predefined leader and reject its reuse for created agents
- [ ] add fallback `smith-N` worker id allocation when create requests omit `agent_id`
- [ ] add a tool to create a local non-leader agent team member
- [ ] add a tool to delete a local non-leader agent team member
- [ ] validate duplicate ids, leader uniqueness, reserved-name reuse, and invalid deletion attempts at the management boundary
- [ ] make created and deleted team members affect local routing and exported status snapshots in the active instance
- [ ] document how board and talk remain the first collaboration primitives for the local team
- [ ] update architecture/runtime docs and PRD index references
