# PRD-098: Normalize Session/Profile Command Surface

- **Status:** Implemented
- **Version:** v0.60.2
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-04

## Summary

- Themion's current `/session` and `/config` slash commands describe closely related profile/runtime state, but their command shapes are not structurally consistent.
- Keep the existing persistent-vs-session-local behavior split, but make `/session` read more like a profile-scoped command family.
- Replace mixed root-level session commands such as `/session model use ...` and `/session reset` with canonical profile-style commands such as `/session profile set model=<value>` and `/session profile reset`.
- Add `/help` as the canonical built-in way to list implemented slash commands so users do not have to infer command availability from family-specific fallback messages.
- Define one explicit expected command list so help text, docs, and implementation all point at the same supported slash-command surface.
- Show supported `/config profile set` forms as concrete commands instead of treating `key=value` as a standalone supported command entry.
- Add persistent profile-management commands for cloning and deleting profiles with explicit safety constraints.
- Avoid changing `/config` persistence semantics or implying that session-local changes were saved to disk.

## Goals

- Make `/session` command structure more consistent with `/config profile ...` patterns.
- Preserve the product distinction between persistent config mutation and session-local runtime overrides.
- Reduce command-surface surprise by grouping session-local override operations under `/session profile ...`.
- Give users one explicit `/help` command that lists the implemented slash-command surface.
- Define a stable expected command list that becomes the source of truth for user-facing slash-command discovery.
- Make the allowed `/config profile set` keys explicit in docs and help output.
- Add clear slash commands for cloning and deleting saved profiles.
- Prevent destructive profile deletion of the active profile or the default profile.
- Keep help and usage output aligned with the command family actually being used.

## Non-goals

- No redesign of profile storage, config file format, or session-state persistence.
- No change to the underlying semantics of session-only overrides established by PRD-076.
- No expansion in this PRD to session-local overrides for every possible profile field unless existing implementation already supports them cleanly.
- No broad rework of unrelated slash commands beyond adding a user-visible help entry point for the existing command surface and clarifying the supported command list.

## Background & Motivation

### Current state

PRD-076 introduced session-only overrides through these command shapes:

- `/session profile use <name>`
- `/session model use <model>`
- `/session show`
- `/session reset`

Current `tui.rs` handling also keeps persistent config behavior under a profile-scoped namespace:

- `/config`
- `/config profile [list]`
- `/config profile show`
- `/config profile create <name>`
- `/config profile use <name>`
- `/config profile set key=value`

Current runtime validation for `/config profile set` accepts these keys:

- `provider`
- `model`
- `endpoint`
- `api_key`

The currently implemented slash-command surface also includes top-level commands such as:

- `/login codex [profile]`
- `/debug runtime`
- `/debug api-log enable|disable`
- `/context`
- `/clear`
- `/unified-search index ...`
- `/exit` and `/quit`

That leaves the overall command experience partially aligned but not fully regular:

- `/config` uses a clearly namespaced `profile ...` family for mutations
- `/session` mixes `profile use` with root-level `model use` and `reset`
- `/session show` exists, but `/config` uses the bare root command for current-state display
- unknown `/config ...` help text currently includes unrelated top-level commands, while `/session ...` help is family-scoped
- there is no single explicit `/help` command that lists implemented slash commands directly
- there is no one documented expected command list that defines what `/help` should show
- the meaning of `key` in `/config profile set key=value` is only implicit unless the user reads implementation messages
- saved-profile lifecycle actions such as clone and delete are not part of the expected command list yet

### Why this matters now

The product problem is not missing capability. The problem is that the slash-command surface asks the user to remember different structural patterns for closely related configuration concepts while also lacking one obvious discovery command and one documented supported-command list.

For users thinking in terms of "current profile state for this session" versus "saved profile state on disk," a more regular command family is easier to learn and easier to extend without ad hoc exceptions. Separately, a built-in `/help` command makes the slash-command surface discoverable without relying on trial, memory, or family-specific error text.

The requested consistency direction is to treat session-local runtime overrides as a profile-scoped command family, for example:

- `/session profile set model=<value>`
- `/session profile reset`
- `/session profile show`

and to add:

- `/help`

This PRD also makes the persistent profile-mutation surface more explicit by documenting the accepted `/config profile set` forms directly rather than treating `key=value` as a standalone supported command entry. It also adds explicit commands for cloning and deleting saved profiles so profile lifecycle actions stay inside the same `/config profile ...` family.

