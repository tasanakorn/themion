# PRD-114: Web UI Model Activity Status Indicators

- **Status:** Implemented
- **Version:** v0.71.1
- **Scope:** `themion-cli`, `themion-cli-web-ui`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-09

## Summary

- The Web UI currently gives weak feedback while an agent waits on the model.
- Add clear browser-visible progress for model turns: preparing, waiting, streaming, tool work, and finishing.
- Reuse runtime-owned activity state from `AppSnapshot` instead of adding browser-only inference.
- Show all active agents in a lightweight animated activity strip at the bottom of the transcript.
- Keep the work focused on status visibility, not a wider Web UI redesign.

## Goals

- Make Web UI prompt submission feel acknowledged immediately after the runtime accepts it.
- Show the user that Themion is still working during slow provider calls, long first-token latency, and tool/model handoff delays.
- Render concise labels for the existing runtime activity states:
  - preparing request
  - waiting for model
  - receiving/streaming response
  - running tool
  - waiting after tool
  - finalizing
  - idle
- Show every agent that currently has activity, so multi-agent work is not hidden behind one global status.
- Recover the current activity after browser refresh or websocket reconnect from runtime-owned snapshots.
- Keep web as an I/O surface over `themion-cli --web`; do not make it a second runtime owner.

## Non-goals

- Do not redesign the full Web UI layout or navigation.
- Do not change provider streaming, tool-call execution, or agent scheduling behavior.
- Do not parse provider traffic in the browser to infer model state.
- Do not depend on TUI rendering code or move TUI display logic into the web crate.
- Do not add a new long-term web runtime outside `themion-cli --web`.
- Do not revive `crates/themion-web` as the implementation target.

## Background & Motivation

### Current state

The TUI already has useful activity feedback during a turn. Runtime state has an `AgentActivity` concept with labels for states such as preparing request, waiting for model, receiving response, running tool, waiting after tool, and finalizing. `AppSnapshot` already exposes a coarse `activity_status` and runtime timestamps such as `activity_changed_at_ms` through the web status shape.

The Web UI consumes `/api/status`, `/api/agents`, `/api/transcript`, and the shared `/api/ws` websocket. It has `busy`, `activity_status`, `local_agents`, runtime summary data, and transcript/tool event rendering. But the browser UI does not make model waiting or response progress visible enough. A submitted prompt can look idle until transcript text, a tool row, or a later status refresh appears.

This is most noticeable when the model takes a long time to return the first token, when a tool finishes and the agent waits for the next model step, or when network/provider latency is high.

### Why this matters now

PRD-106 makes the browser a first-party local surface under `themion-cli --web`. PRD-108 improved web transcript attribution. The next basic usability gap is turn-progress confidence.

The user should be able to answer three questions at a glance:

- Did Themion accept my prompt?
- Which agents are working?
- For each active agent, is it waiting for the model, streaming, running a tool, or finishing?

## Design

### 1. Render an active-agent strip at the bottom of the transcript

The Web UI should show an activity strip at the bottom of the transcript whenever one or more agents are active. This strip is part of the transcript area, not a separate status-page-only widget.

Required behavior:

- place the activity strip at the bottom of the transcript, near the composer, so it is visible during normal chat use
- show one chip per active agent, for example `master · waiting for model` and `smith-1 · running tool`
- show all agents that currently have activity; do not collapse them into only the selected, primary, or most recent agent
- omit idle agents from the active strip by default
- when no agents are active, hide the strip or show a low-emphasis idle state if that is clearer for layout stability
- use lightweight animation only for active chips, for example pulsing dots or a small spinner
- avoid layout jumps when chips appear, disappear, or change labels
- stop animation for an agent when the runtime reports that agent idle, completed, interrupted, or failed

The strip does not need to match the TUI visual style. It must match the TUI's useful meaning: the user can see which agents are alive and what each one is waiting on.

### 2. Use runtime-owned activity as the source of truth

