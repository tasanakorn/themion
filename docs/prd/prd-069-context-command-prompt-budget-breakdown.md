# PRD-069: Add a `/context` Command for Prompt-Budget Breakdown and History-Replay Visibility

- **Status:** Implemented
- **Version:** v0.45.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-29

## Implementation status

Landed in `v0.45.0` as the first complete `/context` inspection feature. The shipped behavior adds a local `/context` TUI command, keeps prompt-budget reporting logic in shared `themion-core` structures, renders a human-readable section and turn replay breakdown in the transcript, and now includes tool-definition cost in the prompt estimate. Follow-on estimator-accuracy work such as tokenizer-backed counting is tracked separately in PRD-070 rather than being treated as unfinished PRD-069 scope.

## Summary

- Themion currently manages prompt context internally, but the user cannot easily see how much of the prompt budget is consumed by system layers, recent history, or the current turn.
- This PRD adds a `/context` command whose real inspection logic belongs with the core prompt-assembly path, while the TUI stays responsible mainly for command intake and rendering the returned report.
- The command should show character counts, rough token estimates using the same `4 chars ≈ 1 token` heuristic already used by prompt replay, and a per-section size breakdown for prompt-visible layers.
- The output should make history behavior obvious: how many turns exist, how many are currently replayed, which turns are full vs reduced, and where omission begins.
- The first version is a local inspection/reporting feature only; it does not change the replay algorithm or add a new compaction policy.

## Goals

- Give users a direct `/context` command in the TUI that explains what prompt-visible context Themion would send on the next model round.
- Reuse the same prompt-construction and history-replay logic used for actual model calls so the report stays aligned with real runtime behavior.
- Keep the architectural ownership correct: prompt analysis and report data assembly should live in `themion-core`, while `themion-cli` should mostly parse the slash command, request the report, and render it.
- Show per-section size information in both characters and rough token estimates based on the existing `chars / 4` heuristic.
- Make recent-history inclusion understandable in turn-oriented terms, including total turn count, the `T0` / prior-turn band, and where omission starts.
- Make it clear which history turns are replayed in full, which are replayed in reduced pure-message form, and which are omitted entirely.
- Keep the feature local and read-only, with no provider call and no mutation of conversation state.

## Non-goals

- No new prompt-budget algorithm in this PRD; `/context` reports existing behavior rather than redefining it.
- No provider-specific tokenizer integration or exact token counting.
- No migration of prompt-analysis ownership into the TUI.
- No transcript redesign beyond the new slash-command output.
- No automatic warnings, blocking behavior, or proactive compaction changes triggered by viewing `/context`.
- No attempt to reconstruct exact server-side tokenization differences across providers.
- No requirement to expose every persisted historical message; the focus is the prompt-visible replay decision.

## Background & Motivation

### Current state

PRD-067 already changed Themion from a fixed-turn-only replay window to a budget-aware history selector in `themion-core`. The current runtime:

- estimates prompt-visible message size with a rough `chars / 4` heuristic
- preserves the active turn (`T0`) first
- downgrades `T-1` through `T-5` into pure-message replay when `T0` alone exceeds the normal effective budget
- omits older turns when replay would exceed the spike ceiling
- emits a recall hint when earlier turns are omitted

Today that behavior is mostly invisible to the user. The TUI has `/debug runtime` and `/debug api-log`, but no command that explains the current prompt budget composition or the history replay result in a human-readable way.

The current codebase also already draws a strong architecture boundary:

- `themion-core` owns prompt assembly, replay policy, tool handling, and model/backend integration
- `themion-cli` owns TUI input handling, local command dispatch, and terminal rendering

That means the product gap is not only “add a `/context` command.” It is “add a `/context` command without collapsing core prompt logic into TUI-local code.”

That gap matters because the current product behavior is intentionally budget-aware and lossy at the prompt layer while durable history remains intact in SQLite. Users need a fast way to answer questions such as:

- how large is the current active turn?
- how much prompt budget is consumed by system/context layers before history is added?
- how many turns exist in session history?
- which turns are still fully present in prompt replay?
- where did Themion start reducing tool-heavy turns or omitting older turns?

Without that visibility, context-window behavior is hard to trust, hard to debug, and hard to explain when the assistant appears to have forgotten something that is still present in durable history.

## Design

### 1. Add `/context` as a local TUI inspection command, but keep the command thin

Themion should add a new slash command, `/context`, alongside the existing local commands such as `/debug runtime` and `/debug api-log`.

Required behavior:

- `/context` runs locally in the TUI and returns immediate human-readable lines without calling the model
- the command is available whenever the interactive TUI session is available, even if the session is currently idle
- if the agent is busy, the command may still inspect the latest stable local state, but it must not interfere with the in-flight provider round
- the TUI command handler should remain thin: it should not independently reconstruct prompt layers or replay policy
- the command output should be concise enough to scan in the transcript, but detailed enough to show which prompt sections are large

The command is a local diagnostic/inspection path, not a provider-visible prompt artifact.

**Alternative considered:** put all `/context` computation directly in `tui.rs` because slash commands are entered there. Rejected: the TUI is the right place for command intake, but not for owning prompt-analysis logic that must stay aligned with real model-call construction.

