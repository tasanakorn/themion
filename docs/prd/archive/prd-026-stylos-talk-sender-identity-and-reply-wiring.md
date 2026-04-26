# PRD-026: Stylos Talk Sender Identity, Prompt Wiring, Busy-Peer Reply Handling, and Lightweight Wait Tool

- **Status:** Implemented
- **Version:** v0.15.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-21

## Summary

- Keep Stylos `talk` as a lightweight peer-to-peer message request, but make the sender identity explicit instead of anonymous.
- When a remote talk message is injected into an agent turn, tell the receiver who contacted them and give a clear reply protocol.
- Add a simple conversation-closing marker such as `***QRU***` so peer agents can tell when no reply is needed and avoid endless back-and-forth loops.
- When an agent should reply but the peer is currently busy, allow a short bounded wait controlled explicitly by `wait_for_idle_timeout_ms` before failing.
- Consider adding a lightweight built-in wait/sleep tool so agents do not need to emulate short waits through shell commands.
- Keep the first implementation best-effort and process-local; do not turn Stylos talk into a durable messaging system or a general task runner.

## Goals

- Make Stylos talk requests carry enough sender identity that the receiving agent can reliably understand who is speaking.
- Improve prompt injection for received talk messages so the receiver sees both the message content and explicit guidance about whether and how to reply.
- Reduce accidental endless reply loops between peer agents by defining one lightweight, easy-to-spot conversation-closing convention.
- Allow useful peer replies to succeed more often when the target peer is temporarily busy by waiting a short bounded time for the peer to become available.
- Make the busy-peer wait behavior explicit in the request contract through a clearly named `wait_for_idle_timeout_ms` field instead of relying on an ambiguous generic timeout.
- Consider a lightweight non-shell wait mechanism that agents can use for short pauses or poll loops without depending on `shell_run_command`.
- Preserve the current architecture boundary where Stylos transport and request wiring stay in `themion-cli` while reusable prompt/tool behavior stays in `themion-core`.

## Non-goals

- No durable mailbox, delivery guarantee, retry queue, or exactly-once messaging semantics.
- No redesign of `tasks/request`, `tasks/status`, or `tasks/result`; this PRD is about direct `talk` behavior.
- No requirement to support arbitrary multi-hop routing or conversations spanning more than the directly addressed peer instances.
- No requirement to expose a full threaded conversation UI for Stylos peer chats in the TUI.
- No requirement to solve every possible loop pattern; the first goal is to prevent the most likely accidental ping-pong cases with a simple explicit protocol.
- No requirement in this PRD to fully design and ship a general-purpose scheduling, timer, or background job framework; any wait helper should stay narrowly scoped.
- No requirement to make `talk` wait for a final assistant answer; `talk` remains an acknowledgement-oriented delivery request, not a task-result API.

## Background & Motivation

### Current state

PRD-022 added `stylos_request_talk` as a direct per-instance request that validates the target agent, checks whether it is currently `idle` or `nap`, and enqueues a user-style message through the normal local input path. The current docs describe `talk` as an acknowledgement-based request, not as a task-like workflow that waits for a final answer.

That first slice is useful, but it leaves several practical gaps for agent-to-agent collaboration:

- the receiver does not have a strong structured indication of who sent the message
- the injected prompt does not yet define a reply convention between peers
- two agents can accidentally keep replying to each other without a simple closing signal
- a useful reply can fail immediately when the target peer is briefly busy, even if waiting a few seconds would have been enough
- short waits today are awkward because the model may fall back to `shell_run_command` with a shell `sleep`, which is heavier and less explicit than a dedicated tool
- the current request shape does not yet say clearly how a caller asks for bounded wait-on-busy behavior

The user specifically asked for stronger instruction and wiring logic around Stylos talk so peer conversations become more identifiable, more useful, and less loop-prone, and asked that the design consider a lightweight wait/sleep tool.

### Why sender identity must be explicit

When a remote message enters the normal local prompt path as just another user-style message, the receiver has to infer who sent it from limited surrounding context or not know at all.

That is weak for peer-to-peer work because follow-up behavior depends on sender identity:

- whether a reply is appropriate
- where that reply should go
- whether the sender is another agent or a human
- how to summarize context efficiently for the specific peer

