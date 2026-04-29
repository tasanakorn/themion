# PRD-070: Improve `/context` and Prompt-Budget Estimation with `tiktoken-rs`

- **Status:** Draft
- **Version:** >v0.45.0 +minor
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-29

## Summary

- Themion currently estimates prompt size with a rough `chars / 4` heuristic, which was good enough for the first budget-aware replay slice but is now visibly inaccurate in `/context` output and budget reasoning.
- This PRD upgrades token estimation in `themion-core` to use `tiktoken-rs` for local model-aware token counting while keeping the existing architectural boundary: core computes estimates, and the TUI only renders them.
- The new estimator should improve both `/context` reporting and prompt-budget replay decisions without requiring network access or provider calls.
- Tool/function schema cost should remain visible as its own section, but should be counted with the selected tokenizer rather than only by raw JSON character length.
- The result should still be presented as an estimate, not as authoritative provider-billed usage, because backend-side framing and provider-specific accounting can still differ.

## Goals

- Replace the coarse `chars / 4` heuristic in current prompt-budget estimation paths with local model-aware token counting using `tiktoken-rs` where supported.
- Improve the usefulness of `/context` so its token totals and section breakdown are materially closer to real provider-reported input usage.
- Improve prompt-budget replay decisions in `themion-core` by using a better token estimate for prompt sections, current-turn size, prior-turn reduction, and omission thresholds.
- Keep the implementation architecture correct: token estimation logic should live in `themion-core`, while `themion-cli` remains responsible for command intake and rendering.
- Keep the implementation local-only and deterministic, with no network lookup required at runtime.
- Preserve explicit visibility into which parts of the prompt cost the most, including tool definitions as a separate section.

## Non-goals

- No claim that local estimation becomes exact provider-billed token usage.
- No requirement to support every provider's hidden framing, proprietary serialization, or future multimodal accounting perfectly.
- No migration of token-estimation logic into the TUI.
- No redesign of `/context` output shape beyond what is needed to explain improved estimate semantics.
- No requirement to add provider-round calibration or automatic bias correction in this PRD.
- No requirement to add new remote inspection or model-invoked tool surfaces for this feature.

## Background & Motivation

### Current state

PRD-067 intentionally started with a cheap `chars / 4` estimate for prompt-budget replay. PRD-069 then exposed that logic through `/context`, first for prompt/history visibility and later with tool-definition cost included. That output is already useful for understanding where prompt budget goes, but real usage comparisons now show that the heuristic is too coarse in both directions:

- it previously undercounted because tool definitions were not included
- after adding tool-definition text cost, it can overcount because raw serialized JSON length is still only a rough proxy for tokenizer behavior

Recent observed behavior shows the problem clearly:

- `/context` rough estimate after a tiny turn: about `7,276` tokens
- provider-reported actual prompt input: about `4,806` tokens

The important product insight is not only that the estimate is off. It is that Themion already has user-facing and runtime-visible features whose behavior depends on prompt token estimates:

- PRD-067 budget-aware replay in `themion-core`
- `/context` section and turn replay reporting in PRD-069

Once the feature is user-visible, a rough heuristic is no longer only an internal engineering shortcut. It becomes a product accuracy issue.

### Research note: `tiktoken-rs`

Focused external research via Codex CLI indicates:

- `tiktoken-rs` is a local Rust tokenizer crate for OpenAI-style BPE encodings
- it supports explicit encodings such as `o200k_base`, `o200k_harmony`, `cl100k_base`, `p50k_base`, `p50k_edit`, and `r50k_base`
- it can select tokenizers by model name via helpers such as `get_bpe_from_model(...)`
- it provides plain-text encoding and chat-style message counting helpers such as `num_tokens_from_messages(...)`
- it does not appear to require runtime network access for ordinary tokenizer use
- it still cannot fully replicate provider-side billing/accounting for hidden framing or all tool/function schema behavior, so results must remain labeled as estimates

That makes it a strong fit for Themion's needs: significantly better local estimates without requiring a provider round-trip.

## Design

### 1. Add a tokenizer-backed estimation layer in `themion-core`

Themion should introduce a shared token-estimation helper in `themion-core` that prefers `tiktoken-rs` for supported models and falls back gracefully when needed.

Required behavior:

- model-aware token estimation lives in `themion-core`
- estimation should select the tokenizer by active model name first when possible
- plain text and serialized JSON payloads should be tokenized through the selected tokenizer rather than only by character length
- when no supported tokenizer mapping is available, the runtime may fall back to the existing rough heuristic rather than failing the feature
- the report/output should make clear when a fallback estimate is used

This keeps prompt-budget reasoning close to the real model family while preserving robustness for unsupported or future model names.

**Alternative considered:** keep `chars / 4` for replay decisions and use `tiktoken-rs` only for `/context`. Rejected: that would reintroduce drift between the user-visible report and the real replay policy.

### 2. Use the same improved estimate path for both replay decisions and `/context`

The tokenizer-backed estimate should feed the same shared core prompt-analysis path used by live model calls and `/context`.

Required behavior:

- PRD-067 replay thresholds should continue to apply in token space, but token counts should come from the improved estimator when supported
- `/context` should use the same section totals, turn totals, and replay-mode decisions the next real provider round would use
- `themion-cli` should continue to receive only structured report data and render it, without owning tokenization logic

This preserves the architectural intent of PRD-069 while improving estimate quality.

**Alternative considered:** perform a separate `tiktoken-rs` pass only for display after replay decisions are already made. Rejected: that would make `/context` more accurate-looking while leaving the actual replay policy on a rougher estimator.

### 3. Count tool definitions explicitly, but with tokenizer-backed accounting

Tool definitions should remain a separate visible section in `/context`, because they are often a dominant cost driver.

Required behavior:

- tool-definition size should still appear as its own section in `/context`
- its token count should be based on tokenizer-backed counting of the serialized schema payload, not only raw chars
- if the estimator falls back for the active model, the tools section may also fall back, but the report should remain explicit that the result is approximate

This preserves the user-visible budgeting insight discovered during the PRD-069 implementation while improving numeric fidelity.

**Alternative considered:** hide tool definitions inside a single total. Rejected: users already learned that tool schemas are often the largest bucket, so removing that visibility would make `/context` less useful.

### 4. Represent estimate quality explicitly in the report

`/context` should remain honest about what kind of estimate it is showing.

Required behavior:

- the core report should carry enough metadata to distinguish tokenizer-backed estimates from rough fallback estimates
- `/context` should be able to render concise wording such as `estimated with tokenizer` or `fallback rough estimate` when useful
- the total should remain described as an estimate, not as exact provider-billed usage

The product goal is better trustworthiness, not false precision.

**Alternative considered:** silently switch implementations and keep the same wording. Rejected: the feature becomes easier to misread if users are not told whether the current model path is tokenizer-backed or fallback-only.

### 5. Keep runtime cost bounded and local

The tokenizer improvement must not introduce hidden online dependencies or excessive repeated setup cost.

Required behavior:

- tokenizer use should remain local-only with no runtime network access
- reusable/singleton tokenizer instances should be preferred where practical rather than rebuilding tokenizers on every estimation call
- the implementation should not noticeably degrade interactive `/context` responsiveness or prompt replay setup time

This matters because prompt estimation may run on every provider round as well as on user-triggered inspection.

**Alternative considered:** initialize tokenizer state ad hoc for every section or every message. Rejected: that adds avoidable overhead to a hot path.

### 6. Keep unsupported-model behavior safe and understandable

Themion supports multiple providers and model names, so tokenizer availability may not always be perfect.

Required behavior:

- unsupported or unknown model names must not break prompt replay or `/context`
- when tokenizer mapping is unavailable, the runtime should fall back to the rough estimate path
- user-facing output should remain available, and should not pretend the fallback is tokenizer-accurate
- implementation should prefer model-name mapping first, but allow explicit encoding fallback where the active provider/model family is known well enough by configuration or code path

This keeps the feature broadly usable without turning tokenizer support gaps into hard runtime failures.

