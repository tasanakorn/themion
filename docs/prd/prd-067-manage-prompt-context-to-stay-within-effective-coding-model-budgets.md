# PRD-067: Manage Prompt Context to Stay Within Effective Coding-Model Budgets

- **Status:** Implemented
- **Version:** v0.43.0
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-29

## Implementation status note

- The landed scope for PRD-067 is intentionally narrow.
- `themion-core` now replaces fixed-turn-only replay with a first-pass budget-aware `T0`-first selector in `crates/themion-core/src/agent.rs`.
- The landed behavior estimates prompt-visible history cost with a rough `chars / 4` heuristic, preserves the active turn (`T0`), downgrades `T-1` through `T-5` into pure-message form when `T0` alone exceeds the normal 170K target, and omits prior turns when `T0` alone exceeds the 250K spike ceiling or when adding older turns would push replay above that ceiling.
- Calibration, `CompactSummary`, and a broader multi-step compaction ladder are explicitly out of scope for this PRD revision.

## Summary

- Themion previously relied mainly on a fixed recent-turn replay window, even though actual prompt cost varies widely by turn size.
- This PRD defines a narrower first implementation that keeps prompt replay within a more practical coding-model working range without redesigning history persistence or adding heavier prompt-compaction machinery.
- The runtime should preserve the full current turn first, degrade the recent prior-turn band when `T0` is already large, and omit older turns once the higher spike ceiling would be exceeded.
- The intended outcome is a deterministic, cheap, budget-aware replay policy that protects coding-session continuity better than a fixed turn count alone.

## Goals

- Keep prompt replay within a normal effective working budget of about 170K tokens by default, with a bounded spike ceiling of about 250K tokens.
- Preserve the full current turn's chat-and-tool message sequence in prompt context whenever possible.
- Replace fixed-turn-only replay with budget-aware recent-history selection.
- Degrade `T-1` through `T-5` into pure-message form when `T0` is already consuming most of the practical budget.
- Omit older turns once replay would exceed the 250K ceiling.
- Keep the design compatible with existing persistent history, history tools, and prompt-layer separation.

## Non-goals

- No calibration or estimate-versus-actual feedback loop in this PRD.
- No `CompactSummary` replay form in this PRD.
- No broader compaction ladder for older turns beyond the landed pure-message downgrade and omission behavior.
- No change to durable history retention in SQLite.
- No requirement to introduce provider-specific tokenizer dependencies.
- No redesign of tool contracts, workflow semantics, or the TUI transcript model beyond prompt replay selection.

## Background & Motivation

Themion currently stores full session history durably, but only a bounded slice is replayed into each provider call. A fixed turn-count window is too coarse for current coding workloads because some turns are tiny chat exchanges while others contain large tool payloads.

This PRD narrows the product requirement to a practical first step:

- estimate prompt-visible replay cost cheaply with `chars / 4`
- preserve the active turn (`T0`) first
- degrade the recent prior-turn band (`T-1` through `T-5`) into pure-message form when `T0` alone is already large
- stop including older turns once the assembled replay would exceed the 250K ceiling

This deliberately targets an implementation that is cheap, deterministic, and already compatible with the current architecture.

## Design

### 1. Add an explicit effective prompt budget

Themion should use an effective replay budget distinct from a provider's advertised maximum context window.

Landed target:

- normal effective prompt budget: about `170_000` prompt tokens
- higher allowed spike ceiling: about `250_000` prompt tokens
- prompt-visible text is estimated with a rough `ceil(chars / 4)` heuristic

Required behavior:

- when replay stays below the 170K target, recent prior turns may remain in normal replay form, subject to the 250K ceiling
- when `T0` alone exceeds the 170K target, `T-1` through `T-5` are downgraded into pure-message form
- when `T0` alone exceeds the 250K ceiling, replay keeps `T0` only and omits `T-1` and older turns
- when adding an older turn would push replay above the 250K ceiling, that turn and all older turns are omitted

**Alternative considered:** keep `window_turns` as the primary policy and add only informal prompt-size awareness. Rejected: that still leaves replay behavior driven mainly by turn count instead of prompt cost.

### 2. Apply explicit `T0` and recent-prior-turn replay rules

For this PRD:

- `T0` is the current running turn
- `T-1` is the immediately previous completed turn
- `T-2` through `T-5` are the next recent completed turns after `T-1`
- turns older than `T-5` are lower-priority historical context

Required behavior:

- preserve `T0` as the highest-priority replay unit
- estimate prompt size from prompt-visible text using `ceil(chars / 4)`
- if `T0` alone exceeds the 170K normal target but not the 250K ceiling, include `T-1` through `T-5` only in pure-message form
- if `T0` alone exceeds the 250K ceiling, omit `T-1` through `T-5` and all older turns from replay
- pure-message form must not replay raw `tool_call` or `tool_result` protocol entries; instead it should preserve assistant-style narrative plus simple tool-call summaries such as `tool call: <name>` and `reason: <short reason>` when available

