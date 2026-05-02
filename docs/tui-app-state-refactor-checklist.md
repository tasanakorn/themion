# TUI to App State Refactor Checklist

Goal: move the TUI toward a pure surface where runtime/app-state owns canonical state and publishes shared snapshots via `watch`.


## 0. Approach, guide, and lessons learned

### 0.1. Current approach

0.1.1. Treat TUI as a surface only: it may receive human input, forward intents/events, hold presentation-local cache, request redraws, and render observed state.
0.1.2. Runtime/app-state must own canonical runtime state, shared observer publication, and cross-surface status truth.
0.1.3. Prefer the flow `runtime/app-state publish -> watch<AppSnapshot> -> surface event/cache -> render`.
0.1.4. Keep `watch::Receiver<AppSnapshot>` subscription setup outside `tui.rs`; the runner/runtime boundary may bridge watch updates into TUI presentation events such as `SnapshotUpdated`.
0.1.5. Keep snapshot construction and observer fanout outside `tui.rs`; use runtime helpers such as `AppSnapshotPublisher` and `AppRuntimeObserverPublisher`.
0.1.6. Until canonical mutation moves out of `App`, TUI may temporarily call a runtime-owned publisher after state transitions, but the publisher implementation and observer policy must not live in TUI.

### 0.2. Implementation guide

0.2.1. Before changing `tui.rs`, ask whether the code is presentation/input only. If it decides runtime truth, agent policy, watchdog policy, Stylos status, board routing, or shared publication timing, move it to runtime/app-state first.
0.2.2. For display work, add fields to `AppSnapshot` or a runtime-owned adjacent snapshot, publish them from `app_runtime.rs`, bridge changes from `tui_runner.rs`, and render from the cached snapshot in `tui.rs`.
0.2.3. Do not bind new `watch` subscriptions, hubs, publishers, or status refreshers directly into `tui.rs`.
0.2.4. Do not rederive runtime truth for display from `App.agents`, Stylos handles, watchdog atomics, or board state when an `AppSnapshot` field exists or should exist.
0.2.5. Move in thin vertical slices: publish one snapshot field, consume it in TUI, remove the old direct display dependency, update this checklist, then validate.
0.2.6. When extracting publication, prefer a runtime-owned fanout helper that updates all observers together: app snapshot, interactive system inspection, Stylos status, and future headless observers.
0.2.7. Keep live executors/resources such as `Agent`, task handles, cancellation tokens, and transport handles out of `AppSnapshot`; publish cheap cloneable view/status data.
0.2.8. Do not mark a TUI responsibility removed just because a helper moved files. The responsibility is removed only when `tui.rs` no longer owns the decision, subscription, publisher, or policy.

### 0.3. Lessons learned

0.3.1. Moving a helper out of `tui.rs` is not enough if TUI still owns the binding; check fields, constructor parameters, imports, and event-loop ownership.
0.3.2. A watcher path is decoupled only when `tui.rs` receives view events/cache updates, not when it owns `watch::Receiver<AppSnapshot>` or `AppSnapshotHub`.
0.3.3. Display decoupling should be verified by searching for direct runtime reads in render functions, especially `app.agents`, Stylos handles, watchdog state, and board state.
0.3.4. Observer publication should be centralized so TUI, Stylos, system inspection, and future headless surfaces do not each reconstruct runtime truth.
0.3.5. Feature flags can hide ownership mistakes; after every touched `themion-cli` slice, run default, `--features stylos`, and `--all-features` checks.
0.3.6. Existing warnings should not be allowed to mask new warnings. If a new warning appears in the touched scope, fix it before moving on.
0.3.7. Checklist updates should distinguish completed display/subscription decoupling from still-partial canonical mutation ownership.
0.3.8. The remaining hard boundary is canonical state ownership: as long as state transitions live on `App` in `tui.rs`, the TUI may still trigger runtime publisher calls even though publishing policy itself is runtime-owned.
0.3.9. When a watchdog or board-flow simplification is the intended product behavior, delete abandoned planning/effect/apply scaffolding instead of preserving an unused abstraction stack.
0.3.10. `tui_runner.rs` is already close to the intended boundary when it only wires terminal input, starts runtime-owned loops, subscribes to `AppSnapshotHub`, and forwards `SnapshotUpdated` events; the heavier remaining debt is in `tui.rs` canonical state ownership.
0.3.11. When deciding whether a remaining TUI call site is acceptable, separate "bridge/wiring" from "source of truth": watch subscription in `tui_runner.rs` is fine, but agent/prompt/activity state ownership and publish timing in `tui.rs` are not.

