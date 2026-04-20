# PRD-022: Stylos Queryables for Agent Presence, Availability, and Task Requests

- **Status:** Implemented
- **Version:** v0.12.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-20
- **Implementation status note:** Discovery queryables, per-instance queryables, git remote normalization including no-scheme comparable repo identities such as `github.com/owner/repo`, in-memory task lifecycle lookup, strict remote `agent_id` execution targeting for the current CLI-local agent set, and feature-gated agent-injected Stylos tool adapters have landed. The harness tool schemas now bridge into the existing CLI-local Stylos runtime/query implementation so the documented query surface and injected tools stay aligned.

## Goals

- Add useful Stylos queryables so external tools and peers can ask across the Themion mesh which agents are alive, which agents are currently free, which agents are attached to git repositories, which agents match a specific git remote URI, and what a chosen process or agent's current status is.
- Add a small request surface for talking to a target agent and for submitting task requests to an available agent, with a task lifecycle that supports later status lookup and optional waiting with timeout, without changing Themion's core provider or workflow architecture.
- Define a matching set of agent-injected Stylos tools that can be exposed in the harness tool palette when Stylos support is available, so an in-turn agent can perform the same discovery and request operations through a stable tool contract.
- Keep the initial design CLI-local and Stylos-feature-gated so the new behavior remains part of `themion-cli` runtime wiring rather than leaking network transport concerns into `themion-core`.
- Reuse the already-landed multi-agent Stylos status model so the new queryables answer questions from the existing per-process and per-agent snapshot rather than inventing a parallel state model.

## Non-goals

- No requirement to move agent execution, tool invocation, or history storage onto Stylos.
- No requirement to ship a full multi-agent task scheduler, durable remote job queue, or guaranteed-delivery protocol in this PRD.
- No requirement to add auth, ACLs, trust policy, or encrypted peer-to-peer messaging in the same change.
- No requirement to expose arbitrary tool execution over Stylos.
- No requirement to add a rich TUI for browsing or managing remote Stylos peers in this first pass.
- No requirement to redesign the existing status heartbeat payload beyond the additive fields needed to support query answers, task-request handling, and future tool adapters.

## Background & Motivation

### Current state

PRD-019 established basic optional Stylos support in `themion-cli`, and PRD-021 changed the exported Stylos status shape so one Themion process reports an `agents` array` with per-agent workflow, activity, profile, model, project directory, and git metadata. PRD-020 then simplified git metadata to a unique list of remote URLs with a small cache.

That means a feature-enabled Themion process already publishes enough information for an observer to infer several practical things:

- whether the process is alive
- which in-process agents exist
- whether an agent is idle or busy
- whether an agent is attached to a git repository
- what workflow and activity status each agent currently has

However, that information currently lives only in periodic status publication and a minimal info queryable. Consumers that want questions such as "who is alive?", "who is free right now?", "who has a git repo?", "who has this git repo?", or "what is this agent doing?" must reconstruct those answers themselves from raw status snapshots.

### Why split discovery queries from per-instance requests

The user asked for both discovery-style questions and direct operations.

Those are different kinds of mesh interaction:

- "who is alive?", "who is free?", and git-repository discovery questions should not require the caller to know a specific `<instance>` first
- "what is this instance doing?", "talk to this agent", and "send this task request here" are direct requests to a chosen running Themion process

The PRD should therefore make that distinction explicit instead of forcing every query through an instance-scoped key. This aligns better with Zenoh-style key hierarchies, where discovery commonly fans out across multiple matching queryables while direct control remains explicitly addressed.

### Why the PRD must distinguish queryables from injected tools

Stylos queryables and agent-injected tools are related, but they are not the same interface layer.

- queryables are a network-facing Stylos surface owned by `themion-cli`
- injected tools are part of the harness tool palette visible to the model during a turn
- queryables may exist without injected tools
- injected tools should act as adapters over the documented query/request surface rather than silently inventing a second protocol

Without stating this explicitly, names such as `stylos_query_agents_alive` can be misread as already-landed harness tools rather than logical operation names for the queryable API. The PRD should therefore define whether these operations are only queryables, or also future tool definitions, and should keep those contracts aligned.

### Why use `query/agents/...` for discovery

Discovery keys should be practical for routing, readable in docs, and extensible if the namespace grows.

Using a resource-style discovery namespace:

```text
stylos/<realm>/themion/query/agents/<leaf>
```

is preferable to both a flat `query/<leaf>` form and a more conversational `query/who/<leaf>` form.

Why:

- `agents` names the resource being queried rather than using a human-language question word
- the path stays machine-oriented and consistent with future protocol growth
- it keeps discovery keys grouped cleanly away from per-instance action/query keys
- it leaves room for future additions such as other agent-focused discovery leaves or optional request-body filters without renaming the family

This means discovery leaves represent operation variants on the `agents` discovery family, not generic payload-side filters.

### Why git discovery should support repo identity, not just repo presence

A simple `query/agents/git` question is useful for "who has any git repo?" but not sufficient for the more practical discovery question:

- who has this repo?

In real usage, the same repository may appear under multiple equivalent remote strings, for example:

- `git@github.com:owner/repo.git`
- `ssh://git@github.com/owner/repo.git`
- `https://github.com/owner/repo.git`
- `https://github.com/owner/repo`
- `github.com/owner/repo`

