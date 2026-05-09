# PRD-115: Limit TUI and Web UI Transcript Buffers

- **Status:** Implemented
- **Version:** v0.71.2
- **Scope:** `themion-cli`, `themion-cli-web-ui`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-09

## Summary

- The TUI and Web UI currently keep and render an unbounded in-memory transcript.
- Long sessions can make redraws, web transcript responses, browser rendering, and CPU usage grow without limit.
- Keep only the most recent `1,000` live transcript entries for UI display.
- Show one clear truncation marker when older visible rows were dropped.
- Limit only the TUI/Web UI live display buffer; prompt generation and persistent storage must not change.

## Goals

- Cap live transcript memory, redraw work, web payload size, and browser render work.
- Keep long-running TUI and Web UI sessions responsive after many messages, tool events, or streamed chunks.
- Use one shared server-side live transcript window so TUI and Web UI show the same recent rows.
- Preserve prompt generation, context replay, durable chat/history persistence, and Project Memory indexing exactly as they work today.
- Keep current prompt submission, streaming, tool display, and activity indicators working normally.
- Make truncation visible so users know the current UI is a recent live window, not the full session archive.

## Non-goals

- Do not delete records from SQLite history or Project Memory indexes.
- Do not change model prompt generation, history replay, context compaction, or `/clear` semantics.
- Do not add transcript search, paging, archive browsing, or “load older” behavior.
- Do not add a user config option in the first slice.
- Do not redesign the TUI or Web UI transcript layout.
- Do not make the browser own the canonical truncation policy.

## Background & Motivation

### Current state

The TUI keeps transcript rows in `App.entries: Vec<Entry>`. `App::push` appends every row, and TUI rendering builds lines from the whole vector for normal view and transcript review. The Web UI projection builds `WebChatEntry` values from the same TUI entries and stores them in an in-memory `Vec<WebChatEntry>` for `/api/transcript` and websocket refreshes.

This means the live UI transcript grows for the full process lifetime. In long sessions, every redraw and web transcript refresh can touch more rows. The UI can become slow, and CPU usage can become high even when the user only needs recent conversation context on screen.

The product needs a bounded live-display transcript only. Prompt generation and durable session data must continue to use their existing sources of truth. The live UI buffer is only the current interactive surface.

## Design

### 1. Keep a bounded live transcript window

Themion must keep at most `1,000` recent live transcript entries for UI display, plus one truncation marker when older rows were omitted.

Required behavior:

- define a constant default live transcript entry limit of `1,000`
- own the limit in the shared CLI-side transcript state used by both TUI and Web UI
- trim after entries are appended, not only during rendering
- keep the newest real transcript entries when the limit is exceeded
- allow at most one truncation marker in addition to the `1,000` real-entry window
- do not add a config setting in this first implementation

The first shipped limit is intentionally conservative. It should reduce worst-case redraw and browser work quickly without adding configuration complexity.

### 2. Track omitted rows with one marker

Users should know when the live transcript no longer includes the start of the session.

Required behavior:

- when older entries are dropped, show one compact marker near the start of the visible transcript
- include the total omitted real-entry count when available, for example `older transcript entries omitted: 1240`
- update the same marker count as more rows are trimmed
- never accumulate repeated marker rows
- make the marker visible in both TUI and Web UI transcript output
- keep the marker out of durable history and model prompt context unless those paths already intentionally consume UI transcript entries

The marker is part of the live UI view only. It is not a persisted chat message.

### 3. Preserve active rows and valid scroll state

Trimming must not break live interaction.

Required behavior:

- current prompt submission and response streaming remain unchanged
- if the current assistant response is streaming, keep its row in the visible window
- pending status/activity indicators continue to render normally
- auto-scroll-to-bottom follows the newest transcript rows
- transcript review opens within the bounded live window
- scroll and review offsets stay valid after trimming
- if the user is scrolled up while trimming happens, clamp the scroll offset rather than panicking or jumping outside the buffer

The implementation does not need to keep arbitrarily old rows just because the user is reviewing them. The bounded live window is the product behavior.

### 4. Keep Web UI projection aligned with the server window

The browser must receive the same bounded transcript window that the TUI uses.

Required behavior:

- `/api/transcript` returns only the bounded live transcript rows and optional marker
- websocket-triggered transcript refreshes rebuild from the bounded server-side window
- the Web UI cached `chat_entries` mirror the bounded projection
- browser-side virtualization may be added later, but it is not the source of truth for this PRD
- if a tool-call row was trimmed before its completion arrives, the completion may appear as a standalone tool-finished row rather than restoring old rows

This keeps the behavior consistent across local surfaces and avoids moving runtime policy into `themion-cli-web-ui`.

### 5. Keep prompt generation and persistence unchanged

