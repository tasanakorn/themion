# PRD-132: In-Memory Provider Request Reconnect and Retry

- **Status:** Implemented
- **Version:** v0.80.1
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-27

## Summary

- Codex websocket requests should survive a live websocket disconnect by reconnecting and retrying the last model request.
- Recovery is live and in-memory only. It must not add database tables or migrations.
- A Codex websocket response is complete only after `response.completed` is received.
- If the websocket closes before `response.completed`, Themion should discard partial streamed output for that attempt and retry from the last committed conversation state.

## Goals

- Maintain the active Codex websocket connection when it is healthy.
- Reconnect when the active websocket disconnects before `response.completed`.
- Retry the same model request in memory after reconnect.
- Retry from committed conversation history, not from partial streamed text.
- Avoid provider-side incremental continuation after a reconnect unless it is known safe.
- Surface clear status messages when reconnect/retry is used or exhausted.

## Non-goals

- Do not add a durable request-attempt table.
- Do not add database migrations or persistent recovery metadata.
- Do not recover an in-flight request after process crash or app restart.
- Do not replay tool execution.
- Do not persist partial streamed chunks for recovery.
- Do not redesign generic agent turn orchestration.
- Do not make the TUI or web UI infer completion from displayed text.

## Background & Motivation

Codex streaming uses a websocket. A live websocket can close before the provider sends the final `response.completed` event. Partial text or partial tool-call arguments do not prove that the provider request completed.

The needed behavior is narrow: while the process is still alive, reconnect the websocket and retry the last request. The retry should use the last committed conversation state. It should not depend on any new durable storage.

This keeps the fix small and avoids storing provider attempt state that the current product does not need.

## Design

### 1. Completion marker is the success boundary

For Codex websocket streaming, the request is successful only after `response.completed` is received.

Rules:

- websocket close before `response.completed` means incomplete
- websocket error before `response.completed` means incomplete when the error is a transport disconnect
- partial text, reasoning, or tool-call arguments do not mean complete
- `response.failed` and provider `error` events are terminal provider failures, not reconnect triggers
- user cancellation interrupts the request and must not auto-retry

### 2. In-memory retry state

The Codex client keeps retry state only for the active `chat_completion_stream` call:

- retry count
- whether a reconnect retry already switched to a full request body
- whether any response event or partial content was seen
- whether `response.completed` was seen
- rate-limit snapshots and stream metadata for the final successful attempt

This state is discarded when the call finishes, fails, is cancelled, or the process exits.

### 3. Retry request shape

Themion may use Codex `previous_response_id` only while the existing websocket context is still live and safe to reuse.

After a reconnect, the retry must send the full request built from committed conversation history:

- do not include partial streamed assistant output from the failed attempt
- do not continue from a provider response id from the failed attempt
- do not use `previous_response_id` from the failed in-flight response
- clear the in-memory stream accumulator before retrying

This avoids mixing partial failed output into the next attempt.

### 4. Retry policy

Default first policy:

- retry up to 2 reconnect attempts for transport disconnects before `response.completed`
- stop immediately on user cancellation
- stop immediately on provider terminal failure
- on retry exhaustion, return a clear error that the stream closed before `response.completed`
- emit status lines for disconnect, reconnect retry, full-request fallback, and retry exhaustion

### 5. Ownership

Codex client responsibilities:

- detect websocket disconnect before `response.completed`
- reconnect the websocket
- retry the last active request from committed input
- report compact status and trace metadata

Generic agent core responsibilities:

- keep normal history commit behavior unchanged
- commit assistant output only from a completed provider response returned by the backend
- keep TUI/web/headless behavior driven by existing streaming callbacks and final backend result

The provider client may own this narrow live reconnect policy because the failure mode and safe request shape are Codex websocket specific.

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-132-durable-agent-turn-recovery.md` | Define the narrow in-memory reconnect/retry behavior and explicitly reject durable request-attempt storage. |
| `crates/themion-core/src/client_openai_codex.rs` | Maintain live Codex websocket reuse, detect disconnect before `response.completed`, reconnect, clear partial state, and retry the full request from committed context. |
| `docs/README.md` | Keep the PRD table aligned with the narrowed scope. |

## Edge Cases

- websocket disconnect before any response event → retry after reconnect using committed request input.
- websocket disconnect after partial text → clear partial text and retry after reconnect using committed request input.
- websocket disconnect after partial tool-call arguments → clear partial tool state and retry after reconnect using committed request input.
- websocket disconnect after `response.completed` → do not retry; treat the response as complete.
- provider `response.failed` → fail the call without reconnect retry.
- provider `error` event → fail the call without reconnect retry.
- user cancellation during stream → close/interruption path wins and does not auto-retry.
- retry limit reached → return a clear incomplete-stream error.

## Migration

No migration. This PRD must not add tables, columns, indexes, or persistent request-attempt metadata.

## Testing

- force websocket close before any response event → verify: Themion reconnects and retries the full request.
- force websocket close after partial streamed text → verify: partial text is cleared and retry uses committed context only.
- force websocket close after partial tool-call arguments → verify: partial tool state is cleared and retry uses committed context only.
- send `response.completed` → verify: stream succeeds and no reconnect retry is attempted.
- send provider `response.failed` → verify: call fails without reconnect retry.
- cancel while streaming → verify: call stops without reconnect retry.
- exhaust reconnect retries → verify: error states that stream closed before `response.completed`.
- run `cargo test -p themion-core client_openai_codex` → verify: focused Codex tests pass.
- run `cargo check -p themion-core` → verify: default core build passes.
- run `cargo check -p themion-core --all-features` → verify: all feature core build passes.
