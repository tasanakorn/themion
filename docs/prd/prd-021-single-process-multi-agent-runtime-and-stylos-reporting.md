# PRD-021: Single-Process Multi-Agent Runtime and Multi-Agent Stylos Status Reporting

- **Status:** Implemented
- **Version:** v0.11.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-20

## Goals

- Redesign the current single-interactive-agent runtime shape so one Themion process can intentionally run multiple agent instances at the same time rather than merely storing them in a future-facing vector.
- Establish one clear agent-identity model inside the process, including an explicit role list instead of relying on today's `is_interactive` field as the only meaningful distinction.
- Make Stylos status reporting able to describe multiple in-process agents in one snapshot so external observers can see the whole Themion process state rather than only one effective agent.
- Keep the first implementation step narrow: redesign the status model and runtime ownership so multi-agent reporting lands before broader background-agent orchestration features.
- Preserve the architecture boundary where reusable agent/runtime behavior lives in `themion-core` and Stylos transport/session wiring remains in `themion-cli`.

## Non-goals

- No requirement in this PRD to ship a full end-user UI for spawning, selecting, renaming, pausing, or terminating arbitrary background agents from the TUI.
- No requirement to add cross-agent message passing, delegation, task scheduling, or supervisor logic in the same change.
- No migration of provider traffic, tool calls, or workflow state onto Stylos.
- No attempt to represent each in-process agent as a separate Stylos session; this PRD keeps one Stylos session per Themion process and reports multiple agents within that process.
- No requirement to collapse all agents onto one shared repository or working directory.
- No database-schema redesign for branching conversations or parent/child agent trees unless a tiny additive field is truly needed.

## Background & Motivation

### Current state

The architecture docs and PRD-002 already show that `themion-cli` stores agents as `App.agents: Vec<AgentHandle>`, but current runtime behavior still effectively assumes exactly one meaningful agent:

- the CLI creates one handle with `is_interactive = true`
- TUI input routing finds the unique interactive handle and sends all user input there
- status bar rendering describes only one active agent state
- Stylos status publishing exports one workflow/activity snapshot per process, not a list of in-process agents

That means the code has a small structural foothold for multi-agent support, but not a real runtime contract for it.

### Why redesign before adding richer multi-agent behavior

If Themion is going to support multiple agents in one process, it needs a stable internal model before adding visible features such as background workers or delegated tasks.

In particular, the current distinction of `is_interactive` is too narrow for the next phase because it conflates at least three different ideas:

- which agent is the main user-facing agent
- which agents may currently receive direct user input
- whether an agent exists only for background/internal work

Likewise, Stylos reporting currently exports process-level state that implicitly assumes a single agent. That makes external monitoring and future mesh-aware coordination less useful once multiple agents exist in one process.

### Why start with Stylos reporting

Stylos status is a good first forcing function for the design because it requires the process to answer basic questions clearly:

- how many agents exist in this process
- which one is the main agent
- what state is each agent in
- what workflow/activity snapshot belongs to which agent

If the runtime cannot answer those questions cleanly for observability, it is not yet ready for richer multi-agent behavior.

## Design

### Replace the implicit single-agent assumption with explicit in-process agent descriptors and roles

Themion should introduce an explicit in-process agent descriptor model rather than treating `AgentHandle` as a mostly-private holder with only `is_interactive` as identity.

The runtime-facing shape should distinguish at least:

- stable agent ID within the process
- human-readable label
- `roles: Vec<String>` or an equivalent role set
- per-agent workflow/activity/model/session metadata

The important contract change is that agent identity and behavior are expressed through roles rather than through a growing set of overlapping booleans. In the initial implementation, the single user-facing agent should carry `roles = ["main", "interactive"]`. Future agents may carry roles such as `background`, `reviewer`, or `planner` without requiring a wire-shape redesign.

Normative role expectations:

- exactly one agent should include the `main` role
- zero or one agents may include the `interactive` role in the initial implementation
- consumers should ignore unknown future roles rather than failing hard

This keeps the runtime honest once additional agent roles appear.

**Alternative considered:** keep separate booleans such as `is_main` and `accepts_input`. Rejected: that would work for the first step but scales poorly as agent roles expand.

### Treat one Themion process as one Stylos node with multiple reported agents

