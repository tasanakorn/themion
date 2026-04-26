# PRD-027: Sender-Side Stylos Talk `from` and `to` Identifier Semantics

- **Status:** Implemented
- **Version:** v0.15.1
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-21

## Summary

- Keep Stylos `talk` sender-aware, but require sender-side `from` and `to` identifiers to use one exact format.
- The sender-side chat panel should log outbound talk as `Stylos talk to=<identifier> from=<identifier>`.
- The identifier format for both fields is exactly `<hostname>:<pid>`.
- Do not append agent IDs, labels, roles, slashes, or any other suffix to that identifier.
- Preserve current acknowledgement-oriented talk behavior; this PRD is about sender-side identifier semantics and chat-panel visibility.
- Receiver-side payload wiring may carry the same identifiers, but that is secondary to the sender-side UX target.

## Goals

- Standardize sender-side Stylos talk identifiers on exactly one format: `<hostname>:<pid>`.
- Ensure outbound `stylos_request_talk` activity can surface both `from` and `to` clearly in the sender-side chat panel.
- Prevent any alternate identifier format such as `<hostname>:<pid>/<agent_id>` from leaking into sender-side logging or related payload fields.
- Keep sender and target instance identity aligned on the same exact identifier representation.
- Document the rule clearly so future Stylos work does not reintroduce mixed identifier formats or sender-side logging ambiguity.

## Non-goals

- No redesign of the Stylos transport, query namespace, or task lifecycle.
- No change to `stylos_request_talk` remaining acknowledgement-oriented.
- No requirement to add agent-specific identity formatting to sender-side chat-panel logs or identifier fields.
- No requirement in this PRD to redesign the receiver-side peer-message prompt beyond preserving compatible identifier semantics where already used.
- No requirement in this PRD to change unrelated task-routing payloads beyond compatibility handling.
- No requirement to let the tool caller set `from` or `from_agent_id` manually.

## Background & Motivation

### Current state

Themion already uses the transport-safe instance identifier `<hostname>:<pid>` for direct Stylos per-instance addressing. The user requested that sender-side Stylos talk reporting use identifiers in exactly that form.

The problem with the previous PRD draft was that it allowed additional display-oriented formats such as `<hostname>:<pid>/<agent_id>`. That is explicitly not desired here. The requested identifier is exact:

```text
<hostname>:<pid>
```

No more, no less.

### Why the identifier must be exact

The instance identifier already has a defined transport-safe shape and meaning in the existing Stylos query surface. Reusing that exact identifier avoids ambiguity and avoids mixing process identity with agent display details.

Normative rule:

- when this PRD says “identifier”, it means exactly `<hostname>:<pid>`
- identifiers must not include `/agent_id`
- identifiers must not include labels, roles, or any other decoration
- identifiers must not use alternate separators or path-like forms

This keeps sender-side logging and any related payload fields predictable.

**Alternative considered:** allow richer sender display values such as `<hostname>:<pid>/<agent_id>`. Rejected: the user explicitly requested the identifier to be exactly `<hostname>:<pid>` and nothing else.

### Why both `from` and `to` still exist

Even though both fields use the same identifier format, they still serve different semantic purposes on the sender side:

- `from`: which local instance initiated the outbound talk
- `to`: which remote instance is being addressed

For sender-side logging, both fields matter even though they use the same exact identifier format. `from` identifies the local sender instance and `to` identifies the addressed remote instance. Keeping both fields explicit avoids overloading one field with two meanings while still honoring the exact identifier constraint.

**Alternative considered:** log only the remote target and omit the sender because it is implicit locally. Rejected: the user explicitly asked for both `from` and `to`, and showing both makes the sender-side event self-contained.

## Design

### Add explicit `from` and `to` fields for sender-side outbound talk reporting

When a local `stylos_request_talk` call is accepted for delivery, sender-side handling should have access to:

- `from`
- `to`