## 1. Current status

1.1. Audit started.
1.2. Focus area: watchdog ownership.
1.3. Current state: partial extraction landed in `crates/themion-cli/src/app_runtime.rs`, but ownership is still mixed because runtime helpers still depend on TUI types and TUI still applies key transitions and snapshot refreshes.
1.4. Direction decision: prefer `app_state` / runtime push-publication to the TUI, specifically `app_state` / runtime `publish -> watch -> TUI`, rather than TUI-owned pull/poll refresh.
1.5. Landed scaffold: a shared snapshot boundary now exists via `AppSnapshot` + `AppSnapshotHub` in `crates/themion-cli/src/app_state.rs`; `tui_runner.rs` passes that hub into `tui.rs`, starts a snapshot watch loop, and forwards updates to the TUI as `SnapshotUpdated` view events.
1.6. Important constraint: this does **not** yet mean runtime/app-state owns canonical mutation or that TUI is a fully pure subscriber; publication timing and several source-of-truth fields are still TUI-driven, but the executable publish -> watch -> TUI redraw path now exists.
1.7. New progress for section 5.2: runtime now owns watchdog-state recomputation logic via `recompute_watchdog_state_from_app(...)`, and the existing TUI sync wrapper delegates to that runtime helper instead of assembling duplicated raw booleans locally.
1.8. New progress for sections 5.3 and 5.4: the watchdog now follows the intended simple model. Its control loop only emits a dispatch trigger; `app_state.rs` selects an idle local agent, claims one pending board note, and injects that work through the existing incoming-prompt / agent-loop path.
1.9. Dead-code cleanup progress: the unused `WatchdogDispatch*` planning stack and now-unused TUI-side shell-command sender plumbing have been removed, and the current `themion-cli` builds are warning-free in the checked configurations.
1.10. Remaining TUI-specific blocker summary: `tui_runner.rs` is mostly reduced to surface wiring, but `tui.rs` still owns canonical `App` / `AgentHandle` state, including local agent state, activity/idle timing, and feature-gated incoming-prompt ownership.
1.11. Remaining publication blocker summary: shared observer publication is centralized in runtime helpers, but TUI-owned methods still decide when several publication calls run.

## 2. State flow target

2.1. Preferred state flow:

```text
TUI --intent/command--> app_state/runtime
app_state/runtime --publish--> watch<AppSnapshot>
watch<AppSnapshot> --latest snapshot--> TUI
watch<AppSnapshot> --latest snapshot--> Stylos/status publication
watch<AppSnapshot> --latest snapshot--> headless/other surfaces
```

2.2. Use `mpsc` for intents/commands flowing into runtime ownership.
2.3. Use `watch` for the canonical latest-value app snapshot flowing out to observers.
2.4. Avoid TUI-owned polling or refresh-timing decisions for canonical runtime truth.
2.5. Keep heavy live resources such as `Agent` objects out of the watched value; publish cheap-to-clone summary state instead.
2.6. Current implementation status: the publish/subscribe boundary scaffold exists, but the snapshot is still thin and the TUI still owns most mutation timing and source-of-truth responsibilities.

## 3. Findings and progress

### 3.1. Finding 1: watchdog recomputation moved out of direct TUI field mutation, but ownership is still mixed

3.1.1. Files:
- `crates/themion-cli/src/app_state.rs`
- `crates/themion-cli/src/app_runtime.rs`
- `crates/themion-cli/src/tui.rs`

