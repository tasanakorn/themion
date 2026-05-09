# PRD-109: Rename Stylos Talk to Message and Inbox Concepts

- **Status:** Implemented
- **Version:** v0.67.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-08

## Summary

- Rename the lightweight Stylos `talk` feature to `message` and `inbox` terms.
- Expose a clearer model-facing tool, `stylos_send_message`, for short volatile peer messages.
- Rename the receiver topic, Rust types, functions, variables, logs, tests, and injected prompt wording consistently.
- Remove `wait_for_idle_timeout_ms`; the volatile inbox is the canonical accepted-message path.
- Preserve the PRD-107 reliability goal while routing accepted peer messages through watchdog/runtime inbox delivery instead of direct query-handler prompt injection; `queue_full` is the normal post-validation rejection.

## Goals

- Make the API easier for models to understand and choose correctly.
- Replace public `talk` wording with `message` for sender actions and `inbox` for receiver-side queued storage.
- Rename `stylos_request_talk` to `stylos_send_message` in model-facing tools and guidance.
- Rename the receiver-side Stylos topic from `query/talk` to `query/messages/send`.
- Rename implementation names that use `Talk`, `talk`, or `queued talk` unless they are historical or compatibility-only.
- Keep board notes clearly separate as the durable async-work path.

## Non-goals

- Do not redesign delivery semantics beyond the naming cleanup and wait-option removal.
- Do not make inbox messages durable in SQLite.
- Do not add threaded chat, read receipts, final-answer waiting, or message history browsing.
- Do not rename board notes, task requests, or unrelated Stylos discovery tools.
- Do not move inbox ownership into `tui.rs` or Stylos transport code.

## Background & Motivation

PRD-107 made `stylos_request_talk` reliable for busy targets by adding volatile queueing. The shipped behavior is now closer to sending a peer message into a target agent's inbox than starting a live conversation.

The word `talk` is ambiguous for models. It can sound like a live chat, a task request, or a final-answer workflow. The current code also mixes `talk`, `peer_message`, and queue terms. This makes tool choice harder and increases naming drift.

This PRD uses two plain concepts:

- `message`: the sender action and payload
- `inbox`: the receiver-side volatile queue and delivery buffer

## Design

### Public tool contract

The canonical model-facing tool is `stylos_send_message`.

Required behavior:

- replace `stylos_request_talk` in the injected tool list with `stylos_send_message`
- use this input shape: `instance`, `to_agent_id`, `message`, and `request_id`
- remove `wait_for_idle_timeout_ms` from the canonical schema
- describe the tool as sending a short user-style message to one target agent
- state that valid known targets enter the receiver's volatile inbox queue
- state that inbox messages are not durable and must not replace board notes for delegated work
- keep the reply fields from PRD-107, including `accepted`, `delivery_state`, `reason`, `correlation_id`, `request_id`, `to_instance`, `to_agent_id`, and `queue_position`
- rename type-level concepts such as `TalkRequest` and `TalkReply` to `MessageRequest` and `MessageReply`

The tool name uses a direct verb because models understand "send message" better than "request talk".

**Alternative considered:** `stylos_request_message`. Rejected because `send` better matches the user action.

### Receiver topic

The canonical receiver-side Stylos query topic changes from:

```text
stylos/<realm>/themion/instances/<instance>/query/talk
```

to:

```text
stylos/<realm>/themion/instances/<instance>/query/messages/send
```

Required behavior:

- local tool invocation uses the new topic
- discovery, status, task, and note topics keep their existing names
- any old-topic compatibility listener delegates to the same canonical send-message handler
- compatibility handling must not duplicate delivery logic

The `messages/send` path is explicit and leaves room for future inbox query or management topics.

### Inbox delivery behavior

The inbox queue replaces bounded wait-for-idle behavior. A sender should not guess how long a busy target will remain unavailable.

Required behavior:

- remove wait-for-idle polling from the canonical receiver path
- when immediate prompt admission is available, deliver immediately
- when immediate prompt admission is unavailable, enqueue immediately if the target inbox has capacity
- when the inbox is full, reject with `delivery_state: "rejected"` and `reason: "queue_full"`
- do not wait for the receiving model's final answer
- keep inbox messages process-local, memory-only, FIFO per target agent, and TTL-limited as defined by PRD-107

Compatibility aliases may ignore or reject an old `wait_for_idle_timeout_ms` field during the transition. The canonical message API must not expose it.

