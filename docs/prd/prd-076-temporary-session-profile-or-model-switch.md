# PRD-076: Temporary Session-Only Profile and Model Switching

- **Status:** Implemented
- **Version:** v0.49.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-30

## Implementation status

Landed in `v0.49.0` as a session-scoped runtime override feature for the TUI. The shipped behavior adds `/session profile use`, `/session model use`, `/session show`, and `/session reset`, keeps persistent `/config` save semantics unchanged, rebuilds the interactive agent from the effective session-only state, and makes temporary overrides explicit in command output without rewriting config files.

## Summary

- Themion currently supports `/config`-based profile changes, but that path persists changes to config and is too heavy when the user only wants a temporary switch for the current live session.
- This PRD adds a session-only switch path so the user can temporarily change the active profile and/or model without rewriting config files.
- The temporary override should affect only the current session's future turns, status surfaces, and rebuilt interactive agent instance.
- Restarting Themion or opening a new session should continue using the normal configured profile unless the user explicitly makes a persistent config change.
- The feature should be explicit in the UI so the user can tell whether the current provider/model state is temporary-session-only or saved configuration.

## Goals

- Let the user temporarily switch the active profile for the current session without persisting that change to config.
- Let the user temporarily override the active model for the current session without persisting that change to config.
- Reuse the existing agent-rebuild/session-wiring path so the temporary switch affects future turns cleanly.
- Keep persistent `/config` behavior available and unchanged for users who want to save settings.
- Make the temporary nature of the switch visible in command output and status surfaces where practical.

## Non-goals

- No replacement of the existing persistent `/config` workflow.
- No requirement to persist temporary overrides across app restarts or new sessions.
- No requirement in this PRD to support temporary overrides for every profile field such as endpoint or API key, unless that falls out naturally from a narrowly scoped implementation decision.
- No requirement to backfill or rewrite earlier turn metadata after a temporary switch.
- No change to non-TUI startup arguments or environment-variable profile selection in this PRD.

## Background & Motivation

### Current state

Themion already has profile/model switching concepts, but the current user-facing path is config-oriented.

Current implementation confirmed in `crates/themion-cli/src/tui.rs`:

- `/config profile use <name>` switches profiles and persists the new active profile through `save_profiles(...)`
- `/config profile set key=value` mutates the active profile values and persists them through `save_profiles(...)`
- both paths rebuild the main interactive agent through `build_replacement_main_agent(...)` so future turns use the new runtime settings

That works for durable configuration changes, but it is the wrong tool when the user only wants a temporary experiment such as:

- try a different model for this session only
- borrow another saved profile for one live debugging session
- temporarily move back to the configured default later without having edited config on disk

The existing runtime/history design already assumes profile or model can change mid-session. PRD-057 explicitly stores turn-level `profile`, `provider`, and `model` metadata because later turns in the same session may differ from earlier ones.

So the missing product behavior is not whether session-local switching is conceptually allowed. It is that the current command surface only offers a persistent config mutation path.

## Design

### 1. Add an explicit session-only switch path separate from persistent `/config`

Themion should expose a dedicated temporary switch path instead of overloading persistent `/config` behavior silently.

Required behavior:

- the user must have a way to request a session-only profile switch
- the user must have a way to request a session-only model override
- using the session-only path must not call `save_profiles(...)` or otherwise rewrite config files
- persistent `/config` commands should keep their current save-to-config semantics

This keeps temporary and persistent intent distinct.

**Alternative considered:** add a hidden flag to `/config profile use` that sometimes suppresses persistence. Rejected: the persistent meaning of `/config` is already established, and a separate explicit session-only path is clearer and safer.

### 2. Apply the temporary switch by rebuilding the interactive agent for the current session

The feature should reuse the current runtime replacement pattern rather than trying to mutate live provider state in place.

Required behavior:

- after a session-only switch request is accepted, Themion should rebuild the main interactive agent using the new effective profile/model state
- future turns in the current session should use the temporary selection
- existing transcript/history remains in the current TUI session view unless the implementation must create a replacement agent/session boundary and documents that choice clearly
- the statusline, `/config`-adjacent status output, and any relevant inspection surfaces should reflect the current effective profile/model after the switch

This follows the current architecture style already used for persistent profile changes.

**Alternative considered:** mutate only a few runtime fields without rebuilding the agent. Rejected: the existing agent replacement path is already the safer place to ensure provider/model wiring stays coherent.

### 3. Define the initial command surface narrowly

The first version should solve the main temporary-switch need without introducing a large new command family.

Normative initial command shapes:

- `/session profile use <name>`
- `/session model use <model>`
- `/session show`
- `/session reset`

Required behavior:

- `/session profile use <name>` temporarily switches to an existing saved profile for the current session only
- `/session model use <model>` temporarily overrides the current session's effective model without saving it into the active profile on disk
- `/session show` displays both the persisted configured profile/model and the current effective session runtime profile/model, making temporary overrides explicit
- `/session reset` clears temporary session-only overrides and returns the current live session to the persisted configured profile/model state
- invalid commands should produce concise usage feedback without mutating config or the current session state

The command family name may differ if implementation review finds a better fit, but the shipped surface should remain clearly session-scoped rather than config-scoped.

**Alternative considered:** make temporary switching a `/config temp ...` subcommand. Rejected: that still centers the experience on config mutation rather than session-local runtime state.

