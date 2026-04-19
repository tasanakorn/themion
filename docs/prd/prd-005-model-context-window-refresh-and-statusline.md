# PRD-005: Model Context Window Refresh and Statusline Display

- **Status:** Implemented
- **Version:** v0.3.0
- **Scope:** `themion-core` (model metadata refresh, provider model info handling, session/runtime state); `themion-cli` (status line rendering and model-switch wiring); docs
- **Author:** Tasanakorn (design) + Claude Code (PRD authoring)
- **Date:** 2026-04-19

> **Implementation note:** The landed implementation refreshes model metadata through the backend abstraction and stores active model info on the agent/session path. The TUI status line now renders `ctx:<used>/<limit>`, where `<used>` is the last turn's prompt token count and `<limit>` prefers `max_context_window`, falling back to `context_window` when needed. Treat the code and current docs as the source of truth where they differ from the original proposed display wording.

## Goals

- Refresh model metadata from the provider `/models` endpoint when a session starts and whenever the active model changes.
- Capture `context_window` and `max_context_window` for the selected model and store them as part of the agent's active model information.
- Surface active context usage and model context capacity in the TUI status line so users can see how much prompt context the last turn used against the model's advertised limit.
- Keep provider-specific `/models` fetching and model metadata parsing in `themion-core`, leaving the TUI responsible only for display.

## Non-goals

- No redesign of prompt windowing or message trimming behavior in this PRD.
- No addition of a new config format for overriding context windows.
- No attempt to infer context limits from `/responses` errors or token usage alone.
- No large refactor of the status line system beyond what is required to show model context metadata.
- No backfilling of historic SQLite sessions with model context metadata from prior runs.

## Background & Motivation

### Current state

Themion already maintains active model, usage counters, and per-turn prompt token counts in the TUI status line. Before this change, it did not expose provider-reported model context metadata, which left users guessing how the current turn's prompt size related to the model's available window.

Separately, the upstream Codex implementation treats `/models` as the source of truth for model metadata including `context_window` and `max_context_window`. It uses that metadata as descriptive model capacity information rather than trying to discover limits from `/responses` failures.

For themion, this supports a clean split:

- `themion-core` fetches and owns provider model metadata
- the active agent/session refreshes that metadata when the model starts or changes
- `themion-cli` renders a compact `ctx:<used>/<limit>` status view

This keeps model context display accurate, lightweight, and provider-grounded.

## Design

### Refresh model metadata from `/models`

When a session starts, the active backend should fetch the model catalog from the provider `/models` endpoint and resolve the selected model's metadata before or during early runtime use. When the user changes model at runtime, themion should refresh the catalog again and update the session's active model info from the latest `/models` response.

The fetched model info should include at minimum:

- model identifier / slug
- display name when available
- `context_window`
- `max_context_window`

Provider-specific request and parsing logic belongs in `themion-core`, following the repository's existing provider/backend separation. The TUI should not call `/models` directly.

**Alternative considered:** fetch `/models` only once at process startup and reuse it for the lifetime of the process. Rejected: a model switch is exactly when the active model metadata matters most, and refreshing at that point keeps the metadata aligned with provider-side changes.

### Active model info in session state

Themion should promote model metadata to explicit session/runtime state rather than leaving it implicit in config strings. The active agent/session state should store a model info struct that includes the selected model's context fields.

This model info should be updated in two cases:

1. initial session construction / early runtime refresh
2. runtime model change via existing profile/config switching flows

If the provider returns a model entry that matches the selected model, the session state should store both `context_window` and `max_context_window`. If the provider cannot resolve the selected model, the state should preserve the selected model name but mark the context fields as unknown.

This keeps the status line and future context-aware behavior reading from one canonical session-owned model info object.

**Alternative considered:** store only raw `context_window` as a standalone numeric field on the session. Rejected: `max_context_window` matters for understanding the model's advertised ceiling, and a structured model info object scales better if more metadata is later surfaced.

### Model resolution behavior

Model resolution should prefer an exact match on the selected model identifier as returned by the provider `/models` endpoint. If themion already supports model aliases or profile-level model names that differ from provider display names, the provider/backend layer should own that translation before session state is updated.

If the backend cannot resolve metadata for the configured model:

- normal chat behavior may continue using the configured model string if that is already supported today
- context metadata should be treated as unavailable
- the UI should avoid fabricating a context window value

This preserves existing behavior while making metadata display opportunistic and accurate.

**Alternative considered:** block model usage unless `/models` returns a matching metadata entry. Rejected: this would turn a metadata lookup failure into a hard regression for model selection, which is too disruptive for a first version.

