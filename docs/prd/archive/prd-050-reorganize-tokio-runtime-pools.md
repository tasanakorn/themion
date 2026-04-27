# PRD-050: Reorganize Tokio Runtime Execution into Domain-Specific Pools

- **Status:** Implemented
- **Version:** v0.31.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-24

## Summary

- Themion no longer relies on `#[tokio::main]` as the only runtime entry shape; `themion-cli` now constructs explicit runtime domains through `RuntimeDomains`.
- The current shipped topology is still single-process, with named domains for `tui`, `core`, `network`, and `background`.
- This PRD is now scoped to executor-domain ownership and task placement only.
- It does not define the broader future split between shared app/runtime logic and TUI presentation.
- Headless-capable shared-runtime architecture, TUI-optional operation, and removal/replacement of overlapping bootstrap scaffolding are deferred to a successor PRD.
- Follow-through work that remained open when this PRD was narrowed has since been completed here or absorbed by successor PRDs while preserving PRD-050 as the shipped runtime-domain foundation.

## Goals

- Make runtime ownership explicit in `themion-cli` instead of relying on one implicit default Tokio runtime.
- Keep Themion as one OS process while introducing named execution domains for major workload classes.
- Preserve the current user-visible behavior in print mode and TUI mode while improving clarity about where long-lived work runs.
- Route major long-lived runtime-sensitive tasks onto explicit domain handles.
- Keep runtime-domain documentation aligned with the code that actually shipped.
- Bound the remaining work to runtime-domain correctness and cleanup rather than expanding into a larger architecture rewrite.

## Non-goals

- No multi-process redesign.
- No rewrite away from Tokio.
- No promise that every remaining responsiveness issue is solved by the current runtime split.
- No requirement in this PRD to make `tui.rs` presentation-only.
- No requirement in this PRD to define a headless shared application runtime.
- No requirement in this PRD to decide the final fate of `app_runtime.rs` beyond noting its overlap with the active bootstrap path.
- No requirement in this PRD to move all non-UI orchestration out of `tui.rs`.
- No broad app-layer refactor beyond what is directly needed for runtime-domain ownership.

## Background & Motivation

### Current state before this PRD

Themion originally started under one implicit Tokio runtime via `#[tokio::main]` in `crates/themion-cli/src/main.rs`. In that model, the process remained single-process and async-task-oriented, but the repository did not express stronger executor-domain ownership for TUI, core orchestration, networking, or lower-priority work.

That older shape made the code simple, but it also made task placement harder to review. Long-lived TUI support tasks, Stylos networking tasks, and core orchestration work all ultimately depended on the same ambient runtime context.

### Current shipped state

The current code has already landed a first runtime-domain split:

- `crates/themion-cli/src/runtime_domains.rs` defines `RuntimeDomain`, `DomainHandle`, and `RuntimeDomains`
- `crates/themion-cli/src/main.rs` constructs `RuntimeDomains::for_print_mode()` or `RuntimeDomains::for_tui_mode()` explicitly
- print mode blocks on the `core` domain
- TUI mode blocks on the `tui` domain and routes startup through `crates/themion-cli/src/tui_runner.rs`
- long-lived Stylos startup runs through the explicit `network` domain in feature-enabled builds
- TUI helper work such as ticks and frame requests is routed through explicit runtime handles rather than relying only on ambient startup context

The code and docs now describe the following named domains:

- `tui`
- `core`
- `network`
- `background`

This is the implemented foundation that PRD-050 should document.

### Why this PRD should now be narrower

PRD-050 ran long enough that it started to accumulate adjacent architecture questions, especially around whether `tui.rs` should remain the app layer and how headless mode should work. Those are valid concerns, but they are not the same design problem as executor-domain ownership.

Keeping those broader concerns in PRD-050 would make the PRD unstable and too broad. The runtime-domain split should stand on its own as one architectural step.

**Alternative considered:** keep extending PRD-050 until it also covers TUI-optional shared-runtime architecture. Rejected: that conflates executor placement with application-layer ownership and makes review harder.

## Design

### Runtime ownership remains CLI-local

`themion-cli` owns runtime construction and lifetime.

Normative direction:

- explicit runtime construction stays in CLI startup code
- runtime domains remain represented by one shared CLI-local topology helper
- subsystems should receive domain handles or runtime-owned entrypoints rather than creating ad hoc Tokio runtimes
- shutdown remains under top-level CLI control

