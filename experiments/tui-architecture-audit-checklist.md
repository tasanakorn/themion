# TUI architecture audit checklist

Status: active audit record (current code rechecked; routine helper extraction in the current seam is exhausted; remaining items are either runtime snapshot finishing work or deliberate larger state-owner redesigns only)
Scope: validate `crates/themion-cli/src/tui.rs` and `crates/themion-cli/src/tui_runner.rs` item-by-item against the repository layering rules before moving code.

## Audit rules

Use these labels for every meaningful struct, enum, and function.

- `OK` — TUI surface or terminal-runner responsibility only
- `MIXED` — combines UI concerns with runtime/orchestrator concerns and should be split
- `VIOLATION` — owns runtime/orchestrator/Stylos/board/roster source-of-truth behavior that the repo says must not live in TUI

Inline classification tags:
- `[set1]` — Stylos-related `VIOLATION`
- `[set2]` — agent management / orchestration `VIOLATION`
- `[set3]` — other runtime-ownership `VIOLATION`
- `[set4]` — `MIXED`

Execution tags:
- `[phaseA]` — compile-baseline repair prerequisite
- `[phaseB]` — shared runtime-owned Stylos prerequisites
- `[phaseC]` — Set 1 Stylos/runtime ownership moves
- `[phaseD]` — Set 3 command/runtime split
- `[phaseE]` — Set 2 and Set 4 follow-on cleanup

Key repository standard being checked:

- `TUI` is an input/output surface
- `HUB / APP_STATE` owns runtime truth
- `STYLOS` and `AGENT CORE` are lower-layer services consumed by runtime ownership code
- TUI must not become canonical owner of roster, workflow, Stylos publication/query state, board routing, or incoming-prompt policy

## Dependency-ordered execution queue

Use the set tags for classification, but execute by dependency order using the phase tags below.

### Phase A `[phaseA]` — restore a compiling baseline first

Progress update:
- [x] compile baseline restored for `themion-cli --features stylos`
- [x] stale Stylos/runtime field references reconciled without reverting to the removed direct snapshot-provider path
- [x] post-repair validation rerun with `cargo check -p themion-cli --features stylos` and `cargo check -p themion-cli --all-features`

Goal: repair the current intermediate state before attempting larger ownership moves.

Why first:
- current `tui.rs` references some Stylos/runtime fields that no longer exist on `App`
- ownership moves are harder to validate while the feature-on build is already broken

Immediate repair targets:
1. [x] reconcile stale references to `last_sender_side_transport_event`
2. [x] reconcile stale references to `stylos_tool_bridge`
3. [x] reconcile stale references to `watchdog_state`
4. [x] reconcile stale references to `board_claims`
5. [x] run `cargo check -p themion-cli --features stylos`

Notes:
- this is a prerequisite phase, not a new classification set
- prefer the smallest repair that restores a trustworthy baseline without reintroducing TUI ownership unnecessarily

### Phase B `[phaseB]` — establish shared runtime-owned Stylos prerequisites

Goal: confirm where the moved state actually lives before migrating more call sites.

Prerequisite state to locate or introduce:
- [x] incoming prompt admission state
- [x] watchdog state
- [x] board-claim / board-follow-up state
- [x] sender-side Stylos transport event state
- [x] Stylos tool-bridge access
- [~] system-inspection publication ownership (runtime-owned build/apply plus refresh-from-runtime assembly are moved; App still computes current debug/activity snapshot values before delegating refresh)

Expected outcome:
- one runtime-owned source of truth for these flows
- TUI consumes intents, events, or snapshots instead of owning policy/state

### Phase C `[phaseC]` — execute Set 1 by slice

Progress update:
- [x] shared status hub path restored and publishing again
- [x] old git metadata/status snapshot shape restored on top of the new shared hub
- [x] Stylos receiver wiring moved out of `App`/`tui.rs` and into `tui_runner.rs`
- [~] status snapshot build/publish policy moved further into `app_runtime.rs`; `refresh_stylos_status()` now delegates snapshot-input assembly, but refresh triggering still originates in `App`

Recommended order:
1. [x] move Stylos receiver wiring and shutdown ownership
2. [~] move incoming prompt and Stylos command policy
   - planning/apply-plan helpers are extracted, but TUI still owns final prompt submission, log-entry emission, and task-failure publication side effects
3. [~] move watchdog and board-note follow-up policy
   - follow-up planning/apply-plan helpers are extracted, but TUI still owns final done-mention emission display and prompt submission side effects
4. [x] move task-registry side effects out of `process_agent_event`
   - direct `set_running` / `set_completed` / `set_failed` calls now live behind runtime-owned publication helpers in `app_runtime.rs`; `App` still decides when to invoke them from the current event flow
5. [~] move system-inspection snapshot/publication ownership
6. [~] verify TUI is only rendering Stylos/runtime state
   - current blocker is that TUI still owns several final side effects around prompt submission, board/task follow-up, and live agent-handle mutation

