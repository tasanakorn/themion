# PRD-070: Improve PRD-067 Replay Budgeting and PRD-069 `/context` Estimation with `tiktoken-rs`

- **Status:** Implemented
- **Version:** v0.46.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-29

## Implementation status

Landed in `v0.46.0` as the implemented tokenizer-backed estimation feature for PRD-067 and PRD-069. The shipped behavior adds local `tiktoken-rs`-backed estimation in `themion-core`, applies the explicit tokenizer-selection order of exact match, trusted fallback mapping, then rough fallback, and updates `/context` to show estimate mode plus tokenizer path when available. Later tuning such as broader mapping coverage or additional calibration may still happen as follow-up work, but it is no longer treated as unfinished PRD-070 scope.

## Summary

- PRD-067 introduced budget-aware prompt replay in `themion-core`, and PRD-069 exposed that budgeting behavior to users through `/context`.
- Both landed features currently rely on a rough `chars / 4` heuristic, which was good enough for the first slice but is now visibly inaccurate for both real replay-budget decisions and `/context` totals.
- This PRD upgrades token estimation in `themion-core` to use `tiktoken-rs` for local model-aware token counting while keeping the existing architectural boundary: core computes estimates, and the TUI only renders them.
- The agreed tokenizer-selection policy is strict and low-surprise: exact model-name match first, explicit trusted fallback mapping second, rough estimate fallback last.
- PRD-069 `/context` should also show which estimate mode and tokenizer path were used, so users can tell whether the estimate came from an exact tokenizer match, a trusted fallback mapping, or the rough fallback path.
- The new estimator should improve both PRD-067 replay budgeting and PRD-069 `/context` reporting without requiring network access or provider calls.
- Tool/function schema cost should remain visible as its own section, but should be counted with the selected tokenizer rather than only by raw JSON character length.
- The result should still be presented as an estimate, not as authoritative provider-billed usage, because backend-side framing and provider-specific accounting can still differ.

## Goals

- Improve PRD-067 budget-aware replay decisions by replacing the coarse `chars / 4` heuristic with local model-aware token counting using `tiktoken-rs` where supported.
- Improve PRD-069 `/context` output so its token totals, section breakdown, and replay-state explanations are materially closer to real provider-reported input usage.
- Keep the PRD-067 and PRD-069 behaviors aligned by using the same improved estimate path for both replay decisions and `/context` reporting.
- Use a tokenizer-selection policy that is explicit and auditable: exact match first, explicit trusted fallback map second, rough estimate last.
- Make PRD-069 `/context` explicitly show the estimate mode and tokenizer path used for the current report.
- Keep the implementation architecture correct: token estimation logic should live in `themion-core`, while `themion-cli` remains responsible for command intake and rendering.
- Keep the implementation local-only and deterministic, with no network lookup required at runtime.
- Preserve explicit visibility into which parts of the prompt cost the most, including tool definitions as a separate section.

## Non-goals

- No claim that local estimation becomes exact provider-billed token usage.
- No requirement to support every provider's hidden framing, proprietary serialization, or future multimodal accounting perfectly.
- No migration of token-estimation logic into the TUI.
- No redesign of `/context` output shape beyond what is needed to explain improved estimate semantics and tokenizer-path visibility.
- No requirement to add provider-round calibration or automatic bias correction in this PRD.
- No requirement to replace PRD-067 replay-policy shape or PRD-069 report structure with a new budgeting model.
- No requirement to infer tokenizer families from arbitrary substrings or undocumented guesses.
- No requirement to add new remote inspection or model-invoked tool surfaces for this feature.

## Background & Motivation

### Current state

PRD-067 intentionally started with a cheap `chars / 4` estimate for prompt-budget replay. That was a practical first implementation because it was deterministic, local, and easy to integrate into the first budget-aware replay slice.

PRD-069 then exposed that budgeting logic through `/context`, first for prompt/history visibility and later with tool-definition cost included. That made the estimate user-visible and much more diagnosable, but also made the heuristic's weaknesses much easier to see.

The current issue is therefore not isolated to `/context`. It affects two already-landed product behaviors:

- PRD-067: replay-budget decisions in `themion-core`
- PRD-069: `/context` token totals and section cost reporting in `themion-cli` via the shared core report path

Real usage comparisons now show that the heuristic is too coarse in both directions:

- it previously undercounted because tool definitions were not included
- after adding tool-definition text cost, it can overcount because raw serialized JSON length is still only a rough proxy for tokenizer behavior