This preserves the existing boundary where runtime/process wiring lives in `themion-cli` and reusable harness behavior lives in `themion-core`.

**Alternative considered:** move runtime ownership into `themion-core`. Rejected: runtime selection and mode wiring are still CLI concerns.

### Stable runtime-domain contract

The current shared contract is:

- `RuntimeDomain`
- `DomainHandle`
- `RuntimeDomains`

Normative direction:

- keep these as the stable names for executor-domain ownership in code and docs
- keep domain handles cloneable and cheap to pass around
- keep callsites reviewable by making domain choice explicit in startup and long-lived task wiring

The exact internals may continue to evolve, but the named-domain contract is now the important public architecture inside the repository.

**Alternative considered:** let each subsystem define its own runtime wrapper. Rejected: that would weaken the whole point of explicit shared domain ownership.

### Current topology is the documented current state

The current topology should be documented as the actual current-state target for PRD-050:

- `tui` domain for TUI-mode event-loop support and terminal-adjacent scheduling
- `core` domain for startup coordination, print-mode execution, and core harness orchestration
- `network` domain for Stylos long-lived networking work when enabled
- `background` domain as an explicit reserved lower-priority lane in the current slice

Current mode split:

- TUI mode constructs `tui`, `core`, `network`, and `background`
- print mode constructs the reduced runtime set currently needed by that path

This PRD should describe the current shipped arrangement accurately, including any limitations.

**Alternative considered:** keep documenting the more ambitious original target as if it were already complete. Rejected: the PRD should match shipped reality.

### Focus on long-lived and domain-sensitive task placement

PRD-050 is primarily about making high-level placement explicit.

Normative direction:

- long-lived TUI support tasks should continue to be routed through the TUI-owned path
- long-lived Stylos networking tasks should continue to be routed through the network-owned path
- print-mode execution should remain rooted in the core-owned path
- remaining short-lived or ambiguous spawn sites should be audited only insofar as they affect runtime-domain clarity or correctness

This keeps the PRD focused on the executor-domain foundation instead of turning it into a rewrite of every async callsite.

**Alternative considered:** require every async spawn in the repository to be converted before PRD-050 can settle. Rejected: too broad for one runtime-foundation PRD.

### Blocking and thread-shape follow-through stays in scope

Even with the runtime split in place, some correctness follow-through still belongs here.

Normative direction:

- document the actual runtime shapes accurately
- keep reviewing blocking-sensitive paths that could undermine domain isolation
- keep thread naming and debug visibility aligned with the real runtime topology
- treat remaining ambiguous spawn or blocking paths as follow-through work, not as proof that the runtime-domain design itself was a mistake

This is the practical remaining scope of PRD-050.

**Alternative considered:** declare the PRD complete and ignore remaining placement/observability mismatches. Rejected: the repository should not freeze inaccurate runtime-domain docs or ambiguous follow-through.

### Defer app-layer and headless architecture to a successor PRD

PRD-050 should now explicitly defer the larger application-architecture question.

Deferred topics include:

- making TUI strictly input/output related
- allowing the app to run cleanly without `tui.rs`
- introducing a shared app/runtime layer used by both TUI mode and headless mode
- moving non-UI orchestration such as agent/bootstrap logic out of `tui.rs`
- removing or replacing overlapping bootstrap scaffolding such as `app_runtime.rs` when doing the broader split

Those topics need a separate PRD because they concern application-layer ownership, not just Tokio executor domains.

