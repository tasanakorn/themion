# Temporary bug log: Stylos local-agent board targeting / tmux inspection

Status: in progress
Owner: Themion agent
Scope: temporary investigation log for reproducing, diagnosing, and fixing local-agent targeting failures on Stylos-enabled instances

## Problem summary

On a Stylos-enabled Themion instance, `local_agent_create` can succeed, but later operations that depend on querying that agent through Stylos may fail. In the reproduced case, a newly created local worker agent was not visible through `stylos_query_status`, and board assignment to that agent failed.

This document is a temporary step-by-step bug log. We should update it as we learn more and as fixes land.

## Goal

- reproduce the bug reliably
- record each test step and result
- identify the exact failing layer
- implement the smallest correct fix
- record how to inspect the target runtime through tmux during debugging

## Runtime requirement clarification

Important clarified requirement:
- this must work from the main runtime state
- it must not depend on TUI correctness for agent visibility
- TUI can expose or trigger behavior, but it must not be the source of truth for Stylos status publication or board-target resolution

Current and future fixes should follow the runtime-ownership rule from `AGENTS.md`: TUI may trigger or display behavior, but hub/app-state/runtime modules must own roster mutation, status publication inputs, and board-target resolution state.

## Working hypothesis

The runtime roster updates after `local_agent_create`, but the Stylos snapshot used by query/board operations is stale or not refreshed. As a result, the instance can know about the new local agent internally while Stylos-facing queries still only expose the earlier snapshot.

## Environment notes

Primary reproduced target instance:
- `vm-02:1200006`

Related local instance used during investigation:
- `vm-02:1187977`

Observed repository/runtime context during testing:
- `cargo run -p themion-cli --all-features`
- Stylos peer mode enabled

## How to access tmux for debugging

These commands were verified from the shell environment available to the coding agent.

### 1. List panes

```sh
tmux list-panes -a -F '#S:#I.#P #{pane_current_command} #{pane_active} #{pane_title}'
```

Purpose:
- discover which tmux panes exist
- identify the active Themion pane

Example observed output:

```text
0:0.0 target/debug/themion 1 vm-02
0:0.1 target/debug/themion 0
```

### 2. Capture recent pane output

```sh
tmux capture-pane -pt 0:0.0 -S -200
```

Purpose:
- inspect the recent terminal contents of a running Themion instance
- confirm prompts, tool calls, task output, and error messages without attaching interactively

Helpful variant:

```sh
tmux capture-pane -pt 0:0.0 -S -120 | tail -n 80
```

Use this when only the most recent section is needed.

### tmux debugging guidance

- use `tmux list-panes` first so we do not assume the pane id
- prefer `capture-pane` over attaching when we only need read-only inspection
- log important findings in this file so the debugging state is durable
- beware pane confusion: the controller instance and the target instance can both be running `target/debug/themion`

## Reproduction log

### Step 1: verify Stylos can see the target instance

Action:
- queried status for `master` on `vm-02:1200006`

Result:
- success
- Stylos query returned `found: true` for `master`

Conclusion:
- Stylos transport is working
- the instance snapshot provider is available at least for the existing master agent

### Step 2: create a local worker on the target instance

Action:
- submitted a remote task to `vm-02:1200006`
- that task used `local_agent_create`

Observed result:
- created agent id: `smith-2`

Conclusion:
- local agent creation succeeds on the target instance

### Step 3: query the newly created agent through Stylos

Action:
- queried status for `smith-2` on `vm-02:1200006`

Observed result:
- `found: false`
- `error: not_found`

Conclusion:
- the newly created agent is not exposed through the queryable Stylos status snapshot
- this strongly suggests a snapshot refresh or live-roster publication issue

### Step 4: attempt board assignment to the new local agent

Action:
- submitted a remote task asking the target instance to assign a board note to `smith-2`

Observed result:
- task completed with tool failure
- returned tool-side failure text rather than a successful board note creation

Conclusion:
- board assignment is consistent with the missing-query visibility problem
- likely failure path: board target validation cannot resolve `smith-2`

## Findings so far

