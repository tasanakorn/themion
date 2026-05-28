# PRD-133: Per-Agent Codex WebSocket Lifecycle and Event Handling

- **Status:** Implemented
- **Version:** v0.80.2
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-27

## Summary

- Keep one Codex WebSocket lifecycle per local agent. Do not share or multiplex one Codex socket across agents.
- Add a Tokio connection actor per Codex-backed agent to own the socket, read events, write commands, and publish status.
- Use async tasks, not one OS thread per socket, unless later evidence shows a real need.
- Treat connection state as runtime-owned data: connecting, connected, streaming, disconnected, reconnecting, degraded, closing, and closed.
- Preserve PRD-132 retry safety: after disconnect before `response.completed`, clear partial state and retry from committed context only.

## Goals

- Make Codex WebSocket ownership explicit and agent-scoped.
- Prevent cross-agent prompt context, continuation id, request id, or socket reuse.
- Keep a live event loop for each active Codex connection so ping, pong, close, idle disconnect, errors, and reconnects are handled promptly.
- Publish per-agent provider connection status through runtime/app-state snapshots for TUI, web, headless, and debug surfaces.
- Keep provider transport policy in `themion-core` or runtime/app-state layers, not in rendering code.

## Non-goals

- Do not create one process-global Codex WebSocket.
- Do not multiplex multiple local agents over one provider WebSocket.
- Do not add durable request-attempt tables, migrations, or crash recovery.
- Do not require a dedicated OS thread for each socket.
- Do not redesign all provider backends in this PRD.
- Do not put connection lifecycle policy in `tui.rs`, web components, or other presentation code.

## Background & Motivation

The current Codex client stores one optional WebSocket inside `CodexClient`. It reads events while a request is active, reuses the socket across turns when it is still present, and reconnects/retries when a request disconnects before `response.completed`.

That design is enough for the narrow recovery behavior in PRD-132, but it leaves important lifecycle gaps:

- idle sockets are not watched by a live read loop
- ping/pong is handled only while a request is streaming
- close frames during idle time are discovered late
- connection status is emitted as text status rows, not as stable runtime state
- agent-specific provider identity is not explicit enough for future multi-agent Codex use

PRD-133 makes the Codex transport lifecycle a first-class provider/runtime component while keeping the retry safety rules from PRD-132.

## Readiness Decisions

These decisions make this PRD ready to implement:

- the Codex connection actor lives in `themion-core` with the provider client, because socket lifecycle, event parsing, reconnect policy, and request retry are provider/runtime behavior
- `themion-cli` builds one backend per local agent and wires provider connection snapshots into app-state; it does not own socket policy
- a second request for the same agent while one Codex request is active is rejected with a clear busy/in-progress error; this PRD does not add per-agent provider request queuing
- idle disconnects update status immediately and reconnect lazily on the next request; active request disconnects may reconnect eagerly within PRD-132 retry limits

## Design

### 1. Per-agent connection ownership

Each local agent using `openai-codex` must own an independent Codex transport lifecycle.

Required behavior:

- each Codex-backed agent has its own Codex client or connection actor
- each agent has separate WebSocket state, prompt context, active request state, and reconnect counters
- one agent must never send a request through another agent's socket
- one socket must not carry model turns for multiple local agents
- session/profile changes must not transfer a live socket or prompt context to another agent

The current `build_agent` path creates a backend per agent. The implementation should preserve that boundary and add tests or assertions so future refactors cannot introduce a shared Codex socket by accident.

### 2. Agent-scoped provider identity

Provider-facing identity must also be isolated by agent.

Required behavior:

- Codex `session-id`, `thread-id`, `x-client-request-id`, prompt-cache keys, and continuation context must not collide across local agents in the same Themion session
- reconnects must use the owning agent's current provider identity
- profile or session changes must close or replace stale provider identity safely
- any displayed or debug status should include enough agent identity to diagnose socket ownership

A Themion `session_id` may contain several local agents. Provider transport identity therefore needs an agent-specific component, not only the local session id.

### 3. Tokio connection actor

Implement the connection actor in `themion-core`, close to `client_openai_codex.rs`.