The first fix should therefore be to carry a structured sender descriptor through the request and prompt wiring path.

**Alternative considered:** keep sender identity only in transport metadata outside the injected prompt. Rejected: the model needs the sender identity in its visible prompt context to reply correctly and concisely.

### Why reply behavior needs an explicit stop marker

Once peer messages can ask for replies, two helpful agents can accidentally create a loop:

1. agent A talks to agent B
2. agent B replies helpfully
3. agent A interprets that reply as needing acknowledgement or another answer
4. agent B replies again

The project does not need a heavy protocol to reduce this risk. A small explicit marker at the message level is enough for a first pass, as long as the injected instructions explain it clearly.

A simple convention such as `***QRU***` can mean "no further reply needed" or "conversation complete unless you have materially new information." That keeps the loop-prevention signal easy for the model to recognize and easy to generate, while remaining Markdown-friendly when it appears in normal text.

**Alternative considered:** use `[QRU]`. Rejected: it is serviceable, but `***QRU***` stands out more clearly in Markdown-formatted content while remaining short.

**Alternative considered:** try to infer automatically whether a reply is needed from message wording only. Rejected: that is too ambiguous and makes loop prevention depend on unreliable language heuristics.

### Why busy-peer reply handling should use an explicit wait field

The current `talk` behavior rejects immediately when the target agent is not currently free. That is reasonable for the first slice, but it is unnecessarily brittle for peer replies.

In practice, a peer may be busy only briefly while finishing a turn. If the system can wait a bounded period such as 5 to 30 seconds for the peer to become idle, a useful reply may succeed without making the sender reinvent retry logic.

That waiting behavior should be requested explicitly with a field such as `wait_for_idle_timeout_ms` rather than being hidden behind a vague generic timeout. The clearer field name makes it obvious that the timeout applies specifically to waiting for the target to become `idle` or `nap`, not to the whole transport exchange or to waiting for a final assistant answer.

**Alternative considered:** use a single generic `timeout_ms` for all talk timeout behavior. Rejected: it is ambiguous about whether it controls network timeout, busy-peer waiting, or final-answer waiting.

**Alternative considered:** always fail immediately and require the sending agent to retry manually. Rejected: it makes common short-busy cases unnecessarily noisy and raises the chance of dropped useful peer follow-up.

### Why a lightweight wait tool is worth considering

Themion already has `shell_run_command`, so an agent can technically ask the shell to sleep. But using a shell sleep for a short pause is awkward for several reasons:

- it is heavier than necessary for a simple timer
- it depends on shell behavior rather than an explicit harness-level capability
- it can look like arbitrary command execution when the real intent is only to wait briefly
- it makes prompt guidance harder because the model has to discover an incidental workaround instead of using a purpose-built tool

A small built-in wait tool could make short pauses, bounded retry gaps, or mesh poll loops cleaner and easier to reason about.

At the same time, the repository should avoid adding unnecessary tools, so this PRD should frame the wait tool as a considered additive helper, not an automatic requirement if the implementation can keep wait logic entirely internal to Stylos reply delivery.

**Alternative considered:** rely exclusively on `shell_run_command` for short waits. Rejected: it works as a workaround but is too indirect for a common lightweight timing need.

## Design

### Extend `talk` requests with explicit sender identity and `wait_for_idle_timeout_ms`

Stylos talk requests should carry a structured sender description in addition to the destination instance, target agent, and message body.

Minimum sender/request fields for the first implementation:

- `from_instance`
- `from_agent_id`
- optional `from_label`
- optional `from_roles`
- optional `request_id` for correlation, preserving current best-effort semantics
- optional `wait_for_idle_timeout_ms`

Normative behavior:

- `stylos_request_talk` tool calls originating from one Themion agent should populate sender identity from the current local process and agent descriptor rather than leaving it blank
- direct external callers that are not another Themion agent may omit some sender fields, in which case the receiver should see an explicit unknown/external sender form rather than fabricated identity
- the receiving side should preserve the sender identity through the queueing and prompt-injection path so the model can act on it
- when `wait_for_idle_timeout_ms` is omitted or zero, the current immediate busy-reject behavior may be used
- when `wait_for_idle_timeout_ms` is positive, the request may wait up to that many milliseconds for the target peer to become `idle` or `nap` before returning a busy-timeout result
- `wait_for_idle_timeout_ms` should be bounded by a documented maximum and expressed in milliseconds consistently with other machine-consumed timeout fields