and similarly for GitLab and Bitbucket.

So git discovery should be designed around repository identity matching, with normalization that maps common ssh and https remote forms for well-known hosts to one canonical comparable form. This lets callers ask for a repo they know and get useful answers even when local clones use a different remote syntax.

### Why task submission should be separate from waiting for task completion

A task request and a task result are different lifecycle stages.

If `tasks/request` were to block waiting for the full result, it would combine:

- submission
- scheduling/selection
- execution
- result retrieval
- timeout behavior

into one operation.

That would make request handling harder to reason about and would couple Stylos query latency directly to model and tool runtime. A better initial design is:

- `tasks/request` for enqueue/accept-or-reject
- `tasks/status` for task state lookup
- `tasks/result` for explicit wait-with-timeout behavior

This keeps task lifecycle semantics visible and gives callers both polling and bounded waiting options.

## Design

### Use separate discovery and per-instance query namespaces

Feature-enabled Themion should expose two related query families under the existing Themion namespace.

Discovery queries answer agent-discovery questions across the mesh and are not addressed to a specific instance:

```text
stylos/<realm>/themion/query/agents/<leaf>
```

Initial discovery leaves:

- `alive`
- `free`
- `git`

Per-instance queries and requests operate on one chosen running Themion process:

```text
stylos/<realm>/themion/<instance>/query/<leaf>
```

Initial per-instance leaves:

- `status`
- `talk`
- `tasks/request`
- `tasks/status`
- `tasks/result`

Normative behavior:

- discovery queries are not target-specific and may receive one reply per responding Themion instance
- callers are expected to aggregate discovery replies when multiple instances respond
- per-instance queries target one logical Themion process identified by `<instance>`
- `alive` returns enough identity and agent data for a caller to answer which agents are currently alive on each responding instance
- `free` returns only agents currently considered available for new work
- `git` supports both broad git-presence discovery and matching for a specific git remote identity
- `status` returns current process and per-agent execution state, optionally filtered by agent ID when requested
- `talk` sends a user-style message to a chosen target agent on the addressed instance and returns an acknowledgement or immediate rejection
- `tasks/request` submits a structured task request to the addressed instance, which either routes it to a qualifying local agent or rejects it clearly
- `tasks/status` returns the current known lifecycle state for a previously accepted task
- `tasks/result` optionally waits up to a caller-specified timeout for task completion and then returns either the completed result or a still-pending state

These queryables are additive to the existing info/status publication behavior and do not replace periodic status broadcasting.

**Alternative considered:** expose only one generic `query` endpoint with ad hoc request bodies. Rejected: named leaves are easier to inspect, document, and evolve for the small initial command set.

### Define operation names and their meaning across queryables and future tools

To make the API easier to implement and easier to talk about in docs, each queryable should also have a stable operation name used in comments, helper functions, future docs/examples, and any future tool wrappers.

Recommended operation names:

| Queryable | Operation name | Scope | Purpose |
| --------- | -------------- | ----- | ------- |
| `stylos/<realm>/themion/query/agents/alive` | `stylos_query_agents_alive` | discovery | Ask which Themion instances and agents are currently alive. |
| `stylos/<realm>/themion/query/agents/free` | `stylos_query_agents_free` | discovery | Ask which agents are currently free for new work. |
| `stylos/<realm>/themion/query/agents/git` | `stylos_query_agents_git` | discovery | Ask which agents are attached to git repositories, optionally matching a specific repo identity. |
| `stylos/<realm>/themion/<instance>/query/status` | `stylos_query_status` | per-instance | Ask one instance for its current process and agent status. |
| `stylos/<realm>/themion/<instance>/query/talk` | `stylos_request_talk` | per-instance | Submit a user-style message to one target agent. |
| `stylos/<realm>/themion/<instance>/query/tasks/request` | `stylos_request_task` | per-instance | Submit a structured task request for local agent routing. |
| `stylos/<realm>/themion/<instance>/query/tasks/status` | `stylos_query_task_status` | per-instance | Look up the current lifecycle state of a submitted task. |
| `stylos/<realm>/themion/<instance>/query/tasks/result` | `stylos_query_task_result` | per-instance | Wait for or retrieve the result of a submitted task. |