3.1.2. Function points:
- `app_state::set_agent_activity`
- `app_state::clear_agent_activity`
- `app_runtime::sync_watchdog_runtime_state`
- `app_runtime::recompute_watchdog_state_from_app`

3.1.3. Evidence:
- activity transitions now call runtime-owned watchdog sync helpers instead of directly mutating watchdog fields in `tui.rs`
- watchdog recomputation derives state from current agent state
- the recompute path still depends on `crate::tui::AgentHandle` and is still triggered from TUI-owned state transitions

3.1.4. Why it is still partial:
- direct watchdog field mutation was removed, which is progress
- but canonical busy/idle/prompt state is still TUI-owned, so runtime watchdog truth is still derived from TUI-owned state

3.1.5. Target owner:
- runtime/app-state layer

3.1.6. Progress:
- [x] `WatchdogRuntimeState` now lives in `crates/themion-cli/src/app_runtime.rs`
- [x] runtime-side sync path exists via `WatchdogRuntimeState::sync_from_runtime_state(...)` and `sync_watchdog_runtime_state(...)`
- [x] runtime-owned `recompute_watchdog_state_from_app(...)` now derives watchdog state from agent/app state
- [~] watchdog-related refresh fanout now routes through `publish_runtime_snapshot()` instead of separate refresh call sites
- [ ] canonical watchdog inputs still live on TUI-owned agent/app state

### 3.2. Finding 2: TUI handles watchdog dispatch orchestration instead of only forwarding or rendering it

3.2.1. Files:
- `crates/themion-cli/src/tui.rs`
- `crates/themion-cli/src/app_runtime.rs`
- `crates/themion-cli/src/app_state.rs`

3.2.2. Function points:
- historical `AppEvent::WatchdogDispatchRequested` / TUI-side dispatch handling
- current `start_watchdog_task(...)`
- current `handle_watchdog_dispatch_event(...)`

3.2.3. Historical evidence before cleanup:
- TUI built watchdog dispatch plans
- TUI finalized note injection, logged watchdog events, applied incoming prompts, and launched agent turns

3.2.4. Why it was a violation:
- This was watchdog policy execution and runtime orchestration living inside the TUI event loop.
- The TUI acted as the owner of watchdog dispatch effects rather than as a surface that observed or forwarded them.

3.2.5. Target owner:
- runtime/orchestrator layer above services and below TUI

3.2.6. Progress:
- [x] removed the unused `WatchdogDispatch*` planning/request/effect/apply scaffolding from `crates/themion-cli/src/app_runtime.rs`
- [x] watchdog control loop now only emits a dispatch trigger/log event
- [x] `app_state.rs` now owns the live dispatch intake path for selecting an idle local agent and claiming one pending note
- [x] watchdog-triggered board work now reuses the normal incoming-prompt / agent-loop path
- [x] TUI no longer owns watchdog dispatch orchestration for this flow
- [~] dispatch ownership is improved, but canonical agent/app state still lives on TUI-owned types

### 3.3. Finding 3: TUI applies incoming-prompt/watchdog linkage directly by mutating agent and watchdog state together

3.3.1. Files:
- `crates/themion-cli/src/tui.rs`
- `crates/themion-cli/src/app_runtime.rs`
- `crates/themion-cli/src/app_state.rs`

3.3.2. Function points:
- historical `App::handle_incoming_prompt_event`
- current `handle_incoming_prompt_event` in `app_state.rs`
- `set_active_incoming_prompt`
- `clear_active_incoming_prompt`
- `apply_active_incoming_prompt`
- `continue_current_note_follow_up`

3.3.3. Evidence:
- incoming-prompt planning/apply helpers live in runtime code
- `app_state.rs` now routes both remote Stylos prompts and watchdog-triggered board-note prompts through one shared `process_incoming_prompt_request(...)` helper
- active-incoming-prompt application still mutates TUI-owned agent handles

