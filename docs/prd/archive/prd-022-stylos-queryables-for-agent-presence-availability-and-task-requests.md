# PRD-022: Stylos Queryables for Agent Presence, Availability, and Task Requests

- **Status:** Implemented
- **Version:** v0.12.1
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-20
- **Implementation status note:** Discovery queryables, per-instance queryable registration, git remote normalization including no-scheme comparable repo identities such as `github.com/owner/repo`, in-memory task lifecycle lookup, strict local `agent_id` execution targeting for the current CLI-local agent set, feature-gated agent-injected Stylos tool schemas, direct per-instance injected-tool RPC bridging, and discovery-tool self-exclusion controls have landed. Discovery tools now support `exclude_self`, which defaults to `true`, and self-exclusion is applied against the decoded reply payload `instance` field rather than inferred from reply-key shape, so local-instance discovery results are omitted unless the caller explicitly opts in.

## Goals

- Add useful Stylos queryables so external tools and peers can ask across the Themion mesh which agents are alive, which agents are currently free, which agents are attached to git repositories, which agents match a specific git remote URI, and what a chosen process or agent's current status is.
- Add a small request surface for talking to a target agent and for submitting task requests to an available agent, with a task lifecycle that supports later status lookup and optional waiting with timeout, without changing Themion's core provider or workflow architecture.
- Define a matching set of agent-injected Stylos tools that can be exposed in the harness tool palette when Stylos support is available, so an in-turn agent can perform the same discovery and request operations through a stable tool contract.
- Keep the initial design CLI-local and Stylos-feature-gated so the new behavior remains part of `themion-cli` runtime wiring rather than leaking network transport concerns into `themion-core`.
- Reuse the already-landed multi-agent Stylos status model so the new queryables answer questions from the existing per-process and per-agent snapshot rather than inventing a parallel state model.
- Make discovery-style injected tools safer by default by excluding the current Themion instance unless the caller explicitly requests full visibility.

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

### Why discovery tools should exclude self by default

Discovery queries are mesh-wide, but when an injected tool is used from within one running Themion instance, the most common intent is to find another instance rather than to rediscover the caller.

Including the local instance by default makes follow-up actions such as `stylos_request_talk` or `stylos_request_task` easier to misroute back to self. A safer default for the injected discovery tools is therefore:

- `exclude_self = true` when omitted
- allow `exclude_self = false` when the caller explicitly wants full mesh visibility including self

This keeps the underlying discovery queryables additive and symmetric while making the model-facing tool layer better match the usual intent of peer discovery. The exclusion check should use the decoded discovery reply payload's `instance` field as the source of truth rather than relying on transport reply-key structure.

**Alternative considered:** keep self-inclusion as the only behavior and require callers to filter locally every time. Rejected: it is easy to forget, produced accidental self-targeting during validation, and adds repetitive caller-side filtering for the common case.

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

The direct instance namespace is kept separate from discovery under `instances/`, and `<instance>` is a transport-safe `<hostname>:<pid>` identifier so Zenoh key hierarchy does not depend on slash-delimited host/process formatting.