**Alternative considered:** support only a narrow allowlist of exact model names and fail otherwise. Rejected: that would make the feature brittle across providers and model upgrades.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/Cargo.toml` | Add `tiktoken-rs` as a new dependency if the final implementation confirms the crate is the chosen tokenizer path. |
| `crates/themion-core/src/` | Add a shared tokenizer/estimation helper layer that maps active models to tokenizers, counts prompt text and serialized tool-schema payloads, and falls back cleanly when unsupported. |
| `crates/themion-core/src/agent.rs` | Replace the current rough estimate usage in prompt replay and prompt-context reporting with the shared tokenizer-backed estimate path where supported. |
| `crates/themion-core/src/context_report.rs` | Extend report metadata so `/context` can show whether counts came from tokenizer-backed estimation or fallback rough estimation. |
| `crates/themion-cli/src/tui.rs` | Render the improved estimate metadata without taking ownership of tokenization logic. |
| `docs/architecture.md` | Document that `/context` and prompt-budget replay now use tokenizer-backed local estimation when supported by the active model. |
| `docs/engine-runtime.md` | Document the estimation path, fallback semantics, and the fact that results remain estimates rather than exact provider accounting. |
| `docs/README.md` | Add the new PRD entry and keep status/version alignment current. |

## Edge Cases

- active model name is not recognized by `tiktoken-rs` → verify: replay and `/context` still work with fallback estimation and make that fallback visible.
- provider uses a model alias that maps imperfectly to tokenizer expectations → verify: the chosen mapping path is deterministic and clearly documented.
- tool schema JSON is large but message history is small → verify: `/context` still shows tool definitions as a dominant explicit cost bucket with tokenizer-backed counting.
- `T0` sits near the 170K or 250K threshold → verify: improved token counting can change replay-mode decisions predictably without breaking the PRD-067 policy shape.
- the active session switches profiles/models → verify: the estimator refreshes to the new model mapping rather than keeping stale tokenizer state.
- tokenizer initialization or lookup fails unexpectedly → verify: the runtime degrades to fallback estimation rather than failing the round or the `/context` command.

## Migration

This feature does not require database migration.

Migration/rollout guidance:

- preserve the existing `/context` output shape as much as practical so the command remains familiar
- switch the underlying estimate source in `themion-core` first, not in the TUI
- keep fallback rough estimation available for unsupported models
- update docs so users understand that accuracy is improved but still approximate

## Testing

- run `/context` on a supported OpenAI-style model using `tiktoken-rs` mapping → verify: the report renders normally and marks the estimate as tokenizer-backed.
- compare `/context` totals before and after the tokenizer-backed implementation on the same short session → verify: the new estimate is materially closer to provider-reported `in=` usage than the old rough heuristic.
- run a session where tool definitions dominate the prompt → verify: the tools section remains explicit and its tokenizer-backed total contributes to the overall estimate.
- run a session near replay thresholds → verify: replay mode decisions still follow the PRD-067 shape, but with improved token counts.
- switch to a model name unsupported by `tiktoken-rs` mapping → verify: `/context` and replay still work using fallback estimation, and the report does not falsely claim tokenizer-backed accuracy.
- switch profiles/models within one TUI session → verify: the estimator updates to the new active model mapping.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check -p themion-core --all-features` after implementation → verify: `themion-core` still builds cleanly across feature combinations.
- run `cargo check -p themion-cli --all-features` after implementation → verify: `themion-cli` still builds cleanly across feature combinations.

## Implementation checklist

- [ ] confirm `tiktoken-rs` is the chosen dependency and add it to `themion-core`
- [ ] add a shared tokenizer-backed estimate helper in `themion-core`
- [ ] support model-name-based tokenizer selection with graceful fallback
- [ ] count prompt sections and tool-definition schema text with the shared tokenizer when supported
- [ ] wire the improved estimator into both prompt replay and `/context`
- [ ] expose estimate-quality metadata so the TUI can distinguish tokenizer-backed from fallback estimates
- [ ] update runtime and architecture docs plus the PRD index

## Technical note: focused `tiktoken-rs` research summary

External research via Codex CLI found:

- `tiktoken-rs` supports OpenAI-style encodings including `o200k_base`, `o200k_harmony`, `cl100k_base`, `p50k_base`, `p50k_edit`, and `r50k_base`
- model-name-based selection is available through helpers such as `get_bpe_from_model(...)`
- plain-text counting is straightforward via a `CoreBPE` encode call
- chat-style counting helpers such as `num_tokens_from_messages(...)` exist, but request-side tool/function schema definition counting still appears to require explicit serialization plus tokenization of that text
- the crate appears local-only for ordinary tokenizer use and does not appear to require runtime network access
- the crate's own message-counting docs and OpenAI cookbook guidance both still treat chat token counts as estimates rather than exact billed usage

That research supports using `tiktoken-rs` as an improved local estimator, not as an authoritative accounting source.
