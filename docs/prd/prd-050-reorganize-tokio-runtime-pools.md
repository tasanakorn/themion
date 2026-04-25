# PRD-050: Reorganize Tokio Runtime Execution into Domain-Specific Pools

- **Status:** Partially implemented
- **Version:** v0.31.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-24

## Summary

- Themion currently starts under one default Tokio runtime from `#[tokio::main]` in `crates/themion-cli/src/main.rs`.
- In the current shape, CPU spikes or blocking-heavy work in one area can contend with unrelated paths because the process does not define separate execution domains.
- Reorganize runtime execution so major workload groups run on separate Tokio runtime pools or explicitly separated executor domains.
- The first intended groups are: TUI, core system, networking, and background tasks.
- Keep Themion as one OS process and preserve current feature behavior; this PRD is about runtime isolation and scheduling boundaries, not a multi-process redesign.
- Prefer explicit runtime ownership in `themion-cli` over relying on the default `#[tokio::main]` macro runtime.
- Refine the implementation direction around repository-owned runtime handles, current task placement, a staged migration away from anonymous `tokio::spawn`, and a stricter first implementation slice.

## Goals

- Reduce cross-impact where a CPU spike or blocking-heavy burst in one domain degrades responsiveness in unrelated domains.
- Replace the implicit single default Tokio runtime setup with an explicit repository-owned runtime topology.
- Define clear execution groups for at least these workload classes:
  - TUI
  - core system / agent orchestration
  - networking-related work
  - background maintenance tasks
- Keep the TUI responsive during bursts from networking, provider streaming, remote query handling, or maintenance work.
- Give the codebase a documented place to assign new tasks to the correct runtime or executor domain instead of defaulting every `tokio::spawn` onto one shared pool.
- Preserve current single-process deployment and current user-facing flows in print mode and TUI mode.
- Keep the design implementable without introducing a large framework or broad architectural rewrite.

## Non-goals

- No multi-process decomposition of Themion in this PRD.
- No rewrite away from Tokio.
- No promise that every performance issue is solved purely by splitting runtimes; hot loops, bad polling, or unnecessary redraws still need direct fixes.
- No immediate redesign of every internal channel or task API unless needed to route work across runtime boundaries safely.
- No requirement to give every feature its own dedicated runtime.
- No attempt to make runtime pool counts, thread counts, or affinities fully user-configurable in the first step unless implementation experience justifies it.
- No change to the logical behavior of tools, history, Project Memory, board notes, or Stylos protocols beyond how their tasks are scheduled.
- No requirement that every piece of code in `themion-core` becomes runtime-aware directly; CLI-owned wiring may continue to hide some executor details behind handles or services.
- No requirement in phase 1 to move every provider-internal async operation into the networking domain if that boundary would require a larger backend refactor.

## Background & Motivation

### Current state

The architecture docs currently describe Themion as one process using Tokio's default runtime created by `#[tokio::main]` in `crates/themion-cli/src/main.rs`. The repository does not currently define a custom Tokio runtime builder or explicit per-domain execution pools.

The documented mental model today is one process with one Tokio runtime handling:

- main startup path
- TUI event loop and redraw scheduling
- input task and tick task
- agent run tasks
- provider/network IO
- Stylos query, command, and status tasks in feature-enabled builds
- blocking work delegated through Tokio's blocking pool via `spawn_blocking`

Source inspection matches that model:

- `crates/themion-cli/src/main.rs` uses `#[tokio::main]`
- `crates/themion-cli/src/tui.rs` starts many long-lived tasks with `tokio::spawn`
- `crates/themion-cli/src/stylos.rs` starts networking-related background tasks with `tokio::spawn`
- `crates/themion-core/src/agent.rs` uses `tokio::task::spawn_blocking` for several database and persistence operations
- `crates/themion-cli/src/tui.rs` also uses `tokio::task::block_in_place` in a few local paths

Representative currently shared-spawned work includes:

- `FrameScheduler::run()` in `crates/themion-cli/src/tui.rs`
- the terminal input stream task built around `EventStream::new()` in `crates/themion-cli/src/tui.rs`
- the 150 ms tick task in `crates/themion-cli/src/tui.rs`
- TUI bridge tasks for agent and Stylos event forwarding in `crates/themion-cli/src/tui.rs`
- Stylos status publisher, queryable server, and command subscriber tasks in `crates/themion-cli/src/stylos.rs`

This is simple and has worked so far, but it also means the repository does not yet express stronger isolation between workload classes.

### Why the current shape is risky

A single shared runtime is convenient, but it can let unrelated work interfere with each other in practice:

- a burst of networking callbacks or provider streaming work can delay local UI-adjacent tasks
- a busy background task can compete with agent orchestration work if both are scheduled through the same worker set
- CPU-heavy local work can reduce responsiveness of remote query handling or TUI updates
- accidental misuse of `block_in_place` or overproduction of tasks in one area can affect unrelated paths because they still share one runtime environment

The user concern is specifically operational isolation: if one part spikes CPU usage, it should not degrade unrelated paths as much as it does under the current one-pool design.

**Alternative considered:** keep one Tokio runtime and only tune task priorities informally. Rejected: Tokio does not give the repository a simple built-in priority scheduler for ordinary `tokio::spawn`, so the current one-pool design still leaves domains contending on the same executor.

### Why explicit domain grouping fits Themion

Themion already has recognizable subsystem boundaries in docs and code:

- the TUI loop and redraw/input handling are CLI-local
- the core harness/agent system is primarily orchestration, history, tool dispatch, and provider interaction
- Stylos and provider IO are networking-oriented
- some work is background or maintenance shaped rather than latency-sensitive

That makes runtime-domain separation a natural next step. The repository does not need a general scheduler product; it needs clear operational lanes for major workload classes.

**Alternative considered:** split only blocking work and keep all async work on one runtime. Rejected: blocking-pool isolation helps only one class of contention and does not address heavy async/network/task bursts competing with UI and orchestration work.

## Design

### Use explicit Tokio runtimes owned by `themion-cli`

The implementation should replace `#[tokio::main]` with explicit runtime ownership in `crates/themion-cli/src/main.rs`.

Normative direction:

- build the runtime topology explicitly at process startup
- keep ownership of runtime creation in `themion-cli`, because CLI startup already owns app/session/runtime wiring
- avoid ad hoc runtime creation deeper inside subsystems
- keep shutdown under top-level control so all domains can be terminated coherently

The preferred implementation direction is multiple Tokio runtime instances, each with a narrow responsibility, rather than one default runtime plus informal conventions.

**Alternative considered:** keep `#[tokio::main]` and create extra ad hoc runtimes from inside tasks. Rejected: runtime ownership becomes harder to reason about, and lifecycle/shutdown behavior becomes less clear than a top-level explicit runtime plan.

### Preferred topology: one coordinator entrypoint plus four named runtime domains

The refined design should use one top-level startup path that constructs four named runtime domains and passes domain-specific handles into the app.

Proposed domains:

- `tui`
- `core`
- `network`
- `background`

The startup code should own a small runtime-registry object, named `RuntimeDomains`, that stores the domain runtimes and exposes explicit spawn methods.

Example shape:

```text
main()
└─ build RuntimeDomains
   ├─ tui runtime
   ├─ core runtime
   ├─ network runtime
   └─ background runtime
      ↓
   pass RuntimeDomains / subhandles into tui::run / print-mode startup / stylos wiring
```

Normative direction:

- the repository should have stable names for the execution domains in code and docs
- thread naming should reflect the domain when practical for debugging
- domain handles should be cloneable and cheap to pass around
- subsystems should depend on a domain handle or spawn facade, not on global runtime state

**Alternative considered:** one runtime with several semantic labels but no real execution separation. Rejected: labels without real executor separation would not address the isolation goal.

### RuntimeDomains should be a concrete shared contract

The implementation should standardize one small shared runtime-routing API instead of leaving executor usage informal.

Preferred shape:

```rust
pub struct RuntimeDomains {
    tui: DomainHandle,
    core: DomainHandle,
    network: DomainHandle,
    background: DomainHandle,
}

#[derive(Clone)]
pub struct DomainHandle {
    name: RuntimeDomain,
    handle: tokio::runtime::Handle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeDomain {
    Tui,
    Core,
    Network,
    Background,
}
```

The exact field layout may vary, but the contract should provide:

- a stable enum for the four domains
- cloneable per-domain spawn handles
- a top-level owner that keeps the underlying runtimes alive for process lifetime
- one place to attach future domain metadata such as counters or thread-name prefixes

This should live in `themion-cli`, because runtime ownership is CLI-local.

**Alternative considered:** let each subsystem define its own runtime wrapper. Rejected: that would fragment the topology and make cross-repository conventions drift.

### Spawn APIs should make domain choice explicit at callsites

The runtime contract should expose small explicit helpers rather than forcing every caller to interact with raw Tokio types.

Preferred API surface:

```rust
impl RuntimeDomains {
    pub fn tui(&self) -> DomainHandle;
    pub fn core(&self) -> DomainHandle;
    pub fn network(&self) -> DomainHandle;
    pub fn background(&self) -> DomainHandle;
}

impl DomainHandle {
    pub fn domain(&self) -> RuntimeDomain;

    pub fn spawn<F>(&self, fut: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;
}
```

Optional convenience helpers such as `spawn_tui(...)` or `spawn_network(...)` are acceptable, but the core contract should still revolve around explicit domain handles.

Normative direction:

- the implementation should make spawn placement obvious in code review
- the default path for new async tasks should be a domain handle, not ambient `tokio::spawn`
- if a callsite needs only one domain, pass only that domain handle instead of the full registry when practical

**Alternative considered:** pass four raw `Runtime` references throughout the program. Rejected: that would make signatures noisy and encourage ad hoc spawning behavior.

### TUI domain should be current-thread, minimal, and latency-first


The TUI domain should prioritize interactive responsiveness rather than throughput.

Preferred direction:

- run the TUI domain on a `current_thread` Tokio runtime or another deliberately narrow runtime shape
- keep the central app event loop, redraw scheduling, terminal input stream, and short UI bridge tasks on this domain
- do not place heavy provider/network callbacks or maintenance loops here
- use channel handoff to communicate with work running on other domains

This domain should own work such as:

- `FrameScheduler::run()`
- the `EventStream::new()` input task
- the periodic tick task
- TUI-facing bridge tasks that forward summarized events into the app loop

The main reason to prefer a current-thread TUI runtime is to make interactive behavior more deterministic and to reduce surprise contention with unrelated worker-heavy domains.

**Alternative considered:** make the TUI domain multi-threaded like the others. Rejected: the TUI loop is fundamentally coordination- and latency-oriented, and a narrow event-loop-style runtime better matches that role.

### Core domain should be multi-threaded and own harness orchestration

The core domain should own agent-turn execution and other harness-centered orchestration work.

Preferred direction:

- use a multi-thread Tokio runtime for the core domain
- move agent run-loop entry, workflow coordination, tool orchestration, and core event production onto this domain
- keep the core domain separate from TUI and background domains even when the resulting tasks communicate frequently

This domain should own work such as:

- `Agent::run_loop(...)` entry tasks
- core-side orchestration around tool calls and model retries
- DB/history/service coordination that is logically part of a live agent turn
- result/event forwarding back toward the TUI domain

The core domain is where throughput matters more than in TUI, but it is still more latency-sensitive than maintenance work.

**Alternative considered:** merge core and background domains because both are local process work. Rejected: background maintenance should not contend directly with live agent turns.

### Networking domain should own Stylos tasks in phase 1 and provider transport where practical

The networking domain should isolate bursty async IO from both the TUI and the core harness domain.

Preferred direction:

- use a multi-thread Tokio runtime for the networking domain
- route Stylos long-lived tasks and other network-facing services here first
- route provider HTTP request/stream handling here when practical, but do not require a large backend refactor in phase 1
- keep transport IO and remote query servicing off the TUI runtime

