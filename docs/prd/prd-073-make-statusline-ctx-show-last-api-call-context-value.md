# PRD-073: Make Statusline `ctx` Show the Last API Call Context Value

- **Status:** Implemented
- **Version:** v0.47.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-30

## Implementation status

Landed in `v0.47.1` as a statusline correctness fix. The shipped behavior keeps cumulative `in/out/cached` counters unchanged, adds a distinct last-API-call input token value to the turn stats emitted from core, and makes the statusline `ctx` field use that last-round value instead of the aggregated turn input total.

## Summary

- Themion's statusline currently shows cumulative session `in/out/cached` token totals and a `ctx` value, but the current `ctx` value is effectively tied to per-turn total input usage across all API calls inside that turn.
- That makes `ctx` misleading when one user turn contains multiple model rounds, because `ctx` should represent the context size of the last API call, not the accumulated input usage of the whole turn.
- This PRD changes the statusline so `ctx` uses the last API call prompt/input token value while keeping cumulative `in/out/cached` totals unchanged for their existing session-usage purpose.
- The feature is a correctness fix for statusline semantics, not a redesign of cumulative token accounting.
- The implementation preserves the existing `ctx:<used>/<limit>` model and only corrects what `<used>` means.

## Goals

- Make statusline `ctx` reflect the prompt/input token value of the last API call rather than cumulative input usage across all API calls in the turn.
- Preserve the existing cumulative `in/out/cached` counters as cumulative session totals for their current purpose.
- Keep the current compact statusline shape and the existing `ctx:<used>/<limit>` meaning from the user's perspective, while correcting the source of `<used>`.
- Keep the production of per-round context/input values in `themion-core` and the rendering of the statusline in `themion-cli`.
- Keep the change small and behaviorally targeted rather than mixing it with unrelated statusline or replay-estimation work.

## Non-goals

- No redesign of the cumulative `in/out/cached` statusline counters.
- No change to provider-reported token collection beyond capturing the last API call input value needed for `ctx`.
- No redesign of `/context` output or PRD-072 effective tool-token estimation.
- No new tokenizer heuristics or prompt-budget policy changes in this PRD.
- No change to the context-window limit side of `ctx:<used>/<limit>`; only `<used>` semantics are being corrected.

## Background & Motivation

### Current state

Historical statusline behavior established by PRD-005 says that the TUI statusline renders `ctx:<used>/<limit>`, where `<used>` is the last turn's prompt token count and `<limit>` is the active model's context limit when known.

Later cumulative-token work established separate session totals for `in`, `out`, and `cached` in the statusline. Those cumulative counters are useful and should remain cumulative.

The current problem is that a user turn can contain multiple API/model rounds, especially when tools are used. Themion already aggregates per-turn totals across all rounds for `tokens_in`, `tokens_out`, and `tokens_cached`. That aggregation is correct for cumulative accounting, but it is not the correct source for `ctx`.

When `ctx` is sourced from the aggregated turn input total, it overstates the size of the final prompt-visible context for the last API call in tool-using turns. This makes the statusline less trustworthy exactly when users are trying to understand actual prompt size.

The desired semantics are straightforward:

- cumulative `in/out/cached` remain cumulative
- statusline `ctx` should show the last API call's input token value

## Design

### 1. Keep cumulative token totals unchanged

Themion should preserve the existing cumulative session token counters exactly as cumulative accounting values.

Required behavior:

- `in` remains the cumulative sum of input tokens across completed turns
- `out` remains the cumulative sum of output tokens across completed turns
- `cached` remains the cumulative sum of cached input tokens across completed turns
- this PRD must not repurpose or redefine those counters

This keeps the existing session-usage reporting intact.

**Alternative considered:** repurpose `in` or another existing counter to behave more like `ctx`. Rejected: cumulative usage and last-call context size are different concepts and should remain separate.

### 2. Track the last API call input token value separately from turn totals

Themion should carry a distinct last-round input token value through the turn lifecycle.

Required behavior:

- when a provider round completes and reports usage, the runtime should capture that round's input token value as the current last API call context value
- when multiple API rounds happen within one user turn, the last completed round should win
- when a turn completes, the TUI should use that last-round value for the statusline `ctx` field
- the aggregated turn `tokens_in` should continue to sum all rounds for that turn independently of the last-round value

This separates two meanings that are currently conflated.

**Alternative considered:** derive `ctx` from the largest round in the turn. Rejected: the statusline should reflect the most recent prompt context shape, not the peak historical round inside the turn.

### 3. Preserve the existing `ctx:<used>/<limit>` display contract

The visible statusline contract should stay the same except for the corrected source of `<used>`.