These names are not shell commands. In this PRD they serve two purposes:

- they are the logical operation names for the Stylos queryable API
- they are the intended canonical names for matching agent-injected tool adapters if and when those tools are registered in the harness

That means a future harness tool named `stylos_query_agents_alive` should be a thin adapter over the documented `query/agents/alive` operation rather than a separate behavior contract.

**Alternative considered:** mirror the raw leaf names everywhere without operation-style names. Rejected: dedicated operation names make logs, helper functions, tests, future tool wrappers, and docs easier to read.

### Expose matching Stylos operations as optional agent-injected tools

When Stylos support is compiled and available at runtime, Themion should be able to expose a matching set of agent-callable tools in the harness tool palette.

Normative intent:

- injected Stylos tools are optional and feature-gated behind `stylos`
- non-`stylos` builds must not advertise or register these tools
- the tools should be adapters over the existing query and request surface rather than a parallel protocol
- the tools should use the same request and response semantics documented for the corresponding queryables where practical
- tool calls should fail clearly when Stylos runtime wiring is unavailable, misconfigured, or disabled
- query-style tools should not require the model to know transport details beyond documented parameters

Initial intended injected tool set:

- `stylos_query_agents_alive`
- `stylos_query_agents_free`
- `stylos_query_agents_git`
- `stylos_query_status`
- `stylos_request_talk`
- `stylos_request_task`
- `stylos_query_task_status`
- `stylos_query_task_result`

Ownership expectations:

- tool schema exposure and tool-call dispatch belong in `themion-core` because they affect the harness tool palette
- actual Stylos transport execution and request wiring remain CLI-local, so `themion-cli` should provide the runtime bridge or capability provider used by those tools
- non-Stylos runtimes should either omit the tools entirely or return a clear unavailable result consistent with the final implementation choice

This preserves the core/CLI boundary: core owns reusable tool interfaces, while the CLI owns the feature-gated local transport implementation.

**Alternative considered:** keep Stylos operations permanently queryable-only and never expose them as harness tools. Rejected: it prevents an in-turn agent from discovering peers or delegating work through the same documented interface, even though the operation set is naturally tool-shaped.

### Keep injected tool schemas aligned with the documented query payloads

If and when the Stylos tool adapters are registered, their schemas should closely mirror the documented queryable request bodies so the same operation is understandable across both entry points.

Recommended alignment:

- `stylos_query_agents_alive` takes no required arguments
- `stylos_query_agents_free` takes no required arguments
- `stylos_query_agents_git` accepts optional `remote`
- `stylos_query_status` accepts required `instance` and optional `agent_id` or `role`
- `stylos_request_talk` accepts required `instance`, `agent_id`, and `message`, plus optional `request_id`
- `stylos_request_task` accepts required `instance` and `task`, plus optional `preferred_agent_id`, `required_roles`, `require_git_repo`, and `request_id`
- `stylos_query_task_status` accepts required `instance` and `task_id`
- `stylos_query_task_result` accepts required `instance`, `task_id`, and optional `wait_timeout_ms`

Response guidance:

- tool results should return machine-readable JSON strings consistent with the corresponding query replies
- transport failures, disabled feature state, or invalid requests should return structured error objects rather than free-form prose where practical
- result shapes should preserve fields such as `accepted`, `task_id`, `timed_out`, `found`, and `git_repo_keys` so callers can reason about behavior consistently across network and tool entry points

**Alternative considered:** simplify injected tool schemas into conversational free-form arguments. Rejected: that would make the tool layer drift from the documented Stylos API and create two different mental models for the same operations.

### Define expected queryable behavior explicitly

Each queryable should have a simple, explicit behavior contract.

#### `alive`

- key: `stylos/<realm>/themion/query/agents/alive`
- request body: empty or omitted in the initial version
- reply cardinality: one reply per responding instance
- expected behavior:
  - return process identity for the responding instance
  - return that instance's current `agents` list
  - do not filter out busy agents
  - include enough data for callers to identify the instance, agent IDs, labels, and roles

Example logical reply shape shown in JSON for readability; the wire encoding should be CBOR:

```json
{
  "instance": "hostA-main",
  "session_id": "7b7d...",
  "agents": [
    {
      "agent_id": "main",
      "label": "main",
      "roles": ["main"],
      "activity_status": "busy"
    }
  ]
}
```

#### `free`

- key: `stylos/<realm>/themion/query/agents/free`
- request body: empty or omitted in the initial version
- reply cardinality: one reply per responding instance
- expected behavior:
  - filter the responding instance's agents to those whose `activity_status` is `idle` or `nap`
  - return a successful empty list when the instance is alive but has no free agents
  - preserve process identity in the reply so callers can route follow-up requests

Example logical reply shape shown in JSON for readability; the wire encoding should be CBOR:

```json
{
  "instance": "hostA-main",
  "session_id": "7b7d...",
  "agents": [
    {
      "agent_id": "background-1",
      "label": "background-1",
      "roles": ["background"],
      "activity_status": "idle",
      "project_dir": "/workspace/themion",
      "project_dir_is_git_repo": true,
      "git_remotes": ["git@github.com:example/themion.git"]
    }
  ]
}
```

#### `git`

- key: `stylos/<realm>/themion/query/agents/git`
- request body: empty, omitted, or an object that asks for matching against a specific repo identity
- reply cardinality: one reply per responding instance
- expected behavior:
  - when the request body is empty or omitted, filter the responding instance's agents to those with `project_dir_is_git_repo = true`
  - when the request body includes a git remote selector, return only agents whose known remotes match that selector after normalization
  - include `project_dir`, `git_remotes`, and normalized comparable repo identities in the reply
  - return a successful empty list when no local agent matches

Recommended request forms:

- empty request or omitted body → who has any git-backed project?
- `{ "remote": "git@github.com:owner/repo.git" }` → who has this repo?
- `{ "remote": "https://github.com/owner/repo" }` → who has this repo, regardless of local ssh/https form?
- `{ "remote": "github.com/owner/repo" }` → who has this repo by canonical comparable identity?

Normalization requirements for matching:

- normalize common GitHub, GitLab, and Bitbucket ssh and https remote formats to a canonical comparable host/path identity
- support direct comparable repo identity strings such as `github.com/owner/repo` in addition to common ssh and https remote formats for supported hosts
- treat equivalent forms such as `git@github.com:owner/repo.git`, `https://github.com/owner/repo.git`, and `github.com/owner/repo` as the same repository identity
- ignore a trailing `.git` suffix when normalizing comparable identity
- host matching should be case-insensitive
- path matching should preserve owner/repo semantics after normalization
- if a remote URI cannot be normalized under the known host rules, preserve the raw remote string and fall back to exact-string comparison rather than guessing

Recommended comparable field in replies:

- `git_repo_keys`: normalized repo identity strings derived from `git_remotes`

Example logical request shape shown in JSON for readability; the wire encoding should be CBOR:

```json
{
  "remote": "https://github.com/example/themion"
}
```

Example logical reply shape shown in JSON for readability; the wire encoding should be CBOR:

```json
{
  "instance": "hostA-main",
  "session_id": "7b7d...",
  "agents": [
    {
      "agent_id": "main",
      "project_dir": "/workspace/themion",
      "project_dir_is_git_repo": true,
      "git_remotes": ["git@github.com:example/themion.git"],
      "git_repo_keys": ["github.com/example/themion"]
    }
  ]
}
```

#### `status`

- key: `stylos/<realm>/themion/<instance>/query/status`
- request body: empty, omitted, or JSON filter object
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - empty request returns the full current process snapshot
  - `{ "agent_id": ... }` filters to one agent when found
  - `{ "role": ... }` filters to matching agents
  - unknown agent returns a structured not-found reply
  - malformed filters return a structured invalid-request reply

Example logical reply shape shown in JSON for readability; the wire encoding should be CBOR:

```json
{
  "instance": "hostA-main",
  "session_id": "7b7d...",
  "agents": [
    {
      "agent_id": "main",
      "activity_status": "busy",
      "activity_status_changed_at_ms": 1760000000000,
      "workflow": {
        "flow": "NORMAL",
        "phase": "EXECUTE",
        "status": "running"
      }
    }
  ]
}
```

#### `talk`

- key: `stylos/<realm>/themion/<instance>/query/talk`
- request body: JSON object with `agent_id`, `message`, and optional `request_id`
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - validate the target agent exists
  - validate the target agent can accept remote input under the initial rules
  - on success, enqueue the message through the normal local input path
  - return acknowledgement, not the final assistant answer
  - on failure, return a structured rejection reason

Implementation note:

- the landed implementation validates against the current local snapshot and enqueues accepted work through the normal local agent-input path
- when a target `agent_id` is supplied and matches a local agent, execution is now routed to that local agent rather than always falling back to the interactive agent

Example logical request/reply shapes shown in JSON for readability; the wire encoding should be CBOR:

```json
{
  "agent_id": "main",
  "message": "Please summarize your current task.",
  "request_id": "req-123"
}
```

```json
{
  "accepted": true,
  "agent_id": "main",
  "request_id": "req-123",
  "correlation_id": "local-456"
}
```

#### `tasks/request`

