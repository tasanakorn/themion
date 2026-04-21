# PRD-028: Receiver-Side Stylos Talk Logging Should Not Duplicate `hear` and `talk`

- **Status:** Implemented
- **Version:** v0.15.2
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-21

## Summary

- Fix a receiver-side Stylos logging bug where one inbound talk currently produces two visible chat lines.
- When a local agent receives a Stylos talk, the UI now shows one `Stylos hear ...` line built directly from the inbound payload fields.
- The receiver-side line preserves the payload fields as `from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id>`.
- The receiver no longer emits a second receiver-side `Stylos talk ...` line for that same inbound event.
- Sender-side outbound logging remains unchanged: accepted outbound talk still shows `Stylos talk to=<hostname>:<pid> from=<hostname>:<pid>`.
- This landed as a patch-level behavior correction because it removes duplicate UI noise rather than adding a new capability.

## Goals

- Eliminate duplicate receiver-side Stylos chat-panel entries for a single inbound talk event.
- Preserve one clear receiver-side indication that a peer message was heard.
- Make the receiver-side `hear` line preserve the inbound payload identity fields directly.
- Preserve sender-side outbound `Stylos talk ...` logging semantics for accepted local outbound requests.
- Clarify the intended separation between inbound receiver-side logging and outbound sender-side logging so future changes do not reintroduce duplication.

## Non-goals

- No redesign of Stylos talk transport, delivery, or acknowledgement semantics.
- No change to the injected peer-message wrapper or `***QRU***` reply guidance.
- No change to the exact `<hostname>:<pid>` identifier format already used for instance identifiers in payload fields.
- No new TUI conversation view, transcript mode, or Stylos-specific filtering UI.
- No change to task request, task status, or task result logging.

## Background & Motivation

### Current state

The current Stylos talk design distinguishes two user-visible directions:

- sender-side accepted outbound talk logs as `Stylos talk to=<hostname>:<pid> from=<hostname>:<pid>`
- receiver-side inbound talk logs as `Stylos hear ...`

That separation matches the intended mental model:

- `talk` is an outbound send action initiated locally
- `hear` is an inbound receive event observed locally

The reported bug was that when a Stylos receiver got a talk, the UI showed both of these lines for the same inbound event:

- `󰀂 Stylos hear from=vm-02:771811 to=main`
- `󰀂 Stylos talk from=vm-02:771811 to=vm-02:771876`

This was noisy and misleading on the receiving side because the second line looked like a local outbound send even though the user was only observing an inbound message.

At the same time, the receiver-side `hear` line should not throw away payload information. The desired receiver-side line preserves the inbound payload fields directly:

- `Stylos hear from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id>`

This keeps the receiver-side entry faithful to the payload while still remaining clearly inbound.

**Alternative considered:** keep both lines because they expose more metadata. Rejected: the receiver-side event should not appear as both an inbound hear event and an outbound talk event; the `hear` line itself should carry the needed payload metadata.

### Why this is a bug fix rather than a new feature

The outbound `talk` line was already meant for sender-side accepted sends, and the receiver already had a distinct `hear` concept. The problem was that the implementation surfaced one inbound event through both pathways.

The required fix was to keep one receiver-side `hear` event, but make that single line complete enough to reflect the payload fields without needing an additional `talk` line.

**Alternative considered:** redefine receiver behavior so both `hear` and `talk` are expected. Rejected: that weakens the sender/receiver distinction and makes the chat panel harder to interpret.

## Design

### Keep one receiver-side inbound line built from payload fields

When a local agent receives a Stylos talk from a remote instance, the receiver-side TUI should emit exactly one Stylos receive log line for that inbound event:

- `Stylos hear from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id>`

Normative behavior:

- receiver-side inbound talk logging must use `hear`, not `talk`
- the receiver-side log for a single inbound talk event must be emitted once
- the receiver-side line must preserve the payload identity fields directly in this order: `from`, `from_agent_id`, `to`, `to_agent_id`
- `from` and `to` should use the exact values already carried in the inbound payload
- `from_agent_id` and `to_agent_id` should use the exact agent IDs already carried in the inbound payload
- do not silently replace payload fields with locally inferred display-only substitutions when the payload already provides the values
- if any field is absent in a compatibility path, the implementation may fall back to the closest truthful available value, but the normal Themion-to-Themion path should preserve the actual payload fields

This keeps the receiver-side entry faithful to the event that arrived.

**Alternative considered:** keep receiver-side `hear` minimal as only `from=<hostname>:<pid>`. Rejected: the user explicitly wanted the receiver-side line to reflect the payload fields, including agent IDs and destination fields.

### Restrict `Stylos talk ...` to sender-side accepted outbound requests

`Stylos talk ...` should remain reserved for the local instance acting as the sender of an accepted outbound `stylos_request_talk` call.

Normative behavior:

- emit `Stylos talk ...` only on the initiating side of an outbound accepted talk request
- do not emit `Stylos talk ...` on the receiver merely because an inbound talk is injected into a local agent turn
- if the receiver later chooses to reply through a separate outbound `stylos_request_talk`, that reply is a new outbound event and may emit its own sender-side `Stylos talk ...` line
- the sender-side/outbound and receiver-side/inbound log paths must remain semantically distinct even when they refer to the same peer conversation