### Status line presentation

The landed TUI status line displays context information in the form:

- `ctx:<used>/<limit>`

Where:

- `<used>` is the last turn's prompt token count (`tokens_in` for that turn)
- `<limit>` prefers `max_context_window`
- if `max_context_window` is unavailable, `<limit>` falls back to `context_window`
- if neither is known, `<limit>` is shown as `?`

Examples:

- `ctx:12k/400k`
- `ctx:8k/273k`
- `ctx:2k/?`

This is slightly different from the original proposal to show only raw model window metadata such as `ctx:273k` or `ctx:273k/400k`. The implemented format is more useful in practice because it shows both recent prompt usage and the model-advertised ceiling in the same compact field.

**Alternative considered:** show only model metadata with no usage value. Rejected: users care most about how close the current turn is to the model's available context, not just the absolute window size.

### Timing of updates in the UI

The status line should update immediately after:

- session initialization resolves model info when available
- a user action changes the active model/profile and the new model info is loaded
- a completed turn reports prompt token usage for the latest request

If model metadata is unavailable at startup, the UI may temporarily show `ctx:<used>/?` until metadata becomes available through the normal refresh path.

This implies the model-switch path in `themion-cli` must rebuild or update the active agent/session state using refreshed model metadata rather than only replacing the model string in config.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/client.rs` | Extended the backend abstraction with provider-supported model metadata fetching via `fetch_model_info`, and added a shared `ModelInfo` type. |
| `crates/themion-core/src/client_codex.rs` | Implemented `/models` request/response handling for the Codex backend, including parsing `context_window` and `max_context_window` from provider metadata. |
| `crates/themion-core/src/agent.rs` | Stores active model info on the agent and exposes refresh/getter methods so runtime state can surface provider model metadata. |
| `crates/themion-cli/src/main.rs` | Added session-carried `model_info` state and reset behavior on profile switch; print mode refreshes model info before execution. |
| `crates/themion-cli/src/tui.rs` | Propagates model metadata into TUI state during startup/switch flows and renders the compact `ctx:<used>/<limit>` status line. |
| `docs/architecture.md` | Should document that model metadata comes from `/models` and that context status uses active model info rather than `/responses` inference. |
| `docs/README.md` | Updated the PRD-005 row status to Implemented. |

## Edge Cases

- Provider `/models` succeeds but does not include the selected model → keep the selected model active, store unknown context metadata, and show `ctx:<used>/?`.
- Provider `/models` request fails at startup → preserve current model selection behavior if possible, but leave context metadata unknown.
- Provider returns only `context_window` and not `max_context_window` → use `context_window` as the displayed limit.
- Provider returns only `max_context_window` and not `context_window` → use `max_context_window` as the displayed limit.
- Provider returns both fields with the same value → show only one limit value because the status line renders a single `<limit>` position.
- User switches profiles quickly while metadata fetch is still in flight → only the active post-switch model info should be committed to TUI state.
- Non-Codex or local providers without a `/models` endpoint or without context metadata support → status line falls back to unknown context metadata without breaking chat behavior.

## Migration

This feature is additive.

Existing profiles and sessions continue to work without config migration. Sessions that start on older providers or providers that do not expose `/models` metadata will simply show an unknown context limit.

No SQLite migration is required for the landed implementation.

## Testing

- start themion with a provider whose `/models` response includes `context_window` and `max_context_window` → verify: the active session stores those values and the status line shows `ctx:<used>/<max>`.
- start themion with a provider whose `/models` response includes only `context_window` → verify: the status line shows `ctx:<used>/<context_window>` and does not invent a separate max.
- start themion with a provider whose `/models` response includes only `max_context_window` → verify: the active model info stores the available field and the UI shows `ctx:<used>/<max_context_window>`.
- start themion when `/models` fails but normal chat setup still succeeds → verify: the session runs with unknown model context metadata and the status line shows `ctx:<used>/?`.
- switch from one model to another where the second model has a different context window → verify: themion refreshes `/models`, updates the active model info, and the status line changes to the new context limit.
- switch models rapidly while a previous model metadata refresh is still pending → verify: the final active model wins and stale metadata is not rendered.
- use a provider/model that is absent from `/models` → verify: chat behavior remains consistent with current behavior and the context display falls back to unknown.
- use a provider where `context_window == max_context_window` → verify: the status line still renders a single `<limit>` value in `ctx:<used>/<limit>` form.
- complete a turn with non-zero prompt token usage → verify: `<used>` reflects the latest turn's prompt token count rather than cumulative session totals.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: model metadata refresh and status line wiring compile cleanly.