- key: `stylos/<realm>/themion/<instance>/query/tasks/request`
- request body: JSON object with `task` and optional routing filters
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - evaluate candidate local agents on the addressed instance
  - apply `preferred_agent_id`, `required_roles`, and `require_git_repo` filters when present
  - choose deterministically when more than one candidate qualifies
  - on success, enqueue the request into the normal local agent-input path
  - allocate a stable `task_id` for later lookup
  - on failure, return a structured rejection such as `no_available_agent` or `invalid_request`

Implementation note:

- the landed implementation performs candidate selection from the exported local snapshot and routes accepted work to the selected local agent through the normal local execution path

Example logical request/reply shapes shown in JSON for readability; the wire encoding should be CBOR:

```json
{
  "task": "Review the current repo and identify failing tests.",
  "required_roles": ["background"],
  "require_git_repo": true,
  "request_id": "req-789"
}
```

```json
{
  "accepted": true,
  "agent_id": "background-1",
  "request_id": "req-789",
  "task_id": "task-456",
  "note": "queued for local delivery"
}
```

#### `tasks/status`

- key: `stylos/<realm>/themion/<instance>/query/tasks/status`
- request body: JSON object with `task_id`
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - return a structured not-found reply when the task ID is unknown or expired
  - otherwise return the current known task lifecycle state
  - do not block waiting for task progress

Recommended lifecycle states:

- `queued`
- `running`
- `completed`
- `failed`
- `rejected`
- `expired`

Implementation note:

- the landed implementation updates task state from query acceptance into the local execution lifecycle for the selected agent, including `running`, `completed`, and busy-path `failed`

Example logical request/reply shapes shown in JSON for readability; the wire encoding should be CBOR:

```json
{
  "task_id": "task-456"
}
```

```json
{
  "found": true,
  "task_id": "task-456",
  "state": "running",
  "agent_id": "background-1"
}
```

#### `tasks/result`

- key: `stylos/<realm>/themion/<instance>/query/tasks/result`
- request body: JSON object with `task_id` and optional `wait_timeout_ms`
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - if the task is already completed or failed, return immediately with the terminal state
  - if the task is still pending and `wait_timeout_ms` is omitted or zero, return immediately with the current non-terminal state
  - if the task is still pending and `wait_timeout_ms` is positive, wait up to that timeout for a terminal state
  - if the timeout expires before terminal completion, return a normal non-error reply indicating the task is still pending and the wait timed out
  - the result query should not create a new task or alter task execution semantics

Implementation note:

- the currently landed slice stores terminal task result text from the final assistant output observed in the bridged local execution path
- richer per-agent task result handling remains future work

Example logical request/reply shapes shown in JSON for readability; the wire encoding should be CBOR:

```json
{
  "task_id": "task-456",
  "wait_timeout_ms": 30000
}
```

Completed reply example:

```json
{
  "found": true,
  "task_id": "task-456",
  "state": "completed",
  "agent_id": "background-1",
  "result": "Summary of failing tests..."
}
```

Timed-out wait reply example:

```json
{
  "found": true,
  "task_id": "task-456",
  "state": "running",
  "agent_id": "background-1",
  "timed_out": true
}
```

### Define "free" in terms of observable agent activity state, with explicit initial rules

The mesh needs a predictable answer to "who is free?" The first version should define a free agent using already-published activity/workflow state rather than introducing a separate scheduler-owned availability field.

Normative initial rule:

- an agent is considered free when its `activity_status` is `idle` or `nap`
- agents in `busy`, `waiting_tool`, `starting`, `error`, or any unknown future non-idle state are not free
- if future runtime work adds an explicit opt-out such as `accepts_task_requests = false`, the free filter should also respect that field

This matches the user request directly and keeps the first implementation aligned with already-existing state reporting.

Returned `free` payloads should include at least:

- `agent_id`
- `label`
- `roles`
- `session_id`
- `project_dir`
- `project_dir_is_git_repo`
- `git_remotes`
- `activity_status`
- `activity_status_changed_at_ms`
- `workflow` summary

**Alternative considered:** treat only `idle` as free. Rejected: the request explicitly called out both idle and nap as useful "free" states.

### Normalize git remote identity for discovery matching

Git discovery should not require all callers and all local clones to use the same remote URL syntax.

Normative behavior:

- derive normalized comparable repo identities from each exported git remote
- perform git query matching against those normalized identities when possible
- continue returning the original `git_remotes` values for transparency and debugging
- add normalized `git_repo_keys` to query responses when available so callers can cache and compare repo identity directly
- normalization should explicitly support common GitHub, GitLab, and Bitbucket ssh/https mappings in the first version
- normalization should also accept direct canonical comparable repo identities such as `github.com/example/themion`
- normalization should not silently rewrite unknown hosts into guessed identities

Recommended canonical comparable form:

```text
<host>/<owner>/<repo>
```

Examples:

- `git@github.com:example/themion.git` → `github.com/example/themion`
- `ssh://git@github.com/example/themion.git` → `github.com/example/themion`
- `https://github.com/example/themion.git` → `github.com/example/themion`
- `github.com/example/themion` → `github.com/example/themion`
- `https://gitlab.com/group/proj.git` → `gitlab.com/group/proj`
- `git@bitbucket.org:team/repo.git` → `bitbucket.org/team/repo`

For hosts whose repo path conventions are not recognized confidently, the implementation may expose no normalized key for that remote and rely on raw exact matching only.

**Alternative considered:** keep only raw `git_remotes` and require callers to normalize. Rejected: the repo-holder discovery question becomes brittle and duplicates normalization logic across every client.

### Track task lifecycle separately from request submission

Task submission should produce an identifier that can be used for later inspection and optional waiting.

Normative behavior:

- successful `tasks/request` replies allocate and return a stable `task_id`
- the addressed instance maintains an in-memory task lifecycle record for accepted tasks
- lifecycle records should track at least task identity, selected agent, current state, and terminal result or failure summary when available
- `tasks/status` reads the current lifecycle record without blocking
- `tasks/result` may block only up to `wait_timeout_ms`
- timeout behavior is part of `tasks/result`, not `tasks/request`
- `talk` remains a fire-and-acknowledge request and does not grow task-result waiting semantics in this PRD

Implementation note:

- the landed implementation tracks accepted tasks in memory and updates them from the normal local runtime bridge for the selected agent
- durable lifecycle persistence remains future work

This keeps submission, execution, and result retrieval distinct while still supporting useful caller workflows.

**Alternative considered:** make `tasks/request` optionally block until completion. Rejected: it complicates transport behavior, timeout handling, and agent runtime coupling.

### Keep request handling best-effort and explicitly non-durable in the first release

The first implementation should be useful without pretending to be a reliable job system.

Normative behavior:

- accepted `talk` and `tasks/request` submissions are in-memory process-local events
- in-memory task lifecycle records for `tasks/status` and `tasks/result` are also process-local in the first version
- process restart drops pending remote requests and task lifecycle records that have not been persisted
- there is no exactly-once guarantee
- duplicate caller `request_id` values may be echoed for correlation, but de-duplication is not required in the first version
- request rejections should be explicit and machine-readable

This keeps the initial transport honest and avoids overcommitting to durability or distributed coordination semantics before they are designed.