1. `master` is visible through Stylos on `vm-02:1200006`
2. `local_agent_create` can create `smith-2`
3. `smith-2` is not visible through `stylos_query_status` immediately afterward
4. board assignment to the new agent fails consistently with that missing visibility
5. therefore the first confirmed bug is not generic Stylos unavailability; it is stale or incomplete publication of the local agent roster

## Confirmed code finding

Earlier reproduction work identified stale snapshot behavior on the TUI path, but per `AGENTS.md` this should be treated as an ownership smell rather than the desired fix location. The durable conclusion is that agent-roster truth and Stylos-published/queryable status must move under hub/app-state/runtime ownership, with TUI reduced to display/input plumbing.

## Suspected code locations

### Runtime ownership / publication path

Files:
- `crates/themion-cli/src/app_runtime.rs`
- `crates/themion-cli/src/app_state.rs`
- `crates/themion-cli/src/stylos.rs`

Current suspicion:
- runtime-owned roster/status publication is incomplete, so Stylos still lacks a fully runtime-owned source of truth for local-agent visibility

Why this matters:
- if roster mutation and published/queryable status are not derived from the same runtime-owned snapshot, newly created workers can diverge from what Stylos answers

### Snapshot content builder

File:
- `crates/themion-cli/src/app_runtime.rs`

Relevant function:
- `build_live_stylos_status_snapshot(...)`

Why this matters:
- this is a better architectural home for runtime-owned status projection than `tui.rs`, provided its inputs come from hub/app-state-owned roster/state rather than UI-owned handles

### Board target validation

File:
- `crates/themion-cli/src/stylos.rs`

Why this matters:
- board note creation appears to validate the destination agent against the current snapshot before accepting the note
- this is likely where `smith-2` becomes `not_found`

## Proposed step-by-step fix plan

1. trace the runtime-owned roster mutation path for `local_agent_create` / delete
2. verify where Stylos status/query handlers obtain their current roster/status truth
3. move publication/query inputs toward a hub/app-state-owned cheap-clone snapshot rather than a TUI-owned roster view
4. rerun the reproduction:
   - query `master`
   - create a new worker
   - query the new worker
   - assign a board note to the new worker
5. record results here

## Fix log

### Attempt 1: install a closure that rebuilt more data at provider-call time

Action:
- patched `install_stylos_snapshot_provider()` to clone more state into the provider closure and rebuild snapshot content there

Result:
- compile succeeded
- relaunch of the target pane later panicked at runtime with:
  - `Cannot start a runtime from within a runtime`

Conclusion:
- that variant was not safe in the current async/runtime context
- reverted that part immediately

### Attempt 2: TUI-side snapshot refresh after local roster mutations

Action:
- earlier investigation tried a TUI-driven refresh after local roster changes

Result:
- this may relieve a stale-snapshot symptom temporarily, but it does not satisfy the repository architecture guide

Current interpretation:
- do not treat a TUI-owned refresh as the architectural fix
- use it only as temporary evidence that stale publication was part of the symptom while continuing the runtime-ownership refactor

## Validation checklist

- [x] reproduce on `vm-02:1200006`
- [x] confirm `master` query works
- [x] confirm new worker creation works
- [x] confirm new worker query fails
- [x] confirm current code used a frozen snapshot provider
- [x] confirm stale-publication symptoms can be influenced by snapshot refresh timing
- [x] validate crate build with `--features stylos`
- [x] validate crate build with `--all-features`
- [ ] rebuild/relaunch the target instance with the safe patch
- [ ] validate new worker query succeeds after patch
- [ ] validate board assignment succeeds after patch
- [ ] complete runtime-owned status publication and roster ownership
- [ ] update this log with final root cause and fix notes

## Important blocker discovered during runtime testing

The repository already has other uncommitted changes in:
- `crates/themion-cli/src/app_runtime.rs`
- `crates/themion-cli/src/stylos.rs`

That means we must be careful not to overwrite or accidentally bundle unrelated in-progress work while continuing the fix and relaunch cycle.

## Notes for future updates

When updating this file, keep entries brief and factual:
- what was tested
- what command/tool was used
- what result happened
- what that result means
- what next action follows from it