3.3.4. Why it is still partially a violation:
- The dedicated intake path moved out of `tui.rs`, which is progress.
- But the canonical state being mutated is still structurally tied to `crate::tui::AgentHandle`.

3.3.5. Target owner:
- app-state/runtime transition layer

3.3.6. Progress:
- [x] runtime-side planning helpers exist:
  - `resolve_incoming_prompt_disposition(...)`
  - `plan_incoming_prompt(...)`
  - `incoming_prompt_apply_plan(...)`
- [x] runtime-side apply helpers exist for prompt state transitions:
  - `set_active_incoming_prompt(...)`
  - `clear_active_incoming_prompt(...)`
  - `apply_active_incoming_prompt(...)`
  - `continue_current_note_follow_up(...)`
- [x] `handle_incoming_prompt_event` intake/apply flow now lives in `crates/themion-cli/src/app_state.rs`
- [x] watchdog-triggered board-note intake now reuses that same shared path
- [ ] active incoming-prompt state still lives on TUI-owned agent handles

### 3.4. Finding 4: runtime extraction is partial because `app_runtime.rs` still depends on TUI-owned types for canonical state and transitions

3.4.1. Files:
- `crates/themion-cli/src/app_runtime.rs`
- `crates/themion-cli/src/tui.rs`

3.4.2. Function points:
- `prepare_agent_turn_runtime_launch`
- `prepare_agent_turn_submit`
- `prepare_agent_turn_execution`
- `build_local_agent_status_entries`
- `set_active_incoming_prompt`
- `current_activity_label`
- `current_activity_detail`

3.4.3. Evidence:
- runtime helpers accept and mutate `&mut [crate::tui::AgentHandle]`
- runtime helper labeling still matches on `crate::tui::AgentActivity`
- watchdog and incoming-prompt helpers operate on `AgentHandle.active_incoming_prompt`, which is still defined in `tui.rs`

3.4.4. Why it is a violation:
- Logic moved out of the main TUI event loop is still structurally coupled to TUI-owned state types.
- As long as runtime code depends on `tui.rs` models, the TUI remains the de facto source of truth and other surfaces cannot share the same hub-owned state cleanly.

3.4.5. Target owner:
- app-state/runtime model layer with TUI-consuming projections only

3.4.6. Progress:
- [x] shared non-UI planning/state helpers have started moving into `crates/themion-cli/src/app_runtime.rs`
- [~] a first runtime-owned shared snapshot boundary scaffold now exists in `crates/themion-cli/src/app_state.rs` via `AppSnapshot` and `AppSnapshotHub`
- [ ] canonical agent/app state types are still defined in or imported from `tui.rs`
- [ ] current `AppSnapshot` is only a thin first slice, not yet the full canonical app-state model

### 3.5. Finding 5: TUI still owns snapshot publication triggers for Stylos status and interactive system inspection

3.5.1. Files:
- `crates/themion-cli/src/tui.rs`
- `crates/themion-cli/src/app_runtime.rs`

3.5.2. Function points:
- `publish_runtime_snapshot()` call sites in `tui.rs`
- runtime observer publication helpers in `app_runtime.rs`

3.5.3. Evidence:
- runtime-side snapshot builders exist, but the TUI still decides when to publish refreshed shared state after many state transitions

3.5.4. Why it is a violation:
- Shared runtime truth for Stylos and local inspection should be emitted by the runtime owner when state changes, not refreshed opportunistically by the UI loop.
- If TUI remains the trigger point, headless and future non-TUI surfaces risk drift or duplicate logic.

3.5.5. Target owner:
- runtime/app-state change application path

3.5.6. Progress:
- [x] runtime-side snapshot builders/helpers exist in `crates/themion-cli/src/app_runtime.rs`
- [~] `tui.rs` now routes the existing refresh fanout through one central `publish_runtime_snapshot()` method
- [ ] `tui.rs` still decides when that shared publication method runs
- [ ] Stylos status and system inspection are still derived from TUI-owned canonical state, even though publication is now centralized