### Phase D `[phaseD]` — execute Set 3 after Set 1 prerequisites are stable

Reason:
- `App::handle_command` depends on runtime boundaries that Set 1 and the compile-baseline repair make clearer
- splitting it earlier risks moving code against unstable ownership lines

Steps:
1. [ ] separate pure UI/display commands from runtime-owned commands
2. [ ] route runtime-owned commands through app-state/runtime intents
3. [ ] leave only local view behavior in `tui.rs`

### Phase E `[phaseE]` — continue with Set 2 and Set 4 follow-on cleanup

Reason:
- roster/orchestration and mixed cleanup become safer once Stylos/runtime ownership paths are real and compiling

## Next-pass execution plan

Use this section as the concrete step-by-step plan for the remaining work. The pending items should be executed as small slices instead of one broad `App` de-ownership pass.

### Slice 1 — finish the remaining narrow Phase C leftovers first

1. [x] inspect `App::refresh_stylos_status()`
   - identified the remaining local work as snapshot-input assembly plus refresh triggering
   - moved snapshot-input assembly behind a runtime helper in `app_runtime.rs`
2. [x] move the remaining status snapshot input assembly out of `tui.rs` if practical
   - `refresh_stylos_status()` now delegates to `refresh_stylos_status_snapshot(...)`
   - `tui.rs` still triggers refresh on state changes only
3. [x] inspect `App::refresh_main_agent_system_inspection`
   - isolated the remaining App-side work as current debug/activity snapshot value gathering only
   - moved refresh-from-runtime assembly/delegation into `app_runtime.rs`
4. [x] decide whether system-inspection input gathering can move fully out of `App`
   - current state is a thin adapter that computes current values and delegates refresh
   - current conclusion: `debug_runtime_lines()` and related current-value gathering should remain local for now because they read live App-owned process/thread/runtime state directly, and moving them further would start to overlap with the broader App/live-agent ownership redesign rather than a narrow safe extraction
5. [x] after each meaningful edit, run:
   - `cargo check -p themion-cli`
   - `cargo check -p themion-cli --features stylos`
   - `cargo check -p themion-cli --all-features`

Exit criteria for Slice 1:
- `refresh_stylos_status()` no longer assembles runtime-owned snapshot inputs inside `tui.rs`; it now acts as a refresh trigger/thin adapter ✅
- `refresh_main_agent_system_inspection` is reduced to a clearly thin adapter that computes current values and delegates refresh ✅
- the checklist can mark the remaining Phase C leftovers as complete for the narrow extraction pass, with any further movement deferred to the larger ownership redesign boundary

### Slice 2 — classify `App::handle_command` before moving code

1. [x] read `App::handle_command` branch-by-branch
2. [x] label each branch as one of:
   - pure UI/view behavior
   - runtime intent / orchestration behavior
   - mixed branch that needs extraction before classification is clean
3. [x] write down the branch map in this checklist or a nearby scratch note before editing code
4. [x] identify the runtime-owned subgroups inside `handle_command`
   - profile/model rebuild paths
   - commands that mutate canonical runtime/session/agent state
   - commands that should become hub/app-runtime intents

Exit criteria for Slice 2:
- every branch in `handle_command` has an ownership label ✅
- runtime rebuild behavior is isolated as a named subproblem instead of buried in the larger function ✅

### Slice 3 — extract pure UI/view command handling first

1. [~] keep only display-local command behavior in `tui.rs`
   - command-local display/reporting branches are already separated from runtime-intent branches in the current seam
   - broader non-command UI behavior such as scrolling, review mode toggles, and input editing remains in `tui.rs` by design
2. [x] move or split mixed helper branches only as much as needed to preserve this boundary
3. [x] re-run narrow validation after the extraction

Exit criteria for Slice 3:
- `handle_command` is visibly smaller relative to the earlier mixed state ✅
- UI-local commands remain in `tui.rs` ✅
- runtime-owned command branches are separated enough to move independently ✅

### Slice 4 — move runtime-owned command branches behind runtime intents

1. [x] introduce or reuse runtime-owned entry points for command-side orchestration
   - runtime commands now go through shared runtime helpers and `AppEvent::RuntimeCommand`
2. [x] convert runtime-owned `handle_command` branches from direct mutation to intent handoff
3. [x] remove direct TUI ownership of canonical runtime/session/roster mutations where practical
4. [x] keep error/reporting results flowing back as renderable state, not as renewed TUI ownership

Exit criteria for Slice 4:
- `handle_command` no longer directly owns most runtime decisions ✅
- TUI primarily translates user input into runtime intents and renders results ✅

### Slice 5 — isolate runtime-rebuild logic as its own mini-refactor