### Runtime naming

Rename talk-oriented runtime names to one consistent message/inbox pattern.

Expected naming direction:

| Current name | New direction |
| --- | --- |
| `TalkRequest` | `MessageRequest` |
| `TalkReply` | `MessageReply` |
| `TalkQueue` | `MessageInbox` or `InboxQueue` |
| `QueuedTalkMessage` | `InboxMessage` or `QueuedInboxMessage` |
| `TalkQueueEnqueueResult` | `InboxEnqueueResult` |
| `talk_queue()` | `message_inbox()` or `inbox()` |
| `handle_talk_query` | `handle_send_message_query` |
| `DrainQueuedTalk` | `DrainMessageInbox` |
| `drain_one_queued_talk_for_agent` | `drain_one_inbox_message_for_agent` |
| `MAX_QUEUED_TALK_PER_AGENT` | `MAX_INBOX_MESSAGES_PER_AGENT` |
| `QUEUED_TALK_TTL_MS` | `INBOX_MESSAGE_TTL_MS` |

Required behavior:

- prefer `message` for payloads and sender actions
- prefer `inbox` for receiver-side queued storage and drain scheduling
- avoid mixed names such as `talk_inbox`, `message_talk`, or `queued_talk_message`
- leave historical PRDs and archive docs unchanged except for cross-reference notes
- keep `type=peer_message` prompt headers unless a later compatibility PRD changes the prompt marker

### Injected agent prompt wording

When a message is delivered now or drained from the inbox, the target agent receives an injected prompt. That prompt must use the new product language because it teaches the receiving model how to respond.

Required behavior:

- describe the injected content as a volatile Stylos peer message, not a talk request
- keep `type=peer_message` stable
- keep sender and target metadata visible in the header, including `from`, `from_agent_id` when available, `to`, and `to_agent_id`
- keep `***QRU***` guidance, phrased as message-reply guidance
- tell the receiving agent to use `stylos_send_message` only when a useful response is needed
- keep no-op guidance: do not send empty acknowledgements or thank-you-only messages
- ensure immediate and inbox-drained messages inject the same prompt shape

Example direction:

```text
type=peer_message from=<instance> from_agent_id=<agent> to=<instance> to_agent_id=<agent>

You received a volatile Stylos peer message. Reply with `stylos_send_message` only if a useful response is needed.
If your response completes the exchange and no further reply should be sent, include ***QRU***.
Do not send empty acknowledgements or thank-you-only messages.
```

### Events, transcript text, and docs

User-visible text should use message/inbox wording.

Examples:

- `Stylos message queued ...`
- `Stylos message delivered ...`
- `Stylos message rejected reason=queue_full ...`
- `watchdog deferred ... because queued Stylos message has priority`

Required behavior:

- update tool-call display helpers so transcript labels show `stylos_send_message`
- update collaboration guidance to say message/inbox for volatile coordination and board notes for durable work
- update architecture and runtime docs that describe the tool, topic, queue, injected prompt, or log wording
- add a short PRD-107 note pointing to PRD-109 when the rename lands, while preserving PRD-107 as the historical behavior contract

### Compatibility policy

The rename changes public tool and topic names. The first implementation may keep aliases to reduce mixed-version failures, but aliases are not the new product surface.

Required behavior:

- expose only `stylos_send_message` in normal model-facing tool schemas
- optionally keep `stylos_request_talk` as a hidden or deprecated compatibility alias for one release
- optionally keep `query/talk` as a receiver topic alias for one release
- compatibility aliases call the canonical send-message handler
- compatibility aliases must not keep wait-for-idle as a model-facing feature
- docs and model guidance point to the new names except in migration notes

If an alias is kept, do not add noisy transcript warnings by default.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Replace `stylos_request_talk` with `stylos_send_message`; remove `wait_for_idle_timeout_ms`; update description and schema. |
| `crates/themion-core/src/agent.rs` | Update collaboration guidance to use `stylos_send_message` for short volatile coordination and board notes for durable work. |
| `crates/themion-cli/src/stylos.rs` | Rename request/reply structs, handler functions, inbox structs, constants, topic registration, tool bridge invocation, injected prompt wording, and tests; remove canonical wait-for-idle handling. |
| `crates/themion-cli/src/app_state.rs` | Rename inbox drain runtime events and helper functions; update queued-message priority text. |
| `crates/themion-cli/src/tui.rs` | Update tool-call display labels for the new tool name only where presentation wiring requires it. |
| `docs/architecture.md` and `docs/engine-runtime.md` | Update references to the tool, topic, volatile inbox, injected prompt, and runtime event wording. |
| `docs/prd/prd-107-queued-delivery-for-stylos-talk.md` | Preserve as historical behavior contract; add a short PRD-109 rename note when implementation lands. |
| `docs/README.md` | List this PRD with status, version target, and scope. |

