# PRD-120: Adapt Proven Delegation Instructions for Themion

- **Status:** Implemented
- **Version:** v0.73.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-10

## Summary

Themion should improve its model-facing coordination guidance by adapting proven delegation instruction patterns. The goal is better system instructions, not a change to Themion's product model.

The adapted guidance must preserve Themion's characteristics:

- local agents in one instance are peers with roles, not permanent subagents
- user-facing and tool language should keep "create local agent" or "create team member", not switch to "spawn agent"
- delegation uses Themion concepts: local agents, durable board notes, inbox messages, explicit ownership, and clear return paths
- another local agent is used only when the user explicitly authorizes delegation, parallel agent work, or another agent's help or review

## Goals

- Adapt proven delegation-prompt patterns into Themion-native guidance.
- Preserve Themion's equal local-agent model and existing tool vocabulary.
- Separate the decision to involve another agent from the technique for coordinating delegated work.
- Prevent accidental multi-agent escalation from vague requests such as "be thorough" or "research deeply".
- Keep delegated-work guidance compact, reviewable, and suitable for prompt injection.

## Non-goals

- Do not rename local agents to subagents as a product concept.
- Do not rename `local_agent_create` or user-facing "create local agent" language to "spawn agent".
- Do not introduce a permanent parent/subagent hierarchy.
- Do not remove local-agent support, board notes, or inbox messaging.
- Do not redesign runtime ownership for local agents.
- Do not add a new approval UI or interactive confirmation flow in this PRD.
- Do not ban delegation when the user explicitly authorizes it.

## Background & Motivation

Themion already supports multiple local agents in one instance. Current guidance says `master` should consider creating or delegating to another local agent for non-trivial work. Related guidance also describes channel choice: answer directly, create a self-note, delegate locally with a durable board note, or send a short volatile inbox message.

That guidance is useful, but it mixes two concerns:

1. whether the current user request authorizes involving another local agent
2. how delegated work should be assigned after delegation is authorized

Well-known agent instruction prompts separate these concerns better. They include practical coordination rules: give helpers narrow ownership, tell them they are not alone in the environment, require a return path, and close delegated work when done. Newer tested variants also include an important authorization boundary: do not use another agent merely because the task is large, deep, or complex.

Themion should reuse those instruction ideas, but translate them into its own architecture and vocabulary. "Subagent" and "spawn" may appear in source material, but they should not become Themion product terms.

## Design

### 1. Translate source prompt terms into Themion terms

Implementation should adapt the source ideas instead of copying source vocabulary.

| Source wording | Themion wording |
| --- | --- |
| subagent | local agent, delegated helper, worker, or reviewer |
| parent/subagent relationship | coordinator/worker relationship for one authorized workflow |
| spawn agent | create local agent, create team member, or use another local agent |
| subagent task handoff | board note with task, constraints, ownership, and return path |

The final prompt should sound native to Themion and should not imply a hierarchy that the product does not have.

### 2. Preserve Themion's local-agent model

Themion's model stays the same:

- local agents inside one instance are peers with roles
- `master` coordinates when the current workflow needs coordination
- durable delegated work uses board notes
- short coordination uses inbox messages
- runtime/App-State owns agent registry, workflow state, board routing, and shared status
- TUI and Web UI display state and send commands; they do not decide delegation policy

This PRD improves instruction quality. It does not replace the product model.

### 3. Separate authorization from technique

Prompt guidance must distinguish two steps:

- **authorization:** whether this request allows involving another local agent
- **technique:** how to choose an agent, assign ownership, use board notes, and return results after delegation is allowed

Role guidance can help with technique. It must not create authorization by itself.

### 4. Add an explicit delegation gate

Required behavior:

- The model must not create additional local agents, assign delegated board-note work to another agent, or initiate parallel multi-agent work unless the user explicitly asks for one of these:
  - delegation
  - parallel agents or parallel work by multiple agents
  - another agent's help or review
- Requests for depth, detail, careful review, investigation, research, or broad codebase analysis do **not** count as permission by themselves.
- If wording is genuinely ambiguous and the choice matters, the model should ask a short clarification instead of assuming delegation permission.

Examples that authorize delegation:

- "delegate this"
- "split this across multiple agents"
- "parallelize the investigation"
- "have another agent review this"

Examples that do not authorize delegation by themselves:

- "be thorough"
- "research this deeply"
- "investigate carefully"
- "do a full analysis"
- "take your time"

### 5. Keep delegation quality rules after authorization

Once delegation is authorized, Themion should keep the useful coordination practices from current guidance and the source prompts.

Required behavior after authorization:

- state the delegated task, expected output, constraints, and return path
- prefer durable board notes for work that must be tracked, resumed, or reported later
- use volatile inbox messages only for short coordination or clarification
- tell workers they are not alone in the environment and must not revert or overwrite unrelated work
- make file or responsibility ownership explicit for implementation work
- make one coordinator responsible for final state and the human-facing summary
- conclude delegated work cleanly when it is done