Normative behavior:

- for accepted outbound talk requests, both `from` and `to` must be known to the sender-side logging path
- `from` must be the local sender instance identifier `<hostname>:<pid>`
- `to` must be the addressed remote instance identifier `<hostname>:<pid>`
- no other format is allowed for those identifier fields
- receiver-side bridge payloads may continue using compatible fields where applicable, but that is not the primary requirement of this PRD

This keeps outbound sender-side visibility explicit and exact.

**Alternative considered:** keep sender-side logging implicit and rely only on generic tool-call output. Rejected: the sender-side chat panel should show a dedicated Stylos talk event in a stable format.

### Define the identifier format as exact and exclusive

The canonical identifier format for this PRD is:

```text
<hostname>:<pid>
```

Normative behavior:

- `hostname` is the same hostname-derived segment already used for Stylos instance keys in the CLI runtime
- `pid` is the process ID already used in the instance key
- the combined identifier must match the existing direct-instance addressing format
- no suffix, prefix, slash-delimited extension, or alternate representation is permitted in `from` or `to`

Examples of valid values:

- `node-1:42`
- `devbox:18371`

Examples of invalid values:

- `node-1:42/main`
- `node-1/main`
- `main@node-1:42`
- `stylos/node-1:42`

This is the core requirement of the PRD.

**Alternative considered:** allow `from` to use a richer display form while keeping `to` exact. Rejected: the user explicitly rejected any format other than `<hostname>:<pid>`.

### Derive sender-side `from` automatically and require exact `to`

This PRD requires the sender identity to be resolved automatically by Themion rather than supplied by the tool caller.

Public `stylos_request_talk` input for this sender-side path should include:

- `to`
- optional `to_agent_id`

Normative behavior:

- the tool caller must not provide `from`
- the tool caller must not provide `from_agent_id`
- Themion must resolve `from` automatically from the calling agent and current local instance
- `to` is mandatory and must be provided as an exact `<hostname>:<pid>` identifier
- `to_agent_id` is optional and defaults to `main` when omitted
- agent metadata must not change the identifier format used for `from` or `to`
- no `reply_to` field is used in this payload shape
- no `reply_to_agent_id` field is used in this payload shape

This keeps the tool contract honest on the sender side: sender identity is always knowable by the runtime, so it should not be caller-supplied.

**Alternative considered:** allow the tool caller to set `from` or `from_agent_id`. Rejected: Themion already knows which local agent is calling the tool, so making the caller provide those fields is unnecessary and error-prone.

### Show exact identifiers in the sender-side TUI chat panel

When a local tool call invokes `stylos_request_talk` and the request is accepted for delivery, the sender-side TUI should show:

- `Stylos talk to=<identifier> from=<identifier>`

Normative behavior:

- `from=` should display the automatically resolved local sender instance identifier
- `to=` should display the exact addressed remote instance identifier
- both displayed values must be exact `<hostname>:<pid>` identifiers
- the sender-side chat panel must emit this dedicated log entry when the outbound talk is accepted for delivery
- the chat panel must not show any slash-delimited or agent-decorated identifier in that line

This gives the user the exact visible sender-side behavior requested.

**Alternative considered:** show agent-decorated sender identity in the TUI for extra clarity. Rejected: the request explicitly requires the exact identifier format and no other format.

### Keep agent-specific metadata out of identifier fields

Themion may still carry sender agent metadata elsewhere in the talk flow, but not inside sender-side `from` or `to` identifiers.

Normative behavior:

- if prompt injection needs sender agent details, they should remain separate from the identifier fields
- the identifier fields are reserved exclusively for exact `<hostname>:<pid>` values
- future work must not repurpose `from` or `to` into composite identity strings

This keeps the identifier contract stable even if richer peer-message context exists elsewhere.