### 3.6. Finding 6: most remaining work is in `tui.rs`, while `tui_runner.rs` is already close to the intended bridge role

3.6.1. Files:
- `crates/themion-cli/src/tui.rs`
- `crates/themion-cli/src/tui_runner.rs`

3.6.2. Function points:
- `App` struct and `AgentHandle` in `tui.rs`
- `App::submit_text`
- `App::maybe_emit_done_mention_for_completed_note`
- `App::new`
- `App::replace_master_agent`
- `App::handle_local_agent_management_request`
- `start_snapshot_watch_loop(...)` in `tui_runner.rs`

3.6.3. Evidence:
- `tui_runner.rs` subscribes to `AppSnapshotHub` outside `tui.rs` and forwards `AppEvent::SnapshotUpdated(snapshot)`
- `tui_runner.rs` starts the tick loop and watchdog loop without reconstructing runtime truth locally
- `tui.rs` still defines `AgentHandle`, stores `active_incoming_prompt`, owns `agents: Vec<AgentHandle>`, `agent_activity`, and idle timing fields
- `submit_text(...)` still inspects `self.agents[*].active_incoming_prompt` and may call `clear_active_incoming_prompt(...)`
- `maybe_emit_done_mention_for_completed_note(...)` still reads `active_incoming_prompt` from TUI-owned agent state and may call `continue_current_note_follow_up(...)`
- `App::new`, `replace_master_agent(...)`, and `handle_local_agent_management_request(...)` still call `app_state_publish_runtime_snapshot(...)`

3.6.4. Why it is a violation:
- The runner is largely acting as a bridge, but `tui.rs` still owns state and transition timing that should belong to runtime/app-state.
- This keeps TUI as the effective source of truth even after the watch boundary landed.

3.6.5. Target owner:
- keep `tui_runner.rs` as bridge/wiring only
- move remaining `tui.rs` canonical state ownership and publication timing into runtime/app-state

3.6.6. Progress:
- [x] `tui_runner.rs` owns watch subscription and `SnapshotUpdated` forwarding
- [x] `tui_runner.rs` is reduced to terminal/runtime wiring for snapshot, tick, and watchdog loops
- [ ] `tui.rs` still owns canonical agent/prompt/activity state
- [ ] `tui.rs` still contains snapshot publication trigger call sites

## 4. Validation already completed

4.1. [x] `cargo check -p themion-cli`
4.2. [x] `cargo check -p themion-cli --features stylos`
4.3. [x] `cargo check -p themion-cli --all-features`

## 5. Execution sequence checklist

### 5.1. Sequence A: strengthen the snapshot boundary

- [ ] 5.1.1. Expand `AppSnapshot` from the current thin boundary into the real runtime-owned summary type.
- [x] 5.1.2. Add watchdog-visible fields to `AppSnapshot`.
- [~] 5.1.3. Add the canonical fields needed by Stylos status and system inspection to `AppSnapshot` or a clearly adjacent runtime-owned snapshot/state bundle.
- [x] 5.1.4. Keep live executor resources such as `Agent` objects outside `AppSnapshot`.

### 5.2. Sequence B: move watchdog state recalculation behind runtime ownership

### 5.2.a. Executable publish -> watcher order

- [~] 5.2.a.1. Add a non-TUI publisher layer.
  - [x] choose the runtime owner: app-state / hub
  - [x] move snapshot construction there
  - [~] publish on runtime-owned state changes
  - [~] make it the shared source of truth
- [x] 5.2.a.2. Add a watcher path where TUI only receives and stores view state.
  - [x] keep `watch::Receiver<AppSnapshot>` outside TUI in the runner/runtime boundary
  - [x] update local presentation cache from snapshot update events
  - [x] request redraw from snapshot update events
  - [x] do not subscribe to `watch` or own snapshot hub/publisher state in TUI
- [x] 5.2.a.3. Wire snapshot state into display.
  - [x] render watchdog/status/review data from watched snapshot
  - [x] keep watchdog review display on presentation snapshot state only
  - [x] remove direct display dependencies on runtime-owned state where snapshot fields exist