Stylos integration should continue to open one session per Themion process, but the status payload should be redesigned to include a list of in-process agents.

The process-level status should still carry shared fields such as:

- Themion version
- realm
- Stylos mode
- process instance
- active profile or default process profile when meaningful
- provider/model defaults when meaningful
- startup project directory that records the process entry directory

But the agent-specific execution state should move into an `agents` list.

Recommended status shape:

```json
{
  "version": "0.11.0",
  "instance": "host/12345",
  "realm": "dev",
  "mode": "peer",
  "startup_project_dir": "/path/to/repo",
  "agents": [
    {
      "agent_id": "main",
      "label": "main",
      "roles": ["main", "interactive"],
      "session_id": "uuid",
      "project_dir": "/path/to/repo",
      "project_dir_is_git_repo": true,
      "git_remotes": ["git@github.com:tasanakorn/themion.git"],
      "provider": "openai-codex",
      "model": "gpt-5.4",
      "active_profile": "codex",
      "workflow": { ... },
      "activity_status": "idle",
      "activity_status_changed_at_ms": 1760000000000,
      "rate_limits": { ... }
    }
  ]
}
```

Normative behavior:

- `agents` is always present, even when there is only one agent
- exactly one entry should include the `main` role
- zero or one entries may include the `interactive` role in the initial implementation
- `startup_project_dir` should record the directory from which the Themion process started, even if later agents use different directories
- `startup_project_dir` is informational provenance; consumers must not assume it matches every agent's `project_dir`
- per-agent `project_dir`, `project_dir_is_git_repo`, and `git_remotes` should reflect each agent's own working directory state
- shared process metadata should not be duplicated into every agent entry unless a field is genuinely per-agent
- timestamps remain machine-consumed epoch milliseconds and keep `_ms` suffixes where applicable

**Alternative considered:** publish one separate Stylos status key per in-process agent. Rejected: that would blur the distinction between process identity and agent identity and would make it harder to observe one Themion process as one node that hosts several agents.

### Introduce an explicit per-agent status snapshot API inside the CLI runtime

The current Stylos snapshot provider in `tui.rs` builds one `StylosStatusSnapshot` from mostly process-global state. That should be replaced with a process snapshot containing a list of per-agent snapshots.

A useful internal model is:

- process snapshot
  - `startup_project_dir`
  - list of agent snapshots
- agent snapshot
  - ID / label / roles
  - session UUID
  - project directory / git metadata
  - workflow state
  - activity state and changed timestamp
  - provider / model / profile
  - optional rate-limit state

This allows the TUI, Stylos publisher, and future diagnostics surfaces to read from the same internal status shape.

The implemented first step computes these snapshots in the CLI crate because the data is assembled from TUI-local runtime state and local session wiring. Focused tests cover role validation and multi-agent snapshot assembly.

**Alternative considered:** move all per-agent runtime snapshot assembly into `themion-core::Agent`. Rejected: workflow state belongs there, but UI routing flags and process-local agent role metadata are CLI concerns.

### Preserve one main agent and one input target in the first shipped step

Although this PRD prepares for true multi-agent execution, the first implementation step should keep behavior constrained:

- the process still boots with one main agent
- that same agent still receives normal user input by default
- the TUI need not yet expose background-agent controls
- Stylos status and internal runtime structures must nevertheless be able to represent more than one agent cleanly

This lets the code land a stable shape before adding broader orchestration behavior.

**Alternative considered:** require visible multi-agent spawning in the same PRD. Rejected: that would combine identity-model redesign, UI design, agent lifecycle management, and status protocol change into one larger and riskier task.

### Make future multi-agent runtime additions additive to PRD-002 rather than contradictory

PRD-002 already established `App.agents: Vec<AgentHandle>` as a compatibility foundation. This PRD should refine that foundation rather than replace it.

Expected evolution:

- `AgentHandle` gains explicit identity/role fields such as `agent_id`, `label`, and `roles`
- helper methods that currently search for `is_interactive` shift to explicit concepts such as `main_agent_mut()`, `interactive_agent_mut()`, or role-based lookups
- database session handling remains per-agent, so each agent still has its own `session_id`
- one process may therefore own multiple agent sessions that share the same DB handle while still using different per-agent project directories

This keeps the multi-agent process story aligned with the already-landed persistent-history model.