This keeps sender identity transport-level and prompt-visible without changing the core meaning of `talk` as a lightweight request.

**Alternative considered:** infer sender identity on the receiver only from the Stylos instance key. Rejected: instance-only identity is not enough once one process can host multiple agents.

### Inject received talk as a peer-message prompt with reply instructions

Received Stylos talk should no longer be injected as a generic user-style message with minimal context. Instead, the receiving agent should see a dedicated peer-message wrapper that states:

- who sent the message
- which local agent received it
- the actual peer message body
- whether the sender appears to expect a reply
- the loop-prevention rule using the closing marker convention

Recommended injected prompt shape, expressed conceptually rather than as final exact wording:

- "Peer message from `<from_instance>/<from_agent_id>` to you (`<local_agent_id>`)."
- "Reply to the sender through Stylos talk if and only if a useful response is needed."
- "If your response completes the exchange and no further reply should be sent, include `***QRU***`."
- "Do not send empty acknowledgements or thank-you-only replies."
- "Prefer one concise useful response rather than a conversational back-and-forth."

Normative behavior:

- peer-message prompt injection belongs in the harness-visible prompt path, not only in hidden transport metadata
- the receiver should be guided to talk back when useful, but explicitly told not to continue the exchange when there is nothing materially useful to add
- the receiver should treat `***QRU***` from the sender as a strong signal that no reply is needed unless there is important corrective information

This keeps the behavior Codex-style and explicit: a distinct contextual instruction plus a clear peer-message content block.

**Alternative considered:** leave the generic prompt injection untouched and rely on a docs note for future behavior. Rejected: the model needs the instruction at the moment it sees the message.

### Define a lightweight closing-marker convention

The first loop-prevention mechanism should be intentionally simple.

Proposed convention:

- `***QRU***` means the sender believes no further reply is needed after this message
- when a received peer message contains `***QRU***`, the receiver should normally not talk back
- when the receiver sends a reply that completes the exchange, it should include `***QRU***`
- `***QRU***` should be treated as protocol text, not as user-facing prose that needs explanation each time

Normative guidance for agent behavior:

- do not append `***QRU***` to a message that explicitly asks the peer for more work or another answer
- do append `***QRU***` when sending the final concise answer, summary, status confirmation, or handoff completion note in the exchange
- do not reply solely to acknowledge a message already marked `***QRU***`

This is intentionally not a full state machine. It is a lightweight visible convention that should eliminate the most common accidental ping-pong pattern.

**Alternative considered:** add numbered conversation phases or turn counters to every talk message. Rejected: too much protocol overhead for the initial problem.

### Add bounded wait-for-idle behavior for reply delivery

When an agent attempts to send a Stylos talk reply and the target peer is busy, the system should support a bounded wait before giving up.

Normative behavior for the first implementation:

- direct `talk` handling should support an optional `wait_for_idle_timeout_ms` path used primarily for peer replies
- `wait_for_idle_timeout_ms` should be the explicit request field that controls this behavior
- the wait should be bounded, with an implementation default in the 5 to 30 second range for peer reply flows and a hard upper cap to prevent indefinite blocking
- while waiting, the sender should poll or re-check the target peer's exported availability state rather than assuming the original busy snapshot is final
- if the target becomes `idle` or `nap` within the wait window, deliver the talk normally
- if the timeout expires, return a structured busy-timeout result rather than pretending delivery succeeded
- this timeout applies to waiting for target availability, not to waiting for the remote agent's final natural-language answer

Recommended first-slice policy:

- peer-initiated reply traffic should set `wait_for_idle_timeout_ms` automatically
- ordinary direct talk requests may keep immediate-fail behavior unless a caller explicitly requests waiting

This keeps the feature focused on the case the user described without forcing all talk callers into extra latency.

**Alternative considered:** queue talks on the receiver until it becomes free. Rejected: that starts to resemble a mailbox/job system and expands scope beyond a bounded best-effort wait.

### Consider a lightweight built-in wait tool for short pauses

This PRD should also consider whether Themion should expose a small built-in wait tool for short sleeps.

Proposed shape:

- a tool such as `time_sleep` or `wait_sleep`
- accepts a bounded duration such as `ms` or `seconds`
- returns a simple structured confirmation when the wait completes or a structured validation error when the request exceeds limits

Intended use cases:

- short agent-controlled backoff before retrying a peer talk or status query
- small polling gaps in mesh coordination flows
- lightweight waiting without invoking the shell just to sleep

Normative guardrails if this tool is implemented:

- keep the maximum duration small enough that the tool cannot become an accidental long-blocking primitive
- prefer milliseconds in the machine-facing parameter if the final schema uses one numeric field
- document clearly that the tool is for short bounded waits, not for general scheduling
- keep the tool optional for this PRD; internal Stylos wait-for-idle logic may still be implemented without exposing a general wait tool to the model

This gives implementation flexibility: the busy-peer delivery path can ship with internal waiting logic first, while the repository still records that a small explicit wait tool may be the cleaner model-facing primitive for future mesh flows.

**Alternative considered:** add no wait tool at all and continue encouraging shell-based sleep workarounds when the model wants a pause. Rejected: the workaround is clumsy and blurs the distinction between "wait briefly" and "run an arbitrary shell command."

### Keep useful-response guidance stronger than politeness guidance

The injected peer-message instructions should emphasize information value over conversational politeness.

Normative guidance:

- a peer reply should contain only information that materially helps the sender continue work, unblock a decision, or confirm a requested result
- avoid pure acknowledgements, pleasantries, or "thanks" replies with no new information
- if the best response is effectively "nothing more to add," prefer no reply, or send one concise closing reply marked `***QRU***` only if the sender clearly needs confirmation

This is important because loop prevention is not just about stop markers; it also depends on discouraging low-value responses that invite another turn.

**Alternative considered:** encourage every received talk to produce some kind of acknowledgement. Rejected: acknowledgement-only replies create noise and are a major source of conversational loops.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Extend the Stylos talk tool schema and runtime invocation contract so sender identity and `wait_for_idle_timeout_ms` can be passed explicitly from an in-turn agent call; if the repository chooses to expose a dedicated wait tool, define its schema here too. |
| `crates/themion-core/src/agent.rs` | Add or refine the prompt-injection path for received Stylos peer messages so the model sees sender identity, reply guidance, and the closing-marker rule as distinct contextual instructions. |
| `crates/themion-cli/src/stylos.rs` | Extend `talk` request/response payloads with sender identity fields, preserve them through the remote prompt bridge, and add bounded wait-for-idle handling controlled by `wait_for_idle_timeout_ms` for peer replies targeting a busy agent. |
| `crates/themion-cli/src/tui.rs` | Carry sender-aware remote prompt requests into the normal local execution path and preserve enough local agent identity for reply generation and task/result correlation. |
| `docs/architecture.md` | Document the updated Stylos talk payload, sender identity semantics, reply guidance, closing-marker convention, explicit `wait_for_idle_timeout_ms` behavior, bounded busy-peer wait behavior, and any model-visible wait tool if it lands. |
| `docs/engine-runtime.md` | Clarify how sender-aware Stylos talk messages are injected into the normal local prompt path, how `wait_for_idle_timeout_ms` remains CLI-local transport/request logic rather than a core scheduler, and where any dedicated wait tool fits into the harness tool model. |
| `docs/README.md` | Add this PRD to the PRD index. |

## Edge Cases

- a talk request arrives from an external caller without full sender fields → the receiver should see an explicit unknown or external sender description rather than a misleading fabricated agent identity.
- a peer message includes `***QRU***` but also asks a direct question that clearly requires correction or urgent reply → the receiver may still reply, but should do so only when materially necessary.
- the sending process hosts multiple agents → sender identity must include both instance and agent ID so the receiver knows exactly whom to answer.
- the receiver is idle when the talk arrives but becomes busy before prompt execution begins → the current best-effort busy rejection path may still fire, and the response should report that clearly.
- a target peer stays busy for longer than `wait_for_idle_timeout_ms` → the sender should receive a structured timeout/busy result and may choose a different follow-up path.
- two peers both send useful final replies at nearly the same time → `***QRU***` on either side should suppress further unnecessary acknowledgements.
- a human or external automation uses `stylos_request_talk` directly and does not know the `***QRU***` convention → the system should still work; the convention is guidance for Themion agents, not a hard protocol requirement for outside callers.
- a dedicated wait tool is exposed and the model requests an excessive sleep duration → the tool should reject or clamp clearly rather than silently blocking for a long time.
- a caller sends a negative or excessively large `wait_for_idle_timeout_ms` → the request should be rejected clearly or clamped according to the documented bound; do not silently reinterpret the value.

