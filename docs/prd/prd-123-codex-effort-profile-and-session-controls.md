# PRD-123: Add Persistent and Session-Local Codex Effort Controls

- **Status:** Implemented
- **Version:** v0.76.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-11

## Summary

Landed in `v0.76.0` as persistent and session-local Codex effort controls for the TUI/runtime path. The shipped behavior adds `/config profile set effort=<value>` and `/session profile set effort=<value>` with accepted values `low|medium|high|xhigh`, shows configured/effective/temporary effort in session/profile displays, keeps `medium` as the fallback default, and applies the resolved effort only on the Codex request path for both initial and continuation Responses API requests.

- Themion currently always sends Codex reasoning `effort="medium"`.
- Add `/config profile set effort=<value>` to save a default effort level on the active profile.
- Add `/session profile set effort=<value>` to override effort for the current session only.
- Keep `medium` as the default when no saved or session-local override exists.
- Support the first-slice value range `low|medium|high|xhigh`.
- Apply effort only on the Codex request path; other providers keep current behavior.

## Goals

- Let users set Codex effort without editing `config.toml` by hand.
- Support both saved profile defaults and session-only overrides.
- Keep the command surface aligned with the existing `/config profile set ...` and `/session profile set ...` families.
- Preserve current behavior when no effort is set: effective effort stays `medium`.
- Make configured and effective effort visible in the same session/profile displays that already show provider and model.
- Define exact implementation targets so this PRD can be implemented without follow-up design work.

## Non-goals

- Do not redesign provider selection, model selection, or login behavior.
- Do not add a separate Codex-only command family.
- Do not expose the broader internal Codex reasoning enum in this first slice.
- Do not add automatic effort tuning by workflow, prompt size, or tool usage.
- Do not change request behavior for non-Codex providers beyond carrying an unused optional field in profile/session state.

## Background & Motivation

### Current state

Current implementation facts:

- `crates/themion-cli/src/config.rs` defines `ProfileConfig` with `provider`, `base_url`, `model`, and `api_key` only.
- `crates/themion-cli/src/main.rs` `Session` tracks temporary overrides for profile and model only.
- `crates/themion-cli/src/app_runtime.rs` `RuntimeCommand::SessionProfileSet` currently accepts only `model` and rejects other keys.
- `crates/themion-cli/src/app_runtime.rs` `RuntimeCommand::ConfigProfileSet` currently accepts only `provider`, `model`, `endpoint`, and `api_key`.
- `crates/themion-cli/src/tui.rs` help and usage text only advertise those existing keys.
- `crates/themion-core/src/client_codex.rs` hard-codes `DEFAULT_CODEX_REASONING_EFFORT = "medium"` and always sends that value in `build_reasoning_payload()`.

The sibling `../codex` repository shows that current Codex GPT-5.x model metadata advertises `low`, `medium`, `high`, and `xhigh` as supported reasoning levels, with default `medium`.

### Why this matters now

Effort changes Codex behavior in a way users can notice. It belongs in the same profile/session control surface as `model`.

Themion already has the product split needed for this feature:

- `/config profile set ...` for saved config on disk
- `/session profile set ...` for live session-only overrides

What is missing is a concrete field, validation, display, and request wiring path for effort.

## Design

### 1. Add `effort` to saved profile config

`crates/themion-cli/src/config.rs` must add an optional `effort` field to `ProfileConfig`.

Required behavior:

- field name in TOML and serde must be `effort`
- type should be optional and string-backed in a way consistent with the current config style unless implementation introduces a small local enum with equivalent serde behavior
- older config files without `effort` remain valid
- config template comments may mention `effort`, but the field must remain optional
- profile clone/create/save flows must preserve the field the same way they preserve `provider`, `base_url`, `model`, and `api_key`

### 2. Add effective and temporary effort state to `Session`

`crates/themion-cli/src/main.rs` `Session` must track effort the same way it tracks effective and temporary model state.

Required state additions:

- one field for the effective current effort value used by the live session
- one field for the temporary session-only effort override

Required behavior:

- `Session::from_config(...)` initializes effective effort from the active profile if present, otherwise `medium`
- `Session::switch_profile(...)` recomputes effective effort from the selected profile and then reapplies any active temporary effort override
- `Session::switch_profile_temporarily(...)` must continue clearing temporary model overrides as it does now and must also clear temporary effort overrides
- `Session::clear_temporary_overrides(...)` must clear temporary effort override together with temporary profile and model overrides
- changing effort must invalidate any cached model/runtime state to the same degree needed for the rebuilt agent path

### 3. Define the first-slice accepted value set exactly

The first implementation must accept exactly these values:

- `low`
- `medium`
- `high`
- `xhigh`

Required validation rules:

- accept case-insensitive input from slash commands, then normalize to lowercase before storing
- reject empty values
- reject any other value, including `minimal`, `none`, and unknown strings
- invalid-value errors must list the exact valid set: `low, medium, high, xhigh`

This intentionally follows current model-advertised Codex GPT-5.x values rather than the broader internal Codex enum.