The browser must render activity from runtime-owned data.

Required behavior:

- use runtime-owned per-agent activity where available; use session-level `AppSnapshot.activity_status` only as a fallback for single-agent or clearly attributed activity
- if the existing web projection is too coarse, extend the runtime-owned snapshot/projection rather than guessing in `themion-cli-web-ui`
- keep browser-local state limited to presentation concerns such as animation phase, selected tab, and reconnect display
- after reconnect or refresh, recover the full active-agent strip from `/api/status` or the next runtime websocket snapshot
- do not use “prompt submitted but no transcript row yet” as the canonical active-state signal

Implementation should start by tracing `AgentActivity`, `activity_status_value`, `publish_runtime_snapshot`, `build_status_response`, and the websocket runtime status payload. The preferred implementation is to expose the existing runtime activity more clearly, not to create a separate web-only state machine.

### 3. Expose per-agent activity for every active agent

The existing `activity_status` is session-level. That is not enough for the required bottom-of-transcript strip because the strip must show every active agent at the same time.

Required behavior:

- add per-agent activity fields to the web projection when the current payload cannot identify each active agent's current status
- each `WebAgentStatus` should expose optional current activity when runtime state has it
- the browser should build the strip from agents with active per-agent activity, not from only `primary_agent_id` or selected `active_agent`
- session-level `activity_status` may remain as compatibility or a single-agent fallback, but it must not hide other active agents
- do not attribute a session-level status to an agent unless the runtime snapshot identifies that agent as the owner

Recommended optional `WebAgentStatus` fields:

```text
activity_status?: string
activity_label?: string
activity_changed_at_ms?: number
```

Use milliseconds for machine-consumed timestamps.

### 4. Map runtime states to browser labels

The browser should convert stable runtime statuses to short human labels.

Recommended mapping:

| Runtime status | Browser label |
| --- | --- |
| `preparing` | `Preparing request` |
| `waiting-model` | `Waiting for model` |
| `streaming ...` | `Receiving response` |
| `running-tool` | `Running tool` |
| `waiting-after-tool` | `Waiting for model` |
| `finalizing` | `Finalizing` |
| `idle` | `Idle` |
| `nap` | `Idle` or `Idle for a while` |
| unknown active value | sentence-case fallback from runtime text |

The UI may show streaming counters when available, but the default label should stay short. Detailed counters belong in the status/debug area, not the bottom transcript activity strip.

### 5. Publish status updates promptly

The indicator is only useful if it updates at the same moments that TUI status changes.

Required behavior:

- when runtime activity changes, the web status snapshot or websocket runtime event should update promptly
- the browser should not need to poll aggressively to see normal activity transitions
- `/api/status` remains useful as refresh/reconnect recovery
- websocket-delivered status should be enough for live UI updates after initial load

If the current websocket runtime payload does not carry the status fields, extend the payload rather than adding a second web-only channel.

### 6. Preserve layering and scope

This change belongs under the CLI-owned web direction.

Required behavior:

- `app_state.rs` remains the owner of runtime activity truth
- `web.rs` projects that truth into status responses and websocket messages
- `crates/themion-cli-web-ui/src/lib.rs` renders labels, chip placement, and animation
- TUI code can be used as a behavior reference only
- no new implementation work should target `crates/themion-web`