## Migration

This is an additive Stylos talk behavior improvement.

Migration expectations:

- existing Stylos status, discovery, and task query behavior remains unchanged
- `talk` payloads gain sender-identity fields and an optional `wait_for_idle_timeout_ms` setting
- older callers that do not send sender identity should still be accepted when practical, with receiver-side fallback labeling
- older callers that do not send `wait_for_idle_timeout_ms` should retain immediate-fail-on-busy behavior unless the caller path applies a documented default for peer replies
- the receiver prompt behavior becomes richer and more explicit, but still enters through the normal local input path rather than a separate execution system
- busy-peer handling remains best-effort and bounded; no persistent queue or database migration is required
- if a dedicated wait tool lands, it is additive and should reduce reliance on shell-based sleep workarounds rather than changing existing shell tool behavior

## Testing

- send a Stylos talk from one Themion agent to another → verify: the receiving agent prompt clearly identifies the sender instance and sender agent ID.
- send a peer message that asks for a useful reply → verify: the receiving agent is instructed to reply through Stylos talk only when it has materially useful information.
- send a peer message ending with `***QRU***` and no unresolved question → verify: the receiving agent does not send a follow-up reply.
- have the receiver generate a final useful reply → verify: the reply includes `***QRU***` when the exchange is complete.
- send a reply to a peer with positive `wait_for_idle_timeout_ms` while that peer is temporarily busy but becomes idle within the configured wait window → verify: delivery succeeds after waiting instead of failing immediately.
- send a reply to a peer with positive `wait_for_idle_timeout_ms` while that peer remains busy beyond the wait window → verify: the sender receives a structured busy-timeout result.
- send a talk request without `wait_for_idle_timeout_ms` to a busy target → verify: the current immediate busy rejection behavior is preserved.
- invoke `stylos_request_talk` from a non-agent or compatibility caller without sender fields → verify: the message is still accepted when otherwise valid and the receiver sees a clear unknown/external sender description.
- if a dedicated wait tool is implemented, invoke it with a short valid delay → verify: the tool completes successfully without using `shell_run_command`.
- if a dedicated wait tool is implemented, invoke it above the maximum allowed delay → verify: the tool returns a structured validation error or clearly documented clamp behavior.
- run `cargo check -p themion-core -p themion-cli --features stylos` after implementation → verify: sender-aware talk wiring, prompt injection, and bounded busy-peer wait compile cleanly.

## Implementation checklist

- [ ] extend Stylos talk request structs and tool schemas with sender identity fields
- [ ] add explicit `wait_for_idle_timeout_ms` to Stylos talk request/tool schemas
- [ ] ensure in-turn `stylos_request_talk` calls populate sender identity from the current local agent descriptor when available
- [ ] preserve sender identity through the CLI-local remote prompt bridge
- [ ] add sender-aware peer-message prompt injection for received Stylos talk requests
- [ ] define and inject explicit guidance for when to reply and when not to reply
- [ ] adopt `***QRU***` as the first closing-marker convention for no-further-reply-needed exchanges
- [ ] suppress unnecessary auto-replies to received messages already marked `***QRU***` unless there is important corrective information
- [ ] add bounded wait-for-idle behavior for peer replies to temporarily busy targets controlled by `wait_for_idle_timeout_ms`
- [ ] return a structured timeout/busy result when the bounded wait expires without delivery
- [ ] document that `talk` remains acknowledgement-oriented and does not wait for a final assistant answer
- [ ] decide whether a dedicated short-duration wait tool should be exposed to the model in this slice or deferred
- [ ] if a dedicated wait tool lands, define its schema, bounds, and user-facing documentation
- [ ] document the updated talk semantics in `docs/architecture.md`
- [ ] document the sender-aware prompt wiring and CLI-local wait behavior in `docs/engine-runtime.md`
- [ ] update this PRD and `docs/README.md` when implementation lands
