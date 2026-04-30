# PRD-077: Reset Temporary Model Override When Switching Session Profile

- **Status:** Implemented
- **Version:** v0.49.1
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-30

## Implementation status

Landed in `v0.49.1` as a focused refinement to PRD-076 session-only switching semantics. The shipped behavior clears any active temporary session model override when `/session profile use <name>` succeeds, rebuilds the interactive agent from the selected profile's configured model, and makes the reset explicit in command feedback while keeping later explicit `/session model use <model>` overrides available.

## Summary

- PRD-076 added session-only profile and model switching, but the current behavior can leave a temporary model override active when the user switches to a different session profile.
- That makes a profile switch feel incomplete because the newly selected profile may not actually use its own configured default model.
- This follow-up PRD requires `/session profile use <name>` to reset any active temporary model override and adopt the selected profile's model by default.
- Explicit temporary model overrides should still remain possible, but only after the profile switch has completed.
- Persistent `/config` behavior and session-only model override commands stay unchanged.

## Goals

- Make session-only profile switching use the selected profile's configured model by default.
- Prevent a stale temporary model override from carrying across profile changes implicitly.
- Keep the temporary override model simple and easy to reason about: profile switch picks the profile's model, explicit model override changes it again.
- Preserve PRD-076's distinction between temporary session state and persisted config state.

## Non-goals

- No redesign of the `/session model use <model>` command or its persistence semantics.
- No change to persistent `/config profile use <name>` behavior unless separate implementation review finds that the same bug exists there and the user explicitly expands scope.
- No requirement to add per-profile remembered temporary model overrides.
- No change to startup profile resolution, saved config files, or cross-session persistence.
- No broader session-override precedence redesign beyond the specific interaction between session profile switching and temporary model overrides.

## Background & Motivation

### Current state

PRD-076 introduced two independent session-scoped controls:

- `/session profile use <name>` for temporarily switching to another saved profile
- `/session model use <model>` for temporarily overriding the effective model

That feature is useful, but it creates an ambiguity in override precedence when both commands are used in one live session. If the user first applies `/session model use <model>` and then later runs `/session profile use <name>`, the current session may continue using the old temporary model override instead of the newly selected profile's configured model.

From a product perspective, that is surprising. A user who switches to another profile usually expects the profile's full runtime identity to become active, including its default model, unless they explicitly override the model again afterward.

The current interaction makes `/session profile use <name>` feel only partially effective and can hide the real model associated with the target profile.

## Design

### 1. Session profile switches should clear any active temporary model override

When the user temporarily switches to another profile, Themion should treat that as selecting a new session baseline.

Required behavior:

- `/session profile use <name>` must clear any currently active session-only model override
- after the switch, the effective model must come from the selected profile's configured model
- the switch acknowledgement should make it clear that the new profile is active and that the session is now using that profile's model unless the user applies a new explicit model override later
- config files must remain unchanged because this is still session-only behavior

This gives profile switching predictable semantics: choose profile first, then optionally override model.

**Alternative considered:** keep the existing temporary model override across profile changes until the user manually resets it. Rejected: that makes profile switching behave like a partial switch and is harder for users to reason about.

### 2. Keep explicit model overrides available after the profile switch

Clearing a stale override during profile switching should not weaken the underlying session-only model override feature.

Required behavior:

- after `/session profile use <name>`, the user may still run `/session model use <model>` to apply a new temporary override on top of the new profile
- `/session reset` should continue clearing all temporary session overrides and returning to the persisted configured profile/model baseline
- `/session show` should continue distinguishing between the persisted configured state and the current effective session state

This preserves the two-step mental model introduced by PRD-076 while removing a confusing precedence edge case.

**Alternative considered:** remove temporary model overrides entirely whenever session-only profile switching exists. Rejected: explicit model experimentation within a session remains useful and was an intended part of PRD-076.

### 3. Make command feedback reflect the reset behavior clearly

The user should not need to infer that the model changed as a side effect of the profile switch.

Required behavior:

- the acknowledgement after `/session profile use <name>` should mention the effective model now associated with that profile when practical
- if a temporary model override was cleared during the switch, the message should make that reset explicit rather than silently changing models
- `/session show` should present the resulting effective profile/model state clearly after the switch

This keeps the runtime state understandable and avoids confusing transitions in multi-step experimentation sessions.

**Alternative considered:** silently reset the model override and rely on `/session show` for later discovery. Rejected: this would preserve the same transparency problem in a different direction.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Update `/session profile use <name>` so it clears any active temporary model override, rebuilds the agent from the selected profile's model, and emits clear acknowledgement text. |
| `crates/themion-cli/src/main.rs` / session state types | Adjust any session-override state handling so profile switches replace the temporary profile baseline and remove stale model overrides. |
| `docs/prd/prd-077-reset-temporary-model-override-when-switching-session-profile.md` | Track the intended follow-up behavior as a focused requirement derived from PRD-076. |
| `docs/README.md` | Add the new PRD entry to the PRD table. |

## Edge Cases

- the user runs `/session model use <model-a>`, then `/session profile use <profile-b>` → verify: the session switches to `profile-b` and uses `profile-b`'s configured model rather than `model-a`.
- the user runs `/session profile use <profile-b>`, then `/session model use <model-c>` → verify: the later explicit model override still takes effect for the current session.
- the user switches between multiple profiles in one session without calling `/session reset` → verify: each profile switch clears any prior temporary model override and adopts the newly selected profile's model.
- the selected profile has the same model as the old override → verify: the state still resets correctly even if the visible model string does not change.
- the user requests a nonexistent profile → verify: Themion reports the error and leaves both the current profile and any current temporary model override unchanged.

## Migration

This feature requires no database or config migration.

Rollout guidance:

- treat this as a behavior refinement to PRD-076 session-only switching semantics
- keep command names and persistence boundaries unchanged
- update user-facing acknowledgement text so the new precedence rule is visible

## Testing

- run `/session model use <model-a>`, then `/session profile use <profile-b>` → verify: the effective model resets to `profile-b`'s configured model and does not keep `model-a`.
- run `/session profile use <profile-b>`, then `/session model use <model-c>` → verify: the explicit later model override still takes effect.
- run `/session show` after a profile switch that cleared an earlier model override → verify: the reported effective state matches the selected profile and no stale temporary model override remains active.
- run `/session reset` after switching profile and model in the same session → verify: the session returns to the persisted configured profile/model baseline.
- run `cargo check -p themion-cli` after implementation → verify: the touched crate builds cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the touched crate still builds with the feature enabled.
- run `cargo check -p themion-cli --all-features` after implementation → verify: the touched crate still builds cleanly across feature combinations.

## Implementation checklist

- [x] clear any active temporary model override when `/session profile use <name>` succeeds
- [x] rebuild the interactive agent from the selected profile's configured model after the switch
- [x] update acknowledgement and inspection text so the reset behavior is explicit
- [x] add the PRD entry to `docs/README.md`