This domain should own work such as:

- Stylos status publisher task
- Stylos queryable server task
- Stylos command subscriber task
- future provider streaming tasks or transport-facing request futures, where the wiring can be separated cleanly

This split gives network bursts their own worker budget rather than letting them contend directly with the TUI or background maintenance work.

**Alternative considered:** keep provider networking in core because the harness initiates it. Rejected as the long-term direction: transport-heavy async IO is one of the main contention sources this PRD is trying to isolate. The phase-1 exception is only to avoid forcing a larger provider refactor into the first runtime-split slice.

### Background domain should be explicitly low-priority by placement, not by implicit hope

The background domain should be where maintenance and non-urgent work goes by default.

Preferred direction:

- use a separate runtime for background work, typically multi-threaded but intentionally bounded
- place periodic maintenance, cleanup, compaction, indexing, or optimization tasks here
- avoid using this domain for directly user-triggered latency-sensitive work

Examples include:

- memory optimization or compaction
- future database cleanup or archival work
- lower-priority caches, indexing, or maintenance scans

This domain exists so the repository has a deliberate answer for "where should non-urgent work run?"

**Alternative considered:** simply call `spawn_blocking` for maintenance work from whichever domain needs it. Rejected: that still leaves placement implicit and mixes domain concerns.

### Minimize implicit `tokio::spawn` inside subsystems

Once the domain topology exists, new or touched code should avoid plain ambient-runtime `tokio::spawn` unless it is already executing inside the intentionally chosen domain and the choice is obvious from surrounding code.

Normative direction:

- convert existing high-level spawn sites to domain-specific spawn helpers first
- treat plain `tokio::spawn` in shared code as suspicious unless the domain is obvious and documented
- prefer reviewable placement decisions over ambient spawning
- if a subsystem still uses `tokio::spawn`, it should do so behind a domain-owned entrypoint or helper rather than relying on whoever happened to call into it

This prevents a regression back to effectively one anonymous pool.

**Alternative considered:** add multiple runtimes but keep widespread direct `tokio::spawn` calls. Rejected: that would preserve ambiguity about where work actually lands and would make regressions likely.

### Clarify how blocking work maps to domains

Themion already uses `spawn_blocking` in `themion-core` and `block_in_place` in some CLI paths. The refined design should make blocking behavior more explicit.

Preferred direction:

- keep synchronous DB/file work off async executors through blocking offload when async-native replacements are not practical
- review current `block_in_place` usage in `crates/themion-cli/src/tui.rs` and prefer cross-domain handoff or `spawn_blocking` style offload where it reduces risk
- avoid letting background maintenance dominate the same blocking pool relied on by live harness persistence paths
- if Tokio's per-runtime blocking pools remain the mechanism, document that clearly and choose which runtime owns each blocking-heavy call path
- if a dedicated blocking service layer is introduced later, keep it compatible with this domain model rather than treating it as a fifth unrelated execution lane

Phase-1 rule:

- existing `themion-core` `spawn_blocking` calls may remain as long as they run under the explicitly chosen core-domain runtime rather than under an implicit global default runtime
- `block_in_place` callsites in TUI code should be reviewed first because they are the most likely to undermine TUI isolation

This PRD does not require eliminating all blocking sections, but it does require making their placement intentional.

**Alternative considered:** ignore blocking work because the main concern is async pools. Rejected: blocking paths are a common source of latent contention and need to be considered in the runtime topology.

### Preserve the existing channel-oriented architecture across domains

Themion's current architecture already relies on channels between TUI, agents, Stylos bridges, and background tasks. The new runtime topology should reuse that shape where possible.

Normative direction:

- prefer cross-domain handoff through existing or slightly extended channel boundaries
- keep subsystem ownership clear: CLI owns TUI/runtime wiring, core owns harness logic
- avoid turning the change into a full rewrite of agent/event semantics
- use explicit bridges between runtimes where one domain needs to notify another
- keep most message payload types unchanged unless a runtime boundary exposes a real ownership problem