**Alternative considered:** keep the current mixed `/session` grammar and only improve help text. Rejected: the user-visible inconsistency is in the command shape itself, not only in the help output.

## Design

### 1. Keep the persistent-vs-session-local split unchanged

This PRD does not reopen the behavior decision from PRD-076.

Required behavior:

- `/config ...` remains the persistent command family that saves to config on disk
- `/session ...` remains the session-local command family that affects only the current live session
- acknowledgements and help text must continue to make the persistence boundary explicit

### 2. Normalize `/session` around a profile-scoped subcommand family

Themion should make `/session` structurally resemble `/config profile ...` where practical.

Required behavior:

- Themion should support `/session profile show` as the canonical session-state display command
- Themion should support `/session profile set model=<value>` for the current session's effective model override
- Themion should support `/session profile reset` to clear temporary session-only overrides
- `/session profile use <name>` remains the session-local profile switch path
- legacy root-level forms such as `/session show`, `/session model use <model>`, and `/session reset` should be removed rather than preserved as supported aliases

This keeps the current semantics but gives the command family one primary structural pattern.

**Alternative considered:** use `/session set ...` and `/session reset` without the `profile` namespace. Rejected: that would improve some regularity, but it would still diverge from the established `/config profile ...` family that users already see.

### 3. Add `/help` as the canonical slash-command discovery surface

Themion should expose one explicit top-level help command for implemented slash commands.

Required behavior:

- `/help` should list the currently implemented slash commands in a concise, user-scannable format
- `/help` should include top-level commands and indicate important subcommand families such as `/config ...` and `/session ...`
- `/help` should present canonical command shapes, including the normalized `/session profile ...` forms defined by this PRD
- `/help` should describe only implemented commands, not aspirational or hidden internal commands

This gives users one predictable discovery path instead of requiring them to trigger fallback help from unrelated commands.

### 4. Expected command list

This PRD defines the expected supported slash-command list that `/help`, docs, and command-family help should agree on.

Expected command list after implementation:

- `/help`
- `/config`
- `/config profile`
- `/config profile show`
- `/config profile create <name>`
- `/config profile clone <source> <dest>`
- `/config profile delete <name>`
- `/config profile use <name>`
- `/config profile set provider=<value>`
- `/config profile set model=<value>`
- `/config profile set endpoint=<value>`
- `/config profile set api_key=<value>`
- `/session profile show`
- `/session profile use <name>`
- `/session profile set model=<value>`
- `/session profile reset`
- `/login codex [profile]`
- `/debug runtime`
- `/debug api-log enable`
- `/debug api-log disable`
- `/context`
- `/clear`
- `/unified-search index show`
- `/unified-search index refresh`
- `/unified-search index rebuild`
- `/exit`
- `/quit`

Requirements for this list:

- `/help` should display these commands using the canonical syntax above
- command-family help for `/config` and `/session` should be consistent with the relevant entries in this list
- removed legacy session forms such as `/session show`, `/session model use <model>`, and `/session reset` must not appear in this list
- the generic placeholder form `key=value` may be used inside usage text, but it is not itself a separate supported command entry in the expected command list
- if implementation later adds or removes supported slash commands, the expected command list in docs should be updated in the same change

### 5. Explicit `/config profile set` key list

The meaning of `key` in `/config profile set key=value` should be documented directly.

Supported keys in the current implementation:

- `provider`
- `model`
- `endpoint`
- `api_key`

Required behavior:

- `/help` and family-scoped help should describe `key` as one of the supported profile field names above
- `model` should be shown explicitly as a concrete example, not only implied through the generic `key=value` form
- unsupported keys such as `api_token` must not be documented unless the implementation adds support for them
- if the supported key set changes, the docs/help text and validation message must be updated in the same change

### 6. Add profile clone and delete commands

Saved profile lifecycle actions should be part of the same `/config profile ...` family.

Required behavior:

- Themion should support `/config profile clone <source> <dest>` to create a new saved profile by copying an existing saved profile
- Themion should support `/config profile delete <name>` to remove a saved profile
- cloning should preserve the source profile's saved fields in the new destination profile unless the command explicitly changes fields later through separate `set` operations
- clone and delete should produce clear success or failure messages

### 7. Delete safety constraints

Profile deletion should be guarded so users cannot break the current session or erase the repository's baseline profile accidentally.