### 4. Keep persistence semantics explicit and visible

Users should be able to tell whether the current state is temporary or saved.

Required behavior:

- after a session-only switch, Themion should emit a clear acknowledgement such as `temporarily switched to profile 'x' for this session only` or `temporarily using model 'y' for this session only`
- `/session show` should be the explicit surface for displaying both persisted and effective current runtime state
- `/config` should continue to show the effective current runtime state because that is what the active session is actually using, but it should not imply that the temporary override was saved to disk
- when a temporary model override is active on top of a profile, the UI should make that layering understandable rather than pretending the saved profile itself changed
- restarting Themion should return to the ordinary configured profile/model unless the user later performs a persistent config change

This prevents user confusion about what did or did not get saved.

**Alternative considered:** rely only on silent runtime changes and let users infer persistence from later behavior. Rejected: temporary overrides are easy to misunderstand unless the UI labels them explicitly.

### 5. Preserve turn-level runtime attribution semantics

Temporary session-only switches should work cleanly with existing turn metadata and status/reporting surfaces.

Required behavior:

- future turns after a temporary switch should record the active effective profile/provider/model for those turns just as ordinary runtime changes already do
- earlier turns remain unchanged
- surfaces such as `/context`, statusline model display, local inspection output, and Stylos status should reflect the currently effective runtime state after the switch
- the feature must not require database schema changes because turn-level metadata support already exists

This keeps history analysis truthful when one session intentionally tries more than one model or profile.

**Alternative considered:** delay or hide metadata/status updates until a later PRD. Rejected: runtime attribution is already an important part of the current product behavior and should stay coherent when temporary switching lands.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Add `/session profile use`, `/session model use`, `/session show`, and `/session reset`; show clear acknowledgements; and reuse the existing agent replacement path without persisting config changes. |
| `crates/themion-cli/src/app_runtime.rs` | Reuse or slightly extend replacement-agent wiring so the effective session-only profile/model state can rebuild the main interactive agent cleanly. |
| `crates/themion-cli/src/main.rs` / session state types | Store any temporary effective profile/model override state needed so current-session status surfaces remain coherent without rewriting config. |
| `crates/themion-cli/src/stylos.rs` | Ensure exported status continues to reflect the currently effective profile/model when a temporary session-only override is active. |
| `crates/themion-core` | No major new abstraction is required, but existing turn metadata and inspection/status consumers must continue to reflect the effective runtime provider/model/profile chosen by the CLI session. |
| `docs/architecture.md` | Document the distinction between persistent config changes and session-only temporary profile/model switching. |
| `docs/engine-runtime.md` | Document that current-session effective profile/model can differ from persisted config when a temporary session override is active. |
| `docs/README.md` | Add the new PRD entry to the PRD table. |

## Edge Cases

- the user temporarily switches to another saved profile, then later restarts Themion → verify: the new app start uses the persisted configured profile rather than the temporary override.
- the user temporarily overrides only the model while staying on the same profile → verify: future turns use the temporary model and config on disk remains unchanged.
- the user issues `/session reset` after a temporary profile or model override → verify: the session returns to the persisted configured profile/model state.
- the user requests a nonexistent profile name → verify: Themion reports the error and leaves both config and current session state unchanged.
- the user temporarily switches profile while session-level API logging or other session-scoped toggles are active → verify: those toggles survive the rebuilt agent just as current profile-switch rebuilds already preserve them when practical.
- the session has prior turns from one model and later turns from another → verify: turn-level runtime metadata remains truthful for each turn.
- Stylos-enabled status export is active during a temporary override → verify: visible peer status reflects the effective current profile/model rather than stale persisted config values.

## Migration

This feature requires no database migration.

Rollout guidance:

- keep persistent `/config` behavior unchanged
- add session-only switching as an additive live-session capability
- update docs/status wording so users can distinguish temporary runtime overrides from saved config state

## Testing

- run `/session profile use <name>` during a TUI session → verify: the main interactive agent rebuilds, future turns use the selected profile, and config files remain unchanged.
- run `/session model use <model>` during a TUI session → verify: future turns use the temporary model override and the saved profile on disk is unchanged.
- run `/session reset` after a temporary override → verify: the session returns to the persisted configured profile/model state.
- run `/config` and any related status/inspection surfaces after a temporary override → verify: they clearly show the effective current runtime state and do not pretend that config on disk was rewritten.
- restart Themion after a temporary session-only override → verify: the session starts from the persisted configured profile/model rather than the old temporary override.
- switch temporarily mid-session and inspect later turn metadata → verify: later turns record the effective runtime profile/provider/model values active when those turns were created.
- run `cargo check -p themion-cli` after implementation → verify: the touched crate builds cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the touched crate still builds with the feature enabled.
- run `cargo check -p themion-cli --all-features` after implementation → verify: the touched crate still builds cleanly across feature combinations.

## Implementation checklist

- [x] add `/session profile use`, `/session model use`, `/session show`, and `/session reset`
- [x] rebuild the interactive agent from the effective temporary session state without calling config persistence
- [x] add a reset path that returns the session to persisted configured profile/model state
- [x] keep status, inspection, and turn-attribution surfaces coherent with the effective runtime state
- [x] update runtime/docs references and add the PRD entry to `docs/README.md`