This keeps the change incremental and aligned with the current architecture.

**Alternative considered:** replace channels with shared mutable cross-runtime state as the main coordination path. Rejected: that would increase coupling and make the reorganization harder to reason about.

### Print mode should use only the domains it needs

Print mode should not be forced to boot a full TUI-centric topology if it does not need it.

Preferred direction:

- print mode should construct only `core` and `network` domains in phase 1 unless a specific startup constraint forces a wider topology
- print mode should not require the TUI runtime
- the background domain is optional in print mode unless a concrete maintenance path needs it

This keeps the non-interactive path simpler and makes the topology reflect actual work.

**Alternative considered:** always create every domain regardless of mode. Rejected for the preferred design because print mode has a smaller runtime surface and should not depend on TUI-specific machinery without a good reason.

### Add runtime-domain observability and thread naming

Once multiple execution domains exist, debugging should reveal where work is running.

Normative direction:

- extend existing runtime debug reporting enough to reflect the domain topology
- include per-domain activity counters, runtime labels, or thread naming where practical
- make it possible to tell whether a hot path belongs to TUI, core system, networking, or background work
- keep observability lightweight and in-process, consistent with current `/debug runtime` philosophy
- when building runtimes explicitly, set thread names that expose the owning domain where practical

Minimum phase-1 observability:

- domain names must appear in thread names where supported
- `/debug runtime` or equivalent diagnostics should at least report which domains are active in the current mode

Without this, the repository would gain more complexity without enough diagnostic value.

**Alternative considered:** rely entirely on external profilers to distinguish domains. Rejected: external tools remain valuable, but the app should expose its own runtime-domain structure in at least a lightweight form.

### Phase the implementation explicitly

The implementation should land in bounded slices rather than as a single big-bang rewrite.

#### Phase 1: runtime split foundation

Required outcomes:

- replace `#[tokio::main]` with explicit runtime construction
- introduce `RuntimeDomains`, `RuntimeDomain`, and `DomainHandle`
- move TUI-local long-lived tasks to the TUI domain
- move Stylos long-lived tasks to the networking domain
- route live agent entry/orchestration onto the core domain
- keep print mode on only the domains it needs
- add minimal runtime-domain observability

Phase-1 allowed deferrals:

- provider-internal transport refactors may remain mostly in core if splitting them cleanly would expand scope too much
- background domain may start lightly used if no current maintenance workloads justify a larger first migration
- not every ambient `tokio::spawn` must disappear immediately, but touched high-level spawn sites should be domain-owned

#### Phase 2: deeper transport and maintenance placement

Candidate follow-up outcomes:

- move more provider transport work into the networking domain
- reduce remaining ambient `tokio::spawn` sites further
- move concrete maintenance workloads into the background domain
- improve per-domain counters and debug reporting
- reevaluate remaining `block_in_place` callsites

This phased contract keeps the first implementation slice realistic while still defining the intended long-term direction.

**Alternative considered:** move every spawn site at once. Rejected: too much surface area changes simultaneously for a runtime-sensitive refactor.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/main.rs` | Replace `#[tokio::main]` startup with explicit Tokio runtime construction, build `RuntimeDomains`, and select the needed domains for print mode versus TUI mode. |
| `crates/themion-cli/src/tui.rs` | Route TUI event-loop support tasks such as frame scheduling, terminal input, periodic ticks, and bridge tasks through the TUI domain instead of relying on anonymous default-runtime spawning. Review and reduce `block_in_place` usage where domain handoff is cleaner. |
| `crates/themion-cli/src/stylos.rs` | Route Stylos status publishing, queryable serving, command subscription, and related network-facing tasks through the networking domain with explicit spawn ownership. |
| `crates/themion-core/src/agent.rs` | Review live harness entry and blocking-offload paths so core orchestration work is assignable to the core domain and blocking calls do not silently depend on one implicit global runtime. |
| `crates/themion-cli/src/runtime_domains.rs` or equivalent new CLI-local module | Define `RuntimeDomain`, `DomainHandle`, `RuntimeDomains`, runtime builders, and thread-name conventions for the four execution domains. |
| `docs/architecture.md` | Update the process/thread/runtime model section to describe explicit domain-specific runtime pools, preferred domain ownership, and the single-process multi-runtime topology. |
| `docs/engine-runtime.md` | Update runtime wiring documentation so startup/runtime ownership, print-mode differences, phase-1 boundaries, and cross-domain execution boundaries are documented clearly. |
| `docs/README.md` | Keep this PRD indexed in the PRD table. |