- [~] 5.2.a.4. Remove TUI-owned publisher/invoker.
  - [x] delete `tui.rs` snapshot publishing/building responsibility
  - [x] remove `AppSnapshotHub` and `watch::Receiver<AppSnapshot>` ownership from TUI
  - [x] bind snapshot publication through runtime-owned `AppSnapshotPublisher`
  - [x] move observer fanout for snapshot, system inspection, and Stylos status into runtime-owned `AppRuntimeObserverPublisher`
  - [~] remove remaining non-visual sync/publication call sites from TUI
  - [~] leave TUI as input/output only
  - [x] route safe workflow/rate-limit/runtime-command refresh call sites through runtime observer publication fanout
  - [ ] remove remaining `app_state_publish_runtime_snapshot(...)` trigger sites from `App::new`, `replace_master_agent(...)`, and local-agent-management handling in `tui.rs`

- [x] 5.2.1. Add a single runtime-owned `recompute_watchdog_state_from_app(...)` helper that derives watchdog fields from canonical app state.
- [~] 5.2.2. Call that helper from every runtime-owned state transition that changes busy/idle, active incoming prompt, or pending watchdog note.
- [x] 5.2.3. Move watchdog recomputation trigger points off `tui.rs` / TUI-owned `App` state changes and into the runtime-owned state-application path.
- [~] 5.2.4. Make TUI render watchdog-derived state from the latest watched snapshot instead of mutating watchdog state itself.

### 5.3. Sequence C: move incoming-prompt transitions under runtime ownership

- [~] 5.3.1. Replace `App::handle_incoming_prompt_event` with a runtime-owned incoming-prompt transition entrypoint.
- [~] 5.3.2. Make that entrypoint perform: disposition planning -> apply accepted prompt state -> recompute watchdog state -> publish snapshot.
- [ ] 5.3.3. Return explicit runtime effects for any follow-up work such as task failure publication, transcript log emission, or agent-turn launch.
- [x] 5.3.4. Ensure prompt accept, busy-target rejection, and missing-target rejection all flow through the same runtime-owned transition path.
- [~] 5.3.5. Keep TUI limited to forwarding the incoming prompt intent and rendering the resulting snapshot/effects.
- [ ] 5.3.6. Remove `active_incoming_prompt` ownership from `tui::AgentHandle` and migrate `submit_text(...)` / `maybe_emit_done_mention_for_completed_note(...)` to runtime-owned prompt state lookups/effects.

### 5.4. Sequence D: move watchdog dispatch orchestration under runtime ownership

- [~] 5.4.1. Introduce a runtime-owned watchdog-dispatch command path that accepts a dispatch request/trigger and returns concrete runtime effects.
- [x] 5.4.2. Move note-claim finalization, incoming-prompt apply, launch preparation, and agent-turn start out of `App::handle_app_event`.
- [ ] 5.4.3. Represent watchdog dispatch results as runtime effects such as:
  - [ ] 5.4.3.a. log line to emit
  - [ ] 5.4.3.b. prompt assignment to apply
  - [ ] 5.4.3.c. agent turn to launch
  - [ ] 5.4.3.d. snapshot update to publish
- [x] 5.4.4. Keep the watchdog loop trigger as intake only.
- [~] 5.4.5. Have runtime publish the post-dispatch `AppSnapshot` through `watch`.
- [~] 5.4.6. Reduce TUI responsibility to forwarding the trigger and rendering emitted lines/snapshots.

### 5.5. Sequence E: move canonical state types out of `tui.rs`

- [ ] 5.5.1. Define remaining runtime-owned canonical state types outside `tui.rs`, at minimum:
  - [ ] 5.5.1.a. `RuntimeAgentState`
  - [ ] 5.5.1.b. `RuntimeIncomingPromptState`
  - [ ] 5.5.1.c. `RuntimeActivityState`