Recent observed behavior shows the problem clearly:

- PRD-069 `/context` rough estimate after a tiny turn: about `7,276` tokens
- provider-reported actual prompt input for that same shape: about `4,806` tokens

Once PRD-069 made the estimate user-visible, and once PRD-067 depended on the same estimate for replay-policy thresholds, the rough heuristic stopped being only an internal shortcut. It became a product accuracy issue shared by both features.

The agreed solution also needs to preserve low-surprise behavior for unsupported model names. Accuracy improvements are valuable, but not if they come from silent tokenizer guesses that users and developers cannot audit. For the same reason, PRD-069 `/context` should show which tokenizer path was actually used, rather than hiding that detail inside the estimate.

### Research note: `tiktoken-rs`

Focused external research via Codex CLI indicates:

- `tiktoken-rs` is a local Rust tokenizer crate for OpenAI-style BPE encodings
- it supports explicit encodings such as `o200k_base`, `o200k_harmony`, `cl100k_base`, `p50k_base`, `p50k_edit`, and `r50k_base`
- it can select tokenizers by model name via helpers such as `get_bpe_from_model(...)`
- it provides plain-text encoding and chat-style message counting helpers such as `num_tokens_from_messages(...)`
- it does not appear to require runtime network access for ordinary tokenizer use
- it still cannot fully replicate provider-side billing/accounting for hidden framing or all tool/function schema behavior, so results must remain labeled as estimates

That makes it a strong fit for improving both PRD-067 and PRD-069: significantly better local estimates without requiring a provider round-trip.

## Design

### 1. Add a tokenizer-backed estimation layer in `themion-core`

Themion should introduce a shared token-estimation helper in `themion-core` that prefers `tiktoken-rs` for supported models and falls back gracefully when needed.

Required behavior:

- model-aware token estimation lives in `themion-core`
- estimation should attempt tokenizer selection by active model name first when possible
- when exact model-name selection fails, the runtime may use a small explicit Themion-maintained trusted fallback map only where that mapping is documented, auditable, and considered safe
- if neither exact model-name selection nor a trusted fallback-map entry applies, the runtime should fall back to the existing rough heuristic rather than guessing a tokenizer silently
- plain text and serialized JSON payloads should be tokenized through the selected tokenizer rather than only by character length
- the report/output should make clear whether estimation came from exact model-name mapping, a trusted fallback-map entry, or the rough fallback estimator

This keeps both PRD-067 replay reasoning and PRD-069 reporting closer to the real model family while preserving robustness for unsupported or future model names.

**Alternative considered:** keep `chars / 4` for replay decisions and use `tiktoken-rs` only for `/context`. Rejected: that would improve PRD-069 while leaving PRD-067 on a drift-prone estimate path.

### 2. Use the same improved estimate path for both PRD-067 replay decisions and PRD-069 `/context`

The tokenizer-backed estimate should feed the same shared core prompt-analysis path used by live model calls and `/context`.

Required behavior:

- PRD-067 replay thresholds should continue to apply in token space, but token counts should come from the improved estimator when supported
- PRD-069 `/context` should use the same section totals, turn totals, and replay-mode decisions the next real provider round would use
- `themion-cli` should continue to receive only structured report data and render it, without owning tokenization logic

This preserves the architectural intent of PRD-069 while ensuring the user-visible report and the real replay policy evolve together.

**Alternative considered:** perform a separate `tiktoken-rs` pass only for display after replay decisions are already made. Rejected: that would make PRD-069 more accurate-looking while leaving PRD-067 on a rougher estimator.

### 3. Count tool definitions explicitly, but with tokenizer-backed accounting

Tool definitions should remain a separate visible section in `/context`, because they are often a dominant cost driver.

Required behavior:

- tool-definition size should still appear as its own section in PRD-069 `/context`
- its token count should be based on tokenizer-backed counting of the serialized schema payload, not only raw chars
- if the estimator falls back for the active model, the tools section may also fall back, but the report should remain explicit that the result is approximate

This preserves the user-visible budgeting insight discovered during PRD-069 implementation while improving numeric fidelity.

**Alternative considered:** hide tool definitions inside a single total. Rejected: users already learned through PRD-069 that tool schemas are often the largest bucket, so removing that visibility would make `/context` less useful.

### 4. Represent estimate quality and tokenizer path explicitly in the report

PRD-069 `/context` should remain honest about what kind of estimate it is showing.