## Edge Cases

- a provider/networking burst produces many callbacks or stream updates → verify: TUI input and redraw responsiveness do not degrade as severely as under the one-runtime baseline.
- a background maintenance task performs heavy CPU work → verify: live agent turns and TUI interaction remain responsive enough because the work is isolated to the background domain.
- Stylos is disabled at compile time or in config → verify: the runtime topology still works cleanly without requiring the networking domain to host Stylos-specific tasks.
- print mode runs without the interactive TUI loop → verify: startup constructs only the needed domains and does not depend on TUI-only tasks.
- a task needs to signal from networking back into the TUI → verify: cross-domain communication happens through explicit channels or bridges rather than accidental shared-runtime assumptions.
- blocking SQLite or file work coincides with maintenance activity → verify: important persistence work does not become starved by unrelated background offload.
- one domain runtime shuts down after an error → verify: shutdown behavior is coordinated and surfaces a clear process-level failure instead of silently orphaning work on other runtimes.
- new code adds a plain `tokio::spawn` without choosing a domain → verify: the implementation pattern or reviewable helper API makes such regressions visible and discourages them.
- the TUI current-thread runtime receives a flood of forwarded events from other domains → verify: the app loop still makes progress and backpressure/queue behavior remains understandable.
- provider transport work spans both networking and core domains → verify: the handoff boundary is explicit and does not create cyclic wait patterns between domains.
- print mode runs without `stylos` and without any background jobs → verify: only the required runtime domains are created and shutdown stays simple.

## Migration

This is an internal runtime-architecture migration inside one process.

Expected rollout shape:

- replace implicit single-runtime startup with explicit runtime construction
- introduce domain-specific spawner/runtime-handle wiring
- move existing long-lived tasks into the appropriate domains incrementally
- keep user-facing commands, tools, session storage, and protocols behaviorally compatible
- update runtime documentation and debug output alongside the code so the new topology is observable

Compatibility expectations:

- no database migration
- no protocol migration required for Stylos or tool calling
- no required user config change in the first implementation step unless a later slice adds optional runtime tuning knobs
- existing feature-flag behavior should remain intact; the runtime topology must still compile and run with and without `stylos`

## Known issues and review boundaries

The runtime-domain architecture described by this PRD remains the intended direction, but review of the implementation slice exposed a few important boundaries between work that is genuinely required for PRD-050 and work that is merely adjacent to it.

### Changes that are in-scope for PRD-050

The following kinds of changes are directly in scope for this PRD:

- replacing implicit default-runtime ownership with explicit CLI-owned runtime domains
- adding `RuntimeDomains` / `DomainHandle` plumbing needed to route work onto the `tui`, `core`, `network`, and `background` domains
- moving long-lived tasks from ambient `tokio::spawn` calls onto an explicit domain handle when that move is required to realize the runtime topology
- preserving existing behavior while relocating tasks to the appropriate domain
- updating architecture/runtime docs so they describe the reviewed runtime-domain topology that actually shipped

Concrete examples of in-scope work include:

- `crates/themion-cli/src/main.rs` runtime bootstrap changes needed to own explicit runtimes
- `crates/themion-cli/src/tui.rs` changes that place TUI scheduling work on the TUI domain
- `crates/themion-cli/src/stylos.rs` changes that move long-lived Stylos status/query/subscriber tasks onto the network domain
- startup wiring needed so TUI mode can pass the correct domain handles into Stylos and other long-lived runtime-owned services

### Changes that are not automatically justified by PRD-050

The following kinds of changes should not be treated as justified merely because they happened near the runtime-domain refactor:

- user-visible interaction changes whose purpose is not runtime placement
- protocol or status-semantics changes not required by executor-domain separation
- broad cleanup or event-loop rewrites that could be reviewed independently
- detached async replacements for formerly synchronous paths when the ordering change is not explicitly required
- removal of previously live wiring that leaves helper code disconnected or silently degrades exported state

Concrete examples that need separate justification or follow-up review if they appear in the implementation diff include:

- removing or failing to call Stylos snapshot refresh wiring such as `refresh_stylos_status()`
- dropping snapshot refresh triggers for workflow-state changes or rate-limit updates
- changing TUI key-event filtering semantics such as removing a `KeyEventKind::Press` guard
- converting small ordered local state updates from synchronous/blocking handoff into fire-and-forget `tokio::spawn` calls without reviewing the new semantics

### Required correctness follow-through for the PRD-050 implementation

Even though those behavior changes are not themselves the goal of PRD-050, the implementation of this PRD should not leave behind correctness regressions in affected paths. In particular:

- Stylos status export must remain correctly wired when runtime ownership changes
- the pull-based Stylos snapshot provider must still receive live TUI-side updates in Stylos-enabled TUI mode
- workflow-state changes, agent-activity changes, and rate-limit changes should continue to propagate to the exported snapshot promptly
- review of `block_in_place` replacements should preserve required ordering semantics where those transitions are externally visible

This section exists to keep PRD-050 scoped correctly: explicit runtime-domain ownership is the product change, while unrelated behavior drift should be split, justified separately, or fixed as follow-up cleanup.

## Testing

- start Themion in TUI mode and exercise normal local interaction → verify: the TUI remains functional and existing user flows still work after explicit runtime construction.
- trigger sustained provider/network activity while interacting with the TUI → verify: local input, redraws, and event handling remain responsive relative to the current baseline.
- run with `--features stylos` and active Stylos background activity → verify: status publishing, query serving, and inbound request handling still function when routed through the networking domain.
- run without the `stylos` feature → verify: the CLI still builds and runs cleanly with the multi-domain runtime design.
- run print mode with a normal prompt → verify: the non-TUI path still works correctly under explicit runtime ownership and does not depend on TUI-only tasks.
- start a background maintenance workload such as a synthetic CPU-heavy task in the background domain → verify: it does not materially stall TUI or core orchestration paths.
- exercise agent turns that perform normal SQLite/history persistence → verify: persistence still succeeds and does not deadlock across domain boundaries.
- inspect `/debug runtime` or equivalent runtime diagnostics after implementation → verify: the reported runtime structure exposes the domain split clearly enough for operator debugging.
- inspect OS-visible thread names after implementation where supported → verify: runtime threads can be associated with `tui`, `core`, `network`, or `background` domains.
- run print mode without Stylos enabled → verify: only the required domains are active and startup/shutdown remain clean.
- run `cargo check -p themion-cli` after implementation → verify: the default CLI build compiles cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the Stylos-enabled CLI build compiles cleanly.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates still compile together under the new runtime wiring.

## Implementation checklist

