# PRD-132: Durable Provider Request Recovery

- **Status:** Draft
- **Version:** >v0.80.0 +minor
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-27

## Summary

- Provider request recovery is a sub-turn concern.
- A provider request is complete only when the provider sends its completion marker, such as `response.completed`.
- If a websocket or HTTP stream fails before that marker, Themion must treat the request as incomplete.
- Incomplete provider requests should be retried from the last committed conversation state.
- Generic agent core owns the recovery rule. Provider clients only report protocol facts.

## Goals

- Track provider request attempts durably inside an agent turn or sub-turn.
- Know whether each provider request reached a graceful completion marker.
- Recover from websocket disconnect, HTTP stream failure, process crash, or app restart during a provider request.
- Retry incomplete provider requests from committed conversation history.
- Keep provider-specific protocol parsing separate from generic recovery policy.

## Non-goals

- Do not redesign full agent-turn orchestration.
- Do not redesign tool execution or tool-call recovery. Tool calls are an outer-loop step reached after completed model output.
- Do not define or require persistence for streamed chunks, draft text, or partial model output.
- Do not require provider clients to own retry policy, history commit policy, or workflow state.
- Do not make the TUI infer whether a provider request completed.
- Do not solve provider-side exactly-once semantics.

## Background & Motivation

The current websocket reconnect work is a tactical fix. It detects that a provider stream ended before `response.completed`, then re-requests. The durable design should preserve that simple rule, but make it reliable across crashes and restarts.

A provider request is like an HTTP API call. If the request fails before the provider returns a complete response, the request did not complete.

Themion needs durable request-attempt state so it can answer one narrow question after a bad situation: did this provider request complete? If not, recovery should start a new provider request from the last committed conversation state.

## Design

### 1. Provider request attempts are durable

Each model call should create a durable provider request attempt record before or when the request is sent.

The record should include at least:

- `session_id`
- `agent_id`
- `turn_id` or equivalent current turn identity
- `request_attempt_id`
- `request_seq` inside the turn or sub-turn
- provider/backend/model identity
- request status
- start/update/finish timestamps in milliseconds
- transport type, such as websocket or HTTP stream
- provider response id when available
- previous response id when used
- request fingerprint or compact request identity when useful
- completion marker flag
- finish reason or completion metadata when available
- error kind and short error detail when failed

The exact database schema is an implementation detail. The requirement is that recovery can inspect durable state after restart.

Required attempt statuses:

- `planned`: request was accepted but not sent yet
- `sent`: request was sent to the provider
- `streaming`: at least one provider stream event arrived
- `completed`: provider completion marker was seen
- `failed_retryable`: request ended before completion and may be retried
- `failed_terminal`: provider or protocol failure should not auto-retry
- `interrupted`: user or runtime cancelled the request
- `superseded`: a later retry replaced this incomplete attempt

### 2. Completion marker is the success boundary

A provider request attempt is successful only after a provider completion marker is seen.

Examples:

- Codex/OpenAI Responses stream: `response.completed`
- other providers: the equivalent provider-specific final completion event

Rules:

- websocket close before completion marker means incomplete
- HTTP stream close before completion marker means incomplete
- partial text does not mean complete
- provider terminal failure means failed, not complete
- user cancellation means interrupted and should not auto-retry

This rule should be provider-neutral. Provider clients translate their protocol into generic request-attempt events.

### 3. Recovery retries from committed conversation state

When an attempt is incomplete, Themion should create a new provider request attempt. The new request is built from committed conversation state.

Suggested first policy:

- completed marker seen and assistant message committed → no recovery needed
- completed marker seen but assistant message not committed → commit from durable completion data if enough data exists; otherwise mark recovery blocked
- transport ended before completion marker → retry from committed conversation state
- provider terminal failure → mark failed or require human action according to existing runtime policy
- user interruption → mark interrupted and do not auto-retry

The retry is at provider-request/sub-turn level. It is not a replay of tool execution.