1. [x] identify all profile/model-triggered rebuild branches inside `handle_command`
2. [x] map the exact runtime-owned state they replace or restart
3. [x] move rebuild orchestration behind one runtime-owned entry point
4. [x] keep TUI responsible only for triggering the rebuild and rendering success/failure
5. [x] validate default, Stylos-on, and `--all-features` builds after the move

Exit criteria for Slice 5:
- runtime rebuild logic is no longer buried in TUI command handling ✅
- rebuild behavior has one runtime-owned orchestration path ✅

### Slice 6 — reassess deeper `App` de-ownership only after the above lands (larger redesign boundary)

1. [~] re-check whether `App` still deserves `VIOLATION [set2]`
   - current answer appears to be “yes, but narrower than before”: command-side and snapshot assembly work moved out, while live agent-handle mutation and Stylos/task side effects still remain in `App`
2. [~] re-check whether `AgentHandle`, `is_interactive_handle`, and `normalize_primary_role` still block the intended boundary
   - `AgentHandle` and direct `self.agents` access remain the main blocker for moving final prompt/task/status side effects out of TUI-owned code paths
3. [x] update this checklist’s classifications based on the new ownership reality
4. [x] decide whether further extraction is justified by a real remaining ownership problem rather than by checklist inertia
   - yes: the remaining issue is no longer command/helper cleanup; it is specifically the live agent/stylos side-effect ownership boundary

Exit criteria for Slice 6:
- the remaining work is a fresh, accurate post-refactor list
- no further broad cleanup is started without a concrete ownership reason
- helper-only extraction is treated as complete unless a new concrete ownership leak is identified


### Current next-pass branch map for `App::handle_command`

Pure UI/view or local-session branches still living in `tui.rs`:
- `/debug runtime`
- `/context`
- `/config`
- `/session show`
- `/config profile [list]`
- `/config profile show`
- usage/help and unknown-command rendering

Runtime-intent branches now routed through `AppEvent::RuntimeCommand`:
- `/login codex`
- `/semantic-memory index`
- `/semantic-memory index full`
- `/debug api-log enable|disable`
- `/clear`
- `/session profile use <name>`
- `/session model use <model>`
- `/session reset`
- `/config profile create <name>`
- `/config profile use <name>`
- `/config profile set key=value`

Command-seam status after the current extraction pass:
- branch classification is effectively complete for the current `handle_command` shape
- the previously mixed runtime-mutation branches in this seam are now routed through runtime intents
- no obvious additional narrow command-branch extraction remains without moving into broader `App`/live-agent ownership questions or non-command TUI concerns

## Latest progress notes

Second big-bang completion pass:
- moved the remaining direct Stylos task-registry publications (`set_running`, `set_completed`, `set_failed`) out of `tui.rs` and behind `app_runtime.rs` helpers.
- `process_agent_event`, submit-target failure handling, and incoming-prompt rejection handling now invoke runtime publication helpers instead of spawning task-registry mutations inline.
- direct task-registry side-effect ownership in `App::process_agent_event` is complete for the current seam; `App` still owns higher-level event-flow decisions until a broader live-agent/App hub redesign lands.


Big-bang completion pass:
- moved runtime-command outcome application into `app_runtime.rs` via `apply_runtime_command_outcome_to_app_runtime`, including the profile/model/session-reset/config-profile master-agent replacement path.
- `App::handle_runtime_command` now executes the runtime command, delegates canonical runtime mutation/rebuild outcome application, refreshes Stylos status when the runtime helper reports an effect, and renders returned lines.
- removed the stale command-output helper so command rebuild paths no longer require TUI-side outcome unpacking.
- validation passed for `cargo check -p themion-cli`, `cargo check -p themion-cli --features stylos`, and `cargo check -p themion-cli --all-features`.


- audited the remaining `process_agent_event` / incoming-prompt / task-registry paths: the extracted planners already cover most narrow helper work, and the real leftover issue is final side-effect ownership around `self.agents`, prompt submission, and Stylos task-registry publication rather than another obvious local helper extraction
- reconciled the `handle_command` branch map with the current code: command-side runtime-mutation branches are already routed through `AppEvent::RuntimeCommand`, so the remaining post-Slice-1 work is primarily larger ownership follow-through rather than another narrow command extraction
- resolved the last narrow Slice 1 question: keep `debug_runtime_lines()` and related current-value gathering in `App` for now as the justified thin-adapter boundary, because further movement would overlap with the broader ownership redesign rather than a safe local extraction
- checklist wording refreshed to distinguish between remaining runtime-snapshot finishing work and genuinely larger live-agent state-owner redesign work; helper-only extraction in the current seam is now considered exhausted unless a new concrete ownership leak appears
- extracted `refresh_stylos_status_snapshot(...)` and `StylosAppStatusRefreshState` into `app_runtime.rs`, so `refresh_stylos_status()` no longer assembles the status snapshot input locally in `tui.rs`
- extracted `refresh_interactive_agent_system_inspection_from_runtime(...)` and `SystemInspectionRuntimeRefreshState` into `app_runtime.rs`, so `refresh_main_agent_system_inspection()` now delegates refresh assembly and only computes current debug/activity values locally
- extracted `apply_runtime_command_outcome_to_agents` and `take_runtime_command_output_lines` into `app_runtime.rs`, so the remaining api-log and clear-context command effects no longer inline their interactive-agent mutation inside `tui.rs` command handling
- extracted `apply_master_agent_replacement` and `apply_agent_ready_update` into `app_runtime.rs`, reducing `tui.rs` ownership of live agent-handle mutation for master replacement and agent-ready state application
- moved role lookup plus local roster/status derivation helpers (`agent_has_role`, `is_interactive_agent_handle`, `build_local_agent_roster`, `build_local_agent_status_entries`) into `app_runtime.rs`, further thinning `tui.rs` helper ownership without changing the live agent vector owner

