# PRD-052: Local System Inspection Tool for Runtime, Tooling, and Provider Readiness

- **Status:** Implemented
- **Version:** v0.33.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-25

## Summary

- Themion already has `/debug runtime` for human-driven local diagnosis inside the TUI, and the implemented `system_inspect_local` tool now gives the model a local structured inspection surface as well.
- The implemented tool is aggregate, read-only, and no-surprise: it reports runtime state, tool availability, and provider readiness without mutating workflow/config/history or triggering shell/probe side effects.
- In TUI mode, the runtime section now includes the same `/debug runtime` lines via `runtime.debug_runtime_lines`, so the model can inspect at least the current human-visible runtime snapshot path.
- In non-TUI paths, the tool falls back to a bounded partial snapshot and marks unavailable runtime details explicitly.
- The first implementation is local-only and does not attempt remote Stylos fleet inspection or automatic repair.

## Goals

- Give the model a tool-level equivalent to `/debug runtime` so it can inspect current local runtime health without requiring a human to type the TUI command.
- Expose a compact system-inspection view covering runtime activity, tool availability, and model/provider readiness in the current local process.
- Reuse the existing runtime instrumentation behind `/debug runtime` where practical so human and model diagnostics stay aligned.
- Make the inspection tool output structured enough for model reasoning rather than only human-formatted prose.
- Help the agent distinguish likely local-environment problems such as disabled features, unavailable tools, provider auth/config issues, or a locally unhealthy runtime state.
- Keep the first release safe and read-only.

## Non-goals

- No remote cross-instance health dashboard over Stylos.
- No automatic repair orchestration in the first slice.
- No hidden side effects such as changing workflow state, mutating config, refreshing auth, or performing write actions as part of inspection.
- No promise of exact OS-level CPU accounting beyond the current lightweight debug/runtime instrumentation.
- No replacement for `/debug runtime`; the human command should remain available.
- No broad redesign of the tool registry solely for this feature.
- No provider benchmark suite or long-running synthetic load test harness.
- No requirement to probe every external dependency deeply; bounded lightweight checks are enough for the first slice.

## Background & Motivation

### Current state

Themion already has a useful human-facing local diagnostic command family in the TUI, especially `/debug runtime`, introduced by PRD-040 and corrected by PRD-041. That command reports local process/runtime activity using lightweight counters and recent-window snapshot deltas.

However, the model itself cannot invoke an equivalent diagnostic capability through the tool interface. When the agent encounters suspicious local behavior, it currently has to infer health indirectly from failures such as:

- a tool call failing after the fact
- a provider request erroring after work has already started
- a feature-gated path not existing in the current build
- an unexpectedly slow or idle-looking local runtime without direct introspection

That is weaker than the human operator experience. A user can type `/debug runtime` and inspect the app's state, while the model has no first-class self-check surface beyond trying tools one by one.

### Why this should be a tool-level system inspection surface instead of only more TUI commands

Themion's tool model is the mechanism the agent uses to inspect and act on its environment. If self-diagnosis is only available through TUI-local slash commands, then the agent cannot reason about its own execution environment in the same structured way that a human can.

A tool-level system inspection surface gives the model a direct way to answer questions such as:

- is the current local runtime active and responsive?
- what recent runtime activity does Themion report for this process?
- which tool families are actually available in this build/session?
- is the configured model/provider ready enough to attempt another turn?
- is a failure likely due to local configuration, feature flags, auth, or transient provider conditions?

**Alternative considered:** teach the agent to shell out to local commands or ask the user to run `/debug runtime` manually. Rejected: that is slower, less structured, and fails to provide a stable built-in self-diagnostic contract.

### Relationship to existing debug/runtime instrumentation

PRD-040 and PRD-041 already established a local runtime-observability model in `themion-cli`. That is the natural starting point for a tool-level inspection feature.

The new feature should not invent a second independent notion of runtime health if the existing counters and snapshot model are already suitable. Human-facing and model-facing diagnostics should differ mainly in presentation and structure, not in the underlying truth source.