**Alternative considered:** start a spinner in the browser immediately on submit and stop it when any transcript row arrives. Rejected because it fails on reconnect, multi-agent activity, tool-only transitions, provider delays, and errors that do not produce a transcript row.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/app_state.rs` | Reuse `AgentActivity`, `activity_status_value`, and snapshot publication; expose enough per-agent activity to show every active agent. |
| `crates/themion-cli/src/web.rs` | Ensure `WebStatusResponse`, `WebAgentsResponse`, websocket runtime status, and `WebAgentStatus` expose the activity fields required by the browser. |
| `crates/themion-cli-web-ui/src/lib.rs` | Add activity label mapping, bottom-of-transcript activity strip rendering, animation classes, reconnect recovery behavior, and focused helper tests where practical. |
| `crates/themion-cli/src/tui.rs` | Reference existing behavior only; avoid unrelated TUI changes. |
| `docs/prd/prd-114-web-ui-model-activity-status-indicators.md` | Track the requirement and implementation notes. |
| `docs/README.md` | List status/version/scope for this PRD. |

## Edge Cases

- model call has long first-token latency → verify: Web UI shows active waiting status before assistant text appears.
- tool finishes and the agent asks the model again → verify: status changes from running tool to waiting for model instead of disappearing.
- streaming starts → verify: status changes to receiving response or otherwise makes streaming progress visible.
- user refreshes mid-turn → verify: `/api/status` restores the current activity chip without waiting for a new transcript row.
- websocket reconnects mid-turn → verify: the next status snapshot/event replaces stale browser state.
- multiple agents are active → verify: the bottom transcript strip shows one chip for each active agent and does not show another agent's activity under the wrong label.
- turn is interrupted or fails → verify: active animation stops and the final idle/error state is visible or cleared promptly.
- runtime status is unknown to the browser mapping → verify: the UI shows a safe fallback label and does not crash.

## Migration

This is an additive UX change. No database migration is required.

The PRD uses a patch target because it improves an existing Web UI surface without changing public configuration or persistent data. If implementation adds a new public websocket/status contract that callers depend on, the implementation note should confirm whether the release target remains patch scope or should become minor scope.

## Testing

- submit a prompt in Web UI with a slow model response → verify: the bottom transcript activity strip appears before transcript output arrives.
- compare TUI and Web UI during a normal model turn → verify: both surfaces clearly show waiting/receiving progress.
- trigger a tool-using turn → verify: status moves through running tool and waiting-after-tool without stale animation.
- refresh browser during active turns → verify: `/api/status` restores all active-agent chips.
- reconnect websocket during an active turn → verify: live status updates resume and stale local state is replaced.
- run with multiple active local agents → verify: the strip shows all active agents and each busy/activity label belongs to the correct agent.
- interrupt or fail a turn → verify: active animation stops promptly.
- run focused web projection tests in `themion-cli` → verify: status responses/websocket payloads include the expected activity fields.
- run `cargo test -p themion-cli-web-ui` → verify: UI status mapping and rendering helpers pass.
- run `cargo check -p themion-cli` → verify: default CLI/web integration compiles.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build compiles.


## Implementation Notes

Implemented in v0.71.1. The landed slice adds runtime-owned per-agent activity tracking keyed by agent session, publishes per-agent activity fields through `AppSnapshotAgent` and `WebAgentStatus`, and renders all active agents in a bottom-of-transcript Web UI activity strip.

The Web UI maps stable runtime status values to short labels, filters idle agents out of the strip, restores active chips from `/api/status` or websocket-delivered status snapshots, and uses `activity_changed_at_ms` as chip metadata.

Validation run for this slice:

- `cargo check -p themion-cli-web-ui`
- `cargo test -p themion-cli-web-ui`
- `cargo check -p themion-cli`
- `cargo test -p themion-cli web::tests::tool_done_merges_into_previous_tool_call`
- `cargo check -p themion-cli --all-features`
- `cargo check -p themion-core`
- `cargo check -p themion-core --all-features`

## Implementation checklist

- [x] trace `AgentActivity`, `activity_status_value`, `publish_runtime_snapshot`, and web status/websocket projection paths
- [x] expose per-agent activity in `WebAgentStatus` or an equivalent web projection so all active agents can be shown
- [x] expose any missing runtime-owned activity fields through `web.rs`
- [x] add Web UI activity-label mapping and a visible animated bottom-of-transcript activity strip
- [x] make refresh and websocket reconnect recover from runtime snapshots
- [x] add focused tests for status mapping and web projection behavior
- [x] update PRD/docs status notes after implementation lands