Recent landed progress from the current cleanup sequence:
- command-side helper extraction for the current seam appears complete; the remaining audit items are status/system-inspection finishing work plus larger state-owner boundaries
- restored the shared-hub Stylos status publication path using `SharedStylosStatusHub`
- restored git-repo/remotes snapshot metadata in `stylos.rs`
- moved status snapshot assembly/build/publish orchestration mostly into `app_runtime.rs`
- moved system-inspection build/apply orchestration into `app_runtime.rs`; `App` now mainly gathers current runtime values before delegating refresh
- moved Stylos receiver stream wiring from `App`/`tui.rs` into `tui_runner.rs`
- extracted runtime-owned incoming-prompt/watchdog apply-plan helpers so `App` consumes more structured runtime policy decisions instead of unpacking raw planning state
- extracted a shared runtime helper for applying accepted incoming-prompt state to the target agent/watchdog, reducing repeated App-owned policy mutation
- extracted shared runtime helpers for clearing and continuing active incoming-prompt state, reducing inline Stylos-command and done-follow-up mutation in `App`
- extracted a runtime-owned completed-note follow-up apply-plan so `App` no longer matches directly on the raw follow-up policy enum
- extracted a structured watchdog dispatch effect view so `App` reads less raw nested watchdog action state directly
- moved submit-time incoming-prompt target resolution into `app_runtime.rs`, and removed the now-unused duplicate Stylos note helper logic from `tui.rs`
- extracted a structured submit-target failure effect so `App` no longer hardcodes the missing-target failure payload inline
- extracted `prepare_agent_turn_submit` so busy/cancellation/session-id submit setup is now shared runtime-owned logic instead of inline App mutation
- extracted `spawn_agent_event_relay` so submit-time agent event relay orchestration is no longer inlined in `App`
- extracted `prepare_agent_turn_execution` so agent take/event-tx setup is now shared runtime-owned logic instead of inline App mutation
- extracted `spawn_agent_turn_core_loop` so submit-time core-loop spawn/error-reporting orchestration is no longer inlined in `App`
- collapsed active incoming-prompt mutation helpers around a shared `set_active_incoming_prompt` path so less duplicate AgentHandle/watchdog mutation remains in runtime helpers
- extracted `build_local_agent_handle` so local-agent creation no longer inlines a full `tui::AgentHandle` literal inside `create_local_agent`
- extracted `remove_local_agent_handle` so local-agent deletion no longer inlines direct `AgentHandle` search/remove logic inside `delete_local_agent`
- extracted `apply_system_inspection_to_interactive_agent` so interactive inspection application is isolated behind a focused runtime helper
- added `prepare_agent_turn_runtime_launch` so `submit_text_to_agent` now delegates submit setup, event-channel wiring, and agent extraction through one composed runtime-owned launch helper
- added `launch_agent_turn_runtime` so `submit_text_to_agent` now delegates the remaining event-relay/core-loop launch orchestration through one runtime entrypoint
- introduced a lightweight runtime-owned `LocalAgentRosterEntry` so local-agent role validation and smith-id allocation no longer depend directly on full `tui::AgentHandle`
- introduced a lightweight runtime-owned `LocalAgentStatusEntry` so status publication, watchdog dispatch planning, and incoming-prompt planning no longer consume full `tui::AgentHandle` slices

Highest-value remaining follow-up:
1. treat the active-incoming-prompt helpers as the main remaining `AgentHandle`-touching path and only extract them further if a concrete ownership bug or duplication appears; today they already centralize the mutation and further splitting would mostly move direct field writes around
2. treat the submit path cleanup as at a sensible stopping point for now; the remaining boundaries still map cleanly onto relay launch versus core-loop launch without pushing runtime orchestration into TUI
3. decide whether the remaining system-inspection gather step should move into a hub-owned snapshot provider or remain as a thin App-side adapter