```text
stylos/<realm>/themion/instances/<instance>/query/<leaf>
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
- `status` returns current process and per-agent execution state, optionally filtered by agent ID, role, or both when requested
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
| `stylos/<realm>/themion/instances/<instance>/query/status` | `stylos_query_status` | per-instance | Ask one instance for its current process and agent status. |
| `stylos/<realm>/themion/instances/<instance>/query/talk` | `stylos_request_talk` | per-instance | Submit a user-style message to one target agent. |
| `stylos/<realm>/themion/instances/<instance>/query/tasks/request` | `stylos_request_task` | per-instance | Submit a structured task request for local agent routing. |
| `stylos/<realm>/themion/instances/<instance>/query/tasks/status` | `stylos_query_task_status` | per-instance | Look up the current lifecycle state of a submitted task. |
| `stylos/<realm>/themion/instances/<instance>/query/tasks/result` | `stylos_query_task_result` | per-instance | Wait for or retrieve the result of a submitted task. |

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
- discovery tools should accept an optional `exclude_self` boolean that defaults to `true`
- when `exclude_self` is enabled, the tool bridge should compare against each decoded discovery reply payload's `instance` field rather than against the transport reply key

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

**Implementation note:** the landed slice also includes `stylos_query_nodes` as a session-level Zenoh visibility helper. That tool is useful, but it is outside the original PRD-022 queryable set.

**Alternative considered:** keep Stylos operations permanently queryable-only and never expose them as harness tools. Rejected: it prevents an in-turn agent from discovering peers or delegating work through the same documented interface, even though the operation set is naturally tool-shaped.

### Keep injected tool schemas aligned with the documented query payloads

If and when the Stylos tool adapters are registered, their schemas should closely mirror the documented queryable request bodies so the same operation is understandable across both entry points.

Recommended alignment:

- `stylos_query_agents_alive` accepts optional `exclude_self`
- `stylos_query_agents_free` accepts optional `exclude_self`
- `stylos_query_agents_git` accepts optional `remote` and optional `exclude_self`
- `stylos_query_status` accepts required `instance` and optional `agent_id` and `role`, which may be used independently or together
- `stylos_request_talk` accepts required `instance`, `agent_id`, and `message`, plus optional `request_id`
- `stylos_request_task` accepts required `instance` and `task`, plus optional `preferred_agent_id`, `required_roles`, `require_git_repo`, and `request_id`
- `stylos_query_task_status` accepts required `instance` and `task_id`
- `stylos_query_task_result` accepts required `instance`, `task_id`, and optional `wait_timeout_ms`

Response guidance:

- tool results should return machine-readable JSON strings consistent with the corresponding query replies
- transport failures, disabled feature state, or invalid requests should return structured error objects rather than free-form prose where practical
- result shapes should preserve fields such as `accepted`, `task_id`, `timed_out`, `found`, and `git_repo_keys` so callers can reason about behavior consistently across network and tool entry points

**Alternative considered:** simplify injected tool schemas into conversational free-form arguments. Rejected: that would make the tool layer drift from the documented Stylos API and create two different mental models for the same operations.

### Define the model-to-Zenoh/Stylos adapter path explicitly

The PRD must describe the full end-to-end path for injected per-instance Stylos tools, because exposing the tool schema alone is not sufficient for a working remote RPC feature.

Normative end-to-end path:

1. the model chooses an injected Stylos tool such as `stylos_query_status`, `stylos_request_talk`, `stylos_request_task`, `stylos_query_task_status`, or `stylos_query_task_result`
2. `themion-core` validates the tool call against the registered schema and forwards the invocation through the feature-gated Stylos tool invoker interface
3. `themion-cli` receives that invocation through the CLI-local Stylos capability bridge
4. the CLI bridge encodes the corresponding request body as CBOR matching the documented queryable contract
5. the CLI bridge issues a direct Zenoh query to the exact addressed per-instance key under `stylos/<realm>/themion/instances/<instance>/query/...`
6. the CLI bridge applies an explicit timeout for the direct query
7. the CLI bridge interprets direct-reply cardinality strictly:
   - zero replies → timeout, offline target, or no responder
   - one reply → normal success or structured application-level error from the addressed instance
   - more than one reply → protocol/configuration error for duplicate ownership of an exact per-instance key
8. the CLI bridge decodes the CBOR reply and returns a machine-readable JSON tool result back through the harness to the model

Normative adapter requirements:

- per-instance injected tools must work for non-local `instance` values on the mesh, not just for the current CLI process
- direct per-instance tool calls must not silently degrade into local-only checks when the addressed instance differs from `self.instance`
- direct per-instance tool calls must preserve the documented semantics of the corresponding queryable, including request and response fields
- direct per-instance tool calls must distinguish transport timeout/no-reply from a responder-side `not_found` or rejection payload
- direct per-instance tool calls should use exact per-instance keys rather than wildcard discovery-style selectors

**Alternative considered:** treat injected Stylos tools as local convenience helpers while leaving true remote use to external clients only. Rejected: it breaks the intended contract that the model-facing tool layer is an adapter over the same documented Stylos operation surface.

### Define expected queryable behavior explicitly

Each queryable should have a simple, explicit behavior contract.

#### `alive`

- key: `stylos/<realm>/themion/query/agents/alive`
- request body: empty or omitted in the initial version
- reply cardinality: one reply per responding instance
- injected tool behavior: accepts optional `exclude_self`, defaulting to `true`; filtering is based on the decoded reply payload `instance` field
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
- injected tool behavior: accepts optional `exclude_self`, defaulting to `true`; filtering is based on the decoded reply payload `instance` field
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
- injected tool behavior: accepts optional `remote` and optional `exclude_self`, defaulting to `true`; filtering is based on the decoded reply payload `instance` field
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
- `{ "exclude_self": false }` → include the caller's own instance in discovery results

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

- key: `stylos/<realm>/themion/instances/<instance>/query/status`
- request body: empty, omitted, or JSON filter object
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - empty request returns the full current process snapshot
  - `{ "agent_id": ... }` filters to one agent when found
  - `{ "role": ... }` filters to matching agents
  - unknown agent returns a structured not-found reply
  - malformed filters return a structured invalid-request reply
  - tool adapters that expose this operation should issue a direct Zenoh query to the addressed key and treat zero replies, one reply, and multiple replies distinctly

#### `talk`

- key: `stylos/<realm>/themion/instances/<instance>/query/talk`
- request body: JSON object with `agent_id`, `message`, and optional `request_id`
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - validate the target agent exists
  - validate the target agent can accept remote input under the initial rules
  - on success, enqueue the message through the normal local input path
  - return acknowledgement, not the final assistant answer
  - on failure, return a structured rejection reason
  - retries should be safe to reason about via `request_id`; if idempotent handling is not implemented yet, docs and callers should treat retries as potentially duplicate side effects

#### `tasks/request`

- key: `stylos/<realm>/themion/instances/<instance>/query/tasks/request`
- request body: JSON object with `task` and optional routing filters
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - evaluate candidate local agents on the addressed instance
  - apply `preferred_agent_id`, `required_roles`, and `require_git_repo` filters when present
  - choose deterministically when more than one candidate qualifies
  - on success, enqueue the request into the normal local agent-input path
  - allocate a stable `task_id` for later lookup
  - on failure, return a structured rejection such as `no_available_agent` or `invalid_request`
  - tool adapters that expose this operation should issue a direct Zenoh query to the addressed key and must not silently substitute a local-only check for a non-local instance target

#### `tasks/status`

- key: `stylos/<realm>/themion/instances/<instance>/query/tasks/status`
- request body: JSON object with `task_id`
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - return a structured not-found reply when the task ID is unknown or expired
  - otherwise return the current known task lifecycle state
  - do not block waiting for task progress
  - tool adapters that expose this operation should issue a direct Zenoh query to the addressed key and treat multiple replies as a protocol/configuration error rather than accepting the first one silently

Recommended lifecycle states:

- `queued`
- `running`
- `completed`
- `failed`
- `rejected`
- `expired`

#### `tasks/result`

- key: `stylos/<realm>/themion/instances/<instance>/query/tasks/result`
- request body: JSON object with `task_id` and optional `wait_timeout_ms`
- reply cardinality: one logical reply from the addressed instance
- expected behavior:
  - if the task is already completed or failed, return immediately with the terminal state
  - if the task is still pending and `wait_timeout_ms` is omitted or zero, return immediately with the current non-terminal state
  - if the task is still pending and `wait_timeout_ms` is positive, wait up to that timeout for a terminal state
  - if the timeout expires before terminal completion, return a normal non-error reply indicating the task is still pending and the wait timed out
  - the result query should not create a new task or alter task execution semantics
  - tool adapters that expose this operation should issue a direct Zenoh query to the addressed key and must distinguish responder timeout from a normal timed-out task wait reply

### Define "free" in terms of observable agent activity state, with explicit initial rules

The mesh needs a predictable answer to "who is free?" The first version should define a free agent using already-published activity/workflow state rather than introducing a separate scheduler-owned availability field.

Normative initial rule:

- an agent is considered free when its `activity_status` is `idle` or `nap`
- agents in `busy`, `waiting_tool`, `starting`, `error`, or any unknown future non-idle state are not free
- if future runtime work adds an explicit opt-out such as `accepts_task_requests = false`, the free filter should also respect that field

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

**Alternative considered:** persist all incoming requests and task state in SQLite immediately. Rejected: useful eventually, but too broad for the first queryable task-routing step.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/stylos.rs` | Add discovery queryables for `query/agents/alive`, `query/agents/free`, and `query/agents/git`; per-instance queryables for `status`, `talk`, `tasks/request`, `tasks/status`, and `tasks/result`; CBOR request/response payload structs; git remote normalization helpers for comparable repo identity; in-memory task lifecycle tracking; a prompt handoff channel for remote request delivery; a direct Zenoh query bridge for per-instance injected tools; and default self-exclusion for injected discovery-tool results using each decoded reply payload's `instance` field as the source of truth. |
| `crates/themion-cli/src/tui.rs` | Expose or reuse process/agent snapshot helpers needed by the query handlers, route remote talk/task submissions into the normal local agent execution path for the selected local agent, capture final assistant text for task results, and preserve existing activity-state reporting used to decide free-vs-busy agents. |
| `crates/themion-core/src/tools.rs` | Define and register feature-gated agent-injected Stylos tool schemas, including `exclude_self` on discovery tools, plus the feature-gated tool invoker hook used to dispatch those operations through CLI-provided runtime wiring. |
| `docs/architecture.md` | Document the richer Stylos query surface, direct per-instance tool behavior, and default self-exclusion for discovery tools. |
| `docs/engine-runtime.md` | Clarify how Stylos-originated talk/task requests enter the normal local agent-input path, how in-memory task lifecycle tracking supports later queries without moving harness logic into the transport layer, and how injected Stylos tools bridge from the harness into the CLI-local transport wiring. |
| `docs/README.md` | Keep this PRD indexed and update status/version to reflect the landed implementation. |