### 6. Keep single-agent work available

The delegation gate applies only to involving another local agent. It must not block normal single-agent work.

Required behavior:

- the current agent may answer directly, investigate directly, or create a self-note for its own work
- self-notes remain allowed for durable self-tracking
- ordinary thoroughness stays single-agent unless the user explicitly authorizes multi-agent work

### 7. Keep runtime and UI ownership unchanged

This PRD is an instruction and documentation change.

Required behavior:

- authorization guidance lives in prompt/instruction assembly and related docs
- local-agent, board-note, and inbox-message tools remain available
- TUI and Web UI do not decide whether delegation was authorized
- runtime enforcement is optional future work, not required for this PRD

**Alternative considered:** enforce the delegation gate only in runtime tools. Rejected for this PRD because a compact prompt-level guidance change fits the current architecture and can land without broad tool-policy redesign.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/agent.rs` | Update `role_instruction("master")` and both `board_guidance_text` variants in `Agent::build_prompt_context_report`. Keep "create local agent" wording, preserve equal local agents, require explicit authorization before involving another local agent, and keep board-note/return-path quality rules. |
| `docs/engine-runtime.md` | Explain the adapted guidance and separate delegation authorization from delegation technique. Update any text that says non-trivial work alone should trigger local-agent creation/delegation. |
| `docs/architecture.md` | Ensure high-level coordination wording does not imply a permanent subagent hierarchy or delegation based only on task size. |
| `crates/themion-core/src/agent.rs` tests | Update or add prompt-context tests so role context and board guidance include the authorization gate and still preserve self-note/direct-answer behavior. |
| Root `AGENTS.md` and other durable instruction docs | Update wording that implies delegation from non-trivial work alone. Keep Themion-native terminology and the authorization gate. |
| `docs/README.md` | List this PRD and update status/version after implementation. |


## Implementation Notes

Implemented in v0.73.0. The landed change updates `crates/themion-core/src/agent.rs` role and board guidance only; no tool names, schemas, runtime ownership, or UI behavior changed. The `master` role now treats local-agent delegation as explicitly authorized only when the user asks for delegation, parallel agent work, or another agent's help or review. Both stylos and non-stylos board guidance variants include the same authorization gate while preserving their existing self-note and volatile-message details.

## Edge Cases

- source prompt says "subagent" → verify: Themion translates it to local-agent delegation and does not make subagent a product identity.
- source prompt says "spawn agent" → verify: Themion translates it to "create local agent" or "use another local agent" and does not rename product language or tools to spawn.
- user asks for a complex implementation but says nothing about other agents → verify: the model may use a self-note or work directly, but does not delegate.
- user asks for deep research only → verify: the model does not treat depth alone as permission to involve another local agent.
- user explicitly asks for parallel work → verify: delegation is allowed and follows board-note/return-path guidance.
- user asks for a review from another agent → verify: delegation is allowed for that review slice.
- a delegated board note already exists from earlier authorized work → verify: follow-up within that delegated workflow remains allowed.
- ambiguity remains about whether the user wants delegation → verify: the model asks a short clarification instead of assuming.
- role guidance says `master` should consider delegation for non-trivial work → verify: wording is updated so it does not override the authorization gate.

## Migration

No database migration is required.

Implementation can begin as a guidance-only change. A later PRD may add tool-level or runtime-level enforcement if prompt-only behavior proves too weak.

Minor-version scope is appropriate because this changes user-visible multi-agent guidance and instruction policy.

## Testing

- inspect generated `master` role context with existing unit tests or a prompt context report → verify: it includes explicit user authorization before creating or delegating to another local agent.
- inspect generated board guidance in default and `--all-features`/`stylos` builds → verify: both variants include the same authorization gate and keep self-note target details.
- inspect generated guidance for a complex task with no delegation request → verify: it does not authorize involving another local agent or parallel delegation.
- inspect generated guidance for a request that explicitly asks for another agent's help, delegation, or parallel work → verify: delegation is allowed and quality rules still appear.
- inspect guidance for review/research wording without explicit delegation request → verify: depth language alone does not authorize delegation.
- inspect injected coordination guidance → verify: self-notes remain allowed while delegated work requires board-note return instructions after authorization.
- review docs after implementation → verify: docs say Themion adapted proven prompt guidance while preserving equal local agents, create-local-agent wording, and runtime ownership.
- run `cargo check -p themion-core` if prompt changes land → verify: core build compiles.
- run `cargo check -p themion-core --all-features` if prompt changes land → verify: all-feature core build compiles.
- run `cargo check -p themion-cli` if CLI doc-wiring or instruction plumbing changes land → verify: default CLI build compiles.
- run `cargo check -p themion-cli --all-features` if CLI code changes land → verify: all-feature CLI build compiles.

## Appendix: Draft Themion Prompt Text

This is the target instruction shape for implementation. The final code may split it across role guidance and board/coordination guidance, but the meaning should stay consistent.

### Draft role instruction for `master`

```text
- master: Lead the team and own coordination. Answer simple direct requests yourself. For non-trivial or branching work, use self-tracking when useful. Only create or delegate to another local agent when the user explicitly asks for delegation, parallel agent work, or another agent's help or review. When delegation is authorized, use durable board notes for delegated work that must be tracked, and include expected result and return-path instructions.
```

This replaces the current implication that non-trivial work alone is enough reason to create or delegate to another local agent.

### Exact delegation gate sentence

Use this sentence in both board guidance variants, immediately after the self-note guidance:

```text
Delegation authorization is separate from delegation technique: do not create another local agent, assign delegated board-note work to another agent, or initiate parallel multi-agent work unless the user explicitly asks for delegation, parallel agent work, or another agent's help or review. Requests for depth, thoroughness, research, investigation, review, or large scope do not count as delegation permission by themselves.
```

Use this sentence to qualify `local_agent_create`:

```text
After delegation is explicitly authorized, when extra capacity or role separation helps, master may consider local_agent_create before delegating.
```

### Draft board and coordination guidance

```text
Board and coordination guidance: choose the lightest durable-enough channel. Answer simple direct requests without notes. Use a self-note when your own non-trivial or branching work needs durable tracking. Delegation authorization is separate from delegation technique: do not create another local agent, assign delegated board-note work to another agent, or initiate parallel multi-agent work unless the user explicitly asks for delegation, parallel agent work, or another agent's help or review. Requests for depth, thoroughness, research, investigation, review, or large scope do not count as delegation permission by themselves.