**Alternative considered:** keep these items as a late section inside PRD-050. Rejected: they are substantial enough to deserve an explicit successor PRD.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/main.rs` | Own explicit mode-specific runtime construction and enter print mode on `core` or TUI mode on `tui`. |
| `crates/themion-cli/src/runtime_domains.rs` | Define `RuntimeDomain`, `DomainHandle`, and `RuntimeDomains` as the shared runtime-topology contract. |
| `crates/themion-cli/src/tui_runner.rs` | Serve as the active TUI bootstrap/orchestration path using explicit runtime-domain handles. |
| `crates/themion-cli/src/tui.rs` | Continue using explicit domain-owned paths where needed for TUI-adjacent work; broader app-layer separation is deferred. |
| `crates/themion-cli/src/stylos.rs` | Keep long-lived Stylos networking tasks on the explicit `network` domain. |
| `crates/themion-cli/src/app_runtime.rs` | Not the active bootstrap path; any future shared-runtime redesign is out of scope for this PRD and belongs in the successor PRD. |
| `docs/architecture.md` | Describe the current explicit runtime-domain topology accurately. |
| `docs/engine-runtime.md` | Describe CLI-local runtime-domain ownership and current mode differences accurately. |
| `docs/README.md` | Keep this PRD indexed with the narrowed current-state scope. |

## Edge Cases

- TUI mode starts with explicit runtime domains → verify: startup still enters the TUI path through the TUI-owned runtime.
- print mode runs without the TUI → verify: the non-interactive path still uses only the domains it needs.
- Stylos is enabled → verify: long-lived Stylos startup remains routed through the explicit network domain.
- Stylos is disabled → verify: the CLI still works without requiring Stylos-owned long-lived tasks.
- a remaining ambient spawn or blocking path exists in touched runtime-sensitive code → verify: it is reviewed for domain ownership or clearly documented as follow-through work.
- docs describe runtime topology → verify: they match the code that actually shipped rather than the earlier broader ambition.

## Migration

This PRD now represents a current-state runtime-topology step rather than a future all-in-one concurrency redesign.

Expected rollout shape:

- keep the explicit domain-owned startup already landed
- keep runtime-domain docs aligned with actual code
- treat successor PRDs as follow-through on adjacent architecture questions without reopening the core runtime-domain split
- preserve PRD-050 as the historical contract for the shipped runtime-domain foundation

There is no database or protocol migration tied specifically to this narrowed PRD scope.

## Testing

- start Themion in TUI mode → verify: startup enters the TUI path through explicit runtime-domain ownership.
- run print mode with a normal prompt → verify: the non-TUI path still runs correctly through the core-owned runtime path.
- run with `--features stylos` → verify: Stylos startup and long-lived tasks still function under explicit network-domain ownership.
- inspect runtime-domain docs after this rewrite → verify: they match current code paths in `main.rs`, `runtime_domains.rs`, and `tui_runner.rs`.
- audit current overlapping bootstrap files → verify: PRD-050 documents `app_runtime.rs` as out of active bootstrap scope rather than pretending it is the shipped path.

## Implementation checklist

- [x] replace implicit `#[tokio::main]` startup with explicit runtime construction in `crates/themion-cli/src/main.rs`
- [x] add CLI-local `RuntimeDomains`, `RuntimeDomain`, and `DomainHandle`
- [x] route TUI-mode startup through explicit TUI runtime ownership
- [x] route print-mode startup through explicit core runtime ownership
- [x] route long-lived Stylos startup through explicit network runtime ownership
- [x] update architecture/runtime docs to describe the shipped runtime-domain topology
- [x] audit remaining ambiguous spawn and blocking paths that materially affect runtime-domain correctness
- [x] improve runtime observability so diagnostics reflect the actual domain topology precisely
- [x] narrow PRD-050 so broader TUI-optional/shared-runtime architecture is deferred to a successor PRD

## Implementation notes

PRD-050 now records the shipped runtime-domain foundation rather than acting as a catch-all future architecture document.

Current shipped state:

- `crates/themion-cli/src/runtime_domains.rs` defines the named runtime-domain contract
- `crates/themion-cli/src/main.rs` explicitly constructs runtime domains for print mode and TUI mode
- `crates/themion-cli/src/tui_runner.rs` is the active TUI bootstrap path
- `crates/themion-cli/src/app_runtime.rs` exists as overlapping scaffolding but is not the active bootstrap path
- docs now treat larger app/runtime-vs-TUI separation as successor work rather than PRD-050 scope

Follow-through that was still open when this PRD was narrowed has since landed across the repository:

- runtime-domain documentation in `docs/architecture.md` and `docs/engine-runtime.md` now reflects the shipped topology
- runtime observability and debug coverage now describe the actual domain and thread model more precisely
- adjacent shared-runtime and TUI-optional architecture work was split into successor PRDs, especially PRD-051, PRD-053, and PRD-054, without changing PRD-050's core runtime-domain contract

PRD-050 should now be read as the implemented historical contract for the runtime-domain split that shipped, while later PRDs carry the follow-on application-architecture refinements.
