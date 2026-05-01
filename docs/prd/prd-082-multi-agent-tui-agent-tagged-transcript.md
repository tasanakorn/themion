# PRD-082: Multi-Agent TUI Agent-Tagged Transcript and Event Highlighting

- **Status:** Implemented
- **Version:** v0.54.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-05-01

## Summary

- Themion can now host multiple local agents and route work to them, but the TUI transcript still reads too much like a single-agent surface.
- The TUI should visibly attribute every message, notification, and runtime event to the responsible local agent when one is known.
- Add a consistent `[agent_id]` prefix with highlight color so multi-agent activity is scannable during concurrent or interleaved work.
- Keep the underlying runtime, board, and Stylos semantics unchanged; this PRD is about transcript clarity and operator usability.
- Agent-produced and agent-targeted transcript lines should always show a local `[agent_id]` tag, while genuinely process-level lines should stay neutral and untagged in this slice.

## Goals

- Make the TUI transcript visibly multi-agent rather than implicitly single-agent.
- Attribute each agent-originated message, targeted notify line, tool/event line, and other runtime-visible transcript item with the responsible local `agent_id` when that attribution is available.
- Use highlight color so agent tags are easy to scan even when messages from multiple agents are interleaved.
- Preserve readability for single-agent sessions while improving clarity for multi-agent local operation.
- Keep the tagging model consistent across normal assistant output, remote/intake events, tool activity, and completion/status lines where a specific local agent is responsible.

## Non-goals

- No redesign of the underlying multi-agent runtime, scheduling, board workflow, or Stylos protocol.
- No requirement in this PRD to add a full split-pane or per-agent transcript view.
- No requirement to add persistent user-configurable color themes or arbitrary per-agent custom palettes in this slice.
- No requirement to retroactively rewrite persisted historical transcript rows in SQLite.
- No requirement to expose remote instance identity styling beyond the local `agent_id` attribution needed for TUI clarity.

## Background & Motivation

### Current state

Current docs already describe Themion as an event-driven TUI over a process that may host multiple local agents. The CLI runtime owns local agent descriptors such as `agent_id`, `label`, and `roles`, and Stylos plus board-note flows can already target a specific local `agent_id`.

That runtime model is now strong enough that multiple local agents may be present and may produce interleaved visible activity. However, the transcript presentation still largely assumes a single foreground agent. Messages and event lines often appear without a clear local-agent attribution marker, which makes it harder to understand:

- which local agent produced a given assistant reply
- which agent received a board-note or Stylos-targeted request
- which agent is running a tool or finishing a turn
- whether two adjacent transcript items belong to the same worker or different workers

This is now a product usability problem rather than only an implementation detail. Once local multi-agent work is possible, the operator needs immediate visual attribution in the transcript itself.

### Why start with transcript attribution first

A full multi-agent TUI redesign could grow large quickly. The smallest high-value step is to make the existing transcript self-identifying.

A stable `[agent_id]` prefix with highlight color improves real-time scanning without forcing a new layout model. It also creates a reusable presentation primitive that later PRDs can build on for richer per-agent status bars, filters, panes, or timeline views.

## Design

### 1. Add a consistent `[agent_id]` prefix for agent-attributed transcript items

The TUI should prefix each agent-attributed visible line with a bracketed local agent identifier in the form `[agent_id]`.

Required behavior:

- assistant responses produced by a specific local agent should render with a visible `[agent_id]` prefix
- tool-start, tool-end, and other per-turn runtime transcript items should render with the same prefix when they belong to a specific local agent
- status, completion, interruption, intake, rejection, and follow-up lines tied to a specific local agent should also use the same attribution style
- remote/intake events that target a known local agent should include that agent's tag in the displayed line even if the upstream event text already names sender or destination metadata
- ordinary local user-submitted input lines may remain untagged in this slice because they represent the shared operator rather than one local agent
- if a transcript item genuinely reflects process-level state rather than one local agent, the TUI should keep it neutral and untagged in this slice rather than inventing a misleading agent tag
- wrapped or multiline rendering of one transcript entry should show the same visual tag ownership for the whole visible entry rather than appearing to switch agents mid-entry