Required behavior:

- the statusline should continue to render `ctx:<used>/<limit>`
- `<used>` should now mean the last API call input token value
- `<limit>` should continue to use the existing model-info limit logic established by PRD-005
- the compact-number formatting already used in the statusline should remain unchanged

This keeps the user-facing shape familiar while fixing the semantics.

**Alternative considered:** rename `ctx` to a more explicit label. Rejected: the compact statusline label is already established; the issue is semantic correctness, not label discoverability.

### 4. Keep the core/CLI ownership boundary clean

The last-round input token value should be produced in `themion-core` and consumed/rendered in `themion-cli`.

Required behavior:

- `themion-core` should emit or expose enough data for the TUI to know the last API call input token value for the completed turn
- `themion-cli` should store that value as the current statusline context-used value
- the TUI should not attempt to reconstruct the last-round value from aggregate counters or transcript text

This follows the existing architecture style used elsewhere in token/accounting behavior.

**Alternative considered:** infer the last-round value in the TUI from cumulative state. Rejected: the core already has the authoritative per-round usage information and should remain the source of truth.

### 5. Handle turns with missing usage data conservatively

The statusline should degrade safely when the last API call input token value is unavailable.

Required behavior:

- if the last API round does not provide usage, the statusline should preserve the existing `ctx` used value unchanged
- the implementation should not invent a synthetic last-round context value from cumulative turn totals when round-level usage is unavailable
- this matches current local statusline behavior more closely than introducing a new unknown marker for the used side
- if a future provider/backend frequently omits usage, that limitation should remain explicit in the runtime model rather than being hidden by a misleading fallback

This avoids reintroducing the same semantic bug under a different fallback path.

**Alternative considered:** always fall back to turn-total `tokens_in` when the last-round value is missing. Rejected: that would reproduce the same incorrect semantics this PRD is fixing.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/agent.rs` | Track the last API call input token value separately from cumulative per-turn token totals and carry it through turn completion reporting. |
| `crates/themion-core/src/` | Extend any shared turn-stats/runtime structures as needed so the last-round input token value is available to the CLI without reconstructing it from totals. |
| `crates/themion-cli/src/tui.rs` | Update statusline state so `ctx` uses the last API call input token value while `in/out/cached` remain cumulative. |
| `docs/engine-runtime.md` | Document that statusline `ctx` now reflects the last API call input token value rather than cumulative turn input usage. |
| `docs/README.md` | Add the new PRD entry and keep the docs index current. |

## Edge Cases

- one user turn contains multiple API rounds because of tool calls → verify: `ctx` shows the last round's input tokens, not the sum across all rounds.
- one user turn contains exactly one API round → verify: `ctx` matches that round's input tokens and behaves the same as before for simple turns.
- the provider omits usage for the last round → verify: the UI leaves the previous `ctx` used value unchanged and does not silently substitute turn-total `tokens_in` as if it were the last-round context value.
- a turn fails before a final provider round completes → verify: the statusline does not report a fabricated new `ctx` value.
- cumulative `in/out/cached` totals continue to grow across turns → verify: they remain unchanged in meaning after the `ctx` fix.

## Migration

This feature requires no database migration.

Rollout guidance:

- keep the visible statusline shape unchanged
- update docs to clarify the corrected `ctx` semantics
- treat the change as a correctness fix for statusline reporting rather than a broader token-accounting redesign

## Testing

- complete a simple turn with one provider round → verify: statusline `ctx` matches that round's input token value and `in/out/cached` remain cumulative.
- complete a tool-using turn with multiple provider rounds → verify: statusline `ctx` matches the last provider round's input token value rather than the turn's aggregated `tokens_in` total.
- compare a tool-using turn's `Turn end [stats: ... in=...]` output with the statusline `ctx` field → verify: `in` may exceed `ctx`, and `ctx` matches the last API round rather than the total turn input.
- complete successive turns and inspect cumulative counters → verify: `in/out/cached` still accumulate across turns independently of the corrected `ctx` value.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check -p themion-core --all-features` after implementation → verify: `themion-core` still builds cleanly across feature combinations.
- run `cargo check -p themion-cli --all-features` after implementation → verify: `themion-cli` still builds cleanly across feature combinations.

## Implementation checklist

- [x] add a distinct last-API-call input token field to the turn/runtime data passed from core to CLI
- [x] preserve cumulative turn and session `in/out/cached` accounting unchanged
- [x] update the TUI statusline state to use the last API call input token value for `ctx`
- [x] update runtime/docs references so `ctx` semantics are documented correctly
- [x] add the PRD entry to `docs/README.md`