### 2. Put reusable prompt-analysis/report construction in `themion-core`

The `/context` command should be backed by the same context-building logic used for actual provider calls in `themion-core`.

Required behavior:

- the same prompt-layer ordering used in real model calls should be reflected in `/context`
- the same history replay selection logic should be reused, including `T0` priority, pure-message downgrade rules, omission rules, and recall-hint inclusion
- the same size-estimation logic should be reused, including the current `chars / 4` token estimate heuristic
- implementation should prefer extracting or sharing a reusable prompt-analysis/report structure from `themion-core` rather than reimplementing parallel logic in `themion-cli`
- the shared core path should be usable both by the real model-call flow and by the `/context` inspection flow so drift is minimized by construction

The product requirement is not merely “show some approximate context stats.” It is “show the same context assembly decision Themion will actually use.”

**Alternative considered:** build a separate TUI-only estimator over transcript text. Rejected: a duplicate estimator would drift from real prompt assembly and would be least trustworthy exactly when users need it most.

### 3. Make the core return structured context-report data and let the TUI render it

The first implementation should preserve the existing crate boundary by separating computation from presentation.

Required behavior:

- `themion-core` should compute a structured context report describing prompt sections, replayed turns, replay modes, omission boundaries, and aggregate estimated size
- `themion-cli` should format that structured report into transcript-friendly lines for `/context`
- any future non-TUI consumer should be able to reuse the same core report data without scraping human-formatted text out of the TUI
- the TUI should remain free to make small presentation decisions such as labels, indentation, and line ordering, but not replay-policy decisions

This keeps the TUI focused on local command intake and rendering, while `themion-core` remains the source of truth for prompt behavior.

**Alternative considered:** have `themion-core` return only already-rendered lines. Rejected: that would reduce reuse and would mix presentation concerns into the core more than necessary.

### 4. Report prompt layers and their sizes section by section

`/context` should show a clear section breakdown for the prompt-visible content that would be sent on the next round.

Minimum first-version sections:

- base system prompt
- predefined coding guardrails
- predefined Codex CLI web-search instruction
- injected contextual instructions such as `AGENTS.md`, when present
- workflow context and phase instructions
- history recall hint, when present
- replayed conversation history, broken down further by turn

Per section, report:

- character count
- rough token estimate using `ceil(chars / 4)` or equivalent current helper behavior
- whether the section is always included, conditionally included, or omitted in the current snapshot

A concise summary line should also show overall estimated prompt size for the next round.

Example output shape for the first version:

- `prompt estimate: 38,420 chars ≈ 9,605 tokens`
- `system prompt: 5,200 chars ≈ 1,300 tokens`
- `coding guardrails: 8,100 chars ≈ 2,025 tokens`
- `AGENTS.md: 3,400 chars ≈ 850 tokens`
- `workflow context: 900 chars ≈ 225 tokens`
- `history replay: 20,820 chars ≈ 5,205 tokens`

Exact wording may differ, but the output should make the largest sections immediately visible.

**Alternative considered:** only show one total token estimate. Rejected: the user needs to understand which layers are driving prompt size, not only the final aggregate.

### 5. Make turn-level history replay status explicit

The history portion of `/context` should be turn-oriented rather than only message-oriented.

Required behavior:

- show total completed or known turns in the current session context set
- identify the active turn as `T0`
- show how many prior turns are included in the current replay view
- show, for each included prior turn, whether it is replayed as `full` or `reduced`
- show where omission begins, for example `omitted: T-6 and older` or `omitted turns: 12`
- when a recall hint is present because older turns were omitted, say so explicitly in the report

For each replayed turn, the first version should report at least:

- turn label such as `T0`, `T-1`, `T-2`
- replay mode: `full` or `reduced`
- character count and estimated tokens for that turn's replayed form
- a lightweight note when a reduced turn excludes raw tool payloads and uses pure-message replay

This should make the compaction ladder visible in product terms rather than requiring the user to infer it from raw message lists.

**Alternative considered:** only show “N turns included.” Rejected: that hides the most important distinction introduced by PRD-067, which is not only count but replay form and omission boundary.

### 6. Clearly explain the reduced-form boundary and omission boundary

The first version of `/context` should explicitly call out the moment where recent prior turns stop being full-fidelity replay.

Required behavior:

- when `T0` alone exceeds the normal budget threshold and recent prior turns are replayed in pure-message form, the report should state that condition plainly
- when older turns are omitted because the spike ceiling would be exceeded, the report should state that omission boundary plainly
- when `T0` alone exceeds the spike ceiling and no older turns are replayed, the report should say that `T0` is the only retained prompt-visible turn

Example phrases that are acceptable in principle:

- `recent prior turns reduced because T0 exceeds normal budget`
- `omitting T-6 and older to stay within spike ceiling`
- `T0 alone exceeds spike ceiling; prior turns not replayed`

This wording is important because the user asked not only for sizes but for clarity about when history is full, reduced, or omitted.

**Alternative considered:** expose only numbers and let users infer the thresholds. Rejected: the product requirement includes understandable replay-state explanations, not only raw measurements.