This creates one visible attribution pattern instead of separate ad hoc formatting rules per entry type.

**Alternative considered:** add agent names only to selected assistant replies and leave events unchanged. Rejected: the usability problem comes from interleaving across all visible transcript activity, not only final assistant text.

### 2. Use highlight color for the agent tag, not only plain text

The `[agent_id]` prefix should be color-highlighted so it remains scannable in a fast-moving transcript.

Required behavior:

- the tag itself should render in a visually distinct highlight color relative to surrounding transcript text
- different agents should receive stable per-agent color assignment within the current session so the same `agent_id` does not visually drift line to line
- the color treatment should prioritize readability on the existing terminal surface rather than decorative intensity
- the first implementation should use a small fixed readable palette with deterministic assignment, not random or user-configured colors
- the text following the tag may keep the current role-based or entry-type styling unless implementation finds a small extension improves readability without overwhelming the transcript

This keeps the design simple: the tag carries the agent identity, and color makes that identity quick to spot.

**Alternative considered:** rely on plain `[agent_id]` text with no color differentiation. Rejected: plain text helps, but color materially improves scan speed when several agents interleave similar-looking events.

### 3. Keep color assignment deterministic and low-complexity

The first slice should use a simple deterministic agent-color policy.

Required behavior:

- each local `agent_id` should map to a stable color for the active session
- the first implementation should assign colors from a small fixed palette by deterministic local roster order so the same visible team ordering gets the same tag colors during the session
- the palette should be intentionally bounded and chosen for terminal readability
- if more agents exist than the preferred palette size, reuse should remain deterministic rather than random
- the initial design should avoid introducing a large color-configuration surface unless a later PRD explicitly expands that area

This keeps the feature predictable without over-designing theming.

**Alternative considered:** let colors depend on transient execution order or message sequence. Rejected: unstable color identity would make the transcript harder rather than easier to scan.

### 4. Preserve single-agent readability and backward compatibility

The tagging behavior should improve multi-agent clarity without making ordinary single-agent use feel noisy.

Required behavior:

- the tag format should stay compact so single-agent sessions remain readable
- the leader/interactive agent should use the same tagging style as worker agents rather than a special one-off format
- single-agent sessions should still show `[master]` on agent-attributed lines in this slice so the interface stays consistent and teaches the same model in both single-agent and multi-agent use
- existing transcript semantics such as message order, streaming behavior, and entry categorization should remain intact
- if there are specific entry types where showing a tag would create obvious clutter without useful attribution, those exceptions should be documented explicitly rather than left inconsistent by accident

This keeps the change additive and understandable.

**Alternative considered:** show tags only when more than one local agent currently exists. Rejected: always-visible attribution is simpler, avoids presentation mode switching, and helps users learn the local team model.

### 5. Carry agent attribution through the TUI entry model explicitly

The TUI should treat agent attribution as structured presentation state, not as ad hoc string concatenation sprinkled across call sites.

Required behavior:

- transcript entry types should carry optional local-agent attribution where relevant
- event-handling paths that already know the responsible agent should pass that attribution into the stored entry representation
- rendering should format the visible `[agent_id]` tag from structured entry metadata rather than parsing human text back out later
- process-level or unattributed events should remain representable without a fake agent id

This gives the TUI a durable foundation for later multi-agent presentation improvements.

**Alternative considered:** prepend `[agent_id]` directly into message strings at the point of logging. Rejected: that would entangle storage, formatting, and future rendering changes unnecessarily.

### 6. Keep remote sender metadata separate from local agent attribution

Some event lines already contain sender/receiver metadata from Stylos or board-note flows. Local agent attribution should complement that metadata rather than replace it.

Required behavior:

- when an event is shown because a specific local agent received or processed it, the local `[agent_id]` tag should indicate which local team member the line belongs to
- any existing sender-side metadata such as remote instance or `from_agent_id` should remain in the event body when it is already useful
- the TUI should avoid conflating remote sender identity with the local responsible agent; both may matter at once