**Alternative considered:** persist all incoming requests and task state in SQLite immediately. Rejected: useful eventually, but too broad for the first queryable task-routing step.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/stylos.rs` | Add discovery queryables for `query/agents/alive`, `query/agents/free`, and `query/agents/git`; per-instance queryables for `status`, `talk`, `tasks/request`, `tasks/status`, and `tasks/result`; CBOR request/response payload structs; git remote normalization helpers for comparable repo identity; in-memory task lifecycle tracking; a prompt handoff channel for remote request delivery; and best-effort request handling that reads from current process snapshots. |
| `crates/themion-cli/src/tui.rs` | Expose or reuse process/agent snapshot helpers needed by the query handlers, route remote talk/task submissions into the normal local agent execution path for the selected local agent, capture final assistant text for task results, and preserve existing activity-state reporting used to decide free-vs-busy agents. |
| `crates/themion-core/src/tools.rs` | Define and register feature-gated agent-injected Stylos tool schemas, plus the feature-gated tool invoker hook used to dispatch those operations through CLI-provided runtime wiring. |
| `crates/themion-cli/src/main.rs` | No dedicated direct wiring changes were required beyond existing startup flow; the active CLI session still owns Stylos runtime startup while TUI-local agent construction now passes the Stylos capability bridge into the harness tool layer. |
| `docs/architecture.md` | Document the richer Stylos query surface and clarify that discovery queries are mesh-wide while remote request routing remains CLI-local runtime behavior on the addressed instance; injected tools are now documented as adapters over the same operation set. |
| `docs/engine-runtime.md` | Clarify how Stylos-originated talk/task requests enter the normal local agent-input path, how in-memory task lifecycle tracking supports later queries without moving harness logic into the transport layer, and how injected Stylos tools bridge from the harness into the CLI-local transport wiring. |
| `docs/README.md` | Keep this PRD indexed and reflect that both the queryables and the matching injected-tool contract are implemented. |

## Edge Cases

- a feature-enabled Themion process is alive but has no agents in a free state → its `free` discovery reply should be a successful empty result, while a per-instance `tasks/request` should return a structured rejection such as `no_available_agent`.
- discovery queries may receive replies from multiple instances → callers must aggregate them and identify each reply by instance.
- a `git` discovery request asks for a repo that is represented locally as ssh while the caller uses https, or vice versa → normalization should still match for supported hosts.
- an agent exports multiple remotes that normalize to the same repo identity → replies may keep all raw remotes but should deduplicate `git_repo_keys`.
- an agent is `idle` but intentionally should not accept remote requests in future runtime work → the first implementation may accept it, but the API should leave room for a future explicit capability flag.
- caller requests per-instance `status` for an unknown `agent_id` → return a clear not-found response.
- caller sends `talk` to a busy or non-interactive agent → runtime should either reject clearly or accept only if that agent's local input model explicitly allows queued remote input; the landed slice rejects busy targets and routes accepted work to the requested local agent when present.
- multiple agents are free and satisfy a `tasks/request` filter on one addressed instance → selection should be deterministic, such as first stable snapshot order, so behavior is inspectable.
- a selected free agent becomes busy immediately after selection → request handling may still race; the implementation should either fail fast on enqueue or accept best-effort and report the selected agent honestly.
- caller sets `require_git_repo = true` but the candidate agent's cached git state is stale within the existing TTL window → the request may use slightly stale repo metadata, consistent with current Stylos git caching behavior.
- a remote URI uses an unsupported host or unexpected syntax → the implementation should avoid false-positive normalization and fall back to raw exact matching.
- caller queries `tasks/status` or `tasks/result` for an unknown, expired, or dropped `task_id` → return a structured not-found or expired reply rather than an ambiguous empty success.
- a task completes after `tasks/result` times out waiting → the timeout reply should remain a normal non-error reply, and a later `tasks/result` or `tasks/status` query may still observe terminal completion if the lifecycle record is retained.
- `wait_timeout_ms` is negative, malformed, or excessively large → return a structured invalid-request reply or clamp according to documented limits; do not block indefinitely by accident.
- Stylos startup succeeds but one queryable registration fails → Themion should remain usable, surface the degraded query state, and continue publishing status where possible.
- non-feature builds remain unaffected → the added API surface must stay fully behind the `stylos` feature.
- injected Stylos tools requested in a non-`stylos` build are omitted from the harness tool palette, and when Stylos runtime is unavailable at call time they return a clear unavailable error rather than pretending the operation succeeded.
- a remote task arrives while the selected local execution path is already busy → the landed slice marks the task as failed with reason `agent_busy`.

## Migration

This is an additive minor feature for Stylos-enabled builds.

Migration expectations:

- non-Stylos builds are unchanged
- existing status heartbeat consumers continue to work
- new discovery-query consumers can ask mesh-wide agent questions without knowing `<instance>` first
- git-repo discovery callers can ask either broadly for any git-backed agent or specifically for agents matching a repo identity
- new per-instance query consumers can target `status`, `talk`, `tasks/request`, `tasks/status`, and `tasks/result` on one chosen process
- request senders must treat `talk`, `tasks/request`, `tasks/status`, and `tasks/result` as best-effort, process-local operations in the first release
- the landed slice supports strict local `agent_id` execution targeting within the current process-local agent set but is still not a durable distributed scheduler
- injected Stylos tools follow the same request and response contract rather than introducing a separate migration surface

No database or provider migration is required for the initial implementation.

## Testing

- run `cargo check -p themion-cli` after implementation → verify: non-feature builds still compile with no Stylos dependency regressions.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the feature-enabled CLI compiles with the new queryables, git normalization helpers, task lifecycle tracking, request-routing helpers, and the remote prompt bridge.
- start multiple feature-enabled Themion instances and query `stylos/<realm>/themion/query/agents/alive` → verify: the query receives one reply per responding instance and each reply includes process identity plus the current `agents` list.
- place one agent in `idle` or `nap` and another in a busy state, then query `stylos/<realm>/themion/query/agents/free` → verify: only the idle/nap agent appears in each responding instance's reply.
- start feature-enabled Themion in a git repo and query `stylos/<realm>/themion/query/agents/git` with no body → verify: only agents with `project_dir_is_git_repo = true` are returned and their `git_remotes` values are included.
- query `stylos/<realm>/themion/query/agents/git` with `{"remote":"git@github.com:example/themion.git"}` while the responding clone reports `https://github.com/example/themion` → verify: the agent still matches and the reply includes normalized `git_repo_keys`.
- query `stylos/<realm>/themion/query/agents/git` with `{"remote":"github.com/example/themion"}` while the responding clone reports ssh or https forms → verify: the agent matches by canonical normalized repo identity and the reply includes normalized `git_repo_keys`.
- query `stylos/<realm>/themion/query/agents/git` with equivalent ssh and https forms for GitHub, GitLab, and Bitbucket test repos → verify: matching works across supported forms.
- query `stylos/<realm>/themion/query/agents/git` with an unknown or unsupported host syntax → verify: the implementation falls back to exact raw matching rather than producing a false normalized match.
- query `stylos/<realm>/themion/<instance>/query/status` with no filter → verify: the full current process snapshot for the addressed instance is returned.
- query `stylos/<realm>/themion/<instance>/query/status` with `{ "agent_id": "missing" }` → verify: the response is a structured not-found result.
- send a valid `talk` request to `stylos/<realm>/themion/<instance>/query/talk` for an eligible agent → verify: the response acknowledges acceptance and the targeted local agent receives the message through the normal local input path.
- send a `talk` request to an ineligible or unknown agent → verify: the response is a structured rejection with a clear reason.
- send a `tasks/request` to `stylos/<realm>/themion/<instance>/query/tasks/request` that requires a git repo when exactly one free git-backed agent exists on that instance → verify: the request is accepted, returns a `task_id`, and reports that agent's ID.
- query `stylos/<realm>/themion/<instance>/query/tasks/status` for an accepted task before completion → verify: the response reports a non-terminal state such as `queued` or `running`.
- query `stylos/<realm>/themion/<instance>/query/tasks/result` with `wait_timeout_ms = 0` for a still-running task → verify: the response returns immediately with the current non-terminal state.
- query `stylos/<realm>/themion/<instance>/query/tasks/result` with a positive timeout for a task that completes within that window → verify: the response returns the terminal state and result without requiring a second query.
- query `stylos/<realm>/themion/<instance>/query/tasks/result` with a positive timeout for a task that does not complete in time → verify: the response returns a normal timed-out pending reply rather than a transport or protocol error.
- send a `tasks/request` while the selected local execution path is already busy → verify: the response lifecycle reaches a terminal `failed` state with reason `agent_busy`.
- send a `tasks/request` when no free agent on the addressed instance matches the filters → verify: the response is a structured rejection such as `no_available_agent`.
- run focused tests for free-agent filtering and deterministic candidate selection → verify: `idle` and `nap` are treated as free and the selected agent is stable for the same snapshot order.
- run focused tests for git remote normalization and repo-key derivation → verify: supported ssh/https remote variants and direct canonical repo identity strings normalize to the same comparable repo key and unsupported forms do not collide incorrectly.
- run focused tests for task lifecycle transitions and timeout handling → verify: accepted tasks move through expected states and timed waits produce the documented non-error pending reply.
- verify CBOR request and reply encoding for all new queryables → verify: payloads round-trip cleanly and match the documented logical shapes.
- run `cargo check -p themion-core -p themion-cli --features stylos` → verify: the harness can register the feature-gated Stylos tool definitions without breaking the core/CLI boundary.
- invoke each injected Stylos tool in a feature-enabled session against a live Stylos mesh → verify: arguments and result JSON match the corresponding documented query/request contract.
- invoke an injected Stylos tool in a non-`stylos` build or with Stylos disabled → verify: the tool is absent or returns a structured unavailable result consistent with the final implementation choice.

