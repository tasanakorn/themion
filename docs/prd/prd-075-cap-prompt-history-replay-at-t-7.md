# PRD-075: Cap Prompt History Replay at `T-7`

- **Status:** Implemented
- **Version:** v0.48.2
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-30

## Implementation status

Landed in `v0.48.2` as a focused prompt-replay policy refinement. The shipped behavior preserves the current budget-aware replay logic inside the eligible recent-turn band, adds a hard outer boundary so prompt replay never includes turns older than `T-7`, makes `/context` explicitly report when older turns were omitted by that cap, and limits the `history turns:` diagnostic listing so it never shows entries older than `T-10`.

## Summary

- Themion's current budget-aware replay policy can still include turns older than `T-7` when those turns are small enough to fit.
- This PRD adds a hard recency cap so prompt replay never includes turns older than `T-7`, even if token budget remains available.
- The change keeps the existing budget-aware replay behavior inside the allowed band, but stops replay at `T0` through `T-7` as an absolute outer limit.
- `/context` should make this boundary visible so users can see when `T-8` and older turns were omitted because of the hard `T-7` replay cap rather than token pressure alone.
- Durable history retention and explicit history tools stay unchanged.

## Goals

- Ensure prompt replay never includes turns older than `T-7`.
- Preserve the current budget-aware replay and compaction behavior for `T0` through `T-7` where practical.
- Keep replay behavior easy to reason about by combining a hard recency boundary with the existing token-budget logic.
- Make the omission boundary visible in `/context` and related replay diagnostics.
- Keep durable session history fully available through persistence and history tools even when prompt replay is capped.

## Non-goals

- No change to durable history retention in SQLite.
- No redesign of history tools such as `history_recall` or `history_search`.
- No change in this PRD to the current normal/spike token budgets themselves unless needed only to preserve the existing logic inside the `T-7` cap.
- No introduction of summaries or new compaction forms beyond the current replay forms already in use.
- No requirement to make the cap user-configurable in this PRD.

## Background & Motivation

### Current state

PRD-067 intentionally replaced a fixed-turn-only replay window with budget-aware selection. That policy preserves `T0` first, applies compaction rules to recent turns when needed, and can still include turns older than `T-5` when the assembled replay remains within budget.

That behavior is visible today in `/context`. In the observed example motivating this PRD, Themion reports:

- `turns: total=10 replayed=10 reduced=0 omitted=0`
- replayed history includes `T0` through `T-9`

Even though that session is small enough to fit comfortably in token budget, it violates the desired product rule that replay should not reach beyond `T-7`.

The problem is not primarily token pressure. It is predictability and recency control. A hard replay boundary is easier to reason about than a purely size-driven policy when the product intent is “never older than this.”

## Design

### 1. Add a hard outer replay boundary at `T-7`

Prompt replay should stop at `T-7` regardless of remaining budget.

Required behavior:

- `T0` remains the active turn
- the oldest turn eligible for replay is `T-7`
- `T-8` and older turns must never be included in prompt replay
- this cap applies before or alongside budget checks so small older turns do not bypass it

This makes replay recency deterministic instead of purely budget-elastic.

**Alternative considered:** keep the current budget-only policy and rely on token budgets to limit replay depth naturally. Rejected: the observed `/context` output shows that small older turns can still extend replay deeper than the desired recency boundary.

### 2. Keep existing budget-aware behavior inside the allowed replay band

The hard cap should narrow the candidate set, not replace the existing replay logic for the turns that remain eligible.

Required behavior:

- current replay rules for `T0` and recent prior turns should continue to apply within the bounded set `T0` through `T-7`
- if token pressure requires compaction or omission inside that band, current budget-aware behavior may still do so
- if token budget would otherwise allow replay of older turns, that extra budget should not be used to include `T-8` and older turns

This keeps the implementation small and preserves the value of the current budget-aware work.

**Alternative considered:** replace the whole replay policy with a strict fixed eight-turn replay window and remove token-aware omission/compaction behavior. Rejected: budget-aware behavior is still useful inside the allowed recency band.

### 3. Make `/context` report the hard replay cap clearly

User-facing replay inspection should distinguish between omission due to token pressure and omission due to the hard recency cap.

Required behavior:

- `/context` should not show any replayed turn older than `T-7`
- when older turns exist, the report should make it clear that `T-8` and older were omitted because of the hard replay cap, using the note text `omitted by T-7 replay cap` in per-turn entries
- the `turns:` summary and per-turn history section should remain honest about which turns were replayed versus omitted
- the visible `history turns:` listing should not show entries older than `T-10`, even when the structured report internally knows about older omitted turns
- if both budget pressure and the hard replay cap matter in the same session, the report should remain understandable rather than collapsing both reasons into one vague omission state

This keeps replay diagnostics trustworthy after the policy changes.

**Alternative considered:** enforce the cap silently without updating `/context` wording. Rejected: users need to understand why older turns disappeared even when token budget is still low.

### 4. Keep durable history access unchanged

This PRD changes prompt replay only, not stored history availability.

Required behavior:

- all turns remain persisted as they are today
- `history_recall` and `history_search` remain the path for reaching older turns on demand
- the omission of `T-8` and older from prompt replay must not imply those turns are lost or discarded

This preserves the current recovery model for long sessions while tightening default replay behavior.

**Alternative considered:** trim or delete old stored turns to match the replay cap. Rejected: prompt replay policy and durable history retention serve different product needs.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/agent.rs` | Add a hard replay-depth boundary so prompt assembly never includes turns older than `T-7`, while preserving current budget-aware compaction and omission logic inside that bounded candidate set. |
| `crates/themion-core/src/context_report.rs` | Reflect the hard replay cap in the structured `/context` report so older-turn omission can be explained accurately. |
| `crates/themion-cli/src/tui.rs` | Render updated `/context` replay-cap messaging and limit the visible `history turns:` listing to `T0` through `T-10` without changing the command's overall shape unnecessarily. |
| `docs/engine-runtime.md` | Document that budget-aware replay is now additionally bounded by a hard outer recency cap at `T-7`. |
| `docs/architecture.md` | Document the replay policy at a high level as budget-aware within a strict `T-7` outer boundary. |
| `docs/README.md` | Add the new PRD entry to the PRD table. |

## Edge Cases

- a session has many tiny turns and low token usage → verify: replay still stops at `T-7` and does not include `T-8` or older despite ample remaining budget.
- a session has fewer than eight total visible turns → verify: replay behavior remains unchanged except that the cap is trivially satisfied.
- token pressure would already omit some turns newer than `T-7` → verify: the hard cap does not interfere with existing budget-aware omission inside the allowed band.
- `T0` alone is very large → verify: existing budget-aware behavior for compaction/omission still applies, but no older-than-`T-7` turns are ever considered.
- `/context` is run in a long session where both the cap and budget matter → verify: the report remains clear about replayed turns and omitted turns.
- `/context` is run in a session longer than eleven visible turn labels → verify: `history turns:` does not print entries older than `T-10`.

## Migration

This feature requires no database migration.

Rollout guidance:

- keep durable history unchanged
- treat the cap as a prompt-replay policy refinement rather than a history-retention change
- update replay diagnostics/docs in the same landing so the new boundary is visible and understandable

## Testing

- run `/context` in a session with at least ten short turns and low token pressure → verify: replay includes at most `T0` through `T-7`, and `T-8` plus older turns are omitted.
- run `/context` in a short session with fewer than eight turns → verify: replay output remains unchanged apart from honoring the cap implicitly.
- run a session where token pressure already reduces replay within the recent band → verify: the existing budget-aware reduction still works inside the `T-7` boundary.
- inspect `/context` omission messaging in a long session → verify: it clearly indicates the hard replay cap when older turns exist.
- run `/context` in a session long enough to produce omitted entries beyond `T-10` → verify: the summary remains accurate, but the visible `history turns:` listing stops at `T-10`.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check -p themion-core --all-features` after implementation → verify: `themion-core` still builds cleanly across feature combinations.
- run `cargo check -p themion-cli --all-features` after implementation → verify: `themion-cli` still builds cleanly across feature combinations.

## Implementation checklist

- [x] cap replay candidate selection at `T-7`
- [x] preserve the current budget-aware replay behavior inside the capped candidate set
- [x] update structured `/context` reporting so the hard replay cap is visible
- [x] update runtime/docs references to describe the new outer replay boundary
- [x] add the PRD entry to `docs/README.md`