## Edge Cases

- a feature-enabled Themion process is alive but has no agents in a free state → its `free` discovery reply should be a successful empty result, while a per-instance `tasks/request` should return a structured rejection such as `no_available_agent`.
- discovery queries may receive replies from multiple instances → callers must aggregate them and identify each reply by instance.
- discovery tools run on a mesh with only the local instance visible and `exclude_self` omitted → the result should be an empty list rather than accidentally including self.
- reply key formatting differs from the original query key shape → self-exclusion should still work because filtering uses the decoded discovery reply payload `instance` field rather than reply-key parsing.
- discovery tools run with `exclude_self = false` → the caller's own instance should be included again.
- a `git` discovery request asks for a repo that is represented locally as ssh while the caller uses https, or vice versa → normalization should still match for supported hosts.
- caller sends `talk` to a busy or unknown agent → runtime should reject clearly.
- a per-instance injected tool is asked to target a different `instance` than the local process → the bridge should issue a direct Zenoh query to that exact key and return the remote reply if present.
- duplicate instance ownership of an exact per-instance RPC key appears on the mesh → direct callers should treat multiple replies as a protocol/configuration error rather than accepting an arbitrary first reply.

## Migration

This is an additive patch-level change for Stylos-enabled builds.

Migration expectations:

- non-Stylos builds are unchanged
- existing status heartbeat consumers continue to work
- new discovery-query consumers can ask mesh-wide agent questions without knowing `<instance>` first
- injected discovery tools now exclude the local instance by default unless `exclude_self=false` is provided
- new per-instance query consumers can target `status`, `talk`, `tasks/request`, `tasks/status`, and `tasks/result` on one chosen process
- request senders must treat `talk`, `tasks/request`, `tasks/status`, and `tasks/result` as best-effort, process-local operations in the first release
- the landed slice supports strict local `agent_id` execution targeting within the addressed process-local agent set and direct remote per-instance tool queries across the mesh