Default-build regression follow-up:
- fixed the default-build feature-gating regression uncovered during the final audit pass by restoring consistent Stylos gating in `tui.rs`/`tui_runner.rs`, restoring always-on availability for shared helpers used by non-Stylos paths, and adding the always-on `DomainHandle` import needed by the submit-launch helpers in `app_runtime.rs`.
- validation now passes again for default, `--features stylos`, and `--all-features` builds of `themion-cli`.

Legend extension used in progress updates:
- `[~]` — partially improved, but still not at the target ownership boundary

## File 1: `crates/themion-cli/src/tui_runner.rs`

### Structs

- [x] `TerminalGuard` — `OK` (terminal lifecycle guard only)
- [x] `RunnerContext` — `OK` (event-loop plumbing only)

### Top-level functions

- [x] `start_tick_loop` — `OK` (generic tick-loop plumbing)
- [x] `start_terminal_input_loop` — `OK` (terminal input thread only)
- [x] `create_frame_requester` — `OK` (draw signaling setup only)
- [x] `install_panic_cleanup_hook` — `OK` (terminal cleanup safety only)
- [x] `wire_stylos_app` — `MIXED` `[set4]` (watchdog task start is runtime-ish, but Stylos stream ownership already moved out of `App`; not a primary violation now)
- [x] `build_app` — `MIXED` `[set4]` (runner assembles runtime dependencies for the TUI surface)
- [x] `perform_initial_draw` — `OK`
- [x] `run_event_loop` — `OK`
- [x] `shutdown_app` — `OK`
- [x] `run` — `MIXED` `[set4]` (bootstraps terminal + runtime wiring before entering the surface loop)

### `impl TerminalGuard`

- [x] `enter` — `OK`
- [x] `terminal_mut` — `OK`
- [x] `drop` — `OK`

### `impl RunnerContext`

- [x] `build` — `OK`
- [x] `shutdown` — `OK`

## File 2: `crates/themion-cli/src/tui.rs`

### Enums

- [x] `AppEvent` — `MIXED` `[set4]` (UI events plus runtime/stylos delivery envelope)
- [x] `NonAgentSource` — `OK`
- [x] `Entry` — `OK`
- [x] `NavigationMode` — `OK`
- [x] `ReviewMode` — `OK`
- [x] `AgentActivity` — `OK`

### Structs

- [x] `AgentHandle` — `MIXED` `[set4]` (still bundles view and live-agent state, but narrower than before)
- [x] `UiDirty` — `OK`
- [x] `FrameRequester` — `OK`
- [x] `FrameScheduler` — `OK`
- [x] `ActivityCountersSnapshot` — `OK`
- [x] `ActivityCounters` — `OK`
- [x] `RuntimeMetricsSnapshot` — `MIXED` `[set4]`
- [x] `TimedRuntimeDelta` — `MIXED` `[set4]`
- [x] `App` — `VIOLATION` `[set2]` (still owns canonical live runtime/roster state)

### Top-level helpers and free functions

- [x] `agent_tag_color` — `OK`
- [x] `agent_tag_style` — `OK`
- [x] `agent_tag_spans` — `OK`
- [x] `non_agent_source_spans` — `OK`
- [x] `center_trim` — `OK`
- [x] `format_count` — `OK`
- [x] `format_context_report` — `OK`
- [x] `self_session_id_fallback` — `OK`
- [x] `agent_id_for_session` — `MIXED` `[set4]`
- [x] `split_tool_call_detail` — `OK`
- [x] `is_interactive_handle` — `MIXED` `[set4]`
- [x] `normalize_primary_role` — `MIXED` `[set4]`
- [x] `has_role` — `MIXED` `[set4]`
- [x] `format_human_count` — `OK`
- [x] `build_context_statusline` — `OK`
- [x] `build_rate_limit_statusline` — `OK`
- [x] `dispatch_terminal_event` — `OK`
- [x] `build_lines` — `OK`
- [x] `stylos_note_header_value` — `OK`
- [x] `stylos_note_display_identifier` — `OK`
- [x] `scroll_from_bottom` — `OK`
- [x] `review_area` — `OK`
- [x] `watchdog_review_area` — `OK`
- [x] `draw` — `OK`
- [x] `area_page_height` — `OK`
- [x] `current_total_and_height` — `OK`
- [x] `build_watchdog_review_lines` — `MIXED` `[set4]`
- [x] `unix_epoch_now_ms` — `OK`
- [x] `format_duration_ms` — `OK`
- [x] `per_second` — `OK`
- [x] `avg_us` — `OK`
- [x] `format_runtime_activity_lines` — `OK`
- [x] `format_runtime_lifetime_lines` — `OK`
- [x] `format_stylos_activity_lines` — `OK`
- [x] `sample_thread_cpu_lines` — `OK`
- [x] `parse_linux_thread_stat_line` — `OK`

### `impl NonAgentSource`

- [x] `label` — `OK`
- [x] `color` — `OK`

### `impl UiDirty`

- [x] `any` — `OK`
- [x] `mark_all` — `OK`
- [x] `clear` — `OK`

