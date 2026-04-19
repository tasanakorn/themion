# PRD-012: Human-Friendly Statusline Token Units

- **Status:** Implemented
- **Version:** v0.6.1
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Make statusline token counters easier to scan by replacing comma-separated raw numbers with compact human-friendly units.
- Apply the compact formatting to the cumulative `in`, `out`, and `cached` counters shown in the TUI statusline.
- Keep the change narrow and presentation-focused, without changing how token accounting is stored or computed.
- Preserve consistency with the existing compact `ctx:<used>/<limit>` display style.

## Non-goals

- No change to the underlying token usage values stored in runtime state, events, or the database.
- No redesign of the statusline layout beyond the token-count formatting.
- No change to turn summary/stat lines outside the statusline unless explicitly implemented as a follow-up.
- No localization or configurable unit style in this PRD.
- No changes to rate-limit rendering.

## Background & Motivation

### Current state

The TUI statusline currently renders cumulative token counters in raw decimal form with thousands separators:

- `in:2,433`
- `out:11`
- `cached:0`

This is accurate, but the comma-separated format is visually noisy in a very compact UI region. The statusline already uses abbreviated token formatting for context display, such as `ctx:2k/?` and `ctx:12k/400k`, which makes the cumulative counters feel inconsistent by comparison.

Because the statusline is intended for fast scanning during interactive use, the most useful representation is usually approximate magnitude rather than full punctuation-preserved precision. `in:24k` is easier to parse at a glance than `in:24,433`, especially when the bar also includes workflow, phase, model, and rate-limit information.

## Design

### Compact token unit formatting

The TUI statusline should render the cumulative `in`, `out`, and `cached` token counters using short human-friendly units rather than comma-separated raw numbers.

Normative examples:

- `11` → `11`
- `999` → `999`
- `1,024` → `1k`
- `2,433` → `2k`
- `24,433` → `24k`
- `999,999` → `999k`
- `1,200,000` → `1m`

For the first implementation, integer whole-unit formatting is preferred over decimals so the display stays short and stable during rapid updates.

**Alternative considered:** keep comma-separated exact numbers. Rejected: exact punctuation is less scannable in the constrained statusline and is inconsistent with the compact `ctx` display already used elsewhere.

### Shared compact-formatting behavior within the statusline

The existing statusline already uses compact formatting logic for `ctx:<used>/<limit>`, but the `in`, `out`, and `cached` counters use a separate comma-separator formatter. This PRD proposes aligning the statusline on one compact-number presentation style.

Implementation should prefer a single helper for human-friendly count formatting so all token-oriented statusline fields use the same thresholds and suffixes where appropriate.

This does not require changing every stats surface in the application. It applies specifically to the statusline counters and any directly shared helper that powers them.

**Alternative considered:** add a second compact formatter only for `in/out/cached` and leave `ctx` formatting separate. Rejected: the formatting rules are conceptually the same, and duplicating them would make future adjustments harder to keep consistent.

### Rounding and suffix expectations

The initial suffix set should be minimal:

- no suffix below `1000`
- `k` for thousands
- `m` for millions

Rounding should favor compactness and predictability over exactness. Truncating to whole units is acceptable for the first version, so values stay visually stable and avoid fractional noise such as `2.4k` or `24.4k`.

Examples:

- `2,433` → `2k`
- `24,433` → `24k`
- `1,900,000` → `1m`

If future UX feedback shows that truncation hides too much useful detail, a later PRD can introduce decimal precision rules. This proposal intentionally keeps the formatting simple.

**Alternative considered:** use one decimal place such as `2.4k` or `24.4k`. Rejected: decimals improve precision but add width and visual churn, which works against the statusline's compact-scanning goal.

### Documentation alignment

The architecture docs currently describe statusline token counts as formatted with thousands separators. That wording should be updated to reflect compact human-friendly units once implementation lands.

Historical PRDs should remain historical records. If an implemented PRD's notes need a brief implementation note to clarify the currently shipped display, that should follow the repository's normal PRD policy rather than rewriting the original proposal as if it had always used compact units.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Replace the comma-separated statusline formatter for cumulative `in`, `out`, and `cached` counters with a compact human-friendly unit formatter, ideally shared with existing context-count formatting behavior. |
| `docs/architecture.md` | Update the TUI/statusline documentation so it describes compact human-friendly token units instead of thousands separators for statusline counts. |
| `docs/README.md` | Add this PRD to the index and keep its status aligned with implementation progress. |

## Edge Cases

- values below `1000` should remain un-suffixed so small counts stay exact and readable.
- values exactly on thresholds such as `1000` or `1_000_000` should format cleanly as `1k` and `1m`.
- large `cached` counts should use the same suffix logic as `in` and `out`, even when usually zero.
- `ctx` formatting should not regress or diverge from the shared compact-formatting rules if helpers are unified.
- truncation near thresholds such as `1999` should remain predictable and not flicker between formats unexpectedly within a single update path.
- narrow terminals should benefit from the shorter counters without introducing line wrapping or layout changes.

## Migration

This change is additive and presentation-only.

No database migration, config migration, or session migration is required. Existing token accounting remains unchanged; only the TUI statusline representation becomes more compact after upgrade.

## Testing

- start the TUI with session totals below `1000` → verify: statusline shows exact unsuffixed values such as `in:11` and `out:42`.
- reach a cumulative input total above `1000` → verify: statusline shows compact `k` units such as `in:2k` instead of comma-separated `in:2,433`.
- reach a cumulative total above `1_000_000` in a formatter test or synthetic state → verify: statusline uses `m` units such as `in:1m`.
- inspect `ctx` rendering after the formatter change → verify: context display remains compact and stylistically consistent with `in/out/cached`.
- compare the updated statusline on a narrow terminal width → verify: the shorter token counters reduce visual crowding without changing layout semantics.
- inspect docs after implementation → verify: `docs/architecture.md` no longer claims statusline token counts use thousands separators.
- run `cargo check -p themion-cli` after implementation → verify: the statusline formatting change compiles cleanly.
