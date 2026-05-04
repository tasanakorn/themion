# PRD-100: Improve Quota-Limit Error Detection and Reset Reporting

- **Status:** Implemented
- **Version:** v0.61.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-04

## Summary

- Themion currently surfaces provider quota failures mostly as a raw `429 Too Many Requests` body string, which makes the important limit details harder to notice and harder to present consistently.
- Detect structured quota-limit error fields from provider error bodies, including HTTP status/code plus `type`, `plan_type`, `message`, `resets_at`, and `resets_in_seconds`.
- Introduce one runtime-owned structured provider error shape in `themion-core` so parsing and formatting do not depend on reparsing raw strings later.
- Show reset timing clearly in user-visible error output with one canonical field order that includes both the absolute reset time and the remaining wait duration when available.
- Preserve the raw upstream message as supporting detail instead of replacing it with opaque Themion-only wording.

## Goals

- Detect quota-limit or usage-limit provider failures as structured errors rather than only raw text blobs.
- Extract and preserve the most useful upstream fields from the current Codex-style 429 body shape:
  - HTTP status or error code
  - `type`
  - `plan_type`
  - `message`
  - `resets_at`
  - `resets_in_seconds`
- Introduce one exact internal ownership path for parsed quota metadata so implementation does not guess where structured information lives.
- Surface reset timing in a user-readable form that includes both the absolute reset time and the remaining wait duration when available.
- Keep provider-specific response parsing in `themion-core` and keep user-facing formatting in CLI/runtime presentation paths.
- Preserve compatibility with provider responses that lack one or more optional fields.
- Keep ordinary non-quota API errors readable and avoid falsely classifying unrelated 429s when the body shape does not match.

## Non-goals

- No redesign in this PRD of rate-limit statusline snapshots, header-derived limit reporting, or background polling behavior.
- No requirement to add a new persistent database schema just to store quota-limit errors.
- No requirement to standardize every provider's error schema in one implementation slice beyond defining a clean internal shape that can grow later.
- No requirement to retry automatically after the reset window expires.
- No requirement to convert provider plan names into Themion-specific entitlement policy.
- No requirement to hide the upstream raw body entirely when it remains useful for debugging.

## Background & Motivation

### Current state

Themion already parses Codex rate-limit headers and exposes useful limit snapshots elsewhere in the runtime, including `resets_at` fields for active rate-limit windows. The provider integration guide already states that `429` responses may still carry useful rate-limit headers and should be parsed when available.

However, the normal error path for a non-success `/responses` call in `crates/themion-core/src/client_codex.rs` currently returns one raw string of the form:

`Codex API error 429 Too Many Requests: {body}`

That means a structured provider body such as:

```json
{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached","plan_type":"plus","resets_at":1777959948,"eligible_promo":null,"resets_in_seconds":61511}}
```

is technically visible, but not product-shaped. The operator has to visually parse JSON inside an inline error string to learn what kind of limit was hit, what plan the provider thinks applies, and when the limit resets.

### Why this matters now

Quota-limit failures are one of the most time-sensitive and action-oriented failure modes in normal use. When they happen, the user usually wants immediate answers to a small set of questions:

- is this actually a quota or usage-limit error
- what kind of quota failure is it
- what plan did the provider report
- when can I try again
- how long is the wait

The current raw-body error shape makes those answers available only indirectly. A modest product-level improvement can keep the original upstream detail while presenting the actionable fields directly and consistently.

**Alternative considered:** leave the body as raw JSON and rely on users or models to parse it mentally. Rejected: the information is present but not surfaced in the place where it matters most, which weakens both human usability and future structured handling.

## Design

### 1. Detect structured quota-limit error bodies on provider failures

Themion should recognize structured provider quota-limit errors instead of treating every 429 as only unstructured text.

Required behavior:

- when the Codex provider returns a non-success response, Themion should inspect the response body for a structured error object before finalizing the surfaced error
- when the HTTP status is `429` and the body contains a matching error object, Themion should extract at least:
  - provider HTTP status or equivalent error code
  - `type`
  - `message`
  - `plan_type`
  - `resets_at`
  - `resets_in_seconds`
