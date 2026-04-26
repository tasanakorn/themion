# PRD-040: Debug Command for Themion Process, Thread, and Task Utilization

- **Status:** Implemented
- **Version:** v0.25.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-23

## Summary

- Themion currently has no built-in command to explain where its own process-level activity is coming from when CPU use looks high.
- Add a Themion-local debug command that reports process, thread, and task-utilization signals for this app only.
- Keep the first slice focused on observability for Themion's own runtime shape: TUI loop, tick loop, input loop, agent turn execution, event bridges, and optional Stylos background tasks.
- Do not promise exact per-Tokio-task CPU percentages; instead, report truthful built-in metrics and label approximations clearly.
- Prefer internal sampling and counters over shelling out to external profilers from the command itself.
- Keep this as a debug/diagnostic surface, not a general performance dashboard or cross-process profiler.

## Goals

- Give users and developers a built-in Themion command to inspect this app's own process/thread/task activity when debugging performance.
- Surface a simple, truthful snapshot of what long-lived runtime pieces are active inside the current Themion process.
- Report process-level and thread-level utilization signals that are available from the local OS/runtime context.
- Report task- or loop-level activity counters for important Themion runtime components such as TUI tick, input handling, redraws, agent turns, and Stylos background loops.
- Help distinguish likely hot sources such as redraw churn, tight async loops, heavy agent activity, or Stylos background work.
- Keep the output useful even when exact per-task CPU accounting is unavailable.
- Document the limits clearly so users do not confuse Tokio task activity with precise kernel CPU accounting.

## Non-goals

- No general-purpose system profiler for other processes.
- No requirement to expose exact per-Tokio-task CPU percentages, because Tokio does not provide that directly in a simple built-in way.
- No mandatory integration with `perf`, `tokio-console`, eBPF, or external tracing stacks in this first slice.
- No remote or cross-instance profiling surface over Stylos.
- No requirement to persist debug samples in SQLite.
- No redesign of Themion's runtime architecture just to support this command.
- No attempt to profile model-provider latency, network hops, or shell-subprocess CPU as if they were internal Tokio tasks.

## Background & Motivation

### Current state

Themion now documents a useful mental model of its runtime in `docs/architecture.md`: one process, a Tokio runtime, a central TUI event loop in TUI mode, spawned async tasks for input and ticks, agent execution tasks, event-bridge tasks, and optional Stylos background tasks.

That documentation is enough to explain the architecture, but it does not yet give the running app a built-in way to answer practical debugging questions such as:

- is the process busy because the TUI is redrawing too often?
- is one runtime thread hot?
- is the tick loop waking constantly while idle?
- is the app spending time in active agent work versus background Stylos work?
- is the observed heat coming from one internal loop spinning or from legitimate agent activity?

Today, users must leave the app and use external tools such as `top -H`, `ps -T`, `pidstat`, or `perf` to investigate. Those tools are still valuable, but they do not explain Themion's internal task/loop model by themselves.

### Why the built-in command should be Themion-local and truthful

Tokio tasks are cooperatively scheduled async tasks. They are not kernel threads, and Themion does not currently have built-in exact CPU accounting per Tokio task.

That means a proposed debug command should not claim more precision than the app can honestly measure. A useful built-in command can still provide strong diagnostics by combining:

- process-level metadata
- thread-level samples where available
- counters and timing around important Themion loops
- current agent/runtime state
- recent event rates and busy/idle indicators

This is enough to answer many real debugging questions without pretending that every Tokio task has an exact CPU percentage.

**Alternative considered:** add a command that simply shells out to `top -H` or `ps -T` and prints raw external output. Rejected: that is useful as a fallback technique, but it does not produce Themion-shaped insights or a stable in-app diagnostic contract.

### Why the first slice should stay inside `themion-cli`

The runtime pieces this command needs to observe are primarily CLI-local:

- the TUI draw loop
- the 150 ms tick task
- input event forwarding
- agent event bridges into the app loop
- optional Stylos status/query/subscriber loops
- app-local busy/idle state for the main interactive agent

Some core agent state may be included where helpful, but the highest-value debug view for this request is the process-local runtime shape already owned by `themion-cli`.

**Alternative considered:** start by building the feature in `themion-core` as a generic runtime profiler. Rejected: the most visible concurrency and wakeup structure the user wants to inspect is owned by the CLI runtime rather than the reusable core harness.

## Design

### Add a dedicated in-app debug command family

Themion should expose a user-invokable debug command in the TUI command surface for runtime-utilization inspection.

Normative direction:

- add a dedicated debug command family rather than overloading unrelated existing commands
- the first implemented entry point may be a single command such as `/debug runtime` or a narrow family such as `/debug runtime`, `/debug threads`, and `/debug hot`
- the command output should be rendered as Themion-owned diagnostic text in the normal app UI rather than requiring the user to leave the app
- the command should be explicitly described as diagnostic/debug output, not normal chat content

This keeps the feature discoverable and consistent with Themion's existing command-driven local controls.

**Alternative considered:** add only a hidden keybinding. Rejected: the request is for a debug command, and command output is easier to reuse, document, and extend.

### Report three layers: process, threads, and Themion task activity

The command output should be organized around three layers of observability.

Normative direction:

- process layer: show basic process identity and app-wide state such as PID, runtime mode, Stylos enabled/disabled state, and whether an agent turn is currently active
- thread layer: show available thread-level sampling or snapshot data for this process only
- task/activity layer: show Themion-owned counters and timing for long-lived runtime loops and important spawned work categories

This gives users a bridge from OS-visible symptoms to Themion-visible runtime structure.

**Alternative considered:** report only a single flat block of counters. Rejected: the user explicitly asked about process, thread, and task utilization, and the output should preserve those levels.

### Use internal counters and sampled active-time metrics for task-level signals

Themion should instrument important loops and handlers with lightweight counters and timing rather than pretending to have exact per-task CPU metering.

Normative direction:

- maintain internal counters for wakeups, handled events, and recent activity on important long-lived loops
- maintain approximate active-time timing for work performed inside those loops where practical
- report these values as activity or busy-time metrics, not as exact kernel CPU percentages unless the implementation truly has that backing
- keep the instrumentation lightweight enough to leave enabled in normal debug-capable builds

Examples of candidates include:

- TUI draw count and average/max draw duration
- tick-loop wake count and handler duration
- input-event count by type
- agent-event bridge counts
- agent-turn started/completed counters and current running duration
- Stylos status publish wake count and timing
- Stylos query handling count and timing
- Stylos command/prompt/event bridge counts

This gives Themion a trustworthy notion of "what has been active" even when exact task CPU is unavailable.

**Alternative considered:** estimate exact task CPU by assigning time to each Tokio task after every poll. Rejected: that is invasive, potentially misleading, and much larger in scope than needed for a first practical debug surface.

### Provide optional per-thread CPU sampling where locally available

Themion should expose thread-level utilization signals for its own process when the local platform makes them reasonably available.

Normative direction:

- thread reporting should be scoped to the current Themion process only
- on Linux, implementation may sample `/proc/self/task/<tid>/stat` or an equivalent local source to compute recent user/system CPU deltas per thread
- if exact or sampled thread CPU is unavailable on a platform, the command should still return a useful snapshot with clear `unavailable` wording rather than failing entirely
- thread output should not be confused with Tokio task output; the command should describe these as OS threads

This gives users a truthful answer to "is one thread hot?" without claiming that each Tokio task maps 1:1 to a thread.

**Alternative considered:** make Linux thread sampling mandatory for the whole feature. Rejected: the command should still be useful even when only task/activity counters are available.

### Keep the debug command focused on Themion-owned runtime categories

The task/activity section should use categories that match Themion's architecture rather than generic executor internals.

Normative direction:

- use names that match the documented runtime model in `docs/architecture.md`
- distinguish TUI-local loops, agent-turn activity, and Stylos-local background loops clearly
- avoid generic labels that imply exact Tokio scheduler internals if the app is really reporting higher-level loop counters
- if the implementation tracks categories rather than individual spawned task instances, the output should say so plainly

Good category examples include:

- `tui.draw`
- `tui.tick`
- `tui.input`
- `agent.turn`
- `agent.event_bridge`
- `stylos.status_publish`
- `stylos.query_server`
- `stylos.cmd_subscriber`
- `stylos.bridge.prompt`
- `stylos.bridge.event`

This keeps the feature aligned with the user's request to focus on Themion itself.

**Alternative considered:** expose raw Tokio worker statistics only. Rejected: users debugging Themion need app-shaped categories, not just executor-shaped ones.

### Show both current state and recent-window samples

A single instantaneous snapshot is not always enough to diagnose a hot loop. The debug command should prefer a small recent-window summary where practical.

Normative direction:

- report current boolean/stateful information such as whether an agent turn is active right now
- also report recent-window counters or rates over a bounded time span where the implementation samples them
- if a first implementation uses cumulative counters plus timestamps rather than rolling windows, the output should still include enough timing context for readers to interpret them

This helps distinguish "currently running a turn" from "was busy five minutes ago but is idle now."

**Alternative considered:** only dump cumulative totals since process start. Rejected: totals alone are weak for hotspot debugging because they blur current activity with old activity.

### Keep output textual, compact, and copyable

The debug result should be readable in a terminal transcript and easy to paste into an issue or chat.

Normative direction:

- output should be plain text or similarly copy-friendly structured text
- avoid large tables that wrap badly in narrow terminals
- include units explicitly for timings, rates, and sampled windows
- label any approximate value as approximate
- if some sections are unavailable, show that explicitly rather than omitting them silently

This keeps the feature useful in real terminal debugging sessions.

**Alternative considered:** build a dedicated popup dashboard first. Rejected: a text-based command is simpler, more scriptable, and easier to land as an initial slice.

### Document the limits of task-utilization claims explicitly

The docs and command wording should be explicit that thread CPU and task activity are different kinds of measurements.

Normative direction:

- process/thread sections may report OS-level CPU data where available
- task/activity sections should be described as Themion loop/task activity metrics, not exact kernel CPU accounting
- docs should explain that Tokio tasks are cooperatively scheduled and do not have built-in exact per-task CPU percentages in this runtime
- the output should avoid shorthand that would mislead users into reading activity time as exact CPU percent unless that exact metric is truly implemented and supported

This protects the feature from becoming a misleading pseudo-profiler.

**Alternative considered:** use the phrase "task CPU" everywhere as a convenience shorthand. Rejected: that would be architecturally inaccurate for the proposed implementation.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Add the user-facing debug command entry point, wire command handling into the existing TUI command surface, and expose current app/runtime state needed for the snapshot. |
| `crates/themion-cli/src/tui.rs` or nearby CLI-local helper module | Add lightweight counters/timers for TUI draw, tick, input handling, agent event bridging, and command output formatting. |
| `crates/themion-cli/src/stylos.rs` | Add optional counters/timers for Stylos-local loops such as status publish, query handling, command subscription, and bridge activity when the feature is enabled. |
| `crates/themion-cli/src/main.rs` | Keep process-level context such as PID and runtime mode available where needed for debug reporting. |
| `docs/architecture.md` | Document the new debug command and explain what process/thread/task utilization means in Themion's async runtime model. |
| `docs/engine-runtime.md` | Clarify that the debug command reports a mix of OS-level process/thread signals and Themion-owned task activity metrics rather than exact per-Tokio-task CPU. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Edge Cases