### `impl FrameRequester`

- [x] `new` — `OK`
- [x] `schedule_frame` — `OK`

### `impl FrameScheduler`

- [x] `new` — `OK`
- [x] `clamp_deadline` — `OK`

### `impl ActivityCounters`

- [x] `snapshot` — `OK`

### `impl ActivityCountersSnapshot`

- [x] `saturating_sub` — `OK`

### `impl AgentActivity`

- [x] `label` — `OK`
- [x] `status_bar` — `OK`

### `impl App`

#### Construction and ownership

- [x] `new` — `VIOLATION` `[set2]` (constructs and stores canonical live runtime/agent state in `App`)
- [x] `interactive_agent_mut` — `MIXED` `[set4]`
- [x] `main_agent_mut` — `MIXED` `[set4]`
- [x] `replace_master_agent` — `VIOLATION` `[set2]`
- [x] `background_domain` — `VIOLATION` `[set2]`
- [x] `any_agent_busy` — `VIOLATION` `[set2]`
- [x] `handle_local_agent_management_request` — `VIOLATION` `[set2]` (delegates to runtime helper, but `App` still owns invocation/context and live roster mutation boundary)

#### UI navigation and local view state

- [x] `enter_browsed_history` — `OK`
- [x] `return_to_latest` — `OK`
- [x] `open_transcript_review` — `OK`
- [x] `open_watchdog_review` — `OK`
- [x] `toggle_watchdog_review` — `OK`
- [x] `transcript_review_open` — `OK`
- [x] `close_review` — `OK`
- [x] `pending_str` — `OK`
- [x] `set_agent_activity` — `OK`
- [x] `clear_agent_activity` — `OK`
- [x] `reset_stream_counters` — `OK`
- [x] `request_interrupt` — `OK`
- [x] `arm_ctrl_c_exit` — `OK`
- [x] `ctrl_c_exit_is_armed` — `OK`
- [x] `expire_ctrl_c_exit_if_needed` — `OK`
- [x] `on_tick` — `OK`
- [x] `mark_dirty_conversation` — `OK`
- [x] `mark_dirty_input` — `OK`
- [x] `mark_dirty_status` — `OK`
- [x] `mark_dirty_overlay` — `OK`
- [x] `mark_dirty_all` — `OK`
- [x] `request_draw` — `OK`
- [x] `clear_dirty` — `OK`
- [x] `is_running` — `OK`
- [x] `finish_initial_draw` — `OK`
- [x] `push` — `OK`
- [x] `activity_status_value` — `MIXED` `[set4]`

#### Stylos/runtime integration points to validate first

- [x] `shutdown_stylos` — `MIXED` `[set4]`
- [x] `wire_stylos_event_streams` — expected `VIOLATION` `[set1]` `[phaseA]` `[phaseC]` (moved out of `App` into runner wiring)
- [x] `process_agent_event` — `VIOLATION` `[set1]` `[phaseA]` `[phaseC]` (still owns Stylos/task side effects alongside UI transcript updates)
- [x] `current_runtime_snapshot` — `MIXED` `[set4]`
- [x] `record_runtime_snapshot` — `MIXED` `[set4]`
- [x] `recent_runtime_delta` — `MIXED` `[set4]`
- [~] `task_runtime_snapshot` — expected `VIOLATION` `[set1]` `[phaseC]`
- [x] `system_inspection_snapshot` — expected `VIOLATION` `[set1]` `[phaseC]` (removed from `App`; refresh delegates to runtime helper)
- [~] `refresh_main_agent_system_inspection` — expected `VIOLATION` `[set1]` `[phaseC]` (build/apply moved to runtime helper; App still gathers current inputs)
- [x] `debug_runtime_lines` — `MIXED` `[set4]`

#### Command and submission flows

- [x] `handle_command` — `VIOLATION` `[set3]`
- [x] `scroll_up` — `OK`
- [x] `scroll_down` — `OK`
- [x] `page_up` — `OK`
- [x] `page_down` — `OK`
- [x] `jump_to_top` — `OK`
- [x] `submit_shell_command` — `MIXED` `[set4]`
- [x] `submit_text_to_agent` — `VIOLATION` `[set2]` (much thinner now, but `App` still initiates live turn execution against canonical handles)
- [x] `submit_text` — `VIOLATION` `[set2]`
- [x] `handle_watchdog_poll` — `VIOLATION` `[set1]` `[phaseA]` `[phaseC]` (policy partly extracted, but `App` still drives the flow)
- [x] `maybe_emit_done_mention_for_completed_note` — `VIOLATION` `[set1]` `[phaseA]` `[phaseC]`
- [x] `submit_input` — `MIXED` `[set4]`

#### Event handlers