- the internal representation should preserve optionality so missing fields do not make parsing fail wholesale
- if the response is `429` but the body does not match the structured shape, Themion should fall back cleanly to the existing generic error path rather than fabricating fields
- if the response is non-429 but still carries a similar structured error object, the implementation may capture it as structured provider error metadata as long as the surfaced message remains truthful about the actual status

This PRD is focused first on the known Codex-style body shape, but the internal structure should not make future provider extension awkward.

### 2. Exact internal ownership path

Themion should use one exact runtime-owned structured error shape so implementation does not guess between ad hoc strings, temporary JSON parsing, or opaque `anyhow` text.

Required behavior:

- `themion-core` should introduce or reuse one provider-owned typed error path that can carry:
  - provider identifier
  - HTTP status code
  - raw response body text
  - optional structured quota metadata
- the structured quota metadata should be represented by one compact typed struct or equivalent named fields, not by a free-form JSON blob in presentation code
- the `/responses` Codex client path in `crates/themion-core/src/client_codex.rs` should construct that structured provider error before converting it into the surfaced error message used by higher layers
- CLI or TUI code should receive either:
  - an already-formatted quota-limit message string produced from the structured core error, or
  - a structured runtime error object whose formatting helper lives outside `tui.rs`
- `tui.rs` must not parse raw JSON provider bodies or decide field extraction rules

Implementation-ready decision:

- the canonical ownership path for this PRD is: provider response body parse in `client_codex.rs` → typed core provider error with optional typed quota metadata → shared formatting helper or `Display` implementation in core/runtime-owned code → surfaced user-facing message in CLI/transcript/error display
- the first implementation should prefer extending an existing provider error path if one already fits cleanly; otherwise introduce one narrow new typed error type rather than scattering `anyhow!(...)` string assembly across multiple call sites

**Alternative considered:** keep `anyhow` as the only error surface and attach no structured intermediate metadata. Rejected: that leaves no stable ownership path for the fields this PRD requires.

### 3. Standardize one internal quota-error metadata shape

Themion should use one compact internal shape for parsed quota-limit metadata so later formatting does not depend on reparsing raw strings.

Required behavior:

- provider code should produce a structured quota-limit metadata object when parsing succeeds
- the shape should preserve:
  - provider name or provider-specific origin when available
  - HTTP status or upstream error code
  - `type`
  - `message`
  - `plan_type`
  - `resets_at`
  - `resets_in_seconds`
  - optional original raw body text for debugging
- this metadata should stay provider/runtime owned in core code; TUI and other surfaces should consume the structured result rather than parse JSON text themselves
- the internal shape should tolerate future additional fields without requiring current consumers to know all of them

Implementation-ready field shape expectation:

- one provider error struct should own the outer error context
- one nested quota metadata struct should own the quota-specific fields
- `resets_at` and `resets_in_seconds` must be stored as integer seconds, not preformatted strings
- unknown or missing optional fields should remain `None` rather than using placeholder strings internally

### 4. Exact surfaced message shape and field order

User-visible quota-limit output should use one canonical field order so implementation and review can verify it concretely.

Required behavior:

- when structured quota metadata is available, the surfaced message should follow this canonical order:
  1. provider name
  2. quota-limit summary phrase
  3. HTTP status
  4. `type`
  5. `plan_type`
  6. provider `message`
  7. reset timing details
- the preferred single-line text form is:

  `Codex quota limit reached (status=429, type=usage_limit_reached, plan=plus): The usage limit has been reached. Resets at 2026-05-05 04:12 local (in 17h 05m).`

- when `plan_type` is absent, omit only the `plan=...` fragment
- when `type` is absent, omit only the `type=...` fragment
- when the provider `message` is absent, retain the rest of the structured quota wording without leaving doubled punctuation
- when only `resets_at` is present, append `Resets at ...` and omit the relative clause
- when only `resets_in_seconds` is present, append `Resets in ...` and omit the absolute clause
- when neither reset field is present, omit the reset sentence entirely
- if no structured quota metadata is available, retain the existing generic provider error wording path

The exact local timestamp display format may reuse existing local time helpers, but the message order above is the product contract for this PRD.

### 5. Surface reset timing in both absolute and relative form

User-visible error reporting should make the retry timing obvious.

Required behavior:

- when `resets_at` is present, Themion should show it in a human-readable local time form
- when `resets_in_seconds` is present, Themion should show a concise remaining duration as well
- when both are present, the user-facing error should include both values rather than choosing only one
- when only one is present, Themion should still present the available field clearly
- when neither is present, Themion should continue to show the upstream message without inventing a reset estimate

Presentation requirement:

- absolute and relative timing should be visibly labeled so the user can tell which is which
- formatting should remain compact enough for transcript/error display, not a multi-line debug dump unless a later surface explicitly opts into richer detail
- relative duration formatting should be stable and compact, for example `45s`, `12m 08s`, `3h 04m`, or `1d 02h`, rather than long prose

### 6. Preserve the upstream message and important identifiers

Improved presentation should not hide the original provider meaning.

Required behavior:

- the surfaced message should still include the provider-reported `message` when available
- the surfaced message should still make the actual HTTP status visible directly in the structured quota message
- `type` and `plan_type` should be surfaced when available because they are often the fastest indicators of what kind of quota condition occurred
- if parsing succeeds but some fields are absent, the message should omit only the missing fields rather than degrade into malformed placeholders
- the raw body text may be retained in structured metadata for logging/debugging, but it should not be dumped by default when the cleaner structured message is available

### 7. Keep formatting ownership out of the TUI policy layer

This feature must respect repository layering rules.

Required behavior:

- body parsing and structured quota-error detection belong in provider/backend code in `themion-core`
- reusable time-formatting helpers may live in core or another runtime-owned shared module if they are needed by multiple surfaces
- TUI or CLI presentation code should only render already-structured error information or a compact formatted message from runtime-owned state
- `tui.rs` must not become the canonical place that recognizes provider JSON error fields or decides quota policy

This keeps the product behavior extensible and avoids reintroducing provider-specific parsing into presentation layers.

### 8. Preserve current generic error behavior as the fallback path

The improvement should be additive and safe.

Required behavior:

- if structured parsing fails, existing generic provider error behavior should remain available
- if the provider changes the error schema unexpectedly, the user should still receive a truthful raw error instead of losing the error altogether
- this PRD must not make non-quota failures less readable while improving quota failures
- any new structured formatting path should be narrow enough that unrelated errors continue to surface as ordinary API errors unless they clearly match the new quota-handling shape

### 9. Align docs with the new error behavior

Active docs should describe the improved structured handling.

Required behavior:

- `docs/codex-integration-guide.md` should be updated to say that 429 quota-limit responses may be parsed from both headers and structured error bodies when available
- the guide should mention the surfaced fields relevant to operator action: `type`, `message`, `plan_type`, `resets_at`, and `resets_in_seconds`
- the guide should state explicitly that `resets_at` and `resets_in_seconds` are interpreted as seconds in the current Codex quota error body shape
- `docs/README.md` should add or update the new PRD entry in sorted order

### 10. Exact field interpretation for this PRD

This PRD defines the minimum field contract for the first quota-limit improvement slice.

Required behavior:

- `type` is the provider-reported error classification string, for example `usage_limit_reached`
- `message` is the provider-reported human-readable explanation and should remain visible in surfaced output
- `plan_type` is the provider-reported plan or entitlement label and should be shown literally when present
- `resets_at` is a provider-reported absolute reset timestamp in Unix seconds and should be documented and handled explicitly as seconds
- `resets_in_seconds` is a provider-reported relative remaining duration in whole seconds and should be documented and handled explicitly as seconds
- if a future provider uses a different unit, implementation and docs must not silently reuse these field names with different semantics

### 11. Future extensibility without overcommitting this patch

The first improvement should solve the current Codex quota-limit experience without pretending to define a complete multi-provider error taxonomy.

Required behavior:

- the implementation should be shaped so future providers can attach similar structured quota metadata if they expose it
- this PRD does not require a provider-agnostic public tool schema or user-facing JSON contract yet
- the first implementation may keep the structured metadata internal and focus on better surfaced error text plus any internal logging/runtime plumbing needed to support it

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-100-improve-quota-limit-error-reset-reporting.md` | Define structured quota-limit detection, exact ownership path, exact message order, and reset-timing surfacing for provider quota failures. |
| `docs/README.md` | Add or update the PRD-100 entry in sorted order and reflect Proposed status/version. |
| `docs/codex-integration-guide.md` | Document that 429 responses may be interpreted from both headers and structured error bodies, including reset-timing fields and their seconds units. |
| `crates/themion-core/src/client_codex.rs` | Parse structured quota-limit error bodies, build the typed provider error plus nested quota metadata, and surface the canonical formatted message. |
| `crates/themion-core` provider/runtime error types or helper modules | Add a compact structured provider error path and quota metadata shape plus reusable formatting helpers if needed. |
| `crates/themion-cli` runtime or display surfaces if touched | Render the improved structured quota-limit message without taking ownership of provider JSON parsing logic. |

## Edge Cases

- a Codex `/responses` call returns `429` with the known structured `error` object and both reset fields → verify: surfaced output shows the quota type, plan, message, absolute reset time, and remaining duration in the canonical field order.
- a Codex `/responses` call returns `429` with `type` and `message` but no reset fields → verify: surfaced output still shows the structured quota classification and message without inventing timing.
- a Codex `/responses` call returns `429` with malformed JSON body → verify: Themion falls back to the generic raw error path cleanly.
- a Codex `/responses` call returns non-429 with a structured error object → verify: surfaced status remains truthful and does not mislabel the error as a quota-limit condition unless the actual status/body support that interpretation.
- the provider returns `resets_at` only → verify: output shows the absolute reset time only.
- the provider returns `resets_in_seconds` only → verify: output shows the relative remaining duration only.
- the provider returns an unknown `plan_type` string → verify: Themion shows it literally rather than rejecting or rewriting it.
- the body includes extra fields such as `eligible_promo` → verify: unsupported fields are safely ignored unless implementation later chooses to expose them.
- a non-quota provider error such as authentication failure occurs after this change → verify: ordinary error readability is preserved.

## Migration

This is an additive error-reporting refinement and should not require data migration.

Expected rollout behavior:

- existing provider calls continue to fail with generic raw errors when no structured quota metadata is available
- known structured quota-limit errors become more informative immediately after upgrade
- no database backfill or user configuration change is required
- docs should be updated in the same change so the improved surfaced behavior matches the integration guide

## Testing

- simulate a Codex `429` response with the known `error` JSON body and both `resets_at` plus `resets_in_seconds` → verify: surfaced output includes provider name, status, type, plan, message, absolute reset time, and relative wait duration in the canonical order.
- simulate a Codex `429` response with only `type` and `message` → verify: surfaced output remains structured and readable without reset placeholders.
- simulate a Codex `429` response with malformed JSON body → verify: generic raw error fallback still surfaces the failure.
- simulate a non-429 structured provider error body → verify: surfaced output remains truthful about the status and does not force quota wording incorrectly.
- simulate a provider error with `resets_at` in Unix seconds → verify: formatting treats the field as seconds, not milliseconds.
- simulate a provider error with `resets_in_seconds` in seconds → verify: relative duration formatting stays compact and accurate.
- run `cargo check -p themion-core` after implementation → verify: default core build stays clean.
- run `cargo check -p themion-core --all-features` after implementation → verify: all-features core build stays clean.
- if `themion-cli` is touched, run `cargo check -p themion-cli` after implementation → verify: default CLI build stays clean.
- if `themion-cli` is touched and the relevant feature mix applies, run `cargo check -p themion-cli --features stylos` after implementation → verify: relevant feature-enabled CLI build stays clean.
- if `themion-cli` is touched, run `cargo check -p themion-cli --all-features` after implementation → verify: all-features CLI build stays clean.

## Implementation checklist

- [x] identify the current provider error path that surfaces raw Codex 429 bodies
- [x] add structured parsing for the known provider quota error object
- [x] introduce or reuse one typed core provider error path with optional nested quota metadata
- [x] preserve fallback behavior for malformed or unmatched error bodies
- [x] implement the canonical surfaced quota-limit message order and omission rules
- [x] format `resets_at` and `resets_in_seconds` clearly using explicit seconds semantics
- [x] update active docs for Codex integration error handling
- [x] validate touched crates in default and all-features configurations, plus any relevant CLI feature mix if CLI code changes
