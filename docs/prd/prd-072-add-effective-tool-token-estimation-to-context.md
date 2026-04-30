# PRD-072: Add Effective Tool-Token Estimation to `/context`

- **Status:** Implemented
- **Version:** v0.47.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-30

## Implementation status

Landed in `v0.47.0` as the first effective tool-token estimation slice for `/context`. The shipped behavior keeps raw tool-schema tokenization visible, adds a Codex Responses-scoped effective tool-token estimate for the `tool definitions` section, and uses that effective estimate in the overall `/context` prompt total on the validated backend path. The current heuristic uses full description contribution plus 0.75× structural schema overhead after live `/context` versus provider-input comparison showed that the earlier 0.5× factor undercounted real prompt cost for tool-heavy requests.

## Summary

- PRD-070 improved `/context` by switching from the rough `chars / 4` heuristic to tokenizer-backed estimation, but the current tool-definitions estimate still relies too much on raw serialized schema text.
- Recent measurement work shows that raw tokenization of the `tools` JSON payload materially overestimates provider-accounted prompt cost for tool definitions, because the provider likely normalizes schema structure before accounting.
- This PRD adds an `estimated effective tool tokens` concept to `/context` so tool-definition cost is reported in a way that is closer to observed API behavior than raw tokenizer output alone.
- The goal is not to claim exact billed usage; it is to improve `/context` trustworthiness by separating raw tool-schema tokenization from a provider-style effective estimate for tool definitions.
- The feature preserves transparency by showing both the raw tool-schema token estimate and the effective estimate, plus the estimation method used.

## Goals

- Improve `/context` so the `tool definitions` section better reflects observed provider-accounted prompt cost rather than only raw serialized JSON tokenization.
- Introduce an explicit `estimated effective tool tokens` concept for tool definitions in `themion-core`.
- Keep the estimator local-only and deterministic, using measured/provider-inspired heuristics rather than live provider calls.
- Preserve transparency by showing the distinction between raw tool-schema tokenization and the effective estimate in user-facing `/context` output.
- Keep estimation logic in `themion-core` and TUI rendering in `themion-cli`.
- Make the estimate method auditable enough that users can understand why tool-definition cost is lower than raw JSON tokenization would suggest.

## Non-goals

- No claim that `/context` becomes exact provider-billed accounting.
- No attempt to reverse-engineer or guarantee the provider's exact private tool-schema encoding format.
- No new provider calls, calibration requests, or online measurement during ordinary `/context` usage.
- No redesign of the full PRD-069 `/context` output shape beyond what is needed to explain raw vs effective tool-token estimates.
- No attempt in this PRD to estimate exact hidden framing for every provider/backend combination.
- No requirement to change the provider request payload itself or to introduce TOON/CBOR/BSON or other alternate wire formats in production request paths.

## Background & Motivation

### Current state

PRD-070 moved `/context` token estimation onto `tiktoken-rs`, which materially improved prompt-budget estimation over the earlier `chars / 4` heuristic. That fixed an important class of under- and over-estimation issues.

However, recent focused measurement work on the Codex Responses backend shows that the current raw tool-schema estimate still overstates real tool-definition prompt cost when it tokenizes the literal serialized `tools` JSON.

Observed research findings on a recorded real request shape for `gpt-5.4` using `o200k_base`-family approximation:

- raw serialized `tools` JSON: about `14,483` bytes
- raw local tokenization of the `tools` JSON: about `3,108` tokens
- live provider-measured with-tools vs no-tools prompt delta: about `2,063` tokens

Additional synthetic experiments suggest:

- tool and parameter descriptions materially affect provider-accounted cost
- tool and parameter names have much smaller effect than raw JSON tokenization would imply
- a large portion of repeated JSON-schema structure appears not to be counted 1:1 the way local raw tokenization would count it

That makes the current `/context` tools estimate directionally useful but still misleading in an important way: it can report the raw schema tokenization cost rather than the more relevant effective provider-style cost.

### Working product hypothesis

The most useful current product hypothesis is:

- provider tool-definition accounting behaves closer to a normalized semantic schema representation than to literal raw JSON tokenization
- descriptions remain significant prompt-bearing text
- schema structure overhead is compacted materially relative to raw serialized JSON tokenization

A simple and currently well-fitting working estimate is that tool-schema structure contributes only a fraction of its raw tokenized cost, while semantically important tool text contributes more directly.

The exact internal provider format is unknown and should remain treated as unknown. The product need is not exact reverse-engineering; it is a more trustworthy local estimate for `/context`.

## Design

### 1. Add a distinct `effective tool tokens` estimate in `themion-core`

Themion should distinguish between two different tool-definition estimates:

- raw tool-schema tokens
- estimated effective tool tokens

Required behavior:

- raw tool-schema tokens continue to represent tokenizer-backed counting of the serialized `tools` payload
- effective tool tokens represent a provider-style adjusted estimate that accounts for likely schema-structure normalization
- the effective estimate is computed locally from a deterministic heuristic model in `themion-core`
- the estimator should be documented as a heuristic approximation, not exact provider truth

This avoids collapsing two materially different notions of “tool cost” into one number.