Required behavior:

- the core report should carry enough metadata to distinguish tokenizer-backed estimates from rough fallback estimates
- `/context` should render an explicit estimate-mode line, for example `estimate mode: tokenizer` or `estimate mode: rough fallback`
- when tokenizer-backed estimation is used, `/context` should also render which tokenizer path was selected, for example `tokenizer: o200k_base (exact model match)` or `tokenizer: o200k_base (trusted fallback mapping)`
- when rough fallback estimation is used, `/context` should either omit the tokenizer line or show a clear unavailable form such as `tokenizer: unavailable`
- the total should remain described as an estimate, not as exact provider-billed usage

The product goal is better trustworthiness for both PRD-067-backed budgeting and PRD-069-backed inspection, not false precision.

**Alternative considered:** silently switch implementations and keep the same wording. Rejected: the feature becomes easier to misread if users are not told whether the current model path is tokenizer-backed or fallback-only.

### 5. Keep runtime cost bounded and local

The tokenizer improvement must not introduce hidden online dependencies or excessive repeated setup cost.

Required behavior:

- tokenizer use should remain local-only with no runtime network access
- reusable/singleton tokenizer instances should be preferred where practical rather than rebuilding tokenizers on every estimation call
- the implementation should not noticeably degrade interactive `/context` responsiveness or prompt replay setup time

This matters because prompt estimation may run on every provider round as part of PRD-067 behavior as well as on user-triggered PRD-069 inspection.

**Alternative considered:** initialize tokenizer state ad hoc for every section or every message. Rejected: that adds avoidable overhead to a hot path.

### 6. Keep unsupported-model behavior safe and understandable

Themion supports multiple providers and model names, so tokenizer availability may not always be perfect.

Required behavior:

- unsupported or unknown model names must not break PRD-067 prompt replay or PRD-069 `/context`
- the runtime should attempt tokenizer selection in this exact order: exact model-name mapping, then trusted fallback-map entry if one exists, then rough estimate fallback
- the runtime must not silently guess a tokenizer for an unsupported model name without an explicit trusted fallback-map entry
- user-facing output should remain available, and should not pretend the fallback is tokenizer-accurate
- trusted fallback-map entries should be conservative, short, and easy to audit, for example when a known provider alias is intentionally mapped to the same tokenizer family as an upstream documented model

This keeps the feature broadly usable without turning tokenizer support gaps into hard runtime failures or silently misleading estimates.