This keeps the active turn intact while making the recent prior-turn band degrade earlier when `T0` is already consuming most of the practical budget.

**Alternative considered:** rewrite or omit `T0` itself first when `T0` grows large. Rejected: the intended policy is to preserve the active turn and degrade older turns first.

### 3. Replace fixed-turn-only replay with budget-aware turn selection

Prompt assembly should no longer rely only on a fixed recent-turn count.

Required replay algorithm shape:

1. assemble non-history prompt layers first
2. assemble full `T0` and estimate its token cost with `ceil(chars / 4)`
3. if `T0` alone exceeds `250K`, replay `T0` only and omit `T-1` and older turns
4. otherwise, walk backward through completed turns from newest to oldest
5. if `T0` exceeds `170K`, represent `T-1` through `T-5` only in pure-message form
6. for each older candidate turn, add it only while the assembled replay remains within the `250K` ceiling
7. when adding a candidate older turn would exceed the ceiling, omit that turn and all older turns
8. emit a history recall hint when older turns were omitted

This policy intentionally allows more than five recent turns to remain in context when they are small enough to fit, and fewer than five when the current turn is unusually large.

### 4. Keep durable history intact and make prompt replay a bounded view

This PRD changes prompt replay policy, not history retention policy.

Required behavior:

- durable session history remains in SQLite
- history tools continue to provide explicit access to older or fuller detail
- prompt assembly may replay a reduced representation instead of the raw stored turn record when needed for budget control
- prompt-budget trimming must not make prior work unrecoverable on demand

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/agent.rs` | Replace fixed-turn-only prompt replay with budget-aware recent-history selection that targets about 170K prompt tokens normally, uses a 250K ceiling, keeps `T0` as the highest-priority replay unit, degrades `T-1` through `T-5` into pure-message form when `T0` exceeds 170K, and omits prior turns when `T0` alone exceeds 250K or when adding older turns would exceed that ceiling. |
| `docs/architecture.md` | Document that prompt replay is now a budget-aware bounded view over durable history rather than only a fixed turn-count window. |
| `docs/engine-runtime.md` | Document the landed narrowed behavior and explicitly note that calibration and richer compaction forms are out of scope for this PRD revision. |
| `docs/README.md` | Keep the PRD index aligned with PRD-067 status, scope, and target version. |

## Edge Cases

- a session has many short interactive turns with little tool use → verify: replay may retain more than five recent turns when the estimated prompt remains within the 250K ceiling.
- the current active turn becomes extremely large → verify: `T0` remains replayed, `T-1` through `T-5` are degraded into pure-message form once `T0` exceeds 170K, and all prior turns are omitted if `T0` exceeds 250K.
- an older turn contains tool calls with `reason` fields → verify: pure-message replay keeps simple assistant-style summaries such as `tool call:` plus `reason:` rather than raw tool protocol payloads.
- older history is omitted for budget reasons → verify: the replay adds a recall hint so the model can still use history tools to fetch prior context on demand.

## Migration

- Existing persisted history remains valid and does not require migration.
- `window_turns` may remain as a compatibility field, but prompt assembly should no longer rely on it as the primary replay controller.
- Legacy sessions should benefit from budget-aware replay immediately when reopened.

## Testing

- run a session with more than five short user/assistant chat turns and little tool traffic → verify: prompt replay can retain more than five recent turns while staying within the 250K ceiling.
- run a large active turn plus several older turns → verify: once `T0` exceeds 170K, `T-1` through `T-5` are replayed only in pure-message form and their tool traffic is converted into assistant-style chat summaries.
- run a pathological active turn where `T0` alone exceeds 250K → verify: `T-1` through `T-5` and all older turns are omitted from replay and `T0` remains the only retained turn context.
- run the relevant crate checks after implementation → verify: touched crates still compile cleanly in default and all-feature configurations.

## Implementation checklist

- [x] create a budget-aware prompt replay path that no longer relies only on a fixed recent-turn count
- [x] define and wire a normal effective prompt budget of about 170K tokens plus a 250K spike ceiling
- [x] add cheap preflight token estimation using the `4 chars ≈ 1 token` heuristic
- [x] apply explicit `T0` / `T-1..T-5` replay rules driven by `T0` size
- [x] degrade `T-1` through `T-5` into pure-message form once `T0` crosses the 170K target
- [x] convert `T-1` through `T-5` tool-call traffic into simple assistant-style `tool call:` / `reason:` summaries in that degraded form
- [x] omit `T-1` through `T-5` and all older turns if `T0` alone exceeds the 250K spike ceiling
- [x] keep durable full-history access intact through persistence and history tools
- [x] update `docs/architecture.md`, `docs/engine-runtime.md`, and `docs/README.md`
