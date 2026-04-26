# PRD-057: Store Turn-Level Runtime Metadata as JSON in `agent_turns.meta`

- **Status:** Implemented
- **Version:** v0.35.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Themion's persistent history currently records token counts, workflow state, and message content, but it does not persist which model, profile, provider, or app version produced a given turn.
- Add a single JSON text column named `meta` to `agent_turns` rather than introducing separate columns for each runtime attribute.
- Store turn-level metadata such as active profile, provider, model, and application version inside that JSON object.
- Keep the schema change narrow and additive so existing history remains readable and older rows can simply have `NULL` or empty metadata.
- Use turn-level metadata rather than session-only metadata so historical turns remain attributable even if profile or model changes during one session.

## Goals

- Persist the active profile, active provider, active model, and Themion application version for each recorded turn.
- Keep the schema flexible by using one JSON field instead of proliferating columns for every runtime attribute.
- Make later history analysis easier without requiring inference from message text or external logs.
- Allow future additive metadata keys without immediate schema migrations.
- Keep the change backward compatible for existing databases and existing query paths.

## Non-goals

- No requirement to backfill old rows with inferred metadata.
- No redesign of how sessions, turns, or messages are otherwise stored.
- No requirement to move all existing workflow fields into JSON.
- No requirement to index every metadata key immediately.
- No requirement in this PRD to expose the new metadata everywhere in the UI, unless a touched status surface naturally benefits.

## Background & Motivation

### Current state

The SQLite history schema currently stores core turn facts in `agent_turns`, including:

- `turn_seq`
- `tokens_in`
- `tokens_out`
- `tokens_cached`
- `llm_rounds`
- `tool_calls_count`
- `created_at`
- workflow and phase state fields

However, the database does not currently persist which runtime configuration produced each turn. In particular, history analysis cannot reliably answer:

- which model generated this turn
- which profile was active for this turn
- which provider handled this turn
- which Themion version produced this turn

Those values exist at runtime, but they are not durably attached to turn history.

### Why turn level instead of session level

A session can change profile or model over time. Storing runtime metadata only at the session level would make later analysis ambiguous when configuration changes mid-session.

Turn-level storage keeps attribution precise:

- each turn records the model actually used
- each turn records the profile active at that time
- each turn records the provider active at that time
- each turn records the app version that produced it

This is especially useful for:

- debugging regressions across versions
- comparing model behavior within long sessions
- analyzing history size or tool behavior by profile/model
- understanding when prompt-building behavior changed across releases

### Why one JSON field instead of separate columns

A single `meta` JSON field keeps the schema narrow and flexible. The requested values are related runtime attributes, and future work may want to add similar metadata such as provider, backend type, feature flags, or prompt-construction mode.

Using one additive JSON object avoids repeated schema churn for each new attribute while keeping the initial implementation small.

Alternative considered: separate `app_version`, `model`, and `profile` columns. This gives simpler SQL for a few fixed attributes, but it is less flexible and increases schema width for metadata that is naturally grouped and may grow over time.

## Design

### Schema change

Add a nullable `meta` column to `agent_turns`:

- column name: `meta`
- type: `TEXT`
- content: JSON object serialized as text

Example value:

```json
{
  "app_version": "0.34.4",
  "profile": "codex",
  "provider": "codex",
  "model": "gpt-5.4"
}
```

The field should remain nullable so older rows and partially migrated databases remain valid.

### Stored keys

Initial required keys:

- `app_version`
- `profile`
- `provider`
- `model`

The JSON object may later include additional keys, but this PRD requires those four initial keys.

Expected semantics:

- `app_version`: Themion application version that created the turn
- `profile`: active profile name at the time the turn began
- `provider`: active provider name at the time the turn began
- `model`: active model name at the time the turn began

### Capture timing

Turn metadata should be captured when the turn record is created, using the runtime values active for that turn.

That means:

- if the user switches profile during a later turn, subsequent turns get the new profile
- existing prior turns keep the metadata they were created with
- analysis can safely correlate each turn with the runtime configuration that actually produced it

### Serialization shape

Store the field as a compact JSON object string.

Guidance:

- omit keys only when the value is genuinely unavailable
- avoid nested structures unless a future requirement needs them
- keep key names stable and human-readable
- preserve exact string values rather than normalizing them into derived labels

### Read behavior

Existing history consumers that do not care about turn metadata should continue to work unchanged.

Consumers that do care may:

- deserialize `meta` when present
- treat `NULL`, empty, or invalid JSON defensively as missing metadata

If there are existing query structs for turn rows, they should be extended in a backward-compatible way to expose optional metadata where useful.

## Changes by Component

### `themion-core`

- add the `meta` column to `agent_turns` schema creation and migration logic
- serialize turn metadata as JSON text when inserting new turn rows
- source values from the active runtime session for:
  - app version
  - active profile
  - active provider
  - active model
- extend any relevant turn-row read models to expose optional parsed or raw metadata
- keep old rows readable when `meta` is absent or null

### `themion-cli`

- ensure the turn-creation path passes the necessary runtime values into core persistence
- if a touched local debug or inspection surface already displays turn information, it may optionally show parsed metadata when present, but UI display is not required for this PRD

### Docs

- update history/runtime documentation to mention that `agent_turns.meta` stores runtime attribution metadata as JSON
- document the initial required keys and their semantics

## Edge Cases

- If the active profile, provider, or model is unavailable for some reason, store whichever keys are known rather than failing turn persistence.
- If JSON serialization fails unexpectedly, persistence should degrade gracefully rather than losing the whole turn when practical.
- If future versions add keys, older readers should ignore unknown keys.
- If a session spans an upgrade or mixed binaries, `app_version` should reflect the process version that actually recorded that turn.
- Query code should not assume all rows contain valid JSON because older or externally modified databases may not satisfy that assumption.

## Migration

- Add `agent_turns.meta` through the existing additive migration path.
- Existing rows remain unchanged and should keep `meta = NULL`.
- No backfill is required.
- New rows written after migration should populate `meta`.

## Testing

- create a new turn with active profile, provider, model, and current app version available → verify: `agent_turns.meta` contains a valid JSON object with `app_version`, `profile`, `provider`, and `model`
- switch profile or model mid-session and create another turn → verify: each turn retains the metadata values active when that turn was recorded
- read legacy rows without `meta` populated → verify: history queries continue to work and missing metadata is treated as optional
- open or migrate an existing database → verify: the additive schema migration adds `agent_turns.meta` without damaging prior history
- deserialize a row with extra unknown keys in `meta` → verify: readers ignore unknown keys without failure


## Implementation checklist

- [x] add nullable `agent_turns.meta` through additive schema migration
- [x] write turn-level JSON metadata for app version, profile, provider, and model on new turns
- [x] keep legacy rows readable when `meta` is absent
- [x] update runtime/history docs to describe `agent_turns.meta`