**Alternative considered:** replace the raw tool-schema token count entirely with the effective estimate. Rejected: `/context` should remain transparent about both the raw tokenizer view and the adjusted effective estimate.

### 2. Base the effective estimate on semantic text plus discounted schema-structure overhead

The effective estimate should not start from raw whole-JSON tokenization alone. It should classify the tool schema into more meaningful buckets.

Required behavior:

- the core estimator should classify tool-definition content into at least:
  - tool descriptions
  - parameter descriptions
  - names / identifiers
  - structural schema overhead
- the estimator should weight those buckets differently rather than treating them all as equal raw JSON text
- the current working default should assume that structural schema overhead is materially compacted relative to raw tokenization
- the first implementation may use a conservative fixed compaction factor for structural overhead if that is simpler and more auditable than a more complex fit
- if names are included in the model, they should have visibly smaller weight than descriptions unless later evidence suggests otherwise

The first implementation should use an explicit conservative heuristic:

- classify tool definitions into descriptions, names, and structural schema overhead
- count description-bearing text with the active tokenizer path already used by PRD-070
- include tool and parameter names in the structural-overhead bucket for the first implementation rather than modeling a separate name term
- discount structural schema overhead by a documented fixed factor in the initial Codex/Responses-backed implementation

The current implementation-ready default should be:

- effective tool tokens ≈ description contribution + 0.75 × structural schema overhead

For v1, `structural schema overhead` means the raw tool-schema token count after subtracting the tokenizer-backed contribution of tool descriptions and parameter descriptions. Tool names and parameter names stay inside that structural bucket for the first version because current evidence suggests they matter much less than descriptions and do not yet justify a separate weighted term.

This factor is intentionally simple and auditable rather than overfit. It can be recalibrated later if future measured evidence shows that a different fixed factor is materially better.

This matches observed evidence better than raw JSON tokenization alone.

**Alternative considered:** use a single global ratio from raw tool-schema tokens to effective tool tokens. Rejected: a bucketed model is more stable and more explainable when descriptions and schema structure change independently.

### 3. Show both raw and effective tool estimates in `/context`

`/context` should make the distinction visible instead of silently swapping one estimate for another.

Required behavior:

- the tool-definitions section should show the raw tokenizer-backed tool-schema token count
- the same section should also show the estimated effective tool-token count
- the overall prompt estimate should continue to show the main figure Themion considers most representative for the next real provider round
- for providers/backends where the effective estimator is supported, the overall total should use the effective tool-token estimate rather than raw tool-schema tokenization
- the report should remain readable and not overload the user with too many parallel totals

The first implementation should render the tools section in one stable line of this form:

- `tool definitions: raw <R> tok; effective ~<E> tok; mode=<MODE>`

Where:

- `<R>` is the tokenizer-backed raw serialized-schema token count
- `<E>` is the effective tool-token estimate when supported
- `<MODE>` is one of `raw_only` or `raw_plus_effective`

The overall prompt total should use `<E>` instead of `<R>` only when `mode=raw_plus_effective`. Otherwise it should continue to use `<R>`.

**Alternative considered:** keep the section raw-only and add the effective value only in docs. Rejected: the entire point is to improve user-visible trust in `/context` itself.

### 4. Make the estimation method explicit in the report metadata

Users should be able to tell when the tool estimate is raw-only versus when an effective estimator was applied.

Required behavior:

- the core context report should carry tool-estimation metadata with these exact fields or their close equivalent:
  - raw tool tokens
  - effective tool tokens as an optional field
  - tool estimate mode
  - estimator backend scope
- `/context` should render `tool estimate mode: raw_plus_effective` for the validated Codex Responses path
- `/context` should render `tool estimate mode: raw_only` for unsupported backends
- when the effective estimate is used, `/context` should render one concise note immediately after the tools line: `effective estimate discounts schema structure overhead`
- when the active backend/provider does not use the effective estimate path, the report should fall back cleanly and say so without implying adjustment

This preserves trust and keeps the report honest.

**Alternative considered:** show only the numeric result and hide the method. Rejected: the feature is easier to trust when the estimation method is explicit.

### 5. Scope the first implementation to the current Codex/Responses-backed evidence path

The current measured evidence comes from the Codex Responses backend, so the first implementation should be explicit about scope.

Required behavior:

- the initial effective tool-token estimator should be enabled only for the `openai-codex` provider with the `responses` backend path that already motivated this PRD
- the implementation should avoid pretending that one heuristic is equally valid for every provider/backend if that has not been validated
- unsupported providers/backends should continue to use raw tool-schema tokenization only and should report `tool estimate mode: raw_only`
- the core report should remain capable of extending the estimator later as more backend-specific evidence is gathered

This keeps the first version honest and avoids cross-provider overreach.

**Alternative considered:** immediately apply one universal effective estimator across all providers. Rejected: the current evidence is strongest for the Codex Responses path and should not be generalized silently.

### 6. Keep the heuristic simple enough to audit and recalibrate

The estimator should be easy to inspect and update as new evidence arrives.

Required behavior:

- the implementation should prefer a small number of explicit weights or factors over a complex opaque model
- those weights/factors should be documented in code comments or docs where appropriate
- the estimator should be easy to recalibrate if future measured provider deltas suggest better parameters
- `/context` should still treat the result as an estimate and should not imply billing precision

This keeps the feature maintainable and reviewable.

**Alternative considered:** fit a complex multi-factor model from local experiments and hide the details in code. Rejected: a simpler heuristic is easier to trust and to adjust when evidence changes.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/context_report.rs` | Extend prompt/context report data structures to carry raw tool tokens, effective tool tokens, and tool-estimate-mode metadata. |
| `crates/themion-core/src/` | Add shared helper logic that classifies tool-definition schema text into semantic text, names, and structural overhead buckets, then computes an estimated effective tool-token value. |
| `crates/themion-core/src/agent.rs` | Use the effective tool-token estimate in the overall `/context` prompt total when the active backend/provider supports the heuristic path. |
| `crates/themion-cli/src/tui.rs` | Render raw vs effective tool-definition token estimates and a concise estimate-method note in `/context`. |
| `docs/engine-runtime.md` | Document the distinction between raw tool-schema tokenization and estimated effective tool tokens, including why the effective estimate exists. |
| `docs/architecture.md` | Document that `/context` can apply provider-inspired effective tool-token estimation rather than only literal schema tokenization. |
| `docs/README.md` | Add the PRD entry to the PRD table. |

## Edge Cases

- backend/provider has no validated effective tool-token heuristic yet → verify: `/context` still shows the raw tool-schema token estimate and labels the tool estimate mode accordingly.
- tool descriptions are short but schema structure is large → verify: the effective estimate still discounts structural overhead relative to raw tokenization.
- tool descriptions dominate and schema structure is modest → verify: the effective estimate remains close to the raw tokenizer-backed semantic text contribution.
- a backend/provider changes its real accounting behavior over time → verify: the heuristic can be recalibrated without redesigning `/context` output shape.
- the user compares `/context` with provider-reported `in=` usage → verify: the effective estimate is directionally closer than raw tool-schema tokenization in the validated backend path.
- future providers use a different tool-accounting model → verify: the implementation can keep the effective estimator backend-scoped rather than silently reusing one global heuristic everywhere.

## Migration

This feature requires no database migration.

Rollout guidance:

- keep the raw tool-schema token count visible during the first rollout so users and developers can compare it against the new effective estimate
- apply the effective estimate path conservatively only where measured evidence supports it
- document the feature as improved estimation rather than exact provider accounting

## Testing

- run `/context` on the validated Codex/Responses path with a tool-heavy session → verify: the tool-definitions section shows both raw and effective token estimates and the overall total uses the effective estimate.
- compare `/context` with-tools vs without-tools against provider-reported input usage on the validated backend → verify: the effective estimate is materially closer than raw tool-schema tokenization alone.
- run `/context` on a backend without an effective-estimator path → verify: raw tool-schema tokenization still appears and the estimate mode is labeled clearly.
- vary tool descriptions while holding schema structure mostly constant in a controlled test → verify: the effective estimate changes in the expected direction and remains explainable.
- vary schema structure while keeping descriptions mostly constant in a controlled test → verify: the effective estimate discounts raw structural token growth relative to whole-JSON tokenization.
- inspect `/context` readability after the change → verify: the added raw/effective distinction improves trust without cluttering the transcript excessively.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check -p themion-core --all-features` after implementation → verify: `themion-core` still builds cleanly across feature combinations.
- run `cargo check -p themion-cli --all-features` after implementation → verify: `themion-cli` still builds cleanly across feature combinations.

## Implementation checklist

- [x] add tool-estimation metadata and raw/effective tool-token fields to the shared context report structures
- [x] implement a shared classifier for tool descriptions, parameter descriptions, names, and structural schema overhead
- [x] implement the first effective tool-token heuristic in `themion-core` using the initial Codex Responses formula: `effective = descriptions + 0.75 × structural_overhead`, with names included inside the structural bucket for v1
- [x] scope the heuristic to validated provider/backend paths and preserve raw-only fallback elsewhere
- [x] wire the effective tool-token estimate into the `/context` overall prompt estimate
- [x] render raw vs effective tool-token reporting in the TUI `/context` output
- [x] document the new estimation mode and add the PRD entry to `docs/README.md`

## Technical note: research basis for the effective estimate

Focused measurement work on the current Codex Responses path found:

- raw minified `tools` JSON tokenized with `o200k_base`: about `3,108` tokens
- live measured with-tools vs no-tools provider prompt delta: about `2,063` tokens
- raw local tokenization therefore overestimates provider-accounted tool cost by about one-third in the measured request shape
- synthetic experiments indicate that descriptions materially affect cost, while heavy shortening of tool and parameter names has only small impact on provider-reported prompt tokens
- local classification also shows that raw JSON schema overhead is large, reinforcing the conclusion that provider-side schema normalization is likely a major factor

That evidence supports a product-level distinction between:

- raw tool-schema tokens
- estimated effective tool tokens

without requiring a claim that Themion knows the provider's exact internal tool representation.