The live UI cap must not affect any non-visual source of truth. It is a display-window limit only.

Required behavior:

- prompt generation must not read from the trimmed UI window if it currently uses a different history/context source
- model history replay and context construction keep their current source of truth
- SQLite session/history records remain intact
- chat message indexing and Project Memory behavior remain unchanged
- persisted agent turns, tool events, and chat messages are not deleted or shortened by this cap
- `/clear` still clears visible transcript and context according to current behavior; after `/clear`, omitted-count and marker state should reset with the visible transcript

**Alternative considered:** only virtualize the browser list. Rejected because the TUI would still slow down, `/api/transcript` would still ship unbounded payloads, and the browser would still receive growing data.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Add bounded display-entry handling in or near `App::push`; track omitted count, preserve one marker, and clamp scroll/review offsets after trimming without changing prompt/history persistence. |
| `crates/themion-cli/src/web.rs` | Ensure web transcript projection and cached `chat_entries` mirror the bounded live transcript and do not rebuild unbounded history. |
| `crates/themion-cli-web-ui/src/lib.rs` | Render the truncation marker clearly if it appears as a transcript row; keep auto-scroll behavior stable. |
| `crates/themion-cli-web-ui/assets/app.css` | Add small marker styling only if existing transcript row styles are not enough. |
| `docs/prd/prd-115-limit-tui-web-transcript-buffer.md` | Define the product behavior and constraints. |
| `docs/README.md` | List the new PRD. |

## Edge Cases

- transcript grows past `1,000` entries during streaming → verify: active response continues and the visible buffer remains bounded.
- entries are trimmed many times → verify: one marker remains and omitted count increases.
- many tool calls occur in a long session → verify: completed tool rows still merge when the original call remains in the buffer.
- a tool call is trimmed before completion → verify: completion is displayed safely without resurrecting old rows.
- user scrolls upward while trimming happens → verify: scroll offset stays valid and does not panic or point outside the bounded window.
- `/api/transcript` is called after a very long session → verify: response size is bounded and includes the truncation marker when rows were omitted.
- browser reconnects after trimming → verify: Web UI receives the same bounded transcript window as the TUI.
- `/clear` runs after trimming → verify: visible transcript and truncation state reset according to current clear semantics.

## Migration

This is an in-memory UI behavior change. Existing persistent history stays intact.

The default limit requires no user action. Users lose only old rows from the live UI window. Historical data remains available through existing durable history mechanisms.

## Testing

- append `1,001` real transcript entries → verify: the live window contains `1,000` real entries plus one marker.
- append far beyond the limit in multiple batches → verify: only one truncation marker is visible and the omitted count is correct.
- render TUI after trimming → verify: no panic, scroll offsets are valid, and newest rows remain visible.
- stream an assistant response while trimming occurs → verify: the streaming row remains visible and continues updating.
- call `/api/transcript` after trimming → verify: `chat_entries` length is bounded and includes the truncation marker.
- run a tool call and completion inside the bounded window → verify: Web UI still merges completion into the tool-call row.
- generate a prompt after trimming → verify: prompt/context input is unchanged except for behavior that already follows existing `/clear` or history rules.
- inspect persisted history after trimming → verify: older durable turns/messages still exist and were not deleted by UI trimming.
- run `/clear` after trimming → verify: visible entries and omitted-count marker reset according to current `/clear` behavior.
- run `cargo test -p themion-cli` → verify: transcript buffer and web projection tests pass.
- run `cargo check -p themion-cli` → verify: default CLI build compiles.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build compiles.
- run `cargo test -p themion-cli-web-ui` if Web UI rendering helpers change → verify: Web UI tests pass.

## Implementation checklist

- [x] define the default live transcript entry limit as `1,000`
- [x] add live transcript omitted-count state owned with the CLI-side transcript window
- [x] trim TUI `App.entries` after append while preserving newest rows
- [x] render or project one truncation marker without storing repeated marker rows
- [x] keep scroll and review offsets valid after trimming
- [x] ensure Web UI transcript projection uses only the bounded live transcript
- [x] add focused tests for repeated trimming, marker count, streaming preservation, and web transcript marker projection; prompt/persistence behavior remains unchanged because trimming is limited to UI `Entry` storage
- [x] update PRD/docs status notes after implementation lands


## Implementation notes

Implemented in v0.71.2. The live TUI transcript now trims `App.entries` to the newest `1,000` real entries and keeps one `TranscriptOmitted` marker with the accumulated omitted-entry count. `/clear` resets the live UI window and omitted count while preserving the existing runtime context-clear behavior. The Web UI projects the same bounded server-side entries and renders the marker as an `OMITTED` transcript row. Prompt generation, context replay, SQLite history, and Project Memory paths were not changed.