No database or provider migration is required for the initial implementation.

## Testing

- run `cargo check -p themion-cli` after implementation → verify: non-feature builds still compile with no Stylos dependency regressions.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the feature-enabled CLI compiles with the new queryables, git normalization helpers, task lifecycle tracking, request-routing helpers, and direct tool bridge.
- start multiple feature-enabled Themion instances and invoke `stylos_query_agents_alive` from one instance with no args → verify: the result omits the caller's own instance by default.
- validate discovery self-exclusion against replies whose transport keys do not mirror the original query path exactly → verify: filtering still excludes the local instance because it reads the decoded reply payload `instance` field.
- invoke `stylos_query_agents_alive` with `{ "exclude_self": false }` → verify: the result includes the caller's own instance again.
- invoke `stylos_query_agents_free` from one instance while another instance is idle → verify: only the other instance appears by default.
- invoke `stylos_query_agents_git` with and without `exclude_self=false` → verify: self-exclusion behavior matches the documented default while repo matching still works.
- invoke each injected per-instance Stylos tool against both the local instance and a different live instance on the mesh → verify: the tool issues a direct query to the addressed instance key, succeeds for the matching remote instance, returns timeout/offline when there is no responder, and treats multiple replies as a protocol/configuration error.
- query `stylos/<realm>/themion/instances/<instance>/query/status` with `{ "agent_id": "missing" }` → verify: the response is a structured not-found result.
- send a valid `talk` request to `stylos/<realm>/themion/instances/<instance>/query/talk` for an eligible agent → verify: the response acknowledges acceptance and the targeted local agent receives the message through the normal local input path.
- send a `tasks/request` when no free agent on the addressed instance matches the filters → verify: the response is a structured rejection such as `no_available_agent`.

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
- [x] enforce strict local `agent_id` execution targeting beyond the existing interactive-agent bridge
- [x] define feature-gated harness tool schemas for the matching Stylos operations in `themion-core`
- [x] provide a CLI-local capability bridge so per-instance Stylos tool calls can reach non-local instances through direct Zenoh queries to the addressed per-instance keys rather than short-circuiting to the current process only
- [x] add a direct per-instance query helper that uses exact instance keys, explicit timeout handling, and single-reply enforcement for direct RPC semantics
- [x] detect and surface multiple direct replies for one exact per-instance RPC key as a protocol/configuration error
- [x] distinguish clearly between responder timeout/no reply and a normal application-level `not_found` payload from the addressed instance
- [x] document and implement discovery-tool `exclude_self` with default `true`
- [x] apply discovery self-exclusion using each decoded reply payload `instance` field rather than transport reply-key parsing
- [x] document the injected Stylos tool names, schemas, and unavailable-state behavior
- [x] document the query namespace, CBOR payloads, git normalization rules, task lifecycle semantics, expected logical reply shapes, and discovery self-exclusion behavior in `docs/architecture.md`
- [x] document how remote requests enter normal local agent execution and how in-memory task lifecycle tracking supports later queries in `docs/engine-runtime.md`
- [x] validate with `cargo check -p themion-cli`
- [x] validate with `cargo check -p themion-cli --features stylos`