This keeps the transcript useful for both local team reasoning and network-delivery tracing.

**Alternative considered:** show only remote sender/destination metadata and skip a local tag. Rejected: transport metadata does not reliably answer which local agent owns the visible line.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Extend transcript entry modeling, event ingestion, and rendering so agent-attributed entries carry structured local `agent_id` metadata and render with a visible `[agent_id]` prefix plus stable highlight color. |
| `crates/themion-cli/src/stylos.rs` | Pass through or preserve any local-target attribution already known at the TUI boundary where Stylos-delivered events or requests resolve to a local agent. |
| `crates/themion-cli/src/tui_runner.rs` | Update only if terminal/render integration needs a small helper for color/style support; avoid moving runtime policy here. |
| `docs/architecture.md` | Document the TUI transcript attribution model as part of the local multi-agent presentation layer once implemented. |
| `docs/engine-runtime.md` | Document how local agent attribution reaches visible transcript entries without changing core harness semantics. |
| `docs/README.md` | Add the new PRD entry and later reflect landed status when implemented. |

## Edge Cases

- one local agent only → verify: transcript remains readable with compact `[agent_id]` tagging and no confusing extra noise.
- two local agents produce interleaved assistant and tool events → verify: every attributed line shows the correct tag and stable color for its agent.
- a process-level status line has no meaningful owning agent → verify: the TUI uses a neutral shared presentation rather than assigning a misleading tag.
- a Stylos or board-note event names a remote sender and targets a local worker → verify: the displayed line keeps useful remote metadata while also showing the local worker tag.
- more local agents exist than unique preferred colors in the palette → verify: color reuse is deterministic and tags remain readable.
- an agent streams a long assistant response over multiple chunks → verify: the visible streaming line keeps the same agent tag throughout the stream lifecycle.
- a targeted incoming prompt is rejected because the target agent is missing or busy → verify: the rejection line is tagged consistently when a responsible local target exists, or clearly falls back to neutral process-level presentation when it does not.

## Migration

This change is presentation-local to the TUI and should not require a database migration.

Rollout guidance:

- keep existing transcript ordering and event semantics
- add structured local-agent attribution to TUI entries where the runtime already knows ownership
- render compact highlighted `[agent_id]` prefixes without changing persisted history formats

## Testing

- start Themion with one local agent and submit a normal prompt → verify: assistant, tool, and turn-complete lines show a compact highlighted `[master]` tag.
- create an additional local worker and route targeted work to it → verify: the worker's transcript lines show `[worker-id]` with a stable distinct color from `[master]`.
- trigger interleaved local activity across two agents → verify: visible transcript items remain attributable by both tag text and color.
- inject a board note or Stylos-targeted request for a specific local agent → verify: intake and follow-up lines show the correct local agent tag while preserving useful remote metadata.
- render process-level status updates such as startup or shared shell/login flows → verify: unattributed lines remain neutral and are not mislabeled as belonging to one agent.

Implementation status note: this PRD has landed in the TUI transcript layer. Agent-attributed assistant replies, tool lines, status lines, remote intake events, and turn-complete entries now carry structured local `agent_id` attribution and render with compact highlighted `[agent_id]` prefixes. Single-agent sessions still show `[master]`, ordinary user-input lines remain untagged, and process-level lines remain neutral in this slice. The first implementation uses a small deterministic roster-order palette for local agent tags.

## Implementation checklist

- [x] add structured optional local-agent attribution to relevant TUI entry types
- [x] thread known local-agent attribution through assistant, tool, status, remote-event, and turn-complete entry creation paths
- [x] implement deterministic per-agent tag color assignment for the active session
- [x] render transcript lines with compact highlighted `[agent_id]` prefixes when attribution is present
- [x] document the landed TUI multi-agent attribution behavior in `docs/architecture.md`, `docs/engine-runtime.md`, and `docs/README.md`
