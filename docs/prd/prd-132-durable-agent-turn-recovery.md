# PRD-132: In-Memory Provider Request Reconnect and Retry

- **Status:** Implemented
- **Version:** v0.80.1
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-27

## Summary

- Treat Codex `response.completed` as the success boundary for websocket streaming.
- If the live websocket closes before completion, reconnect and retry the active request in memory.
- Retry from committed conversation context only. Do not reuse partial streamed text or partial tool-call state.
- Do not add durable request-attempt storage, migrations, or crash recovery for this fix.
- Keep generic agent turn commit behavior unchanged: only completed backend responses become committed assistant output.

## Problem

Codex websocket streams can close before the provider sends `response.completed`. Partial text, reasoning, or tool-call arguments may already be visible, but that does not prove the provider finished the request.

Without a clear reconnect rule, Themion can treat an interrupted provider attempt like a normal failure. The active turn then loses work even though the process is still alive and can safely rebuild the request from committed context.

This PRD records the narrow implemented fix: live, in-memory retry for Codex websocket disconnects before completion.

## Scope

In scope:

- Codex websocket streaming in `themion-core`
- reconnecting after transport disconnect before `response.completed`
- retrying the active request from committed conversation input
- clearing partial streamed state before retry
- clear status/error messages for reconnect, retry, and exhaustion
- focused Codex client tests and docs alignment

Out of scope:

- durable request-attempt tables or migrations
- process-crash or app-restart recovery
- replaying tool execution from a failed attempt
- persisting partial streamed chunks for recovery
- generic agent turn orchestration redesign
- TUI or web logic that infers provider completion from displayed text

## Current behavior

Codex streaming uses a websocket. The stream may produce text, reasoning, tool-call arguments, rate-limit snapshots, and metadata before the final completion event.

The only reliable success marker is `response.completed`. A transport close before that event leaves the attempt incomplete. Provider terminal events such as `response.failed` and provider `error` are different from reconnectable transport disconnects.

## Expected behavior

Required behavior:

- a Codex websocket response is complete only after `response.completed`
- websocket close before `response.completed` is incomplete and may trigger reconnect retry
- websocket error before `response.completed` may trigger reconnect retry only when it is a transport disconnect
- provider `response.failed` and provider `error` fail without reconnect retry
- user cancellation stops the request and must not auto-retry
- partial text, reasoning, or tool-call arguments from a failed attempt must not be committed or reused
- after reconnect, the retry must send the full request from committed conversation context
- retry exhaustion must return a clear incomplete-stream error

## Fix approach

### 1. Keep retry state live and in memory

The Codex client keeps retry state only inside the active `chat_completion_stream` call.

The live state tracks:

- retry count
- whether `response.completed` was seen
- whether response events or partial content were seen
- whether the retry has switched to a full request body
- final successful attempt metadata such as rate-limit snapshots

This state is discarded when the call succeeds, fails, is cancelled, or the process exits.

### 2. Retry from committed context after reconnect

A retry after reconnect must rebuild the provider request from committed conversation input.

Rules:

- clear the stream accumulator before retrying
- do not include partial assistant output from the failed attempt
- do not continue from a failed in-flight provider response id
- do not use `previous_response_id` after reconnect unless the existing websocket context is still live and known safe
- keep normal history commit behavior unchanged

This avoids mixing incomplete provider output into a new attempt.

### 3. Use a bounded reconnect policy

Default policy:

- retry up to two reconnect attempts for transport disconnects before `response.completed`
- stop immediately on user cancellation
- stop immediately on provider terminal failure
- emit compact status lines for disconnect, reconnect retry, full-request fallback, and retry exhaustion
- return a clear error when the stream closes before completion and retry budget is exhausted

### 4. Keep ownership provider-local

The Codex client owns this policy because the failure mode and safe retry shape are Codex websocket specific.

Generic agent core keeps its existing responsibilities:

- build committed conversation context
- call the backend
- stream events through existing callbacks
- commit assistant output only after the backend returns a completed response

## Risks / edge cases

- websocket disconnect before any response event → verify: reconnect and retry with committed request input
- websocket disconnect after partial text → verify: clear partial text before retry
- websocket disconnect after partial tool-call arguments → verify: clear partial tool state before retry
- websocket disconnect after `response.completed` → verify: do not retry; treat the response as complete
- provider `response.failed` → verify: fail without reconnect retry
- provider `error` event → verify: fail without reconnect retry
- user cancellation during stream → verify: stop without reconnect retry
- retry limit reached → verify: return a clear incomplete-stream error

## Migration

No migration.

This PRD must not add tables, columns, indexes, or persistent request-attempt metadata.

## Validation

- force websocket close before any response event → verify: Themion reconnects and retries the full request
- force websocket close after partial streamed text → verify: partial text is cleared and retry uses committed context only
- force websocket close after partial tool-call arguments → verify: partial tool state is cleared and retry uses committed context only
- send `response.completed` → verify: stream succeeds and no reconnect retry is attempted
- send provider `response.failed` → verify: call fails without reconnect retry
- cancel while streaming → verify: call stops without reconnect retry
- exhaust reconnect retries → verify: error states that stream closed before `response.completed`
- run `cargo test -p themion-core client_openai_codex` → verify: focused Codex tests pass
- run `cargo check -p themion-core` → verify: default core build passes
- run `cargo check -p themion-core --all-features` → verify: all-feature core build passes

## Implementation notes

The implemented slice keeps recovery live-only and provider-local in `crates/themion-core/src/client_openai_codex.rs`.

Landed behavior:

- detects Codex websocket disconnects before `response.completed`
- reconnects and retries the active request from committed context
- clears partial stream accumulators before retry
- avoids durable request-attempt storage and migrations
- preserves normal agent history commit behavior

Validation completed for the landed slice:

- `cargo test -p themion-core client_openai_codex`
- `cargo check -p themion-core`
- `cargo check -p themion-core --all-features`