- [x] replace `#[tokio::main]` startup in `crates/themion-cli/src/main.rs` with explicit runtime construction
- [x] add a CLI-local `runtime_domains` module or equivalent shared home for runtime topology code
- [x] define `RuntimeDomain`, `DomainHandle`, and `RuntimeDomains`
- [~] assign thread names and initial builder settings for each domain runtime
- [x] verify the concrete runtime type for each domain matches the documented topology, especially whether `tui` is truly `current_thread` versus a single-worker multi-thread runtime
- [x] move TUI-local spawned tasks such as frame scheduling, input, periodic tick, and bridge tasks onto the TUI domain
- [x] verify the TUI input path itself runs by the intended mechanism on the TUI domain rather than escaping into an unmanaged blocking OS thread
- [x] move Stylos long-lived tasks onto the networking domain
- [~] route live agent entry/orchestration tasks through the core domain where practical
- [ ] audit remaining ambient `tokio::spawn` / `std::thread::spawn` / `spawn_blocking` usage in `themion-cli` and convert long-lived or domain-sensitive paths to explicit domain ownership
- [x] keep print mode limited to the domains it actually needs
- [ ] verify print mode avoids starting unnecessary domain-owned services beyond the required runtimes
- [x] define phase-1 provider/network boundary and document any intentional temporary placement in core
- [~] introduce an explicit home for background maintenance work
- [ ] either place a concrete maintenance workload on the background domain or document that the domain remains intentionally reserved in phase 1
- [~] review `spawn_blocking` and `block_in_place` usage and align blocking work with the new topology
- [~] verify cross-domain shutdown and task lifecycle behavior, including cancellation propagation and clean exit when TUI or Stylos tasks are active
- [ ] verify cross-domain communication/backpressure remains acceptable for TUI bridges and Stylos event forwarding
- [ ] verify startup and background-task errors still surface clearly after moving work onto domain-owned runtimes
- [~] add minimal runtime-domain observability to `/debug runtime` or equivalent diagnostics
- [ ] ensure runtime diagnostics report the active domains and their actual topology, not just generic process/task counters
- [ ] verify documentation matches the actual shipped runtime types, task placement, and input model exactly
- [~] update `docs/architecture.md` and `docs/engine-runtime.md`
- [x] keep `docs/README.md` aligned with this PRD entry
- [~] validate the touched CLI runtime topology in both default and `--features stylos` builds and treat feature-on/feature-off coverage as a checklist item, not just an implementation note


## Implementation notes

Implemented phase 1 as a CLI-owned explicit runtime topology:

- added `crates/themion-cli/src/runtime_domains.rs` with four named domains: `tui`, `core`, `network`, and `background`
- replaced `#[tokio::main]` startup in `crates/themion-cli/src/main.rs` with explicit runtime construction
- print mode now starts under `RuntimeDomains::for_print_mode()` and blocks on the core runtime
- TUI mode now starts under `RuntimeDomains::for_tui_mode()` and passes runtime handles into `tui::run(...)`
- TUI-local long-lived tasks such as input, tick, frame scheduling, event-forwarding bridges, and other domain-sensitive helper tasks now spawn through explicit TUI/core/background domain handles instead of ambient `tokio::spawn(...)`
- terminal input now runs through `crossterm::EventStream` on the explicit TUI Tokio domain and shuts down via a domain-owned broadcast signal during TUI teardown
- Stylos long-lived tasks now spawn on the networking domain
- thread naming is applied to multi-thread runtime workers as `themion-core`, `themion-network`, and `themion-background`; the `tui` domain now runs as a `current_thread` runtime

Phase-1 limitation that remains intentional:

- some shorter-lived or lower-level async work in `tui.rs` may still use ambient spawning patterns, and provider-internal transport work remains largely on the core execution path for now
- `/debug runtime` docs are updated to describe active runtime domains, but the runtime debug output itself was not expanded into per-domain counters in this slice
- the background domain now serves as an explicit home for domain-routed helper work, but this slice still does not demonstrate a dedicated maintenance workload placed there

Validation run for the implemented slice:

- `cargo check -p themion-cli` → passed
- `cargo check -p themion-cli --features stylos` → passed
- `cargo check -p themion-core -p themion-cli` → passed
- `cargo test -p themion-cli` → passed
- `cargo test -p themion-cli --features stylos` → passed

Follow-up analysis gaps still open after the first implementation slice:

- several `tokio::spawn` / blocking paths in `tui.rs` still need an explicit domain audit even after the latest TUI-domain routing pass
- `block_in_place` remains in TUI/Stylos-sensitive paths and should be treated as an intentional review target rather than “done enough”
- the background domain exists structurally, but this slice does not yet demonstrate a concrete maintenance workload placed there
- `/debug runtime` exposes useful activity counters, but it does not yet report the active runtime-domain topology precisely enough to prove docs/code parity on its own
- shutdown and cross-domain lifecycle behavior improved in the latest TUI task-routing slice, but they still are not exhaustively validated end-to-end