- [ ] 5.5.2. Move ownership of canonical busy/activity/incoming-prompt fields into those runtime-owned types.
- [ ] 5.5.3. Make runtime planning/apply helpers depend on runtime-owned types instead of `crate::tui::AgentHandle` and `crate::tui::AgentActivity`.
- [ ] 5.5.4. Add projection/helpers that convert runtime-owned snapshot state into the view data TUI needs.
- [ ] 5.5.5. Keep only presentation-local state in `tui.rs` such as focus, scroll, composer buffers, and render-local formatting.
- [ ] 5.5.6. Remove `AgentHandle` / `AgentActivity` canonical ownership from `tui.rs`; if view-specific wrappers remain, make them pure projections over runtime-owned state.

### 5.6. Sequence F: move shared observers onto the same source of truth

- [ ] 5.6.1. Move snapshot refresh triggering into the same runtime-owned change-application path that mutates canonical state.
- [ ] 5.6.2. Derive Stylos status and system inspection from the same canonical snapshot or adjacent runtime-owned state updated in the same transition path.
- [ ] 5.6.3. Remove TUI-owned decisions about when shared status is refreshed.
- [ ] 5.6.4. Keep TUI as a subscriber/consumer of snapshots and a renderer of resulting lines only.
- [ ] 5.6.5. Validate that TUI, headless, and Stylos observe the same updated state source.


### 5.7. Execute-next slice for the remaining TUI extraction work

- [ ] 5.7.1. Create a runtime-owned replacement for the canonical state now stored on `tui::App` / `tui::AgentHandle`.
- [ ] 5.7.2. Move `active_incoming_prompt` ownership out of `tui::AgentHandle` and into that runtime-owned state.
- [ ] 5.7.3. Port runtime helpers in `app_runtime.rs` to depend on the new runtime-owned state instead of `crate::tui::AgentHandle` / `crate::tui::AgentActivity`.
- [ ] 5.7.4. Update incoming-prompt submission and completed-note follow-up paths so `tui.rs` forwards intents/effects instead of reading or mutating canonical prompt state directly.
- [ ] 5.7.5. Move the remaining shared snapshot publication trigger sites out of `tui.rs`, specifically the calls from `App::new`, `replace_master_agent(...)`, and local-agent-management handling.
- [ ] 5.7.6. Recompute watchdog/app snapshot publication from the runtime-owned change-application path after the canonical state move lands.
- [ ] 5.7.7. Keep `tui_runner.rs` limited to bridge/wiring duties unless the runtime-owned state move proves a narrow runner change is strictly required.
- [ ] 5.7.8. Validate the slice with `cargo check -p themion-cli`, `cargo check -p themion-cli --features stylos`, and `cargo check -p themion-cli --all-features`.

## 6. Validation sequence checklist

- [x] 6.1. After each meaningful migration slice, run the narrowest useful validation first.
- [x] 6.2. When changing `themion-cli`, use:
  - [x] 6.2.a. `cargo check -p themion-cli`
  - [x] 6.2.b. `cargo check -p themion-cli --features stylos`
  - [x] 6.2.c. `cargo check -p themion-cli --all-features`
- [ ] 6.3. When the slice specifically touches incoming-prompt, watchdog, or status-publication behavior, also perform targeted behavior checks where practical.
- [x] 6.4. Before declaring the refactor slice done, confirm that no new warnings were introduced in the touched scope.

## 7. Reference summary