## Edge Cases

- old caller uses `stylos_request_talk` → verify: the compatibility alias works for the transition window, or the failure clearly names `stylos_send_message`.
- mixed-version peer sends to `query/talk` → verify: the old topic is accepted through the canonical handler if a topic alias is kept.
- new model sees tool schemas → verify: only `stylos_send_message` appears in normal tools, with no `wait_for_idle_timeout_ms` field.
- target is busy and inbox has room → verify: the message queues immediately without idle waiting.
- inbox is full → verify: the message rejects with `delivery_state: "rejected"` and `reason: "queue_full"`.
- queued message drains later by watchdog/runtime scheduling → verify: the injected prompt uses the message-oriented shape and there is no direct send-query-to-agent prompt injection shortcut.

## Implementation Notes

Implemented in v0.67.0. The landed slice renames the model-facing tool to `stylos_send_message`, changes the canonical receiver topic to `query/messages/send`, renames runtime queue concepts to message/inbox terminology, removes canonical wait-for-idle handling, routes accepted peer messages through the volatile inbox for watchdog/runtime delivery, updates injected peer-message prompt wording, and updates sender/receiver runtime event text. No `stylos_request_talk` model-facing compatibility alias remains in the normal tool list.

## Migration

This is a naming migration plus removal of the old wait-for-idle option.

Callers should move from `stylos_request_talk` to `stylos_send_message` and stop sending `wait_for_idle_timeout_ms`. The inbox queue is now the only canonical fallback for busy or non-idle targets.

If compatibility aliases are kept, mark them deprecated in code comments and remove them in a later PRD or cleanup task. New docs should use message/inbox wording. PRD-107 remains historical context for the queued-delivery behavior that shipped under the old name.

The repository version target is `v0.67.0` because this is a user-visible tool and protocol naming change.

## Testing

- inspect model-facing tool schemas → verify: `stylos_send_message` is present, `wait_for_idle_timeout_ms` is absent, and `stylos_request_talk` is absent unless testing compatibility.
- call `stylos_send_message` to an idle known target → verify: it queues immediately and returns `delivery_state: "queued"` without injecting a prompt directly.
- call `stylos_send_message` to a busy known target → verify: it queues immediately and returns `delivery_state: "queued"` without wait polling.
- let the watchdog drain a queued inbox message into the target agent prompt → verify: the prompt says peer message/message, keeps sender metadata, keeps `***QRU***`, and names `stylos_send_message` for useful replies.
- fill the per-agent inbox queue → verify: the next non-immediate message rejects with `queue_full`.
- send through any kept `query/talk` compatibility topic → verify: it reaches the same canonical handler.
- search for `talk`, `Talk`, and `queued talk` after implementation → verify: remaining uses are historical PRD/archive text, compatibility aliases, or explicitly justified comments.
- run `cargo check -p themion-core -p themion-cli` → verify: default builds compile.
- run `cargo check -p themion-cli --features stylos` → verify: Stylos-enabled wiring compiles.
- run `cargo check -p themion-core --all-features` and `cargo check -p themion-cli --all-features` → verify: all-feature builds compile.

## Implementation checklist

- [x] rename model-facing tool schema from `stylos_request_talk` to `stylos_send_message`
- [x] remove `wait_for_idle_timeout_ms` from the canonical message tool/schema and receiver path
- [x] update tool guidance and collaboration prompt text
- [x] rename Stylos topic from `query/talk` to `query/messages/send`
- [x] decide and implement any one-release compatibility aliases
- [x] rename Rust request/reply/inbox structs and functions consistently
- [x] rename runtime drain events and helper variables consistently
- [x] update injected peer-message prompt wording to message/inbox terminology
- [x] update transcript/log text to message/inbox wording
- [x] update architecture/runtime docs and PRD-107 cross-reference note
- [ ] run default, Stylos-feature, and all-feature validation for touched crates
