# PRD-086: Support Multiple Codex Profiles with Profile-Scoped Login State

- **Status:** Implemented
- **Version:** v0.56.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-05-02

## Summary

- Themion already supports multiple saved LLM profiles, but Codex login state is still effectively global, so one Codex login overwrites another.
- Users should be able to keep more than one Codex-backed profile and switch between them without re-logging every time or losing the previous Codex identity.
- Keep the existing device-code login flow, but bind Codex auth to the targeted profile instead of one shared `auth.json` file and one hard-coded `codex` profile outcome.
- `/login codex <profile>` should be the explicit way to log into a named Codex profile, while `/login codex` should follow one predictable default rule instead of silently updating the wrong profile.
- OpenRouter, llama.cpp, `/config` profile semantics, and `/session` temporary override semantics should remain unchanged.

## Implementation status

Landed in `v0.56.0` with profile-scoped Codex auth storage in `themion-cli`, explicit `/login codex [profile]` targeting, per-profile token-refresh writeback, and a narrow legacy `auth.json` migration path for obvious single-profile upgrades. The shipped behavior keeps non-Codex profile semantics unchanged, preserves immediate switch-to-target-profile login behavior, and now makes Codex readiness depend on the active profile's own auth state instead of one shared global login blob.

## Goals

- Let users maintain multiple saved `openai-codex` profiles with independent persisted login state.
- Ensure switching between Codex profiles uses the auth associated with that selected profile rather than one shared global Codex login.
- Preserve the existing device-code login experience while making profile targeting explicit and understandable.
- Keep Codex profile behavior aligned with Themion's existing profile-centric configuration model.
- Avoid forcing unnecessary re-login when the user switches back to a previously authenticated Codex profile.
- Define migration behavior clearly enough that existing one-account users do not lose a working Codex setup on upgrade.

## Non-goals

- No redesign of the OpenAI device-code flow itself.
- No requirement in this PRD to support multiple simultaneously active Codex accounts inside one single profile.
- No requirement to add a full profile-management wizard or GUI around login.
- No change to OpenRouter or llama.cpp auth semantics.
- No requirement to introduce cross-machine sync, export, or import of Codex auth state.
- No requirement to redesign temporary session-only profile or model overrides from PRD-076 and PRD-077.
- No requirement to introduce a profile rename or aliasing system in this PRD.

## Background & Motivation

### Current state

Themion already supports multiple named profiles in config and lets the user switch between them persistently or for one session only.

Relevant existing behavior documented in `docs/architecture.md`, `docs/engine-runtime.md`, and PRD-076/PRD-077:

- `themion-cli` owns config loading, login flows, and local profile/auth behavior
- `/config profile use <name>` persists a profile change
- `/session profile use <name>` changes only the current session
- runtime turns already record the effective profile/provider/model in turn metadata because profile identity may change during a session

But current Codex login behavior breaks that profile model in practice.

Current implementation behavior confirmed from the code:

- `/login codex` completes one device-code login flow
- login success always saves auth to one shared file, `~/.config/themion/auth.json`
- login success always creates or overwrites one hard-coded `codex` profile and switches the session to it
- when a Codex client is later built, it loads auth from that same shared file instead of from the selected profile

So Themion supports multiple profiles in general, but does not support multiple independent Codex-backed identities. Logging into one Codex account replaces the effective login for every Codex profile.

This is a product gap because users may reasonably want more than one Codex profile for purposes such as:

- separate personal and work Codex subscriptions
- different Codex accounts for different organizations or billing contexts
- a stable default Codex profile plus one experimental or temporary Codex profile
- switching between Codex and other providers without losing the exact previous Codex identity bound to a named profile

### Why profile-scoped auth is the right product model

Themion already teaches the user that profile is the durable unit of provider/model configuration.

The natural expectation is therefore:

- if two profiles are different named profiles, their provider-specific durable state should also be separable when that state affects whether the profile can actually run
- switching to a profile should make that profile's full effective provider identity active, not merely its model name or base URL

For Codex, persisted login state is part of that effective provider identity. If login remains global while profile settings are local, the product appears to support a distinction that it cannot actually preserve.

## Design