### 4. Extend `/config profile set` with persistent effort mutation

`crates/themion-cli/src/app_runtime.rs` `RuntimeCommand::ConfigProfileSet` and related parsing/help text must accept `effort=<value>`.

Required behavior:

- `/config profile set effort=<value>` updates the effective in-memory session effort immediately
- it also writes `effort` into `context.session.profiles[active_profile]`
- it persists through the existing `save_profiles(...)` path
- success output should match current command style, for example `effort=xhigh saved`
- invalid values should fail before profile save and must not modify in-memory or persisted profile data
- the valid-key message for unknown keys must become `provider, model, endpoint, api_key, effort`

Implementation constraint:

- this change must be wired into the existing `RuntimeCommand::ConfigProfileSet { key, value }` path rather than by adding a parallel command family

### 5. Extend `/session profile set` with session-only effort mutation

`crates/themion-cli/src/app_runtime.rs` `RuntimeCommand::SessionProfileSet` and related parsing/help text must accept `effort=<value>`.

Required behavior:

- `/session profile set effort=<value>` sets only the temporary session-only effort override
- it must not call `save_profiles(...)`
- it must rebuild the replacement interactive agent through the same existing path used for temporary model overrides
- success output should match current style and clearly say the change is temporary, for example `temporarily using effort 'xhigh' for this session only`
- invalid values must not change session state
- the valid-key message for unknown session keys must become `model, effort`

Implementation constraint:

- this change must stay inside the existing `RuntimeCommand::SessionProfileSet { key, value }` path
- do not add a separate `/session effort ...` command family

### 6. Reset and profile-switch semantics for effort

Effort override semantics must follow the current temporary model override rules closely.

Required behavior:

- `/session profile reset` clears temporary effort override together with temporary profile/model overrides
- if no temporary profile, model, or effort override is active, `/session profile reset` still reports `no temporary session override is active`
- `/session profile use <name>` clears any previous temporary effort override when switching to the requested profile, just as it clears temporary model override today
- after reset or temporary profile switch, effective effort becomes the selected profile's saved effort, or `medium` when unset

### 7. Show configured and effective effort in existing display surfaces

`crates/themion-cli/src/app_state.rs` display helpers must expose effort in the same surfaces that already show profile and model state.

Required output changes:

- `session_config_lines(...)` must include the current effective effort line
- `session_show_lines(...)` must include:
  - configured effort
  - effective effort
  - temporary effort override
- when any temporary override is active, the existing note about session-only override should still appear

Display rules:

- when saved profile effort is unset, configured display should make the default behavior visible rather than showing a confusing blank value
- effective effort display should always show one concrete value
- temporary effort override display should show `(none)` when absent, matching the current style for temporary model override

### 8. Update help and usage text to the exact accepted forms

`crates/themion-cli/src/tui.rs` must update help and usage output.

Required changes:

- `/help` list must include `/config profile set effort=<value>`
- `/help` list must include `/session profile set effort=<value>`
- `config_help_lines()` must include the new persistent effort command
- `session_help_lines()` must include the new session-local effort command
- `/config profile set` usage text must become:
  - `usage: /config profile set provider=<value>|model=<value>|endpoint=<value>|api_key=<value>|effort=<low|medium|high|xhigh>`
- `/session profile set` usage text must become:
  - `usage: /session profile set model=<value>|effort=<low|medium|high|xhigh>`

### 9. Wire effective effort into Codex requests only

`crates/themion-core/src/client_codex.rs` must stop hard-coding request effort to `medium` for all calls.

Required behavior:

- replace `build_reasoning_payload()` with a form that accepts the effective effort value from caller state, or equivalent wiring with the same result
- default to `medium` only when no explicit effective effort was provided by the CLI/runtime layer
- use the effective effort in both the initial responses request body and continuation request body
- non-Codex providers must keep their current request behavior unchanged

Implementation constraint:

- the effort value must come from resolved session/profile state, not from ad hoc TUI-only state
- do not move provider behavior into `tui.rs`

### 10. Keep implementation runtime-owned and additive

This is a runtime/config feature, not a presentation-only feature.

Required behavior:

- validation and session-state ownership belong in the CLI runtime/app-state layer
- request payload application belongs in `themion-core`
- `tui.rs` remains responsible only for parsing/dispatch/help text and display wiring it already owns
- no database migration is required
- no existing command should be removed or renamed in this PRD

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/config.rs` | Add optional `effort` to `ProfileConfig`, keep old configs valid, and preserve the field through profile load/save/create/clone flows. |
| `crates/themion-cli/src/main.rs` | Add effective and temporary effort state to `Session`, initialize default `medium`, and carry effort through profile switching and temporary-override reset behavior. |
| `crates/themion-cli/src/app_runtime.rs` | Extend `ConfigProfileSet` and `SessionProfileSet` to validate and mutate `effort`, update reset semantics, and keep agent rebuild behavior aligned with current temporary-override flows. |
| `crates/themion-cli/src/app_state.rs` | Add configured/effective/temporary effort lines to session and profile display helpers. |
| `crates/themion-cli/src/tui.rs` | Add help entries and exact usage text for persistent and session-only effort commands. |
| `crates/themion-core/src/client_codex.rs` | Replace the hard-coded `medium` reasoning payload with the resolved effective effort value while preserving `medium` as fallback. |
| CLI-to-core client construction/wiring | Pass effective effort from runtime-owned session state into the Codex client/request path without affecting non-Codex providers. |
| `docs/architecture.md` and/or `docs/engine-runtime.md` | Document persistent and session-local effort controls plus the first-slice accepted range. |
| `docs/README.md` | Track this PRD and later update status/version when implemented. |

## Edge Cases

- a user runs `/config profile set effort=LOW` → verify: the value is accepted and normalized to lowercase.
- a user runs `/config profile set effort=minimal` → verify: the command is rejected because `minimal` is outside the first-slice contract.
- a user runs `/config profile set effort=` → verify: the command is rejected as invalid and does not save.
- a user runs `/session profile set effort=xhigh` → verify: only live session state changes and config on disk remains unchanged.
- a user runs `/session profile reset` after only a temporary effort override → verify: the override clears and effective effort returns to saved value or `medium`.
- a user switches temporary session profile after setting temporary effort override → verify: the old temporary effort override is cleared.
- a user saves `effort=xhigh` on a non-Codex profile and later runs with OpenRouter → verify: config/state remain valid and provider request behavior is unchanged.
- a user shows session state with no saved effort and no override → verify: configured/effective output makes the default `medium` visible.

## Migration

No database migration is required.

This is an additive config and command-surface change:

- existing profiles without `effort` remain valid
- old behavior remains unchanged until the user sets `effort`
- the effective fallback remains `medium`
- first-slice accepted values are `low|medium|high|xhigh`

Minor-version scope is appropriate because this adds a new user-visible configuration capability without breaking existing commands.

## Testing

- run `/config profile set effort=low` → verify: the active saved profile stores `effort="low"` and success output says it was saved.
- run `/config profile set effort=xhigh` → verify: the active saved profile stores `effort="xhigh"` and later profile reload uses it.
- run `/config profile set effort=minimal` → verify: the command is rejected and the valid value list remains `low, medium, high, xhigh`.
- run `/config profile set effort=invalid` → verify: the command is rejected and no profile save occurs.
- run `/session profile set effort=high` → verify: effective session effort changes and config on disk is unchanged.
- run `/session profile set effort=xhigh` → verify: effective session effort changes and replacement agent rebuild path still succeeds.
- run `/session profile reset` after a temporary effort override → verify: effective effort returns to saved value or `medium`.
- run `/session profile use <name>` after a temporary effort override → verify: the override clears and the selected profile's saved/default effort becomes effective.
- inspect `/help`, `/config` help, and `/session` help → verify: all effort commands appear with the exact accepted range.
- inspect session/profile display output → verify: configured, effective, and temporary effort values are shown correctly.
- send a Codex request with no configured effort → verify: the reasoning payload uses `medium`.
- send a Codex request with effective effort `low` → verify: the reasoning payload uses `low`.
- send a Codex request with effective effort `xhigh` → verify: the reasoning payload uses `xhigh` in both initial and continuation request bodies.
- use a non-Codex provider with `effort` present in state → verify: request behavior is unchanged.
- run `cargo check -p themion-core` → verify: default core build stays clean.
- run `cargo check -p themion-cli` → verify: default CLI build stays clean.
- run `cargo check -p themion-cli --features stylos` → verify: touched CLI code still builds with Stylos enabled.
- run `cargo check -p themion-core -p themion-cli --all-features` → verify: touched crates still build across feature combinations.

## Implementation checklist

- [x] add optional `effort` to `ProfileConfig` in `crates/themion-cli/src/config.rs`
- [x] preserve `effort` through profile load/save/create/clone flows
- [x] add effective effort and temporary effort override fields to `Session` in `crates/themion-cli/src/main.rs`
- [x] initialize effective effort from profile or fallback `medium`
- [x] reapply temporary effort override in `Session::switch_profile(...)`
- [x] clear temporary effort override in `Session::switch_profile_temporarily(...)` and `Session::clear_temporary_overrides(...)`
- [x] extend `RuntimeCommand::ConfigProfileSet` to accept and validate `effort`
- [x] extend `RuntimeCommand::SessionProfileSet` to accept and validate `effort`
- [x] update unknown-key and invalid-value messages to list the exact supported keys/values
- [x] rebuild the replacement interactive agent after session-local effort changes
- [x] add effort lines to `session_config_lines(...)` and `session_show_lines(...)`
- [x] update `/help`, `config_help_lines()`, `session_help_lines()`, and usage text in `tui.rs`
- [x] replace hard-coded Codex reasoning effort with runtime-resolved effective effort in both initial and continuation request bodies
- [x] keep non-Codex provider behavior unchanged
- [x] update runtime/docs guidance for the new effort controls and accepted value range
- [ ] add focused tests for config persistence, session override, reset, profile switch clearing, invalid values, display output, and Codex payload mapping