**Alternative considered:** collapse all in-process agents into one shared session ID. Rejected: separate sessions per agent preserve clear turn ownership and are more consistent with the existing history model.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Redesign `AgentHandle` and related app helpers to use explicit role lists instead of relying only on `is_interactive`; assemble a process-level multi-agent status snapshot for Stylos and future UI use. |
| `crates/themion-cli/src/stylos.rs` | Replace the single-agent status payload shape with a process payload containing `agents: [...]`; keep CBOR encoding and process-level Stylos session lifecycle unchanged. |
| `crates/themion-cli/src/main.rs` | Preserve current startup behavior while wiring the revised single-main-agent descriptor into app construction. |
| `crates/themion-core/src/agent.rs` | Expose or preserve the per-agent workflow/session data needed for the richer per-agent snapshot model without moving CLI-local role metadata into core. |
| `docs/architecture.md` | Update the runtime description so it no longer implies only one meaningful in-process agent and document that Stylos status reports a list of agents per process when the feature is enabled. |
| `docs/engine-runtime.md` | Clarify the distinction between process-local agent roles in `themion-cli` and per-agent harness/workflow state in `themion-core`. |
| `docs/README.md` | Add this PRD to the PRD index. |

## Edge Cases

- a process has only one agent → verifyable behavior should still publish `startup_project_dir` plus `agents` as a one-element list with `roles` containing both `main` and `interactive`.
- two agents are accidentally given the `main` role → runtime should treat this as an invalid internal state and degrade clearly rather than publishing ambiguous status.
- no agent is given the `main` role → runtime should either reject construction or synthesize a deterministic fallback only during startup repair, not silently during normal operation.
- one background agent has a different profile/model than the main agent → Stylos status should report those fields per agent rather than only once at process level.
- two agents use different project directories → Stylos status should preserve one process-level `startup_project_dir` while reporting distinct per-agent project and git metadata.
- one agent points at a git repo and another points at a non-git directory → per-agent git fields should differ without corrupting the other agent's snapshot.
- one agent is busy while the main agent is idle → status payload should show distinct per-agent activity states rather than flattening to one process-level activity string.
- a background agent exists but does not accept direct user input → status should represent that through its roles without requiring it to be the main agent.
- future runtime work adds multiple input-capable agents → the status shape should already allow the `interactive` role to evolve without breaking compatibility.
- an external Stylos consumer still expects the old single-agent status shape → that consumer must be updated because this PRD intentionally changes the exported status contract.

## Migration

This is an additive runtime-design change with a wire-shape update for Stylos status.

Runtime migration expectations:

- non-Stylos builds are unaffected at the transport layer
- feature-enabled Stylos builds continue publishing on the same process status key
- the top-level payload retains process identity and adds `startup_project_dir` as the directory where the process was started
- the published status payload changes from single-agent fields to a process payload with an `agents` list
- internal code that looked up the sole meaningful agent via `is_interactive` must migrate to explicit role-based helpers

Database migration is not required if the initial implementation keeps one session row per agent as already established in PRD-002.

## Testing

- run Themion with the `stylos` feature and one normal main agent → verify: the published status payload contains top-level `startup_project_dir`, plus `agents` with one entry, `roles` including `main` and `interactive`, and the expected workflow/activity/profile/model fields.
- construct an app state with more than one in-process agent in a focused test → verify: the process snapshot reports all agents and exactly one `main` role assignment.
- mark one agent busy and leave another idle in a focused runtime test → verify: the exported `agents` list preserves distinct per-agent activity states and timestamps.
- configure two in-process agents with different models or profiles in a focused test harness → verify: per-agent provider/model/profile fields are reported independently.
- configure two in-process agents with different project directories in a focused test harness → verify: `startup_project_dir` stays fixed at process-start value while each agent reports its own `project_dir`, `project_dir_is_git_repo`, and `git_remotes`.
- attempt to build or publish a state with zero agents carrying the `main` role → verify: the runtime surfaces a clear error or rejects the invalid state.
- attempt to build or publish a state with two agents carrying the `main` role → verify: the runtime surfaces a clear error or rejects the invalid state.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the revised Stylos reporting and CLI runtime compile cleanly.
- run the narrowest relevant tests for any added process-snapshot helpers or agent-role validation logic → verify: the new identity model behaves deterministically for one-agent and multi-agent cases.