### 1. Codex auth should be scoped to the selected profile

Each saved profile whose provider is `openai-codex` should be able to own its own persisted Codex auth state.

Required behavior:

- a Codex-backed profile must be able to persist login state independently of other Codex-backed profiles
- selecting one Codex profile must load that profile's own auth state when constructing the backend client
- logging into one Codex profile must not overwrite another profile's saved Codex auth state
- refreshed access tokens must be written back only to the currently active profile's auth state
- the product should continue to tolerate profiles that exist before login and profiles that are created as part of the login flow

Implementation may use a profile-keyed auth store, profile-specific auth files, or another durable mapping, but the user-visible contract is profile-scoped persistence and profile-scoped loading.

**Alternative considered:** keep one global Codex auth blob and only let profiles vary by model or base URL. Rejected: that preserves the current mismatch between named profiles and actual provider identity.

### 2. Define a narrow explicit login command surface

The login flow should no longer silently mean "log into the one shared Codex account and switch to the hard-coded `codex` profile."

Normative command shapes:

- `/login codex`
- `/login codex <profile>`

Required behavior:

- `/login codex <profile>` logs into that named profile explicitly
- if `<profile>` does not exist, Themion should create it as a Codex profile with sensible defaults before binding the new auth to it
- `/login codex` should target the current active profile if that profile already has provider `openai-codex`
- otherwise, `/login codex` should target the literal profile name `codex`
- when `/login codex` falls back to `codex`, the completion or prompt text should make that default explicit so the user is not surprised
- invalid extra arguments should produce concise usage feedback without mutating config or auth state

This keeps the common path short while removing ambiguity.

**Alternative considered:** require a profile name on every Codex login. Rejected: always requiring a profile name would be stricter than necessary for the common single-profile path, as long as the default rule is deterministic and visible.

### 3. Successful login should update the targeted profile, not a hard-coded shared profile outcome

Current behavior always creates or overwrites the single `codex` profile and switches to it after login.

That should change.

Required behavior:

- successful Codex login should affect the targeted profile, not always the literal profile name `codex`
- if the target profile does not exist yet, Themion should create it with Codex defaults such as `provider = "openai-codex"` and the current default Codex model unless the user later edits those settings
- if the target profile already exists, Themion should preserve unrelated profile fields unless the login flow intentionally updates Codex-specific defaults required for a valid Codex profile
- if the target profile exists but currently names another provider, Themion should convert that profile into an `openai-codex` profile as part of the explicit Codex login action
- after success, Themion should switch the current live session to the targeted profile, matching the current login flow's immediate-use behavior
- the completion message should say which profile was updated and which Codex identity was authenticated

Example acknowledgement shape:

- `logged in as <account_id> — switched to Codex profile 'work' (gpt-5.4)`

This keeps Codex aligned with the rest of the config model, where named profiles are the durable unit of configuration.

### 4. Profile switching should use profile-bound Codex readiness

When the user switches profiles, provider readiness should reflect the selected profile's own Codex login state.

Required behavior:

- switching to a Codex profile that already has saved auth should work without re-running login
- switching to a Codex profile with no saved auth should produce a clear readiness or startup error that points the user to logging into that specific profile
- status and inspection surfaces that currently summarize provider readiness should reflect whether the active profile has Codex auth available
- session-only profile switches should use the same profile-scoped auth resolution as persistent profile switches
- temporary model overrides layered on top of a Codex profile should continue to use that same profile's auth

Recommended error wording principle:

- mention the missing profile by name, for example: `no Codex auth for profile 'work'; run /login codex work`

This preserves existing profile-switch behavior while making Codex readiness truthful per profile.

### 5. Existing non-Codex profile behavior should stay unchanged

This PRD is not a general profile-system rewrite.

Required behavior:

- OpenRouter and llama.cpp profile behavior should remain as it is today
- existing profile creation, listing, showing, persistent switching, and session-only switching semantics should remain intact except where Codex-specific login targeting needs small help text or validation changes
- existing device-flow prompting, browser handoff, and token refresh behavior for one Codex profile should continue to work once auth is loaded for that profile
- no new persistent config fields should be required for non-Codex profiles

