# PRD-085: Show Clear Source Labels on Transcript Messages Without an Agent Owner

- **Status:** Draft
- **Version:** >v0.55.0 +patch
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-05-01

## Summary

- Some TUI transcript lines show system or event output without making their origin obvious at a glance.
- Keep the current agent-associated presentation for any transcript line that is owned by or attached to a specific local agent.
- Do not override agent-associated events such as turn lifecycle lines just because their wording looks runtime-like.
- For transcript messages without a specific agent owner, show a short visible source label before the message text.
- Use consistent wording and color for each non-agent source category so users can learn the transcript quickly.
- Preserve the rest of the line as much as practical instead of turning this into a general transcript rewrite.

## Goals

- Make transcript lines easier to scan when they are not spoken by, or directly owned by, a specific local agent.
- Show a clear visible label that tells the user where that message comes from.
- Use stable wording and visual treatment for common non-agent-origin message categories.
- Keep agent-owned messages visually distinct from non-agent-owned system or event messages.
- Avoid regressions where agent-associated event lines lose their current agent prefix or become reclassified as generic system events.
- Improve clarity without broadly rewriting the rest of each transcript line.

## Non-goals

- No full transcript wording redesign.
- No attempt to humanize or rewrite every metadata-heavy event line.
- No changes to board semantics, Stylos semantics, runtime semantics, or persistence formats.
- No requirement to remove existing metadata fields such as `note_slug`, `column`, `from`, or `to`.
- No requirement to introduce a large theme system or user-custom label palette in this PRD.
- No requirement to relabel agent-associated turn, status, or progress lines that already belong to a specific agent.

## Background & Motivation

Some transcript lines currently appear in the TUI without a clear indication of what kind of thing produced them.

A concrete trigger was a line of the form:

- `Board note posted note_slug=... column=todo`

That line may be technically correct, but in the transcript it does not clearly communicate whether it is:

- a message from an agent
- a board event
- a Stylos or network event
- a local runtime or status event
- some other system-originated event

The product problem is not that the underlying information is missing. The problem is that the transcript does not clearly label the origin of messages that do not belong to any one agent.

The product should therefore make the source of non-agent-owned transcript entries obvious at a glance.

This distinction matters because some lines may describe runtime-like activity but still be associated with a specific agent. For example, a line like `[master] 󰇺 turn 1 started` should remain agent-associated because it belongs to `master`, even though the wording describes a turn lifecycle event.

A label-only solution is better than no label, but consistent wording and color are part of the scanability requirement. If the same kind of event appears under different names or with inconsistent visual treatment, the user still has to stop and re-interpret the transcript.

## Design

### 1. Ownership decides whether agent presentation stays

When a transcript message is directly associated with a specific local agent, the product should continue using the normal agent-associated presentation.

The first display decision is ownership:

- if the message belongs to a specific local agent, keep the existing agent-associated display model
- if the message does not belong to any one local agent, show a source label

This keeps the transcript distinction centered on ownership and origin, which is the actual user-facing problem.

Examples that must remain agent-associated if they belong to one agent:

- `[master] 󰇺 turn 1 started`
- `[smith-2] tool call started`
- `[master] waiting for provider response`

These lines should not be relabeled as `RUNTIME`, `SYSTEM`, or any other non-agent source category merely because their wording describes lifecycle, progress, or status activity.

### 2. Non-agent-owned messages must show a source label

When a transcript message is not directly associated with any specific local agent, the product must show a short visible source label before the message text.

That source label must tell the user what kind of origin produced the message. Examples include:

- board-originated events
- Stylos or network-originated events
- local runtime or status events that are not owned by one agent
- watchdog or background follow-up events

The exact label words should be short, readable, and consistent across the TUI. The important requirement is that the user can tell what the message is from without needing to infer it from the rest of the line.

The product must not rely on a generic placeholder token. The label must convey origin meaning.

### 3. Each non-agent source category should use stable wording and color

For non-agent-owned transcript messages, the product should use one default display label per source category and keep that wording stable.

The product should also assign a stable color treatment per source category so users can visually scan repeated event types more quickly.

This PRD does not require an advanced theming system. It requires a consistent default mapping.

Recommended default mapping:

| Source category | Default label text | Color intent | Notes |
| --- | --- | --- | --- |
| Board events | `BOARD` | yellow or amber | For board note creation, movement, completion, or other board-originated status lines. |
| Stylos or network events | `STYLOS` | cyan or blue | For remote agent, routing, network, or peer-message related transcript lines. |
| Local runtime or status events without an agent owner | `RUNTIME` | magenta or purple | For local runtime lifecycle, mode, session, or orchestration status lines that are not owned by one agent. |
| Watchdog or background follow-up events | `WATCHDOG` | red or light red | For watchdog notices, follow-up reminders, timeout-related supervision, or background intervention events. |
| Other uncategorized system events | `SYSTEM` | gray | Fallback only when the event is non-agent-owned but does not fit a more specific known category. |

The labels should be uppercase by default to make the source marker compact and easy to distinguish from the body text.

If implementation constraints make the exact named colors unavailable, the product should choose the nearest existing TUI palette colors that preserve the same semantic distinction and relative contrast.