When delegation is authorized, keep Themion terminology: create or use another local agent or team member; do not describe Themion agents as permanent subagents or use "spawn agent" as the product term. Prefer durable board notes for delegated work another agent must complete, resume, or report later. Delegated notes should state the task, expected output, constraints, ownership, and return path; if a durable response is needed, ask the worker to update the note result or create a done mention. Use short volatile messages only for clarification, urgent nudges, participant-facing state updates, or final wrap-up with no durable result; they are not durable task tracking.

For authorized multi-agent activity, assign one coordinator as authoritative state owner, use stable activity/turn/note ids, state participants, response channel, and expected reply shape, separate authoritative updates from discussion, broadcast only meaningful state transitions, define completion/timeout/late-input rules up front, say who is waiting on whom when needed, and end with a clear final outcome. When you receive a done-mention note, treat it as informational completion notification rather than a fresh work request.
```

Implementation may keep the existing self-note target details, such as `to_instance=local to_agent_id=<self>`, in the surrounding generated guidance. Those details do not change the authorization rule.

### Draft worker guidance for delegated notes

```text
You are not alone in the environment. Do not revert, overwrite, or disturb unrelated work. Follow the assigned ownership and adapt to other changes you discover. If your assignment is complete, return the requested result through the stated return path and stop. Do not create or delegate to more local agents unless the assignment explicitly says that is allowed.
```

### Consistency rules for implementation

- Keep existing direct-answer and self-note behavior. The authorization gate applies only to involving another local agent.
- Keep board notes as the durable path for delegated work that must be tracked.
- Keep inbox/Stylos messages as short volatile coordination only, when available in the current build.
- Keep runtime/App-State ownership unchanged; TUI and Web UI should not become delegation-policy engines.
- Replace or qualify existing wording such as "for non-trivial work, consider creating or delegating" so it cannot override the explicit authorization gate.
- Do not rename `local_agent_create`, schema descriptions, docs, or user-facing text to "spawn agent".

## Appendix: Source Guidance Extracts

These extracts explain the source ideas. They are not Themion product terminology.

### Collaboration template extract

- Use delegated helper agents for large scoped tasks, reviews, debate/fresh context, or noisy test/config work.
- Tell subagents they are not alone in the environment and must not impact or revert others.
- Tell them whether they may spawn more subagents.
- Close subagents when done.

### Orchestrator template extract

- Prefer multiple delegated agents to parallelize work.
- Wait for sub-agents before yielding.
- If you delegate work, your role becomes coordination only.
- For multi-step plans, spawn one agent per parallelizable step.

### Built-in role-guidance extract

- `explorer`: use for specific, well-scoped codebase questions; parallelize independent questions; reuse explorers; trust results enough to avoid redundant exploration.
- `worker`: use for execution or production work; explicitly assign ownership of files or responsibility; tell workers they are not alone in the codebase, must not revert others, and must adapt to others' changes.

### Tested authorization-rule extract

- "Only use `spawn_agent` if and only if the user explicitly asks for sub-agents, delegation, or parallel agent work."
- "Requests for depth, thoroughness, research, investigation, or detailed codebase analysis do not count as permission to spawn."
- "Agent-role guidance below only helps choose which agent to use after spawning is already authorized; it never authorizes spawning by itself."

### Themion interpretation

The source material is useful because it combines coordination technique with a tested authorization boundary. Themion should adopt those useful instruction patterns without adopting the source product's vocabulary or hierarchy.