Required behavior:

- `/config profile delete <name>` must reject deletion of the current active profile
- `/config profile delete <name>` must reject deletion of the `default` profile
- failed delete attempts should return clear user-facing explanations rather than silently doing nothing
- successful deletion should update persisted config on disk

**Alternative considered:** allow deleting the active profile and auto-switch elsewhere. Rejected: that adds surprising policy and hides an important destructive transition behind one command.

### 8. Canonical forms replace legacy session command shapes

This PRD intentionally chooses replacement over compatibility aliases.

Required behavior:

- canonical help text should show only the normalized `/session profile ...` forms
- old forms such as `/session show`, `/session model use <model>`, and `/session reset` should no longer execute successfully once this PRD is implemented
- when practical, removed forms should return a concise migration hint pointing the user to the canonical replacement
- `/help` should list only currently supported commands

This keeps the command surface simple and avoids carrying two grammars for the same behavior.

### 9. `/config` help and `/session` help should follow the same scoping rule

The two families should not only look similar in syntax; they should also behave similarly when the user asks for help or mistypes a subcommand.

Required behavior:

- `/config` help or fallback output should list `/config`-family commands only
- `/session` help or fallback output should list `/session`-family commands only
- canonical examples shown in help should use the normalized command shapes defined by this PRD
- `/config` help should list the supported `profile set` keys explicitly rather than only saying `key=value`
- `/config` help should include the clone and delete commands with their canonical argument shapes

### 10. Session-local `set` semantics must stay explicitly session-local

Using `profile set` under `/session` must not imply mutation of the saved profile object.

Required behavior:

- `/session profile set model=<value>` changes only the effective runtime state for the current session
- it must not rewrite config files or mutate the saved profile definition on disk
- command acknowledgements and `/session` display output should make that session-local nature clear

