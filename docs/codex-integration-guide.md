# Codex Integration Guide

This document describes the OpenAI Codex provider integration from Themion's implementation perspective. It is intentionally focused on integration contracts, provider-observed behavior, and parsing expectations rather than TUI wording or product-level UX decisions.

## Scope

The Codex integration in Themion covers:

- OAuth-backed authentication and token refresh
- model metadata fetches from Codex `/models`
- response generation via Codex `/responses`
- rate-limit and quota extraction from HTTP headers
- translation between Themion's internal chat/tool model and the Codex Responses API

Implementation lives primarily in:

- `crates/themion-core/src/client_codex.rs`
- `crates/themion-core/src/client.rs`
- `crates/themion-core/src/agent.rs`

## Endpoints

Themion currently uses the following Codex backend endpoints:

- `GET https://chatgpt.com/backend-api/codex/models?client_version=1.0.0`
- `POST https://chatgpt.com/backend-api/codex/responses`
- `POST https://auth.openai.com/oauth/token`

The `/models` endpoint is used for model discovery and model metadata.

The `/responses` endpoint is used for streamed model execution.

The OAuth token endpoint is used only for refresh-token exchange.

## Authentication

Codex uses OAuth-style credentials rather than an API key.

Themion expects persisted auth material containing at least:

- `access_token`
- `refresh_token`
- `expires_at`
- `account_id`

Before making Codex API calls, the client refreshes the access token when expiry is near.

Requests to Codex backend endpoints include:

- `Authorization: Bearer <access_token>`
- `chatgpt-account-id: <account_id>`
- `originator: pi`

Requests to `/responses` additionally include:

- `OpenAI-Beta: responses=experimental`
- `Accept: text/event-stream`

## Model metadata contract

Themion treats `GET /models` as the provider source of truth for model metadata that is exposed by the backend.

The current integration parses the following fields from each model entry when present:

- `id`
- `slug`
- `display_name`
- `context_window`
- `max_context_window`

Model resolution currently matches the requested model string against:

- `id`
- `slug`

If a model entry is found, Themion stores:

- canonical model id
- optional display name
- optional `context_window`
- optional `max_context_window`

If no entry is found, Themion preserves the requested model name and treats model metadata as unavailable.

### Live observed `/models` behavior for `gpt-5.4`

A live check against the current Codex `/models` endpoint returned a `gpt-5.4` entry containing:

```json
{
  "slug": "gpt-5.4",
  "display_name": "gpt-5.4",
  "context_window": 272000,
  "auto_compact_token_limit": null,
  "truncation_policy": { "mode": "tokens", "limit": 10000 }
}
```

Observed integration-relevant facts from that response:

- `context_window` is present and set to `272000`
- `max_context_window` is not present
- no separate visible field advertising `1_000_000` context was present in the returned model entry

From the integration perspective, Themion should therefore treat `272000` as the provider-reported context metadata currently exposed through `/models` for `gpt-5.4`.

If upstream later exposes a distinct larger hard-limit field, the parser can be extended to capture it. Until then, the integration should not infer undocumented model-capacity values from external marketing material or assumptions.

## Request translation for `/responses`

Themion's internal message model is translated into the Responses API input format.

Current translation rules:

- first internal `system` message becomes top-level `instructions`
- subsequent internal `system` messages become `developer` input messages
- internal `user` messages become `role: user` input messages with `input_text`
- assistant text messages become `role: assistant` output messages with `output_text`
- assistant tool calls become `function_call` items
- tool results become `function_call_output` items

Tool definitions are translated from Chat Completions-style tool schemas into Responses API function tool objects:

- input `type: function` tool entries are mapped to objects with `type`, `name`, `description`, and `parameters`

The request body sent to `/responses` includes at least:

- `model`
- `store: false`
- `input`
- `tools`
- `stream: true`
- `instructions` when a first system prompt exists

## Stream parsing contract

Codex `/responses` uses named SSE events rather than Chat Completions `data:`-only frames.

Themion currently handles these event families:

- `response.output_text.delta`
- `response.function_call_arguments.delta`
- `response.output_item.added`
- `response.completed`
- `response.failed`
- `error`
- `codex.rate_limits`

Current behavior:

- text deltas append to assistant content incrementally
- function call arguments are accumulated by `item_id`
- `response.output_item.added` is used to register function calls
- `response.completed` provides usage accounting
- `response.failed` and `error` are surfaced as request errors
- `codex.rate_limits` is currently parsed opportunistically but is not the primary source for immediate status extraction

## Usage accounting

On `response.completed`, Themion reads usage from:

- `response.usage.input_tokens`
- `response.usage.output_tokens`
- `response.usage.input_tokens_details.cached_tokens`

These values are mapped into Themion's internal `Usage` and `UsageDetails` structures.

## Rate-limit and quota extraction

For Codex rate-limit reporting, Themion treats HTTP response headers as the immediate source of truth.

The current integration recognizes these header families for a limit bucket such as `codex`:

- `x-codex-primary-used-percent`
- `x-codex-primary-window-minutes`
- `x-codex-primary-reset-at`
- `x-codex-secondary-used-percent`
- `x-codex-secondary-window-minutes`
- `x-codex-secondary-reset-at`
- `x-codex-credits-has-credits`
- `x-codex-credits-unlimited`
- `x-codex-credits-balance`
- `x-codex-limit-name`
- `x-codex-active-limit`

The parser also supports equivalent prefixed families such as:

- `x-ratelimit-codex-*`
- normalized limit ids discovered from headers

Extraction model:

- each limit bucket may have a `primary` window
- each limit bucket may have a `secondary` window
- each limit bucket may have a credits snapshot
- `x-codex-active-limit` selects the currently active bucket when present

Themion can also derive limit snapshots from streamed data or from non-success responses, but header parsing is the primary integration path.

## Error handling expectations

Current integration behavior:

- non-success `/models` responses return an error with status and body text
- non-success `/responses` responses return an error with status and body text
- `429` responses may still carry useful rate-limit headers and should be parsed when available
- token refresh failures return an error with upstream status and body text
- malformed or missing optional metadata fields are tolerated by using `None`

## Non-goals for the integration layer

The integration layer should not:

- infer undocumented hard context limits from external sources
- merge pricing assumptions into model metadata
- treat TUI display wording as part of the provider contract
- depend on `/models` for rate-limit percentages
- depend on quota headers for model context metadata

## Updating this guide

Update this document when any of the following change:

- Codex endpoint paths or required headers
- auth refresh behavior
- `/models` field parsing expectations
- `/responses` event handling behavior
- rate-limit header names or bucket selection logic
- observed upstream metadata for default Codex models when that metadata affects integration assumptions
