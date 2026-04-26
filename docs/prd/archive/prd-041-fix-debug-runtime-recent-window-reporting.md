# PRD-041: Fix `/debug runtime` Recent-Window Counter and Rate Reporting

- **Status:** Implemented
- **Version:** v0.25.1
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-23

## Summary

- The current `/debug runtime` output labels one section as a recent window, but its activity counts are still lifetime totals.
- That makes the displayed per-second rates incorrect because they divide cumulative process-start counters by only the recent wall-time window.
- Fix the command so recent-window counts and rates are computed from snapshot deltas, not from raw cumulative totals.
- Keep lifetime counters if they are still useful, but label them explicitly as lifetime values instead of mixing them into the recent-window section.
- Keep the command truthful: it should report recent Themion activity metrics, not misleading pseudo-hotspot numbers.

## Goals

- Make the `/debug runtime` recent-window section numerically correct.
- Ensure counts shown under a recent-window heading are derived from the same bounded window.
- Ensure per-second rates are computed from delta counts over delta wall time.
- Preserve the lightweight in-app instrumentation model introduced by PRD-040.
- Keep the output copyable and easy to interpret during real debugging.
- Update docs so they describe the fixed recent-window semantics accurately.

## Non-goals

- No redesign of the whole `/debug` command family.
- No change to the underlying decision to use lightweight counters rather than exact per-task CPU accounting.
- No requirement to add a persistent metrics store or SQLite history for runtime snapshots.
- No requirement to implement exact OS thread CPU percentages in this PRD.
- No broad TUI or Stylos refactor unrelated to the recent-window math bug.
- No mandatory dashboard or visual charting surface.

## Background & Motivation

### Current state

PRD-040 added `/debug runtime` in `themion-cli` as a lightweight process/thread/task-activity diagnostic command. The implemented command keeps cumulative counters in memory and also stores a small deque of recent snapshots.

However, the current recent-window output is internally inconsistent:

- the command prints a bounded recent window such as `recent window=2.25s`
- the activity counts shown below that are taken from the latest cumulative snapshot rather than the difference between the earliest and latest snapshots in that window
- the rate lines then divide those cumulative totals by only the recent window duration

This can produce obviously impossible output, such as thousands of draws or ticks per second while the app is effectively idle.

That makes the command misleading at exactly the moment it is supposed to help diagnose runtime heat or idle churn.

### Why this should be fixed as a patch-level debug correctness issue

The problem is not that the command lacks more advanced profiling. The problem is that a user-visible section currently claims to describe recent-window activity while actually showing process-lifetime totals.

That is a correctness bug in an existing diagnostic surface, so the right first step is a narrow fix:

- keep the command shape
- keep the lightweight counters
- fix the math and labels so the output is truthful

**Alternative considered:** leave the current behavior and document that the counts are lifetime totals. Rejected: the current section is explicitly presented as a recent window, so documentation alone would not fix the misleading output.

### Why the fix should preserve both windowed and lifetime interpretation when useful

Lifetime totals can still be useful for understanding long-running sessions. The bug is not the existence of cumulative counters; it is presenting them under recent-window semantics.

A good fix can therefore do one of these:

- show only recent-window deltas in the recent section, or
- show both recent and lifetime values, with each labeled clearly

Either approach is acceptable as long as a reader cannot confuse lifetime totals with recent-window activity.

**Alternative considered:** remove cumulative counters entirely. Rejected: cumulative values may still be useful, and removing them is unnecessary for fixing the bug.

## Design

### Compute recent-window activity from snapshot deltas

The recent-window section should be derived from the difference between two runtime snapshots: the oldest retained snapshot in the current window and the newest one.

Normative direction:

- keep storing cumulative counters in snapshots
- when rendering recent-window activity, subtract the earliest snapshot counters from the latest snapshot counters
- use the same earliest/latest timestamps to compute the wall-time duration for that delta window
- if the latest snapshot timestamp is not newer than the earliest snapshot timestamp, report the recent window as unavailable rather than fabricating numbers

This preserves the current lightweight instrumentation model while making the reported recent activity numerically meaningful.

**Alternative considered:** reset counters after every snapshot so each snapshot is already window-local. Rejected: cumulative counters are simpler and more robust; delta rendering is enough.

### Restrict recent-window rates to delta counts from the same window

Per-second rates should be calculated only from counts observed within the same bounded recent window.

Normative direction:

- rate lines under the recent-window section must use delta counts, not lifetime totals
- the denominator must be the corresponding delta wall time for the same earliest/latest snapshot pair
- if the window is too short or unavailable, the output should degrade clearly rather than showing inflated or nonsensical rates

This makes values such as draw, tick, input, and agent-event rates interpretable.

**Alternative considered:** smooth rates with ad hoc heuristics or moving averages before fixing the basic math. Rejected: the first requirement is correctness, not extra smoothing.

### Label lifetime values explicitly if they remain in the output

If the command continues to show cumulative process-start totals, those lines should be labeled as lifetime totals or since-start totals.

Normative direction:

- any cumulative counts shown alongside recent-window output must be labeled explicitly as lifetime or since-start
- recent-window headings must contain only recent-window-derived data
- wording should make it impossible to read a lifetime total as a recent sample accidentally

This keeps the command readable without sacrificing accuracy.

**Alternative considered:** keep the current generic `activity counts` label. Rejected: that label is too ambiguous once both recent and lifetime views are possible.

### Apply the same recent-window delta treatment to task-activity counters and timing aggregates where appropriate

