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
- `response.output_item.added` is used to register function calls and capture assistant message ids for continuation
- `response.completed` provides usage accounting and `end_turn` handling
- `response.failed` and `error` are surfaced as request errors
- `codex.rate_limits` is parsed best-effort and appended to provider reporting
- known metadata-only events stay silent
- other unhandled events emit one compact provider/runtime-visible notice per distinct event name per provider turn

Routing rule:

- assistant text from `response.output_text.delta` goes only through the assistant chunk callback and renders as normal assistant output
- provider diagnostics such as `codex stream: ...` notices go through the separate status callback and render as status rows rather than assistant messages
- do not inject provider notices into assistant chunk streaming, or they will appear as normal agent text in transcript surfaces

### Current implementation checklist

- [x] stream assistant text from `response.output_text.delta`
- [x] accumulate function-call arguments from `response.function_call_arguments.delta`
- [x] register function-call slots from `response.output_item.added` when `item.type == "function_call"`
- [x] capture assistant message ids from `response.output_item.added` when `item.type == "message"`
- [x] read usage from `response.completed`
- [x] parse provider `end_turn` from `response.completed`
- [x] try continuation when `response.completed` reports `end_turn=false`
- [x] combine usage across continuation segments into one logical provider-turn usage result
- [x] surface provider errors from `response.failed`
- [x] surface transport/provider errors from `error`
- [x] parse streamed rate-limit payloads from `codex.rate_limits` on a best-effort basis
- [x] keep `Created`, `ServerModel`, `ModelVerifications`, `ServerReasoningIncluded`, and `ModelsEtag` silent as known-ignored metadata events
- [x] emit `codex stream: unhandled event=<event_name>` at most once per distinct unhandled event name per provider turn
- [x] keep `response.output_item.done` and related content-part/output-text done events in the known-ignored silent set
- [ ] handle reasoning-summary/content delta events beyond visible unhandled notices
- [ ] handle reasoning-summary part-added events beyond visible unhandled notices

### Current event handling table

| SSE event / upstream chunk | Implemented | Current handling in Themion | Current use |
| --- | --- | --- | --- |
| `response.output_text.delta` / `OutputTextDelta(String)` | Yes | Appends `delta` to accumulated assistant text and forwards the same delta only to the assistant chunk callback. | Live assistant text streaming and final assistant message content. |
| `response.function_call_arguments.delta` / `ToolCallInputDelta { item_id, call_id, delta }` | Yes, partial | Uses `item_id` to find an existing tool-call slot and appends `delta` to the slot's argument buffer. `call_id` from the delta event is not used here. | Build final tool-call arguments for post-stream tool execution. |
| `response.output_item.added` / `OutputItemAdded(ResponseItem)` | Yes, partial | Handles function-call items by creating a tool-call slot with `item.id`, `item.call_id`, and `item.name`. Handles message items by capturing the assistant message id for continuation. Other item types are ignored. | Registers tool calls and preserves the assistant message id needed for continuation. |
| `response.output_item.done` / `OutputItemDone(ResponseItem)` | Yes, known-ignored | Recognized and intentionally silent. | None in the current product slice. |
| `response.completed` / `Completed { response_id, token_usage, end_turn }` | Yes | Reads usage from `response.usage`, captures `response.id`, records `end_turn`, and ends only the current SSE segment. If `end_turn=false`, Themion tries one Codex continuation request and keeps accumulating text, tool calls, usage, and notices into the same logical provider turn. If continuation cannot be completed, Themion emits `codex stream: completed end_turn=false continuation=failed` and stops safely. | Provider-turn completion semantics, usage capture, and continuation control. |
| `response.failed` | Yes | Extracts `response.error.message` and returns an error. | Surfaces provider-declared response failure. |
| `error` | Yes | Extracts top-level `message` and returns an error. | Surfaces generic stream/provider errors. |
| `codex.rate_limits` / `RateLimits(RateLimitSnapshot)` | Yes, limited | Parses the payload best-effort and appends it to provider reporting, while response headers remain the primary immediate source. | Opportunistic streamed rate-limit visibility plus header-derived reporting. |
| provider notices like `codex stream: ...` | Yes | Routed through the status callback rather than assistant chunk streaming. | Low-priority provider/runtime-visible status rows without changing assistant transcript ownership. |
| `Created` | Yes, known-ignored | Recognized and intentionally silent. | None in the current product slice. |
| `ServerModel(String)` | Yes, known-ignored | Recognized and intentionally silent. | None in the current product slice. |
| `ModelVerifications(Vec<ModelVerification>)` | Yes, known-ignored | Recognized and intentionally silent. | None in the current product slice. |
| `ServerReasoningIncluded(bool)` | Yes, known-ignored | Recognized and intentionally silent. | None in the current product slice. |
| `ReasoningSummaryDelta { delta, summary_index }` | No, visible | Emits `codex stream: unhandled event=ReasoningSummaryDelta` once per provider turn. | Discoverability for future reasoning-event support. |
| `ReasoningContentDelta { delta, content_index }` | No, visible | Emits `codex stream: unhandled event=ReasoningContentDelta` once per provider turn. | Discoverability for future reasoning-event support. |
| `ReasoningSummaryPartAdded { summary_index }` | No, visible | Emits `codex stream: unhandled event=ReasoningSummaryPartAdded` once per provider turn. | Discoverability for future reasoning-event support. |
| `ModelsEtag(String)` | Yes, known-ignored | Recognized and intentionally silent. | None in the current product slice. |
| `response.content_part.done` | Yes, known-ignored | Recognized and intentionally silent. | None in the current product slice. |
| `response.output_text.done` | Yes, known-ignored | Recognized and intentionally silent. | None in the current product slice. |
| `response.content_part.added` | Yes, known-ignored | Recognized and intentionally silent. | None in the current product slice. |

### `response.completed` and `end_turn`

Themion now distinguishes these separate facts:

- the current SSE stream segment finished
- the provider said the turn should end or continue
- Themion ended or continued the provider turn
- the broader local harness turn may still continue later for normal tool/runtime reasons

Current `response.completed` behavior:

- usage is read from `response.usage`
- provider `response.id` is captured when present
- `end_turn=true` finishes the provider turn after the current SSE segment
- `end_turn=false` makes Themion try to continue the Codex provider turn using the existing continuation flow
- if continuation succeeds, continued chunks follow normal Codex streaming behavior and accumulate into the same logical provider turn
- if continuation fails or cannot be attempted, Themion emits exactly `codex stream: completed end_turn=false continuation=failed` and stops safely
- if `end_turn` is omitted, Themion keeps the existing single-segment stop fallback

## Usage accounting

On `response.completed`, Themion reads usage from:

- `response.usage.input_tokens`
- `response.usage.output_tokens`
- `response.usage.input_tokens_details.cached_tokens`

These values are mapped into Themion's internal `Usage` and `UsageDetails` structures.

When Codex continuation is used after `end_turn=false`, usage values from each completed segment are accumulated into one combined logical provider-turn usage result.

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
- non-success `/responses` responses return an error with status and body text, and known structured quota-limit bodies are reformatted into a clearer surfaced message
- `429` responses may still carry useful rate-limit headers and should be parsed when available
- known structured Codex quota bodies may also provide `type`, `message`, `plan_type`, `resets_at`, and `resets_in_seconds`
- `resets_at` and `resets_in_seconds` in the current Codex quota body shape are interpreted as seconds
- token refresh failures return an error with upstream status and body text
- malformed or missing optional metadata fields are tolerated by using `None`, with fallback to generic raw error text when structured quota parsing does not match

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