### 7. Keep the output human-facing, compact, and TUI-friendly

The output should fit Themion's transcript-oriented TUI rather than dumping verbose raw JSON.

Required behavior:

- default `/context` output should be line-oriented human-readable text in the transcript
- ordering should prioritize the overall estimate first, then section breakdown, then turn-by-turn history details
- line labels should be stable enough that users can compare repeated `/context` snapshots across a session
- if detailed machine-readable introspection is useful later, it may be added through another path, but that is not required for this PRD

The first version should optimize for “I ran `/context` and immediately understood why my prompt is large and how much history is still replayed.”

**Alternative considered:** return a pretty-printed JSON blob in the transcript. Rejected: the main audience is a human TUI user trying to inspect prompt state quickly.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/agent.rs` | Extract or expose reusable prompt-analysis logic from the existing model-call context builder so `/context` can report the same replay decision, section ordering, and `chars / 4` estimates used by real calls. |
| `crates/themion-core/src/` | Add small shared report types/helpers for prompt sections, turn replay entries, omission boundaries, and aggregated estimates so prompt analysis stays in core and presentation stays outside core. |
| `crates/themion-cli/src/tui.rs` | Add thin `/context` slash-command handling that requests the structured context report, formats it into transcript-friendly lines, and includes the command in local help/unknown-command guidance. |
| `docs/architecture.md` | Document the existence of `/context` as the user-facing local inspection command for prompt-budget composition and history replay visibility, and note that prompt-analysis ownership remains in `themion-core`. |
| `docs/engine-runtime.md` | Document that `/context` mirrors the live prompt-construction logic and reports the current section and turn replay breakdown using the shared heuristic estimate. |
| `docs/README.md` | Add the new PRD entry and keep the docs index aligned. |

## Edge Cases

- no completed turns yet beyond the current message set → verify: `/context` still reports prompt layers and shows `T0` or an empty-history explanation without crashing.
- no `AGENTS.md` is present → verify: the contextual-instructions section is reported as absent or omitted cleanly.
- older turns are omitted and a recall hint would be added → verify: `/context` shows that omission count and notes that a recall hint is part of the prompt.
- `T0` is small and many short turns fit → verify: the report can show more than five prior turns when the current replay logic includes them.
- `T0` is large enough to force reduced prior-turn replay → verify: included `T-1` through `T-5` are labeled `reduced` with sizes based on their reduced pure-message form rather than their stored raw message form.
- `T0` alone exceeds the spike ceiling → verify: `/context` reports that only `T0` is replayed and that all prior turns are omitted.
- an agent round is currently in flight → verify: the command reports a stable local snapshot without mutating or corrupting the in-progress turn.
- future non-TUI consumers need the same information → verify: the core report data can be reused without depending on TUI-formatted strings.

## Migration

This is a local inspection and documentation feature only.

- no database migration is required
- no provider contract changes are required
- no persisted history format changes are required
- no TUI-to-core ownership inversion is required; the change should reinforce the existing architecture boundary

## Testing

- start a short session and run `/context` → verify: output includes total estimate plus a section-by-section breakdown for the prompt layers that are currently present.
- run `/context` in a session with `AGENTS.md` instructions loaded → verify: the contextual-instructions section appears with non-zero size and is counted into the total.
- create a session where many short turns fit in replay → verify: `/context` shows more than five prior turns when the real prompt replay would include them.
- create a session where `T0` exceeds the normal budget target but not the spike ceiling → verify: `/context` labels recent prior turns as `reduced` and explains why that downgrade happened.
- create a session where older turns are omitted by the spike ceiling → verify: `/context` reports the omission boundary and the presence of the recall hint.
- create a pathological session where `T0` alone exceeds the spike ceiling → verify: `/context` reports `T0` only and shows that prior turns are not replayed.
- compare `/context` output with the real prompt-assembly path used for the next provider round in a controlled test → verify: section ordering, inclusion decisions, and rough size estimates come from the same shared logic rather than diverging duplicate code.
- inspect the implementation boundary after landing → verify: prompt-analysis and report-data assembly live in `themion-core`, while `themion-cli` remains limited to command dispatch and rendering.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check -p themion-core --all-features` after implementation → verify: `themion-core` still builds cleanly across feature combinations.
- run `cargo check -p themion-cli --all-features` after implementation → verify: `themion-cli` still builds cleanly across feature combinations.

## Implementation checklist

- [x] add a shared prompt-analysis/report path in `themion-core` that exposes the same section ordering, replay decision, and `chars / 4` estimates used by real provider calls
- [x] make the real model-call path and `/context` inspection path consume the same core prompt-analysis decision where practical
- [x] represent prompt sections and turn replay entries with enough structure for TUI formatting without copying replay logic into `themion-cli`
- [x] add thin `/context` slash-command handling in `themion-cli`
- [x] format human-readable TUI output for overall estimate, section sizes, and turn replay status
- [x] include explicit explanation of reduced-turn and omitted-turn boundaries
- [x] update command help / unknown-command guidance to mention `/context`
- [x] update `docs/architecture.md`, `docs/engine-runtime.md`, and `docs/README.md`