- [x] `handle_mouse_event` — `OK`
- [x] `handle_paste_event` — `OK`
- [x] `handle_tick_event` — `MIXED` `[set4]`
- [x] `handle_agent_ready_event` — `MIXED` `[set4]`
- [x] `handle_login_prompt_event` — `OK`
- [x] `handle_agent_event_for_run` — `MIXED` `[set4]`
- [x] `handle_draw_event` — `OK`
- [x] `handle_shell_complete_event` — `OK`
- [x] `handle_stylos_cmd_event` — `VIOLATION` `[set1]` `[phaseC]`
- [x] `handle_stylos_event_text` — `OK`
- [x] `handle_incoming_prompt_event` — `VIOLATION` `[set1]` `[phaseA]` `[phaseC]` (policy partly extracted, but admission/follow-up orchestration still originates in `App`)
- [x] `handle_key_event` — `MIXED` `[set4]`

### Test helpers and tests in `tui.rs`

- [x] `handle` — `OK`
- [x] `stylos_note_display_identifier_prefers_slug` — `OK`
- [x] `stylos_note_display_identifier_falls_back_to_note_id` — `OK`
- [x] `validate_agent_roles_accepts_one_master_and_one_interactive` — `OK`
- [x] `validate_agent_roles_rejects_zero_master` — `OK`
- [x] `validate_agent_roles_rejects_two_master` — `OK`
- [x] `allocates_next_free_smith_id` — `OK`
- [x] `targeted_remote_request_prefers_matching_agent_id` — `OK`
- [x] `targeted_remote_request_does_not_fall_back_to_interactive_when_missing` — `OK`
- [x] `sender_side_stylos_talk_log_format_is_exact` — `OK` `[phaseA]`

## Action sets

### Set 1 — `VIOLATION` related to Stylos

Goal: remove TUI ownership of Stylos stream wiring, incoming prompt policy, task-registry side effects, board-note completion policy, and system-inspection publication.

Items:
- [~] `tui_runner::wire_stylos_app`
- [x] `App::wire_stylos_event_streams` `[phaseA]` `[phaseC]`
- [x] `App::handle_stylos_cmd_event`
- [x] `App::handle_incoming_prompt_event` `[phaseA]` `[phaseC]`
- [x] `App::handle_watchdog_poll` `[phaseA]` `[phaseC]`
- [x] `App::maybe_emit_done_mention_for_completed_note` `[phaseA]` `[phaseC]`
- [x] `App::process_agent_event` `[phaseA]` `[phaseC]`
- [~] `App::task_runtime_snapshot`
- [x] `App::system_inspection_snapshot` (removed from `App`; covered above)
- [~] `App::refresh_main_agent_system_inspection`

Expected destination:
- runtime/app-state/orchestrator modules own these flows
- TUI receives already-decided events or read-only snapshots to render

Suggested execution order:
1. [x] move Stylos receiver wiring out of TUI surface
2. [~] move incoming prompt and Stylos command policy out of `App`
3. [~] move watchdog/board-note follow-up policy out of `App`
4. [x] move task-registry and inspection snapshot side effects out of `App::process_agent_event`
5. [~] re-check that TUI only renders Stylos/runtime state, not owns it

### Set 2 — `VIOLATION` related to agent management / orchestration

Goal: remove TUI ownership of roster mutation, live-agent orchestration, and runtime rebuild flows.

Items:
- [x] `App`
- [x] `App::new`
- [x] `App::replace_master_agent`
- [x] `App::background_domain`
- [x] `App::any_agent_busy`
- [x] `App::handle_local_agent_management_request`
- [x] `App::submit_text_to_agent`
- [x] `App::submit_text`
- [x] runtime-rebuild branches inside `App::handle_command`

Expected destination:
- app-state/app-runtime/hub modules own canonical roster, agent lifecycle, orchestration state, and submission policy
- TUI emits intents and renders projections

Suggested execution order:
1. [~] define or confirm runtime-owned roster/submission interfaces
2. [~] move local-agent management request handling out of `App`
3. [~] move live-agent submission logic out of `submit_text_to_agent` / `submit_text`
4. [x] move profile/model-triggered runtime rebuild logic out of `handle_command`
5. [~] shrink `App` so it no longer owns canonical runtime state

### Set 3 — `VIOLATION` related to other runtime ownership

Goal: capture non-Stylos, non-roster ownership violations that still leave TUI acting like the runtime hub.

Items:
- [x] `App::handle_command`

Notes:
- `handle_command` is broader than one concern; it includes UI-safe command handling plus runtime-owned behavior.
- When working this set, split the function rather than moving the whole thing blindly.

Sub-actions:
- [x] separate pure UI/display commands from runtime-owned commands
- [x] route runtime-owned commands through app-state/runtime intents
- [x] leave only formatting or local-view command behavior in `tui.rs`

### Set 4 — `MIXED`

Goal: split combined view/runtime code so TUI keeps only surface concerns.

Items from `tui_runner.rs`:
- [x] `build_app`
- [x] `run`