7.1. Boundary scaffold landed: yes.
7.2. Canonical runtime-owned app state landed: no.
7.3. TUI as pure subscriber landed: partial; watchdog review display now consumes a local snapshot cache updated from `watch`, but mutation/publication timing is still not fully runtime-owned.
7.4. Shared `watch` snapshot exists: yes, but still thin.
7.5. Highest-priority next step: move canonical agent/activity/incoming-prompt state out of `tui.rs` so runtime/app-state stops mutating `crate::tui::AgentHandle` directly.
7.6. 5.2.a progress: snapshot construction and publishing now live in `crates/themion-cli/src/app_runtime.rs` via `AppSnapshotBuildState` + `AppSnapshotPublisher`; observer fanout now lives in `AppRuntimeObserverPublisher`; `tui_runner.rs` owns the snapshot watch loop; `tui.rs` only receives `SnapshotUpdated`, caches that view snapshot, and redraws from it. Watchdog review summary, Stylos status text, and local-agent rows now render from `AppSnapshot`. TUI still invokes the runtime observer publisher after TUI-owned state transitions because canonical mutation has not yet moved out of `App`, so publication timing remains partial.
7.7. Validation status: `cargo check -p themion-cli`, `cargo check -p themion-cli --features stylos`, and `cargo check -p themion-cli --all-features` passed for the current watchdog/app-state slice.
7.8. 5.2.a follow-up progress: `AppSnapshot` now includes `local_agents` and feature-gated `stylos_status`; watchdog review display no longer reads direct agent rows or Stylos handle state where those snapshot fields exist. Safe workflow/rate-limit/runtime-command refresh sites now route through runtime observer publication fanout. TUI no longer owns `AppSnapshotHub`, `watch::Receiver<AppSnapshot>`, watch subscription setup, snapshot publishing, system-inspection refresh, or Stylos-status refresh. Remaining partial items require a larger runtime-owned mutation/publication path, not another TUI display patch.
7.9. 5.2.a stopping point: within the requested slice, display/watch wiring is complete and safe TUI direct refreshes were centralized. The only remaining partial 5.2.a items are architectural owner moves: making runtime/app-state the mutation/publication trigger and removing the TUI `publish_runtime_snapshot()` invoker entirely. That crosses into 5.2.2/5.2.3/5.6 work because canonical state transitions still live on `App` in `tui.rs`.
7.10. Decoupling correction: implementation binding was moved off the TUI side. `tui.rs` no longer owns the snapshot hub/receiver or watch subscription; `AppSnapshotPublisher` lives in `app_runtime.rs`, and `tui_runner.rs` bridges watched snapshot updates into presentation events.
7.11. 5.2.a.3-5.2.a.4 implementation: `AppRuntimeObserverPublisher` now owns observer fanout for `AppSnapshot`, interactive system inspection, and Stylos status. `tui.rs` no longer imports or calls the individual refresh helpers; it supplies current state to the runtime publisher until canonical state transitions move out of `App`.
7.12. Watchdog simplification update: the live watchdog path now matches the intended product behavior. The control loop only looks for an idle window and triggers dispatch; `app_state.rs` selects an idle agent plus a pending board note and injects it into the normal incoming-prompt / agent-loop path. The abandoned `WatchdogDispatch*` abstraction stack was removed instead of preserved.
7.13. Remaining `tui*.rs` work summary: `tui_runner.rs` is largely in the desired shape as a bridge that wires input, starts runtime-owned loops, subscribes to `AppSnapshotHub`, and forwards `SnapshotUpdated` events. The heavier remaining work is all in `tui.rs`: move canonical `App` / `AgentHandle` ownership out, remove `active_incoming_prompt` from TUI-owned agent state, and delete the remaining TUI-owned snapshot publication trigger sites.
7.14. Next slice: start with Sequence E and the linked publication follow-through items. Do not spend more time on `tui_runner.rs` watch wiring or display-only `tui.rs` edits first.


7.15. Best next fix checklist:
- [ ] 7.15.1. Move `active_incoming_prompt` off `tui::AgentHandle`.
- [ ] 7.15.2. Move `agent_busy`, workflow, roster, watchdog, and Stylos status ownership into app-state/runtime-owned canonical state and snapshot projection.
- [ ] 7.15.3. Make `submit_text_to_agent(...)` a runtime/app-state action instead of a TUI method.
- [ ] 7.15.4. Reduce `tui::App` to UI-local state only:
  - [ ] 7.15.4.a. composer
  - [ ] 7.15.4.b. scroll/review/focus
  - [ ] 7.15.4.c. dirty flags
  - [ ] 7.15.4.d. local render cache