### 4. The label should be visually separate from the body text

For non-agent-owned lines, the source label should appear before the message body as a compact prefix-like marker.

Recommended presentation shape:

- source label first
- body text second
- existing metadata or event detail after that, using the current line style unless a small readability adjustment is needed

Example shapes for non-agent-owned messages:

- `BOARD Board note posted note_slug=triage column=todo`
- `STYLOS Routed task to vm-03:smith-2`
- `RUNTIME Session resumed for workspace-coder`
- `WATCHDOG Follow-up reminder for stalled task`
- `SYSTEM Local maintenance event ...`

Counter-example that must stay agent-associated rather than being reformatted into a non-agent label:

- `[master] 󰇺 turn 1 started`

The label should remain short enough that it improves scanning rather than dominating the line.

### 5. Preserve the rest of the line unless a small adjustment is needed

After adding the source label to non-agent-owned messages, the product should preserve the current message body and metadata as much as practical.

For example, a board-related line may continue to include details such as `note_slug` and `column`. A Stylos-related line may continue to include `from` and `to` details.

This PRD is about improving source clarity for non-agent-owned lines, not about rewriting the information structure of every event.

### 6. Keep this as a TUI presentation concern

This behavior belongs in the `themion-cli` transcript presentation layer.

If the existing event or transcript pathways already carry enough information to determine ownership and source, the implementation should use that information directly rather than introducing new protocol or persistence shapes.

The implementation should classify lines by ownership first and only apply non-agent source labeling after confirming that no specific agent owns the line.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Add the display rule that shows a clear source label on transcript messages without a specific agent owner, including stable wording and color per non-agent source category, while preserving the current agent-associated presentation path. |
| `crates/themion-cli/src/app_runtime.rs` | Adjust only if a small helper is useful for surfacing ownership and source cleanly to the presentation layer. |
| `crates/themion-cli/src/stylos.rs` | Preserve current event facts and message content unless a minimal compatibility adjustment is needed for clearer source labeling. |
| `docs/README.md` | Keep the PRD title and status aligned with this product requirement. |

## Edge Cases

- a runtime-shaped message still belongs to one local agent, such as `[master] 󰇺 turn 1 started` → verify: it keeps agent-associated presentation and does not receive a non-agent source label.
- a board event has no single agent owner → verify: it shows the `BOARD` label with the board-event color treatment.
- a Stylos or network event has no single agent owner → verify: it shows the `STYLOS` label with the Stylos-event color treatment.
- a local runtime or orchestration event has no single agent owner → verify: it shows the `RUNTIME` label with the runtime-event color treatment.
- a background or watchdog event has no single agent owner → verify: it shows the `WATCHDOG` label with the watchdog-event color treatment.
- a non-agent-owned event does not match any currently recognized category → verify: it falls back to `SYSTEM` instead of showing no label.
- a long metadata-heavy non-agent message gains a source label → verify: the line becomes easier to identify without losing important existing detail.
- the TUI palette lacks the exact preferred named color → verify: the nearest existing color with the same visual role is used consistently.

## Migration

This is a presentation-only change with no data or protocol migration.

Recommended rollout shape:

- identify transcript entries that have a specific agent owner versus those that do not
- preserve the existing agent-associated format for owned lines
- identify the visible source category for the non-agent-owned entries
- map each known non-agent source category to a stable default label and color
- render only those non-agent-owned entries with the mapped source label before the existing message text
- preserve the rest of the message text unless a tiny local formatting adjustment is needed for readability

## Testing

- emit a transcript message directly associated with one local agent, such as `[master] 󰇺 turn 1 started` → verify: it uses the normal agent-associated presentation.
- emit another agent-owned lifecycle or status line → verify: it still uses the agent-associated presentation and is not relabeled as `RUNTIME` or `SYSTEM`.
- emit a board-related transcript message without a specific agent owner → verify: it shows the `BOARD` label and the board-event color treatment.
- emit a Stylos or network transcript message without a specific agent owner → verify: it shows the `STYLOS` label and the Stylos-event color treatment.
- emit a runtime transcript message without a specific agent owner → verify: it shows the `RUNTIME` label and the runtime-event color treatment.
- emit a watchdog transcript message without a specific agent owner → verify: it shows the `WATCHDOG` label and the watchdog-event color treatment.
- emit an uncategorized non-agent-owned system event → verify: it shows the `SYSTEM` fallback label and fallback color treatment.
- compare before and after transcript output for affected lines → verify: non-agent source becomes obvious at a glance while agent-owned lines keep their current identity and event details remain available.

## Implementation checklist

- [ ] identify which transcript entries are agent-owned versus non-agent-owned
- [ ] preserve the current agent-associated presentation for any owned line, including agent-owned lifecycle and turn events
- [ ] identify the source categories that must be visible in the transcript for non-agent-owned entries
- [ ] define stable label wording and color mapping for those non-agent source categories
- [ ] render only non-agent-owned transcript entries with those source labels and colors
- [ ] add a fallback category for uncategorized non-agent-owned system events
- [ ] preserve existing message detail as much as practical
- [ ] keep `docs/README.md` and this PRD aligned with the final product wording