The implementation should prefer the smallest clean product change that fixes Codex profile identity correctly rather than reworking all profile handling.

### 6. Define one concrete migration rule for legacy global Codex auth

Current users may already have one global Codex login stored in `~/.config/themion/auth.json` and one or more Codex-related profiles in config.

The product should preserve working setups without silently binding the old global login to the wrong profile.

Normative migration rule:

- legacy global Codex auth should be treated as a one-time migration source, not as the long-term canonical store after this feature lands
- if legacy global auth exists and exactly one profile is an obvious target, Themion should migrate that auth automatically to that profile
- for this PRD, an obvious target means the active configured profile if it is `openai-codex`, otherwise the literal `codex` profile if it exists and is `openai-codex`, otherwise the only profile whose provider is `openai-codex`
- if more than one plausible Codex target profile exists, Themion must not guess; instead it should preserve the legacy auth material and require an explicit `/login codex <profile>` for any profile the user wants to keep using
- once a profile has profile-scoped auth, that profile-scoped auth always wins over any legacy global auth fallback
- the product may keep the legacy global file on disk during the transition, but it should stop writing refreshed auth back to the legacy location once a profile-scoped store is in use

User-visible requirement for the ambiguous case:

- Themion should make the situation understandable with a concise message that says legacy global Codex auth could not be assigned safely and that the user should log into the intended profile explicitly

**Alternative considered:** delete the old global auth file and force every user to log in again. Rejected: that creates avoidable friction and discards still-valid durable state.

### 7. Bind auth by profile name and document rename behavior narrowly

This PRD should keep profile binding simple.

Required behavior:

- profile-scoped Codex auth should bind by profile name
- if the user manually renames a Codex profile in config, Themion is not required to migrate or discover that rename automatically in this PRD
- after a manual rename, the product may treat the new name as a different profile with no Codex auth and ask the user to log in again
- the product must not silently attach the old auth to the wrong new profile name

This keeps the first implementation predictable without introducing a separate profile identity layer.