**Alternative considered:** use `from` as a general sender-description field and add another exact instance field later. Rejected: this PRD is specifically defining `from` and `to` as exact identifier fields.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/stylos.rs` | Resolve sender-side `from` automatically from the calling local instance, require exact `to` for outbound talk, and make both available to sender-side logging at acceptance time. |
| `crates/themion-cli/src/tui.rs` | Emit a sender-side chat-panel log entry `Stylos talk to=<identifier> from=<identifier>` when outbound `stylos_request_talk` is accepted, using the exact identifier format. |
| `docs/architecture.md` | Clarify that Stylos talk identifiers in this payload/UI path are exactly `<hostname>:<pid>` and not agent-decorated forms. |
| `docs/engine-runtime.md` | Document that the CLI-local talk bridge resolves `from` automatically from the calling local agent/instance and uses mandatory `to` in exact `<hostname>:<pid>` form. |
| `docs/README.md` | Keep the PRD index entry aligned with implemented status and renamed file path. |

## Edge Cases

- a talk request is sent by the local `main` agent on instance `node-1:42` with `to = node-2:77` → sender-side logging should resolve `from` as `node-1:42` and show `to` as `node-2:77`.
- a talk request includes optional target-agent metadata such as `to_agent_id` → that metadata must not change the exact `<hostname>:<pid>` values used for `from` or `to`.
- the tool caller does not supply any sender fields → the sender-side runtime should still resolve `from` automatically from the calling local instance.
- a non-talk path such as task routing does not have meaningful talk identifiers → no sender-side Stylos talk log line should be emitted for that path.
- future multi-agent-per-process work wants richer sender display → that richer information must live outside these exact identifier fields.

## Migration

This is a sender-side logging and identifier-semantics clarification for outbound Stylos talk.

Migration expectations:

- public `stylos_request_talk` callers should stop supplying sender fields and instead provide only the target instance plus optional target agent metadata
- the sender-side runtime now resolves `from` automatically and uses mandatory `to` as the addressed instance identifier
- sender-side chat-panel visibility should rely on the dedicated `Stylos talk to=<hostname>:<pid> from=<hostname>:<pid>` log format rather than generic tool-call text alone
- any prior local usage that expected caller-supplied sender fields or agent-decorated sender formatting must be updated to the exact `<hostname>:<pid>` rule
- no database, provider, or protocol namespace migration is required

## Testing

- send a Stylos talk from the local `main` agent on `node-1:42` to `to = node-2:77` without passing sender fields → verify: the sender-side runtime resolves `from = node-1:42` and `to = node-2:77`.
- send a Stylos talk with optional `to_agent_id` populated → verify: the sender-side chat panel still shows `from` and `to` as exact `<hostname>:<pid>` values with no suffixes or decorations.
- invoke `stylos_request_talk` successfully from a local agent turn → verify: the sender-side chat panel shows `Stylos talk to=<hostname>:<pid> from=<hostname>:<pid>`.
- inspect implementation for any slash-delimited or agent-decorated identifier output in this sender-side path → verify: none appears in `from` or `to`.
- send a non-talk request such as a task request → verify: no sender-side Stylos talk log entry is emitted for that path.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: Stylos-enabled CLI builds compile cleanly with the exact identifier semantics.

## Implementation checklist

- [x] resolve sender-side `from` automatically from the calling local instance
- [x] require sender-side `to` as an exact `<hostname>:<pid>` identifier
- [x] emit a sender-side chat-panel log entry `Stylos talk to=<identifier> from=<identifier>` when outbound talk is accepted
- [x] ensure neither sender-side field ever uses `<hostname>:<pid>/<agent_id>` or any other alternate format
- [x] preserve compatible exact-identifier semantics in related receiver-side payload fields where applicable
- [x] document the exact sender-side identifier rule in `docs/architecture.md`
- [x] document the sender-side derivation and logging path in `docs/engine-runtime.md`
- [x] update this PRD and `docs/README.md` when implementation lands
