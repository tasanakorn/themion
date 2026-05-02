# TUI architecture violation cleanup checklist

Status: active
Scope: remove remaining `AGENTS.md` architecture-guideline violations where `tui.rs` or `tui_runner.rs` owns runtime, roster, Stylos, or board-routing responsibilities that belong in hub/app-state/runtime layers.

## Required outcome

- `tui.rs` is only an input/output surface plus UI-local view state
- `tui_runner.rs` only handles terminal-mode orchestration
- runtime/app-state-owned modules own canonical roster state, Stylos publication/query inputs, board routing, and incoming-prompt/runtime policy
- TUI/headless/Stylos consume the same runtime-owned source of truth

## Detailed progress log

### Completed removals

#### `crates/themion-cli/src/tui_runner.rs`

- deleted `wire_stylos_app(...)`
  - this helper previously wired Stylos event streams into the TUI app layer
- deleted the `wire_stylos_app(&mut app, &runtime_domains, &ctx.app_tx)` call from `run(...)`
- deleted Stylos shutdown ownership from `shutdown_app(...)`
  - removed the `app.shutdown_stylos()` call
  - removed the `stylos.shutdown().await` call
- kept `shutdown_app(...)` only as terminal/input-loop shutdown orchestration

#### `crates/themion-cli/src/tui.rs`

- deleted `App::wire_stylos_event_streams(...)`
  - removed TUI-owned bridging from Stylos receivers to `AppEvent`
  - removed use of:
    - `handle.take_cmd_rx()`
    - `handle.take_prompt_rx()`
    - `handle.take_event_rx()`
- deleted `App::shutdown_stylos(...)`
  - removed TUI exposure of Stylos lifecycle ownership
- deleted `App::handle_local_agent_management_request(...)`
  - removed TUI-owned local roster mutation and runtime command execution path
  - removed TUI call path to:
    - `build_local_agent_tool_invoker(...)`
    - `runtime_handle_local_agent_management_request(...)`
    - `LocalAgentRuntimeContext`
- deleted `App::handle_watchdog_poll(...)`
  - removed TUI-owned watchdog dispatch and board-note routing policy
  - removed TUI call path to:
    - `select_watchdog_dispatch(...)`
    - `finalize_board_note_injection(...)`
- deleted `App::handle_stylos_cmd_event(...)`
  - removed TUI-owned handling of incoming Stylos command policy and prompt submission routing
- deleted `App::handle_incoming_prompt_event(...)`
  - removed TUI-owned incoming-prompt admission and failure policy
  - removed TUI call path to:
    - `resolve_incoming_prompt_disposition(...)`
    - Stylos task-registry `set_failed(...)` from this handler
- deleted `AppEvent` dispatch arms inside `App::handle_app_event(...)` for:
  - `AppEvent::StylosCmd(...)`
  - `AppEvent::IncomingPrompt(...)`
  - `AppEvent::WatchdogPoll`
  - `AppEvent::LocalAgentManagement(...)`

### What these removals changed

- TUI no longer wires Stylos receiver streams into the app surface
- TUI runner no longer owns Stylos shutdown lifecycle
- TUI no longer owns these direct runtime-policy handlers:
  - local agent management
  - watchdog dispatch
  - incoming Stylos command handling
  - incoming prompt admission handling
- this was a deletion-first cleanup to remove architecture violations before runtime replacement paths are fully rehomed

### Validation after deletions

- ran: `cargo check -p themion-cli --features stylos`
- result: fails currently, expected after deletion-first cleanup

Current reported failure:
- non-exhaustive `match` in `App::handle_app_event(...)` because `AppEvent` still declares removed variants:
  - `StylosCmd(crate::stylos::StylosCmdRequest)`
  - `IncomingPrompt(IncomingPromptRequest)`
  - `WatchdogPoll`
  - `LocalAgentManagement(LocalAgentManagementRequest)`