The actor is provider-owned. It should not depend on CLI, TUI, web, or Stylos modules. If status must cross crate boundaries, expose small public data types from `themion-core` and let `themion-cli` copy them into app-state snapshots.

Use a Tokio task as the default connection owner. Do not allocate a dedicated OS thread per socket.

Required behavior:

- the actor owns the `WebSocketStream`
- the actor runs the read/write loop while the socket is open, including idle periods
- callers send bounded commands such as connect, send request, cancel active request, close, and refresh auth
- the actor emits structured events and a cheap-to-clone status snapshot
- the actor stops when its owning agent/backend session is dropped, removed, or reconfigured

A dedicated OS thread is out of scope unless later measurement proves Tokio tasks cannot meet the lifecycle needs.

### 4. Request-scoped streaming on a persistent socket

The socket may stay open across turns, but each model request remains a distinct operation.

Required behavior:

- allow at most one active Codex response request per agent socket in this PRD
- route deltas, tool-call arguments, usage, errors, and completion only to the active request
- reject a second request for the same agent while a request is active; do not queue it and do not interleave frames
- cancellation interrupts only the owning agent's active request
- provider `response.failed` and provider `error` fail the active request without treating the failure as a successful reconnect path
- disconnect before `response.completed` follows PRD-132: clear partial state and retry from committed context only

This PRD explicitly does not add provider-level request multiplexing. A later PRD may revisit that only if the Codex protocol exposes safe correlation semantics.

### 5. Event-driven connection state

The actor must publish connection state as data, not only as transcript text.

Required states:

- `disconnected`: no active socket
- `connecting`: WebSocket upgrade is in progress
- `connected`: socket is open and idle
- `streaming`: one request is active
- `reconnecting`: transport failed and reconnect policy is running
- `degraded`: socket is open but health is uncertain
- `closing`: intentional close or cancellation close is in progress
- `closed`: actor has stopped

Required snapshot fields:

- `agent_id`
- `provider`: `openai-codex`
- `state`
- `state_changed_at_ms`
- `last_connected_at_ms`
- `last_disconnected_at_ms`
- `last_error`
- `reconnect_attempts`
- `active_request_id`
- `last_response_completed_at_ms`

Machine-consumed timestamps must use UTC epoch milliseconds.

### 6. Idle health and reconnect policy

The actor should keep the socket observable while idle.

Required behavior:

- respond to provider `Ping` with `Pong` whenever the socket is open
- detect provider `Close` frames while idle and publish `disconnected`
- detect read errors while idle and publish `disconnected` or `reconnecting`
- avoid hot-loop reconnects with bounded backoff and jitter
- stop reconnecting when the owning agent is removed or its backend is replaced

Reconnect policy should be conservative:

- use eager reconnect while a request is active, subject to the PRD-132 retry limits
- use lazy reconnect for idle sockets; reconnect when the owning agent sends the next request
- idle close/error detection must still update status promptly even though reconnect waits for the next request

### 7. Runtime-owned status surfaces

Connection events should flow through runtime/app-state ownership.

Required behavior:

- provider actor emits structured status events
- runtime/app-state stores the current per-agent provider connection snapshot
- TUI, web, headless, Stylos status, and debug surfaces read the same snapshot when they need this status
- transcript status rows may still be emitted, but they are secondary visibility, not the source of truth

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-133-per-agent-codex-websocket-lifecycle.md` | Add this PRD. |
| `crates/themion-core/src/client_openai_codex.rs` | Own the Codex connection actor; split request streaming from socket lifecycle; preserve PRD-132 retry rules. |
| `crates/themion-core/src/client.rs` | Add provider connection status types only if the generic backend abstraction needs them. |
| `crates/themion-cli/src/app_state.rs` and runtime helpers | Preserve one backend/actor per local agent and publish per-agent provider connection snapshots. |
| TUI/web/headless/status adapters | Render runtime-owned provider connection state without owning policy. |
| `docs/codex-integration-guide.md` | Document per-agent socket ownership, actor lifecycle, event handling, status fields, and timestamp units after implementation. |
| tests near Codex client/runtime code | Add mock WebSocket and multi-agent ownership coverage. |
| `docs/README.md` | Add this PRD entry. |

## Implementation Notes

Implemented in v0.80.2.

- `themion-core` exposes provider connection status types through `ChatBackend` and `Agent`.
- `CodexClient` rejects concurrent same-agent Codex requests instead of queuing or multiplexing them.
- Codex connection status now reports connecting, connected, streaming, reconnecting, disconnected, degraded, and closed states with UTC epoch millisecond timestamps.
- An async idle monitor watches an idle Codex WebSocket, replies to `Ping` with `Pong`, detects idle `Close` frames and read errors, and publishes disconnected status. Idle reconnect remains lazy on the next request.
- Active request disconnects still use the PRD-132 reconnect/retry limits and clear partial stream state before retry.
- `themion-cli` builds Codex provider identity from both Themion session id and local agent id, and app-state/web snapshots expose provider connection status per agent.
- `docs/codex-integration-guide.md` documents the lifecycle and status contract.

## Edge Cases

- two Codex agents stream at the same time → verify: each uses a different WebSocket, request id, and prompt context
- two Codex agents share one Themion session → verify: provider `session-id`, `thread-id`, `x-client-request-id`, and continuation context do not collide
- one agent disconnects while another streams → verify: only the disconnected agent changes connection state
- idle socket receives `Ping` → verify: actor sends `Pong`
- idle socket receives `Close` → verify: runtime status changes to `disconnected` without waiting for the next prompt
- user cancels an active request → verify: only that agent's active request is interrupted and no auto-retry runs
- disconnect happens after partial text or tool arguments → verify: partial stream state is cleared before retry
- auth token refresh is needed while reconnecting → verify: reconnect uses fresh auth or reports auth failure clearly
- agent is removed or backend is replaced → verify: actor shuts down and no orphan socket remains
- repeated reconnect failures → verify: backoff prevents hot-looping and `last_error` is visible

## Migration

No database migration is expected.

This is a runtime/provider lifecycle change. Existing sessions should continue to work. New status fields must be additive and safe for older UI surfaces to ignore.

## Testing

- create two local agents using `openai-codex` and start overlapping turns → verify: two independent WebSocket connections are created and no socket state is shared
- create two Codex agents in the same Themion session → verify: provider `session-id`, `thread-id`, `x-client-request-id`, and continuation context do not collide
- close one agent's socket from a mock server while the other remains connected → verify: only that agent reports `disconnected` or `reconnecting`
- send `Ping` while idle from a mock server → verify: actor replies with `Pong`
- send `Close` while idle from a mock server → verify: runtime status changes without waiting for the next prompt
- force disconnect before `response.completed` after partial text → verify: retry follows PRD-132 and clears partial state
- cancel a streaming request → verify: no auto-retry runs after cancellation
- send a second request for the same agent while one request is active → verify: the second request is rejected with a clear busy/in-progress error
- drop/remove an agent with an open Codex socket → verify: actor stops and closes the socket
- run `cargo test -p themion-core client_openai_codex` → verify: Codex transport tests pass
- run `cargo check -p themion-core` → verify: default core build passes
- run `cargo check -p themion-cli` → verify: default CLI build passes
- run `cargo check -p themion-core --all-features` and `cargo check -p themion-cli --all-features` → verify: feature combinations compile

## Implementation checklist

- [ ] Define Codex connection state and event types.
- [ ] Add a per-agent Codex connection actor or equivalent async owner.
- [ ] Move socket read/write ownership into the actor.
- [ ] Make provider WebSocket identity and continuation context agent-scoped.
- [ ] Add request-scoped channels for streamed chunks, reasoning deltas, status, completion, and errors.
- [ ] Preserve PRD-132 reconnect/retry behavior.
- [ ] Publish per-agent provider connection status through runtime/app-state snapshots.
- [ ] Add mock WebSocket tests for idle ping/close, reconnect, cancellation, and multi-agent separation.
- [ ] Update `docs/codex-integration-guide.md` after implementation.