## Implementation checklist

- [x] define stable queryable keys for discovery (`query/agents/alive`, `query/agents/free`, `query/agents/git`) and per-instance requests (`status`, `talk`, `tasks/request`, `tasks/status`, `tasks/result`)
- [x] add CBOR-serializable request/response payload structs in `crates/themion-cli/src/stylos.rs`
- [x] add a shared process-snapshot reader reused by status publication and query handlers
- [x] implement discovery query handlers that reply once per local instance
- [x] implement git query request parsing for optional repo matching
- [x] implement git remote normalization helpers for supported GitHub, GitLab, and Bitbucket ssh/https URI forms
- [x] include normalized `git_repo_keys` in git discovery replies when available
- [x] implement per-instance `status` filtering and structured error replies
- [x] implement `talk` validation and enqueue path into the normal local agent-input flow
- [x] implement `tasks/request` candidate filtering, deterministic selection, stable `task_id` allocation, and enqueue path
- [x] implement in-memory task lifecycle tracking for accepted tasks
- [x] implement `tasks/status` lookup responses
- [x] implement `tasks/result` immediate and wait-with-timeout responses
- [x] keep all new runtime behavior behind the `stylos` cargo feature
- [x] enforce strict remote `agent_id` execution targeting beyond the existing interactive-agent bridge
- [x] define feature-gated harness tool schemas for the matching Stylos operations in `themion-core`
- [x] provide a CLI-local capability bridge so feature-gated Stylos tool calls can reach the existing query/request implementation without duplicating transport logic
- [x] document the injected Stylos tool names, schemas, and unavailable-state behavior
- [x] document the query namespace, CBOR payloads, git normalization rules, task lifecycle semantics, and expected logical reply shapes in `docs/architecture.md`
- [x] document how remote requests enter normal local agent execution and how in-memory task lifecycle tracking supports later queries in `docs/engine-runtime.md`
- [x] validate with `cargo check -p themion-cli`
- [x] validate with `cargo check -p themion-cli --features stylos`