If implementation later expands `/session profile set` beyond `model=...`, that expansion must still preserve session-local semantics.

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-098-normalize-session-profile-command-surface.md` | Define the command-surface normalization requirement, the canonical session-profile command grammar, the `/help` discovery command requirement, the expected supported command list, the explicit `/config profile set` key list, and the new clone/delete profile commands. |
| `crates/themion-cli/src/tui.rs` | Normalize `/session` parsing and help output around `/session profile ...` forms, remove legacy session command forms, add `/help`, scope `/config` fallback help to the `/config` family only, list supported `/config profile set` keys explicitly, and add clone/delete help entries. |
| `crates/themion-cli/src/app_runtime.rs` | Keep `/config profile set` validation and user-facing error text aligned with the documented supported keys, and implement clone/delete profile runtime behavior with deletion safeguards. |
| `crates/themion-cli/src/config.rs` | Support persisted profile clone/delete operations in config storage behavior when needed. |
| `docs/architecture.md` | Update command examples if the canonical session-local forms change from the current PRD-076 wording, document `/help` as the built-in slash-command discovery surface, and keep user-facing command examples aligned with the expected command list. |
| `docs/README.md` | Add the new PRD entry to the table. |

## Edge Cases

- a user types `/help` → verify: Themion lists implemented slash commands using canonical syntax only.
- a user compares `/help` output with `/config` or `/session` family help → verify: they agree on the relevant supported commands.
- a user types `/config profile set api_token=<value>` → verify: Themion rejects it because `api_token` is not in the supported key list.
- a user runs `/config profile clone x y` and `y` already exists → verify: Themion rejects the clone with a clear explanation instead of overwriting an existing profile.
- a user runs `/config profile clone x y` and `x` does not exist → verify: Themion rejects the clone with a clear explanation.
- a user runs `/config profile delete default` → verify: Themion rejects the delete with a clear explanation.
- a user runs `/config profile delete <active-profile>` → verify: Themion rejects the delete with a clear explanation.
- a user runs `/config profile delete <nonexistent>` → verify: Themion rejects the delete with a clear explanation.
- a user runs `/config profile delete <inactive-nondefault>` → verify: Themion deletes the saved profile and persists the change.
- a user types `/session` with no subcommand → verify: Themion shows session-family help or the canonical session-profile display behavior consistently.
- a user types removed legacy syntax such as `/session model use <model>` → verify: Themion rejects it with a concise migration hint to `/session profile set model=<model>`.
- a user types removed legacy syntax such as `/session show` or `/session reset` → verify: Themion rejects it with a concise migration hint to the canonical replacement.
- a user types `/session profile set` without `key=value` → verify: Themion returns concise family-scoped usage text and does not mutate session state.
- a user types `/session profile reset` when no temporary override is active → verify: Themion responds safely and leaves effective state unchanged.
- a user compares `/config profile set model=...` with `/session profile set model=...` → verify: the former persists and the latter does not, and the output makes that distinction obvious.
- a user mistypes `/config ...` → verify: fallback help stays scoped to `/config` commands only.

## Migration

This is a command-surface normalization change, not a storage or schema migration.

Preferred rollout behavior:

- keep existing implemented semantics from PRD-076
- replace legacy `/session` command forms with normalized `/session profile ...` forms
- add `/help` as the canonical discovery surface for implemented slash commands
- update active docs so examples use the canonical syntax consistently
- keep `/help`, docs, and family-specific help aligned with the expected command list
- keep `/config profile set` key documentation aligned with runtime validation
- add clone/delete profile commands within the `/config profile ...` family
- return brief migration hints for removed session command forms where practical

## Testing

- run `/help` → verify: the output lists implemented slash commands and uses canonical syntax.
- compare `/help` with the expected command list in docs → verify: the supported commands and syntax match.
- run `/config profile clone x y` with valid source and new destination → verify: profile `y` is created as a copy of `x` and persisted.
- run `/config profile clone x y` when `y` already exists → verify: the command is rejected without overwriting `y`.
- run `/config profile delete <inactive-nondefault>` → verify: the profile is removed from persisted config.
- run `/config profile delete default` → verify: the command is rejected with a clear explanation.
- run `/config profile delete <active-profile>` → verify: the command is rejected with a clear explanation.
- run `/config profile set provider=<value>` → verify: the active profile is updated and saved.
- run `/config profile set model=<value>` → verify: the active profile model is updated and saved.
- run `/config profile set endpoint=<value>` → verify: the active profile endpoint is updated and saved.
- run `/config profile set api_key=<value>` → verify: the active profile API key is updated and saved.
- run `/config profile set api_token=<value>` → verify: the command is rejected and the valid key list remains explicit.
- run `/session profile use <name>` → verify: session-local profile switching still works and does not persist config changes.
- run `/session profile set model=<model>` → verify: the effective session model changes without rewriting config on disk.
- run `/session profile reset` after a temporary override → verify: the session returns to the persisted configured state.
- run `/session` and `/session profile show` → verify: the displayed session/runtime state is consistent with the canonical help surface.
- run removed legacy syntax such as `/session model use <model>` → verify: the command is rejected and points to `/session profile set model=<model>`.
- run removed legacy syntax such as `/session show` or `/session reset` → verify: the command is rejected and points to the canonical replacement.
- enter an unknown `/config ...` subcommand → verify: help output stays scoped to `/config` commands.
- run `cargo check -p themion-cli` after implementation → verify: the touched crate builds cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the touched crate still builds with Stylos enabled.
- run `cargo check -p themion-cli --all-features` after implementation → verify: the touched crate still builds cleanly across feature combinations.

## Implementation checklist

- [x] define the canonical `/session profile ...` command grammar in CLI parsing and help text
- [x] add `/session profile set model=<value>` and `/session profile reset`
- [x] remove `/session show`, `/session model use`, and `/session reset` as supported commands
- [x] add `/config profile clone <source> <dest>`
- [x] add `/config profile delete <name>` with active-profile and default-profile safeguards
- [x] add `/help` that lists implemented slash commands using the expected command list
- [x] scope `/config` fallback help to `/config` commands only
- [x] list supported `/config profile set` keys explicitly in help and docs
- [x] keep `/config profile set` validation and user-facing key list aligned
- [x] update active docs/examples so canonical syntax is consistent
- [x] keep `/help`, family-specific help, and docs aligned with the expected command list


## Implementation notes

- Landed in `v0.60.2` in `themion-cli` and docs.
- Added `/help`, normalized the session-local command family to `/session profile show|use|set model=<value>|reset`, added `/config profile clone <source> <dest>` and `/config profile delete <name>`, and blocked deletion of the active profile and `default`.
- `/config profile set` remains implemented for `provider`, `model`, `endpoint`, and `api_key`.
- Legacy `/session show`, `/session model use <model>`, and `/session reset` now return migration guidance to the canonical `/session profile ...` forms instead of remaining supported commands.