At the same time, the model needs more than runtime churn numbers alone. A useful inspection tool should also summarize the locally available tool surface and basic model/provider readiness.

**Alternative considered:** expose only the raw `/debug runtime` text through a tool wrapper. Rejected: the model needs structured machine-usable fields and at least a minimal view of tool/provider readiness beyond one block of diagnostic text.

## Design

### Use one explicit `system_`-prefixed inspection tool name

Themion should expose one dedicated inspection tool for local diagnostics.

Normative direction:

- use the explicit tool name `system_inspect_local` in the first implementation
- keep the tool read-only and bounded
- scope the tool to the current local Themion process and active agent context
- return structured data rather than freeform prose as the primary contract

### Make the first-slice contract no-surprise and aggregate-first

The initial tool contract should be conservative and predictable.

Normative direction:

- `system_inspect_local` should be an aggregate read-only inspection tool, not a command runner or recovery hook
- default behavior should gather only local already-available state plus bounded cheap checks
- the tool should not trigger model generations, shell commands, auth refresh flows, workflow transitions, board mutations, or memory writes
- expensive or disruptive live probes, if ever added later, should require explicit opt-in fields rather than happening implicitly
- the tool should return partial results with explicit `unknown` or `unavailable` markers when some sections cannot be inspected cheaply

This keeps the first slice safe for routine model use. The model should be able to call the tool without worrying that inspection itself changed the system under inspection.

**Alternative considered:** make inspection implicitly perform active provider pings or self-repair attempts when data is missing. Rejected: that would violate the no-surprise diagnostic contract.

### Leave room for future narrower `system_inspect_*` tools without requiring them now

The first slice should be one aggregate local inspection tool, but the naming should leave room for future decomposition if the surface grows.

Normative direction:

- start with `system_inspect_local` as the only required new tool in this PRD
- treat it as the aggregate local snapshot covering runtime, tools, and provider readiness
- reserve optional future sibling names such as `system_inspect_runtime`, `system_inspect_tools`, or `system_inspect_provider` only if later experience shows the aggregate result is too coarse or too expensive
- do not require those narrower tools in the first implementation

This keeps the first implementation simple while avoiding naming regret.

**Alternative considered:** introduce a whole family of `system_inspect_*` tools immediately. Rejected: that would add surface area before proving the aggregate inspection contract is actually useful.

Why this name:

- `system_` matches the requested naming direction for inspection-oriented tools
- `inspect` frames the tool as observational and read-only rather than corrective or "healing"
- `local` makes the scope explicit and avoids sounding like remote Stylos fleet inspection

**Alternative considered:** `system_healthcheck`. Rejected: too strongly implies pass/fail health semantics when the requested surface is broader inspection.

**Alternative considered:** `system_self_inspect`. Rejected: `self` is unnecessary once `local` already establishes scope and `inspect` already establishes purpose.

**Alternative considered:** `runtime_self_healthcheck`. Rejected: wrong prefix and too narrow because the tool also covers tool-surface and provider readiness.

**Alternative considered:** overload `workflow_get_state` or another existing tool with health data. Rejected: workflow state and runtime health are related but distinct concerns.

### Reuse `/debug runtime` instrumentation as the runtime-health source of truth

The runtime section of the inspection tool should reuse the same underlying counters, recent-window snapshots, and activity semantics that power `/debug runtime`.

Normative direction:

- factor existing runtime debug data assembly so both the TUI command and the tool can consume the same underlying snapshot builder where practical
- preserve the recent-window delta semantics established by PRD-041
- allow the TUI command to keep its human-readable text rendering while the tool returns structured fields
- avoid maintaining two separate implementations of recent runtime metrics unless there is a strong reason

This keeps human and model diagnostics aligned and reduces drift.

**Alternative considered:** build a completely separate tool-only runtime metrics path. Rejected: that would create unnecessary divergence and duplicate maintenance.

### Include tool-surface availability and feature visibility

The inspection tool should report what the current local tool surface actually is.

Normative direction:

- include a summary of available tool families or tool names in the current build/runtime
- make feature-gated availability visible, especially for major optional surfaces such as Stylos-related tools
- distinguish between "tool exists in this build" and "tool call may still fail at runtime for environmental reasons"
- keep the report compact enough that the model can use it as context without wasting too many tokens

This helps the agent avoid reasoning from instructions alone when the build or session shape differs from expectation.

**Alternative considered:** rely on the prompt-injected tool list only. Rejected: prompt-visible tool definitions do not by themselves confirm the current local runtime shape or explain feature-gated omissions clearly.

### Include bounded model/provider readiness checks

The inspection tool should include a lightweight readiness view for the configured model/provider path.

Normative direction:

- report the active provider/profile/model identity already known locally
- include basic readiness signals such as whether auth/config appears present, whether the local session has model metadata, and whether the provider is currently known to be rate-limited or degraded from recent state
- if an active lightweight probe is added, it must be bounded and conservative
- avoid expensive or stateful provider calls by default in the first slice unless they are clearly necessary

The goal is to help the model decide whether a failure is likely local/provider-related, not to run a full connectivity benchmark.

**Alternative considered:** make the healthcheck always perform a live model completion ping. Rejected: that would be costly, potentially rate-limit-unfriendly, and too disruptive for a default diagnostic tool.

### Return machine-usable status plus compact human summary

The tool should primarily return structured fields, but a compact summary string is also useful for logs and chat display.

Normative direction:

- return structured JSON-friendly fields with explicit names and units
- include a concise summary string highlighting major problems or the overall healthy/degraded/unavailable state
- prefer explicit booleans, enums, counts, timestamps, and bounded recent-window metrics over prose-only explanation
- preserve milliseconds for machine-consumed timestamps where timestamps are included

This gives the model a stable reasoning surface while keeping the result readable in transcripts.

**Alternative considered:** return only a human-readable multiline text block. Rejected: that is harder for the model to reason over reliably.

### Define a bounded first-slice status model

The first implementation should keep the status model simple and extensible.

Normative direction:

- include an overall health classification such as `ok`, `degraded`, or `unavailable`
- include subsections for runtime, tools, and provider/model readiness
- allow each subsection to carry its own status plus brief issues/warnings lists
- include recent-window runtime metrics using the same semantics as `/debug runtime`
- keep unknown/unavailable fields explicit rather than inventing defaults

A reasonable no-surprise first-slice shape could include:

- `overall_status`
- `summary`
- `runtime.status`
- `runtime.recent_window_ms`
- `runtime.counters` and bounded timing fields
- `tools.available_names` or grouped families
- `tools.missing_expected_features`
- `provider.status`
- `provider.active_profile`
- `provider.provider`
- `provider.model`
- `provider.rate_limits` when known
- `issues[]` and `warnings[]`

**Alternative considered:** start with an unstructured blob and standardize later. Rejected: this is exactly the kind of diagnostic contract that benefits from structure early.


## Implementation notes

What landed in this implementation:

- `system_inspect_local` was added to the core tool registry in `crates/themion-core/src/tools.rs`
- the tool returns a structured result with top-level `overall_status`, `summary`, `runtime`, `tools`, `provider`, `warnings`, and `issues`
- in TUI mode, the main agent receives a live inspection snapshot derived from current app state
- the runtime section includes `runtime.debug_runtime_lines`, which reuse the same `/debug runtime` text assembly path so the tool at least covers the existing command output
- the tool and provider sections use already-available local state such as defined tools, active profile/provider/model, auth presence, base URL presence, and recent rate-limit report metadata when known
- non-TUI execution paths return a bounded partial snapshot and explicitly report that full `/debug runtime` coverage is unavailable there

Known gaps versus the ideal end-state described above:

- the implementation currently exposes `/debug runtime` coverage through `runtime.debug_runtime_lines` rather than a fully normalized structured decomposition of every runtime counter line
- runtime recent-window metrics are therefore machine-usable primarily through the carried text snapshot today, not yet as a richer dedicated nested metrics schema
- provider readiness remains conservative and local-state-based; it does not perform active live provider pings by default

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Added `system_inspect_local`, shared inspection result structs, and safe fallback behavior for non-TUI paths. |
| `crates/themion-core/src/agent.rs` | Added agent-carried inspection snapshot wiring so tool execution can consume current local inspection state. |
| `crates/themion-cli/src/tui.rs` | Wired live inspection snapshots from current app state and reused `/debug runtime` output through `runtime.debug_runtime_lines`. |
| `crates/themion-cli/src/app_runtime.rs` | Added bounded non-TUI inspection snapshot generation for local provider/tool/runtime context. |
| `docs/README.md` | Updated PRD status to reflect implementation. |

## Edge Cases

- Themion is running in non-TUI or explicit `--headless` mode → verify: the inspection tool still works and does not depend on slash-command-only UI code.
- recent runtime snapshots are not yet available → verify: runtime recent-window fields report unavailable status clearly rather than fabricating numbers.
- a feature-gated tool family such as Stylos is not compiled in → verify: the inspection result reports that absence explicitly instead of implying a runtime failure.
- provider config is partially present but auth is missing or expired → verify: provider readiness reports a degraded or unavailable state without requiring a full model call.
- rate-limit information is absent because no recent provider response supplied it → verify: the result marks that data as unknown rather than healthy-by-assumption.
- the inspection tool itself is invoked during an ongoing busy turn → verify: it reports current local activity truthfully and remains read-only.
- the tool is invoked repeatedly during normal work → verify: it remains read-only, bounded, and does not create surprise side effects or hidden expensive probes.

## Migration

This is an additive diagnostic capability.

Expected rollout shape:

- keep `/debug runtime` as the human-facing TUI command
- factor shared runtime-health data assembly behind a reusable local diagnostic layer
- add one tool-level inspection tool contract for the current local process and active agent context
- extend the structured result over time if new diagnostic sections become useful

No schema migration is required unless a later implementation chooses to persist health snapshots, which is out of scope for this PRD.

## Testing

Implemented validation so far:

- `cargo check -p themion-core -p themion-cli` → verify: default builds pass after adding the new tool and snapshot plumbing.
- `cargo check -p themion-cli --features stylos` → verify: the feature-on CLI build still compiles cleanly with the new inspection wiring.
- `cargo test -p themion-core` → verify: core tests pass after updating `ToolCtx` test construction for the new inspection field.
- `cargo test -p themion-cli` → verify: CLI tests still pass with the new TUI-side inspection snapshot wiring.


- invoke the `system_inspect_local` tool in a normal TUI session → verify: it returns structured runtime, tool, and provider/model readiness data for the current local process.
- invoke the `system_inspect_local` tool in explicit `--headless` mode → verify: it works without depending on TUI-only slash-command code paths.
- compare runtime-health fields against `/debug runtime` in the same session → verify: recent-window counters and timing semantics match the existing debug/runtime truth source.
- run a build without an optional feature such as Stylos and invoke the tool → verify: the reported tool/feature availability reflects the compiled runtime shape accurately.
- invoke the tool early in process startup before enough snapshots exist → verify: unavailable recent-window data is labeled clearly and does not present fake rates.
- trigger a provider auth/config failure state and invoke the tool → verify: the provider section reports degraded or unavailable readiness with a machine-usable explanation.
- invoke `system_inspect_local` repeatedly during normal work → verify: results stay read-only and bounded, with no workflow/config/history/board mutations caused by inspection itself.

## Implementation checklist

- [x] add and document the `system_inspect_local` response schema
- [x] document the no-surprise contract: read-only, bounded, and no implicit expensive probes
- [x] factor `/debug runtime` data assembly into a reusable non-TUI diagnostic builder where practical
- [x] expose current local runtime-health data to the tool implementation
- [x] expose current tool/feature availability in a compact machine-usable form
- [x] expose bounded provider/model readiness signals without requiring an expensive default live probe
- [x] implement the tool in `themion-core`/CLI integration without making it depend on TUI-only rendering code
- [x] update `docs/architecture.md` and `docs/engine-runtime.md`
- [x] update `docs/README.md` with this PRD entry