**Alternative considered:** create stable hidden per-profile ids just for Codex auth binding. Rejected: that is a larger profile-system change than needed for this slice.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/login_codex.rs` | Keep the current device-code flow but apply its result to an explicitly targeted profile rather than only a hard-coded shared outcome. |
| `crates/themion-cli/src/auth_store.rs` | Replace the single global Codex auth storage model with profile-scoped persistence or an equivalent mapping that can load and save auth by profile name, while supporting one-time legacy migration behavior. |
| `crates/themion-cli/src/config.rs` | Preserve the existing profile model while ensuring new Codex-targeted profile creation uses sensible defaults and does not require unrelated config changes. |
| `crates/themion-cli/src/tui.rs` | Update `/login codex` command handling, help text, usage feedback, completion messaging, and post-login profile switching behavior so the flow is explicit about which profile was authenticated. |
| `crates/themion-cli/src/app_state.rs` | Build the Codex backend using the active profile's own persisted auth state and report missing-auth errors in a profile-specific way. |
| `crates/themion-cli/src/main.rs` / session state types | Keep runtime session/profile switching behavior unchanged except for using profile-scoped Codex auth resolution and any small login-target bookkeeping needed by the CLI layer. |
| `crates/themion-core/src/client_codex.rs` | Preserve token refresh behavior while allowing refreshed auth to be written back through a profile-aware writer from the CLI layer. |
| `docs/architecture.md` | Document that Codex login state is profile-scoped rather than globally shared once implemented. |
| `docs/engine-runtime.md` | Document how active-profile selection, provider readiness, and legacy-auth migration interact after profile-scoped auth lands. |
| `docs/README.md` | Keep the PRD entry aligned with this proposal and later reflect landed implementation status. |

## Edge Cases

- a user has one existing global Codex auth and one `codex` profile at upgrade time → verify: the old working setup remains usable without unnecessary re-login.
- a user has one existing global Codex auth and one non-`codex` active Codex profile at upgrade time → verify: the old auth migrates to that active Codex profile automatically.
- a user has one existing global Codex auth and multiple Codex-related profiles at upgrade time → verify: Themion does not silently guess the wrong profile binding.
- a user runs `/login codex work` while another Codex profile named `personal` already exists → verify: `work` auth is updated without disturbing `personal` auth.
- a user runs `/login codex` while the active profile is already a Codex profile named `work` → verify: the login targets `work`, not the literal `codex` profile.
- a user runs `/login codex` while the active profile is non-Codex → verify: the login targets `codex` by default and reports that default clearly.
- a user switches temporarily with `/session profile use work` to a Codex profile that already has saved auth → verify: the session rebuild uses `work`'s auth without rewriting config.
- a user switches to a Codex profile with no auth saved yet → verify: the error clearly points to logging into that specific profile.
- a refreshed access token is obtained while using one Codex profile → verify: the refreshed auth is persisted back only to that same profile.
- a non-Codex profile is active and the user runs a Codex login targeting another named profile → verify: only the targeted profile's durable Codex auth/config changes, and the resulting active-session switch behavior is clearly reported.
- a profile is renamed manually in config after it previously held Codex auth → verify: Themion does not silently reuse the old auth under the new name and instead requires an explicit re-login if needed.

## Migration

This feature is additive, but it changes where Codex auth is bound.

Migration requirements:

- preserve compatibility for users who currently have one working global Codex login
- migrate or retain existing auth material in a way that avoids data loss
- avoid silently binding one old global login to the wrong profile when more than one Codex profile could plausibly claim it
- stop treating the old global auth file as the durable source of truth once a profile has its own scoped auth

Recommended rollout shape:

- introduce profile-scoped Codex auth storage
- inspect legacy global auth only as a compatibility or one-time migration source
- migrate automatically only when the target profile is obvious under the rule defined above
- otherwise require explicit profile-targeted login instead of guessing
- prefer profile-scoped auth consistently once it exists for a profile

## Testing

- create `personal` and `work` Codex profiles and log into each separately → verify: each profile can be selected later without overwriting the other's saved auth.
- switch from `personal` to `work` using `/config profile use work` → verify: the rebuilt Codex client uses `work`'s auth and not `personal`'s auth.
- switch from `personal` to `work` using `/session profile use work` → verify: the temporary session rebuild uses `work`'s auth without changing the configured default profile.
- run `/login codex work` when `work` does not yet exist → verify: Themion creates or initializes the `work` profile with Codex defaults, binds the new login to that profile, and switches the live session to it.
- run `/login codex personal` when `personal` already exists and has prior Codex auth → verify: only `personal` auth is replaced.
- run `/login codex` while the active profile is an existing Codex profile named `work` → verify: the login applies to `work` rather than to a separate `codex` profile.
- run `/login codex` while the active profile is non-Codex and no `codex` profile exists yet → verify: Themion creates or initializes `codex`, binds the login there, and reports that default behavior clearly.
- upgrade from a legacy setup with one global `auth.json` and one obvious Codex profile → verify: the user can still use Codex without unnecessary manual recovery.
- upgrade from a legacy setup with one global `auth.json` and multiple plausible Codex profiles → verify: Themion avoids silent misbinding and instead requires explicit login for the intended profile.
- use a Codex profile long enough to trigger token refresh → verify: refreshed auth is saved back to the same profile-scoped store.
- manually rename a previously authenticated Codex profile in config → verify: Themion does not silently transfer auth and instead asks for an explicit login when the renamed profile is used.

## Implementation checklist

- [ ] define the durable storage model for Codex auth keyed by profile name
- [ ] update Codex client construction so active-profile selection loads the matching auth state
- [ ] update token-refresh writeback so refreshed auth persists to the correct profile-scoped store
- [ ] implement `/login codex` default-target behavior and `/login codex <profile>` explicit-target behavior
- [ ] remove the hard-coded post-login `codex` profile overwrite behavior while preserving automatic switch-to-targeted-profile behavior
- [ ] implement the legacy global-auth migration and ambiguous-case fallback behavior defined by this PRD
- [ ] update status/help/error surfaces so Codex readiness and login targeting are profile-specific and understandable
- [ ] update docs in `docs/architecture.md`, `docs/engine-runtime.md`, and `docs/README.md` when the feature lands
