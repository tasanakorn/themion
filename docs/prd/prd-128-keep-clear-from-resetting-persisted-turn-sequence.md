# PRD-128: Keep `/clear` from Resetting Persisted Turn Sequence

- **Status:** Implemented
- **Version:** v0.78.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (bug report) + Themion (PRD authoring)
- **Date:** 2026-05-16

## Summary

- `/clear` is meant to clear future prompt context, not to start turn numbering from `1` again.
- Today `/clear` resets the in-memory turn counter to `0`.
- A later turn in the same session can then reuse an older `turn_seq` value and fail when persisted, including the unique-key violation reported in the bug.
- Fix `/clear` so it clears conversation context without rewinding persisted session turn identity.
- Keep existing user-facing `/clear` meaning, but preserve monotonic turn numbering inside the active session.

## Problem

Themion currently treats `/clear` as a context reset and also resets the agent's in-memory `turn_seq_counter`.

That is unsafe for a live session. Persisted turn rows still belong to the same `session_id`, so the next turn after `/clear` can reuse an older `turn_seq` value.

The reported failure is a unique-key violation during later turn persistence. Even beyond that specific error shape, reusing an old `turn_seq` inside one live session breaks the core invariant that turn identity must move forward monotonically.

## Scope

In scope:

- `/clear` behavior in the live interactive session
- agent turn sequence ownership in `themion-core`
- any CLI/runtime path that calls `clear_context()` or equivalent context-reset logic
- docs that describe `/clear` semantics

Out of scope:

- changing the user-visible purpose of `/clear`
- creating a new session when the user runs `/clear`
- rewriting historical turn rows
- unrelated session/profile/runtime reset behavior

## Current behavior

Current documented behavior says `/clear` clears chat history before this point from future context.

Current code in `crates/themion-core/src/agent.rs` also does this during `clear_context()`:

- clears in-memory messages
- clears turn boundaries
- resets `turn_seq_counter` to `0`

This is the key implementation detail behind the bug:

- `/clear` is session-local and does not allocate a new `session_id`
- old turn rows remain persisted for that same live session
- the next turn can therefore reuse a previously used session-local sequence number

That mixes two different concepts:

- prompt-context visibility
- persisted session turn identity

The first reset is correct for `/clear`. The second is not safe inside the same session.

## Expected behavior

Required behavior:

- `/clear` must keep its current meaning of clearing prior conversation from future prompt context
- `/clear` must not reset the persisted turn sequence for the active session
- the next turn after `/clear` must use a new monotonic `turn_seq` value for that same `session_id`
- no runtime path should be able to reuse an already-persisted `(session_id, turn_seq)` pair for a continued live session
- clearing context must not create a new session or replacement `session_id`
- already persisted turn rows and message rows must remain unchanged
- if Themion restores or clones agent state, the preserved turn counter must still reflect the latest turn already owned by that live session

Product rule:

- `/clear` is a context cutoff, not a session identity reset

## Fix approach

### 1. Separate context reset from turn-sequence ownership

The runtime should treat these as separate pieces of state:

- conversation/context replay state
- persisted per-session turn sequencing

Implementation should clear the first without rewinding the second.

This PRD intentionally keeps the fix small. The preferred direction is to make `clear_context()` a true context-only operation, or to introduce a narrower context-only reset path and make `/clear` use that path.

### 2. Keep `turn_seq` monotonic within one session

Required behavior:

- once a session has recorded turn `N`, later turns in that same session must use `N+1` or higher
- `/clear` must not make the next turn reuse `1` or any previously used sequence number
- if an agent instance is rebuilt from existing live session state, its turn counter must be restored from preserved runtime state or persisted session history rather than defaulting to zero incorrectly

Invariant:

- a live session may forget prior prompt context after `/clear`, but it must not forget which turn numbers were already consumed by persistence

### 3. Preserve current `/clear` user experience

This bug fix should not broaden `/clear` into a full session reset.

Required behavior:

- the future prompt window should stop including the cleared earlier conversation under current rules
- the visible transcript/context handling should keep current intended behavior unless a separate PRD changes it
- command help and docs should continue describing `/clear` as a context-clear action, not a new-session action

### 4. Harden the runtime against duplicate turn insertion

This bug came from one reset path, but the invariant should be explicit.

Required behavior:

- runtime/session code should maintain one authoritative next-turn sequence for the active session
- code paths that recreate agent state for an existing session must not silently reinitialize turn numbering to zero
- add a targeted regression check that proves a clear-context flow cannot reuse an earlier `turn_seq`

If the codebase has more than one reset-style helper, only the helpers that truly mean “new session” may reset turn numbering.

## Risks / edge cases

- user runs `/clear` multiple times in one session → verify: turn numbering still increases monotonically
- user runs `/clear` after many prior turns → verify: the next persisted turn does not collide with an older one
- live session state is restored or rebuilt after `/clear` → verify: the correct next turn sequence is still used
- history/context replay becomes empty after `/clear` → verify: this does not affect persisted turn identity
- feature-gated runtime builds still share the same monotonic turn-sequence rule → verify: the fix does not depend on one UI surface only

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-128-keep-clear-from-resetting-persisted-turn-sequence.md` | Add the durable bug-fix PRD. |
| `crates/themion-core/src/agent.rs` | Stop `clear_context()` from resetting persisted turn-sequence state, or split the API so `/clear` uses a context-only reset path. |
| `crates/themion-cli` slash-command/runtime path | Keep `/clear` mapped to context clearing only, without any session turn-identity reset side effect. |
| tests near affected runtime code | Add a regression test for clear-then-next-turn sequencing in one session and for repeated `/clear` calls in the same session. |
| `docs/README.md` | Index the new PRD and later update status/version when implemented. |

## Validation

- start a session, send one prompt, run `/clear`, then send another prompt → verify: the second persisted turn uses the next `turn_seq` instead of reusing `1`
- run `/clear` several times between prompts → verify: each later turn keeps increasing `turn_seq`
- inspect the failing persistence path from the bug report → verify: the unique-key violation no longer occurs after `/clear`
- inspect prompt/context behavior before and after `/clear` → verify: old context is excluded from future prompt replay according to current `/clear` semantics
- restore or rebuild agent state for the same session after a clear-context action → verify: next-turn sequencing still follows the latest session turn
- run `cargo check -p themion-core -p themion-cli` → verify: touched crates compile with default features
- run `cargo check --all-features -p themion-core -p themion-cli` → verify: touched crates still compile across feature combinations

## Implementation checklist

- [x] remove the `/clear` side effect that resets `turn_seq_counter` for an existing session
- [x] confirm the active-session next-turn counter survives context clearing and any same-session runtime restore path
- [x] add regression tests that cover turn creation before and after `/clear`, including repeated clear calls in one session
- [x] update docs/help text only if they currently imply that `/clear` resets the session rather than context only

## Implementation notes

Implemented in `crates/themion-core/src/agent.rs` by making `clear_context()` preserve `turn_seq_counter` while still clearing messages and turn boundaries. Added a targeted regression test that proves context clearing no longer rewinds the in-memory turn counter for the active session.