This ensures `talk` remains an action line and `hear` remains an observation line.

**Alternative considered:** allow receiver-side `talk` lines when they include different `to`/`from` fields. Rejected: the semantic confusion remains even if the fields differ.

### Deduplicate at the event-source boundary

The implementation should suppress duplication at the narrowest practical boundary.

Possible sources include:

- a receiver-side transport event being logged directly as `hear`
- the same inbound event later being mirrored through a generic Stylos talk logging path
- a local injected prompt or tool-bridge event accidentally reusing the outbound logging formatter

Normative behavior:

- identify the specific receiver-side code path that emits the extra `Stylos talk ...` line
- prevent that path from logging outbound-style talk text for inbound receives
- prefer a targeted guard or event-shape distinction over a broad TUI log filter, unless the current architecture makes a transcript-layer dedupe significantly safer
- do not suppress legitimate sender-side outbound talk events originating from the local instance
- preserve the receiver-side payload-derived `hear` line while removing only the duplicate outbound-style line

This fixes the root cause without muting valid logs.

**Alternative considered:** deduplicate purely by string comparison in the transcript renderer. Rejected: that is brittle and risks hiding legitimate repeated events.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Ensure receiver-side inbound Stylos messages render only one `Stylos hear from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id>` line and do not also surface as `Stylos talk ...` for the same event. |
| `crates/themion-cli/src/stylos.rs` | Distinguish inbound receive events from outbound accepted-talk events by carrying `from_agent_id` and `to_agent_id` through the remote prompt bridge so the TUI can log the payload fields directly. |
| `docs/architecture.md` | Clarify that receiver-side `hear` logging reflects inbound payload fields and remains distinct from sender-side outbound `talk` logging if wording needs a small update. |
| `docs/README.md` | Add this PRD to the index table and keep status aligned with implementation. |

## Edge Cases

- one remote instance sends a talk to local agent `main` → verify receiver-side UI shows one inbound `Stylos hear from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id>` line, not an additional `Stylos talk ...` line.
- the local agent receives an inbound talk and then sends a manual or model-generated reply → verify the inbound event shows as `hear` with payload fields preserved, while the later local reply may separately show as outbound `talk`.
- two remote peers send separate inbound talks in quick succession → verify each inbound event yields one `hear` line; dedupe logic must not collapse distinct events.
- the same remote peer sends repeated messages with identical text → verify each distinct inbound delivery can still surface once as `hear`; dedupe must be event-based, not content-based.
- a compatibility caller omits one of the agent-id fields → verify the receiver-side line uses truthful fallback behavior only for the missing field and does not emit a duplicate `talk` line.
- sender-side accepted outbound talk from the local instance to a remote peer → verify the existing `Stylos talk ...` line still appears on the sender side.

## Migration

This is a UI behavior correction with no protocol or storage migration.

Migration expectations:

- no change to the public `stylos_request_talk` tool schema
- no change to the meaning of payload identity fields
- transcript consumers should expect one receiver-side `hear` line per inbound talk instead of duplicated `hear` plus `talk` lines
- the receiver-side `hear` line preserves payload fields more completely than before
- no database or compatibility migration is required

## Testing

- send one Stylos talk from remote instance `vm-02:771811` with payload fields `from`, `from_agent_id`, `to`, and `to_agent_id` to a local receiver agent → verify: the receiver-side transcript shows exactly `Stylos hear from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id>` and does not show a second `Stylos talk ...` line for that same inbound event.
- send an outbound Stylos talk from the local instance to a remote peer → verify: the sender-side transcript still shows `Stylos talk to=<hostname>:<pid> from=<hostname>:<pid>`.
- receive an inbound talk and then send a reply from the local instance → verify: the inbound event is logged as `hear` with payload fields preserved, and the separate outbound reply is logged as `talk`.
- receive two inbound talks from the same remote peer in sequence → verify: two `hear` lines appear, one per delivery, without suppression of legitimate distinct events.
- exercise any compatibility path with a missing agent-id field if such a path is still supported → verify: fallback behavior is truthful and still produces one `hear` line only.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the Stylos-enabled CLI compiles cleanly with the corrected logging behavior.

## Implementation checklist

- [x] identify the receiver-side code path that emits duplicate `hear` and `talk` entries for one inbound Stylos talk
- [x] prevent inbound receive events from using the outbound `Stylos talk ...` logging path
- [x] preserve one receiver-side `Stylos hear from=<from> from_agent_id=<from_agent_id> to=<to> to_agent_id=<to_agent_id>` transcript entry per inbound talk delivery
- [x] ensure the receiver-side `hear` line uses payload values directly in the normal Themion-to-Themion path
- [x] preserve sender-side outbound `Stylos talk ...` transcript behavior for accepted local sends
- [x] update any nearby docs if a wording clarification is needed
- [x] update this PRD and `docs/README.md` when implementation lands