Items from `tui.rs`:
- [x] `AppEvent`
- [~] `AgentHandle`
- [x] `RuntimeMetricsSnapshot`
- [x] `TimedRuntimeDelta`
- [x] `agent_id_for_session`
- [~] `is_interactive_handle`
- [~] `normalize_primary_role`
- [x] `has_role`
- [x] `build_watchdog_review_lines`
- [x] `interactive_agent_mut`
- [x] `main_agent_mut`
- [x] `activity_status_value`
- [x] `shutdown_stylos`
- [x] `current_runtime_snapshot`
- [x] `record_runtime_snapshot`
- [x] `recent_runtime_delta`
- [x] `debug_runtime_lines`
- [x] `submit_shell_command`
- [x] `submit_input`
- [x] `handle_tick_event`
- [x] `handle_agent_ready_event`
- [x] `handle_agent_event_for_run`
- [x] `handle_key_event`

Split guidance:
- [~] keep UI-local rendering, formatting, and terminal control in TUI
- [~] move runtime state ownership and policy to app-state/runtime modules
- [~] replace mixed structs with view-model snapshots where practical
- [~] prefer intent/event boundaries instead of direct runtime mutation from TUI

## Highest-priority violation queue

Validate and migrate these one by one, in order.

1. [x] `wire_stylos_event_streams` `[phaseA]`
2. [x] `handle_incoming_prompt_event` `[phaseA]`
3. [x] `handle_watchdog_poll` `[phaseA]`
4. [x] `maybe_emit_done_mention_for_completed_note` `[phaseA]`
5. [x] `process_agent_event` `[phaseA]`
6. [x] `sender_side_stylos_talk_log_format_is_exact` `[phaseA]`
7. [x] `App` `[phaseE]`
8. [x] `App::new` `[phaseE]`
9. [~] `tui_runner::wire_stylos_app` `[phaseC]`
10. [x] `submit_text_to_agent` `[phaseE]`
11. [x] `handle_local_agent_management_request` `[phaseE]`
12. [x] `task_runtime_snapshot` `[phaseC]`
13. [x] `system_inspection_snapshot` `[phaseC]`
14. [x] `refresh_main_agent_system_inspection` `[phaseC]`
15. [x] runtime-rebuild paths inside `handle_command` `[phaseD]`

## Validation workflow for each item

For every item above:

- [x] read the full item and its direct callers/callees
- [x] write down why it is `OK`, `MIXED`, or `VIOLATION`
- [x] identify the exact state it owns or mutates
- [x] map that state to the correct layer: `TUI`, `HUB / APP_STATE`, `AGENT CORE`, or `STYLOS`
- [x] decide whether the item should stay, split, move, or be deleted
- [x] make the smallest targeted change
- [x] run the narrowest useful validation
- [x] re-check both default and relevant feature-on builds before marking the item complete

## Minimum validation targets after meaningful edits

- [x] `cargo check -p themion-cli`
- [x] `cargo check -p themion-cli --features stylos`
- [x] `cargo check -p themion-cli --all-features`

## Coverage note

This checklist is intended to cover all meaningful structs, enums, top-level functions, impl methods, and test helpers present in `tui.rs` and `tui_runner.rs`. Re-check coverage after either file changes.

## Notes

- This file is an audit/checklist artifact, not the migration itself.
- Do not skip directly to edits without first validating the specific item against this checklist.
- Prefer one completed migration slice at a time over broad deletions.

Current stopping-point assessment:
- The remaining `AgentHandle` touchpoints in `app_runtime.rs` are concentrated in a few narrow helper families: submit setup/execution, interactive system-inspection apply, local-agent handle construction/removal, and active incoming-prompt mutation.
- The submit path now crosses `prepare_agent_turn_runtime_launch` plus `launch_agent_turn_runtime`, which leaves `tui.rs` responsible mainly for user-flow counters/activity state and runtime intent handoff.
- The active incoming-prompt helpers are the only notable remaining direct field-mutation seam, but they already sit behind a shared `set_active_incoming_prompt` helper and also need whole-slice visibility to recompute watchdog state. More extraction right now would add indirection faster than it would reduce ownership risk.
- Conclusion: this cleanup sequence is at a sensible pause point unless a later bug or feature change reveals a clearer hub-owned view to extract.

## Final audit status for this experiment

- The checklist is now updated against the current codebase rather than left as a stale speculative queue.
- Most easy ownership wins from this cleanup sequence have landed, and the final pass also closed the default-build feature-gating regression that briefly remained.
- The remaining open items are now only larger-scope refactors: deeper `App` de-ownership and any future hub-owned replacement for the thin system-inspection gather step. Profile/model runtime rebuild outcomes and Stylos task-registry publication calls now apply through `app_runtime.rs` rather than direct `tui.rs` mutation/publication code.
- Treat this checklist as complete for the current audit pass: no routine stale review work or immediate build-fix follow-up remains.