### 4. Generic core owns policy; provider clients report facts

Generic agent core responsibilities:

- create and update durable request attempts
- decide whether an incomplete attempt may retry
- build retry from committed conversation state
- commit or continue normal message handling only after provider completion
- publish runtime snapshots for TUI, web, headless, status tools, and Stylos query paths

Provider client responsibilities:

- send provider requests
- report normalized request events: `sent`, `stream_delta`, `completed`, `failed_retryable`, `failed_terminal`, and `interrupted`
- report provider ids, previous response ids, finish metadata, transport failures, and protocol failures
- build provider-specific request bodies from generic inputs and committed conversation state

Provider clients must not own history commits, workflow transitions, or global recovery policy.

### 5. Runtime snapshots reflect durable truth

The hub/app-state layer should publish cheap snapshots derived from durable request-attempt state plus live runtime state.

Snapshots should expose:

- active turn id when available
- active provider request attempt id
- request status
- whether the completion marker was seen
- recovery status and last recovery reason

TUI, web, headless mode, status tools, and Stylos query paths should read this runtime-owned state. They should not infer completion from displayed text.

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-132-durable-agent-turn-recovery.md` | Replace broad turn-recovery draft with narrow provider-request recovery requirement. |
| `crates/themion-core/src/db.rs` | Add durable request-attempt persistence. Keep exact schema an implementation detail. |
| `crates/themion-core/src/agent.rs` and generic harness code | Drive provider requests through durable attempts. Retry incomplete attempts from committed conversation state. |
| provider/client traits in `themion-core` | Define normalized provider request events and request-building boundary. |
| provider clients such as `client_openai_codex.rs` | Translate provider protocol facts into normalized events. Do not own recovery policy. |
| `crates/themion-cli/src/app_state.rs` and runtime helpers | Publish runtime snapshots from durable/live request state and handle restart recovery admission. |
| TUI/web/headless adapters | Render recovery status from runtime snapshots. |

## Edge Cases

- websocket disconnect before any response event → verify: request attempt is incomplete and can retry from committed conversation state.
- websocket disconnect after partial text → verify: request attempt is incomplete and retry uses committed conversation state.
- HTTP stream closes before completion marker → verify: request attempt is incomplete.
- provider sends completion marker, then process crashes before message commit → verify: recovery commits from durable completion data if available, otherwise blocks safely.
- process restarts during streaming → verify: active request attempt is loaded as recovery state.
- user cancels during streaming → verify: attempt is interrupted and does not auto-retry.
- provider reports terminal protocol failure → verify: attempt is failed, not retried as a transport disconnect.
- two agents stream at the same time → verify: request attempts stay isolated by session/agent/turn/request identity.

## Migration

This design likely needs database migration.

Required behavior:

- existing sessions and messages remain readable
- old messages without request-attempt rows are treated as completed historical history
- new recovery tables start empty for existing history
- absence of recovery metadata must fail safe, not guess that a request completed

## Testing

- force websocket close before completion marker → verify: request attempt becomes incomplete/retryable and a new attempt starts from committed context.
- force websocket close after partial streamed text → verify: request attempt becomes incomplete/retryable and retry starts from committed context.
- persist completion marker, then simulate crash before final assistant message append → verify: recovery safely commits or blocks with a clear reason.
- simulate restart with request state `streaming` and no completion marker → verify: request enters recovery and follows retry policy.
- force provider terminal failure event → verify: request is terminal failed and does not use transport-retry path.
- run two local agents with overlapping streams → verify: each agent keeps separate request-attempt state.
- run `cargo check -p themion-core -p themion-cli` → verify: touched crates compile with default features.
- run `cargo check -p themion-core --all-features` and `cargo check -p themion-cli --all-features` → verify: feature combinations compile.

## Open Questions

- Should the current tactical Codex websocket retry remain as a stopgap, or should it be removed once durable recovery exists?
- What is the minimal generic provider-attempt event contract that supports Codex Responses today without overfitting the core to Codex-only fields?