The command currently reports several cumulative counters and timing aggregates beyond simple draw/tick totals.

Normative direction:

- use snapshot deltas for command count, input counts, agent events, incoming prompts, shell completions, and agent-turn start/completion counters in the recent section
- use delta total timing for averages such as draw average within the recent window
- if a metric such as max duration cannot be meaningfully windowed with the current stored data, either:
  - label it as lifetime max, or
  - stop presenting it as a recent-window metric until window-correct data is available

This avoids fixing only part of the section while leaving other lines semantically mixed.

**Alternative considered:** fix only the headline rates and leave task lines cumulative. Rejected: that would still leave a confusing hybrid section.

### Keep fallback behavior truthful when too few snapshots exist

The command already has an early-process case where fewer than two snapshots exist.

Normative direction:

- if there are not yet enough snapshots to compute a delta window, the command should say so clearly
- fallback output may show lifetime totals, but only if labeled as lifetime or since-start values
- the command must not imply that a one-snapshot or zero-snapshot view is a recent sampled window

This keeps startup behavior honest and avoids another class of misleading numbers.

**Alternative considered:** fabricate a pseudo-window from process start when only one snapshot exists. Rejected: that collapses lifetime and recent semantics again.

### Update docs to describe the corrected semantics

The architecture/runtime docs should describe the command as reporting recent-window activity from snapshot deltas plus any separately labeled lifetime totals.

Normative direction:

- `docs/architecture.md` should describe recent-window activity as bounded delta-based activity metrics
- `docs/engine-runtime.md` should explain that the command keeps cumulative counters internally but renders recent activity from snapshot differences
- any docs wording that currently implies recent rates without clarifying the delta model should be updated

This keeps the docs aligned with the corrected behavior.

**Alternative considered:** update code only and leave the docs roughly as-is. Rejected: the semantics are subtle enough that docs should state them directly.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Fix runtime snapshot delta calculation for `/debug runtime`, render recent-window counts/rates from earliest-to-latest snapshot differences, and label any lifetime values explicitly. |
| `docs/architecture.md` | Update the runtime debug command section to describe recent-window activity as delta-based rather than ambiguous cumulative activity. |
| `docs/engine-runtime.md` | Clarify the corrected semantics of recent-window activity and any separately labeled lifetime counters. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Edge Cases

- the app has only one retained snapshot → verify: the command reports that a recent window is unavailable and any shown counts are clearly labeled as lifetime values.
- the app is effectively idle between the earliest and latest snapshots → verify: recent-window counts and rates stay low or zero instead of showing inflated lifetime-derived numbers.
- the app just handled a burst of input or redraw activity → verify: recent-window counts and rates reflect that burst only within the bounded window.
- a counter somehow decreases unexpectedly due to future code changes or reset behavior → verify: the delta-rendering path handles it defensively rather than panicking or emitting nonsensical negative output.
- average timing uses zero delta count in the window → verify: average timing degrades cleanly to zero or unavailable wording rather than dividing by zero.
- lifetime max timing is still shown while averages are windowed → verify: the output labels max timing clearly so readers do not mistake a lifetime max for a recent-window maximum.

## Migration

This is an additive bug-fix migration for an existing debug command.

Expected rollout shape:

- keep the `/debug runtime` command name and general output structure
- correct recent-window calculations in place
- relabel any lifetime values explicitly where they remain useful
- avoid changing unrelated command behavior or instrumentation scope in the same patch

No config migration, schema migration, or history migration is required.

## Testing

- start Themion and run `/debug runtime` before two snapshots exist → verify: the command reports recent-window data as unavailable and does not present lifetime totals as recent rates.
- leave Themion idle for several seconds and run `/debug runtime` → verify: recent-window draw, tick, and input rates are plausible for the idle app rather than inflated by process-lifetime totals.
- generate a short burst of keyboard, mouse, or command activity and then run `/debug runtime` → verify: recent-window counts/rates increase only for the affected categories and remain bounded by the actual recent interval.
- trigger agent activity and then inspect `/debug runtime` during and after the turn → verify: recent-window agent-event and turn counters reflect the recent burst instead of accumulated session totals.
- inspect output lines that still show cumulative values → verify: they are labeled as lifetime or since-start values explicitly.
- review updated docs → verify: they describe recent-window semantics as snapshot deltas and do not imply that cumulative totals are recent-window values.
- run `cargo check -p themion-cli` and `cargo check -p themion-cli --features stylos` after implementation → verify: the fix compiles cleanly in both configurations.

## Implementation checklist

- [x] compute a delta snapshot for `/debug runtime` recent-window rendering
- [x] use delta counts for recent-window rate calculations
- [x] use delta timing aggregates for recent-window average timing where supported
- [x] relabel any remaining cumulative values as lifetime or since-start
- [x] keep insufficient-snapshot fallback output truthful
- [x] update `docs/architecture.md` and `docs/engine-runtime.md`
- [x] update `docs/README.md` with the new PRD entry


## Implementation notes

The implemented fix landed with these concrete behaviors:

- `/debug runtime` now computes its recent-window section from the delta between the oldest and newest retained runtime snapshots
- recent count and rate lines now use bounded-window deltas instead of process-lifetime totals
- the command now prints separately labeled lifetime counts and lifetime task totals so cumulative values remain available without being mistaken for recent activity
- fallback output for insufficient snapshots now reports the recent window as unavailable and shows only lifetime-labeled totals
- docs now describe the recent-window section as delta-based rather than ambiguous cumulative activity