**Alternative considered:** support only a narrow allowlist of exact model names and fail otherwise. Rejected: that would make the feature brittle across providers and model upgrades.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/Cargo.toml` | Add `tiktoken-rs` as a new dependency if the final implementation confirms the crate is the chosen tokenizer path. |
| `crates/themion-core/src/` | Add a shared tokenizer/estimation helper layer that maps active models to tokenizers, counts prompt text and serialized tool-schema payloads, applies trusted fallback-map entries when needed, and falls back cleanly to the rough estimator when unsupported. |
| `crates/themion-core/src/agent.rs` | Replace the current rough estimate usage that currently powers both PRD-067 prompt replay and PRD-069 prompt-context reporting with the shared tokenizer-backed estimate path where supported. |
| `crates/themion-core/src/context_report.rs` | Extend report metadata so PRD-069 `/context` can show estimate mode, tokenizer used, and whether counts came from exact tokenizer-backed estimation, trusted fallback-map selection, or fallback rough estimation. |
| `crates/themion-cli/src/tui.rs` | Render the improved estimate metadata for PRD-069 `/context` without taking ownership of tokenization logic. |
| `docs/architecture.md` | Document that PRD-067 prompt-budget replay and PRD-069 `/context` now use tokenizer-backed local estimation when supported by the active model, with explicit trusted-fallback behavior for unsupported model names. |
| `docs/engine-runtime.md` | Document the estimation path, tokenizer-selection order, `/context` estimate-mode visibility, fallback semantics, and the fact that results remain estimates rather than exact provider accounting. |
| `docs/README.md` | Add the new PRD entry and keep status/version alignment current. |

## Edge Cases

- active model name is not recognized by `tiktoken-rs`, but Themion has an explicit trusted fallback-map entry for that exact model family → verify: PRD-067 replay and PRD-069 `/context` use that fallback tokenizer path and clearly label it as such.
- active model name is not recognized by `tiktoken-rs`, and Themion has no trusted fallback-map entry → verify: PRD-067 replay and PRD-069 `/context` fall back to the rough estimator without crashing or silently guessing a tokenizer.
- provider uses a model alias that maps imperfectly to tokenizer expectations → verify: the chosen mapping path is deterministic, conservative, and clearly documented.
- tokenizer-backed estimation is used → verify: PRD-069 `/context` shows both estimate mode and the tokenizer path used.
- rough fallback estimation is used → verify: PRD-069 `/context` shows rough-fallback estimate mode and does not misleadingly imply a tokenizer was used.
- tool schema JSON is large but message history is small → verify: PRD-069 `/context` still shows tool definitions as a dominant explicit cost bucket with tokenizer-backed counting.
- `T0` sits near the 170K or 250K threshold → verify: improved token counting can change PRD-067 replay-mode decisions predictably without breaking the existing policy shape.
- the active session switches profiles/models → verify: the estimator refreshes to the new model mapping rather than keeping stale tokenizer state.
- tokenizer initialization or lookup fails unexpectedly after a supported model was selected → verify: the runtime degrades to fallback estimation rather than failing the round or the `/context` command.

## Migration

This feature does not require database migration.

Migration/rollout guidance:

- preserve the existing PRD-069 `/context` output shape as much as practical so the command remains familiar
- switch the underlying estimate source in `themion-core` first, not in the TUI
- keep fallback rough estimation available for unsupported models
- update docs so users understand that accuracy is improved for both replay and reporting, but still approximate

## Testing

- run PRD-069 `/context` on a supported OpenAI-style model using direct `tiktoken-rs` model-name mapping → verify: the report renders normally, marks the estimate as tokenizer-backed, and shows the tokenizer path used.
- run PRD-069 `/context` on a model name that is unsupported upstream but has an explicit trusted Themion fallback-map entry → verify: the report renders normally, uses the fallback tokenizer path, and labels it explicitly.
- run PRD-069 `/context` on a model name with no upstream support and no trusted Themion fallback-map entry → verify: the report stays available, labels the result as rough fallback estimation, and does not claim a tokenizer path was used.
- compare PRD-069 `/context` totals before and after the tokenizer-backed implementation on the same short session → verify: the new estimate is materially closer to provider-reported `in=` usage than the old rough heuristic.
- run a session where tool definitions dominate the prompt → verify: the tools section remains explicit and its tokenizer-backed total contributes to the overall estimate.
- run a session near PRD-067 replay thresholds → verify: replay mode decisions still follow the PRD-067 shape, but with improved token counts.
- switch profiles/models within one TUI session → verify: the estimator updates to the new active model mapping.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check -p themion-core --all-features` after implementation → verify: `themion-core` still builds cleanly across feature combinations.
- run `cargo check -p themion-cli --all-features` after implementation → verify: `themion-cli` still builds cleanly across feature combinations.

## Implementation checklist

- [x] confirm `tiktoken-rs` is the chosen dependency and add it to `themion-core`
- [x] add a shared tokenizer-backed estimate helper in `themion-core`
- [x] support model-name-based tokenizer selection with an explicit trusted fallback map and rough fallback behavior
- [x] count prompt sections and tool-definition schema text with the shared tokenizer when supported
- [x] wire the improved estimator into both PRD-067 replay budgeting and PRD-069 `/context`
- [x] expose estimate-mode and tokenizer-path metadata so the TUI can distinguish tokenizer-backed, trusted-fallback-mapped, and rough-fallback estimates
- [x] update runtime and architecture docs plus the PRD index

## Technical note: focused `tiktoken-rs` research summary

External research via Codex CLI found:

- `tiktoken-rs` supports OpenAI-style encodings including `o200k_base`, `o200k_harmony`, `cl100k_base`, `p50k_base`, `p50k_edit`, and `r50k_base`
- model-name-based selection is available through helpers such as `get_bpe_from_model(...)`
- plain-text counting is straightforward via a `CoreBPE` encode call
- chat-style counting helpers such as `num_tokens_from_messages(...)` exist, but request-side tool/function schema definition counting still appears to require explicit serialization plus tokenization of that text
- the crate appears local-only for ordinary tokenizer use and does not appear to require runtime network access
- the crate's own message-counting docs and OpenAI cookbook guidance both still treat chat token counts as estimates rather than exact billed usage

That research supports using `tiktoken-rs` as an improved local estimator for both PRD-067 and PRD-069, not as an authoritative accounting source.