- the app is idle in TUI mode with no active turn → verify: the command still shows useful baseline process and loop activity rather than an empty report.
- the app is busy in an active agent turn → verify: current agent-turn state and recent loop/task activity make that obvious.
- the app is built without the `stylos` feature → verify: Stylos sections are omitted or marked disabled cleanly without errors.
- the app is built with `stylos` but Stylos is disabled in config → verify: the report distinguishes feature availability from runtime-enabled state.
- the local platform cannot provide thread CPU sampling → verify: the process and task/activity sections still work and the thread section reports `unavailable` clearly.
- one runtime thread is hot because a cooperative async task is not yielding → verify: the thread section can show a hot thread while the task/activity section still helps narrow likely Themion loop categories.
- cumulative counters are large in a long-lived process → verify: the output includes timestamps, recent-window rates, or enough context that totals are interpretable.
- the command is invoked repeatedly while the app is already hot → verify: collecting the snapshot does not itself introduce disproportionate extra work or redraw churn.

## Migration

This is an additive debug feature.

Expected rollout shape:

- add the command without changing existing harness, workflow, provider, or Stylos behavior
- keep the instrumentation lightweight and local to runtime areas that already exist
- make unavailable metrics degrade clearly rather than adding platform-specific hard failures
- if a later PRD adds deeper tracing or external-profiler integration, it should extend this command family rather than redefining the meaning of the initial output silently

No SQLite schema migration or config migration is required for the first slice unless implementation later decides to add optional debug toggles, which are not required by this PRD.

## Testing

- start Themion in normal TUI mode and run the new debug command while idle → verify: the output includes process identity, thread snapshot status, and Themion task/activity sections with sensible idle values.
- start an active agent turn and run the debug command during the turn → verify: the report shows current busy state and higher recent activity for relevant agent/TUI categories.
- trigger representative input, redraw, and tick activity → verify: the reported counters or rates move in the expected categories.
- build and run without the `stylos` feature → verify: the command still works and does not reference unavailable Stylos internals.
- build and run with the `stylos` feature enabled → verify: Stylos runtime categories appear only when relevant and report sensible values.
- run on a platform or environment where thread CPU sampling is unavailable → verify: the command degrades gracefully with explicit unavailable wording.
- invoke the command repeatedly in a tight loop by a user → verify: the command remains stable and does not noticeably distort the app's own utilization picture.
- review updated docs → verify: they explain the difference between OS thread CPU signals and Themion task-activity metrics clearly.
- run `cargo check -p themion-cli` and `cargo check -p themion-cli --features stylos` after implementation → verify: the debug command and optional Stylos instrumentation compile cleanly in both configurations.

## Implementation checklist

- [x] define the debug command shape and user-facing invocation syntax in `themion-cli`
- [x] add process-level snapshot reporting for the current Themion process
- [x] add thread-level reporting with clear degradation when sampled CPU data is unavailable
- [x] add lightweight task/activity counters and timings for key TUI runtime loops
- [x] add optional Stylos loop counters and timings for feature-enabled builds
- [x] format command output as concise copyable diagnostic text with explicit units and approximation labels
- [x] update `docs/architecture.md` and `docs/engine-runtime.md` to explain the new debug surface and its measurement limits
- [x] update `docs/README.md` with this PRD entry

## Implementation notes

The implemented slice landed with these concrete behaviors:

- added `/debug runtime` in `crates/themion-cli/src/tui.rs` as the first built-in diagnostic command for Themion-local runtime activity
- command output now includes process identity, app busy/workflow state, Linux thread snapshots from `/proc/self/task` when available, and explicit fallback wording when thread CPU sampling is unavailable
- TUI runtime now tracks lightweight activity counters for draw, tick, input, commands, agent events, incoming prompts, shell completions, and agent turn start/completion
- draw timing is reported as approximate average/max handler duration, and task lines are labeled as activity metrics rather than exact Tokio task CPU percentages
- Stylos-enabled builds now expose matching lightweight counters for status publishing, query handling, and bridge event categories through the same debug output
- docs should describe the command as Themion-owned diagnostic observability, not as an exact per-Tokio-task CPU profiler