Current follow-on warnings:
- unused imports in `crates/themion-cli/src/tui.rs` left behind by the deletions, including:
  - `build_local_agent_tool_invoker`
  - `handle_local_agent_management_request as runtime_handle_local_agent_management_request`
  - `LocalAgentRuntimeContext`
  - `finalize_board_note_injection`
  - `resolve_incoming_prompt_disposition`
  - `select_watchdog_dispatch`
  - `IncomingPromptDisposition`

## Remaining likely violations to remove or rehome

### `crates/themion-cli/src/tui.rs`

- `App.agents`
  - still looks like canonical local roster ownership in the TUI layer
- `App::replace_master_agent(...)`
  - still mutates runtime-owned agent/roster state from the TUI layer
- `App::new(...)`
  - still builds the main runtime agent through `build_main_agent(...)`
  - still derives Stylos runtime context through `tool_bridge(...)` and local instance calculation
  - still initializes runtime-owned watchdog/board state in the TUI layer
- session/profile/model reset flows still rebuild runtime agents from inside TUI using `build_replacement_main_agent(...)`
- `process_agent_event(...)`
  - still performs some Stylos/runtime side effects such as task-registry `set_running(...)`
- `refresh_main_agent_system_inspection()` and its use of `build_system_inspection_snapshot(...)`
  - still suggests runtime snapshot/system-inspection assembly in TUI
- fields that still suggest runtime ownership in `App`:
  - `stylos`
  - `local_stylos_instance`
  - `watchdog_state`
  - `board_claims`
  - `stylos_tool_bridge`
  - `local_agent_mgmt_tx`

### `crates/themion-cli/src/tui_runner.rs`

- runner is much cleaner now
- remaining review point: ensure future runtime lifecycle setup/teardown stays in app-state/runtime modules, not terminal orchestration

## Checklist

### 1. Find remaining ownership violations in `tui.rs`

- [x] identify every place where `tui.rs` mutates or owns the canonical local agent roster
- [x] identify every place where `tui.rs` constructs runtime policy/context for local-agent management
- [x] identify every place where `tui.rs` directly owns Stylos-related routing/policy instead of only rendering or forwarding intents
- [x] identify every place where `tui.rs` owns board-routing or watchdog policy rather than observing runtime-owned state

### 2. Move runtime ownership out of TUI

- [ ] move local-agent roster mutation out of `tui.rs` into runtime/app-state/orchestrator code
- [x] move local-agent management request handling out of `tui.rs` into runtime-owned command handlers
- [ ] move Stylos-facing runtime policy/state assembly out of `tui.rs`
- [x] move board-target resolution inputs to runtime-owned shared state
- [x] move incoming-prompt admission and related non-visual policy fully out of TUI if any ownership remains there

### 3. Establish one runtime-owned source of truth

- [ ] define the hub/app-state-owned snapshot for process/runtime/agent status
- [ ] ensure Stylos status/query handlers read that runtime-owned snapshot instead of TUI state
- [ ] ensure TUI renders from runtime-owned snapshots or projected view state instead of owning the canonical truth
- [ ] ensure headless and TUI modes use the same runtime-owned roster/status source

### 4. Remove obsolete TUI-only plumbing

- [x] delete TUI-only fields/methods that existed only to support runtime-owned behavior in `tui.rs`
- [ ] delete unused Stylos receiver/event plumbing left behind after removing TUI bridging
- [ ] delete dead `AppEvent` variants once their runtime replacement path exists
- [ ] remove temporary warning suppressions once the ownership move is complete

### 5. Validation

- [x] run the narrowest useful checks while refactoring
- [x] run `cargo check -p themion-cli --features stylos`
- [ ] run `cargo check -p themion-cli --all-features`
- [ ] confirm no new warnings remain in touched scope
- [ ] confirm the docs still match the final ownership model

## Notes

- Violating the architecture guideline is not acceptable; preserving an incorrect TUI-owned design is not a valid tradeoff for convenience.
- Prefer deleting TUI-owned runtime logic over extending it.
- If a temporary break is needed during refactor, runtime ownership correctness takes priority over keeping the old TUI-owned path alive.
