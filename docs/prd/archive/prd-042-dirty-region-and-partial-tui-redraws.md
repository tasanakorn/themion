# PRD-042: Dirty-Region and Partial TUI Redraws

- **Status:** Implemented
- **Version:** v0.26.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-23

## Summary

- The current TUI redraw loop redraws the full terminal on every event-loop iteration, even when only a small part of the screen changed.
- This is simple and correct, but it can amplify redraw churn during idle ticks, streaming output, status animation, and high event rates.
- Add explicit dirty-state tracking so the app redraws only when something visible changed.
- Follow the implementation shape used by `codex-rs`: separate frame requesting from frame execution, coalesce redraw requests, and avoid an unconditional draw-at-top-of-loop model.
- When a redraw is needed, prefer partial terminal updates by preserving Ratatui's normal diffing path and restructuring app state so unchanged regions stay byte-stable.
- Keep the first slice focused on the main conversation pane, input pane, status bar, and overlays rather than inventing a custom renderer.
- Do not change user-facing layout or transcript semantics in this PRD; the goal is redraw efficiency, not a visual redesign.

## Goals

- Reduce unnecessary full-screen redraw work in the TUI event loop.
- Avoid drawing on ticks or other wakeups when no visible state changed.
- Make visible updates more local by tracking which UI regions are dirty.
- Preserve current visual behavior, streaming behavior, input behavior, and transcript semantics.
- Improve responsiveness and reduce CPU use during idle and high-frequency event periods.
- Keep the implementation understandable and compatible with Ratatui's normal buffer-diff rendering model.
- Give `/debug runtime` better visibility into redraw causes, redraw suppression, and coalescing behavior.

## Non-goals

- No migration away from Ratatui or Crossterm.
- No pixel-precise custom terminal renderer or escape-sequence writer outside the normal backend path.
- No redesign of the visual layout, color scheme, or command surface.
- No change to chat history persistence, workflow semantics, or agent-turn logic beyond what redraw gating needs.
- No promise that every update becomes rectangle-minimal at the terminal protocol level; the first requirement is to stop needless redraw passes and keep unchanged regions stable.
- No attempt to optimize model/network/tool latency in this PRD.

## Background & Motivation

### Current state

`docs/architecture.md` currently describes the TUI loop as:

1. draw the terminal UI
2. wait for the next `AppEvent`
3. handle that event
4. redraw on the next iteration

That description matches the current implementation in `crates/themion-cli/src/tui.rs`: the main loop calls `terminal.draw(|f| draw(f, &app))?` on every loop iteration before it waits for the next event. The app then wakes again for many event types, including the 150 ms tick.

That means Themion can redraw even when the visible frame is effectively unchanged. The effect is especially noticeable in cases such as:

- idle ticks that only advance internal counters
- spinner/status animation updates that may not need a full-layout recomputation
- streaming assistant chunks where only the conversation tail changed
- input edits where only the input box and cursor changed
- overlay open/close transitions that invalidate only one portion of the screen

Ratatui already computes buffer diffs between frames, which helps avoid blindly repainting every cell. But the app still pays the cost of rebuilding the full frame and entering the draw path on every loop iteration, even when there is no visible change to render.

### Research note: `codex-rs` uses requested-frame scheduling rather than unconditional redraw

Research in `../codex/codex-rs/tui` shows a stronger redraw structure that is directly relevant here.

Current `codex-rs` behavior includes these notable pieces:

- `tui/src/tui/frame_requester.rs` defines a cloneable `FrameRequester` that any widget or async task can use to request a redraw
- a dedicated scheduler task coalesces multiple requests into one draw notification on a broadcast channel rather than drawing immediately for every request
- `tui/src/tui/frame_rate_limiter.rs` clamps redraw notifications to a minimum frame interval of about 8.33 ms, or roughly 120 FPS
- many event paths explicitly call `schedule_frame()` only when visible state changes, rather than relying on an always-redraw outer loop
- several view/composer handlers return `needs_redraw` booleans so redraw intent is part of state transition logic, not an accidental side effect

That architecture does not require a custom renderer. It keeps Ratatui in place, but introduces a truthful separation between:

- app wakeups
- redraw requests
- actual draw execution

This is the most useful implementation guide for Themion because the stack is similar: Rust, Crossterm, Ratatui, async tasks, streaming output, overlays, and time-driven UI surfaces.

**Alternative considered:** treat Codex research as interesting but keep this PRD fully generic. Rejected: the user explicitly asked for guidance from `codex-rs`, and the implementation pattern is directly applicable.

### Why dirty-state gating should come before deeper rendering tricks

The biggest avoidable cost in the current loop is not necessarily that Ratatui lacks diffing. It is that Themion schedules a draw pass unconditionally, even when no visible region changed.

A good first improvement therefore has two parts:

- stop calling `terminal.draw` unless something visible is dirty
- when something is dirty, keep unchanged UI sections byte-stable so Ratatui's existing diff path can emit a smaller terminal update naturally

This keeps the implementation close to the current architecture and avoids prematurely replacing Ratatui's renderer with custom terminal patch logic.

**Alternative considered:** build a custom terminal patch writer that manually updates rectangles. Rejected: higher complexity and maintenance cost than needed for the first meaningful performance win.

### Why partial updates should be app-shaped rather than event-blind

Not every event dirties the same visual region. The current app already has natural UI regions:

- conversation pane
- input pane
- status bar
- transcript review overlay
- login/status/remote-event surfaces reflected in the conversation transcript

Tracking dirtiness at that same level gives Themion a practical approximation of partial updates without overfitting to individual terminal cells.

**Alternative considered:** use only a single global dirty flag. Rejected: better than unconditional redraw, but too coarse to guide future optimization or debugging around redraw hotspots.

## Design

### Adopt a Codex-style frame requester instead of unconditional loop redraw

Themion should separate redraw requesting from redraw execution.

Normative direction:

- add a lightweight frame-request handle in `crates/themion-cli/src/tui.rs` or a nearby helper module
- allow event handlers, input handlers, streaming paths, and timer-driven status surfaces to request a future draw instead of drawing immediately
- use a small scheduler task or equivalent internal mechanism to coalesce many requests into one wakeup for the main TUI loop
- keep the first slice simple: one pending draw notification is enough; the implementation does not need a complex render queue

This follows the useful architecture already present in `codex-rs/tui/src/tui/frame_requester.rs`.

**Alternative considered:** add dirty tracking but keep redraw execution embedded directly in every event branch. Rejected: that makes coalescing harder and spreads redraw policy across too many call sites.

### Introduce explicit dirty-region tracking in `App`

The TUI state should track whether visible regions need redraw.

Normative direction:

- add a small dirty-state type in `crates/themion-cli/src/tui.rs`, such as booleans or a bitflag-style struct for `conversation`, `input`, `status`, and `overlay`
- treat a full-screen invalidation as a derived or explicit state rather than the only redraw mode
- initialize the app as fully dirty once at startup so the first frame still renders normally
- centralize dirty marking through helper methods rather than scattering raw field mutations everywhere practical
- where practical, let state-changing helpers return a redraw decision or mark dirtiness internally, similar to `codex-rs` handlers that return `needs_redraw`

This gives the event loop a direct answer to "do we need to draw at all?" and gives future work a stable place to refine partial-update behavior.

**Alternative considered:** infer redraw need ad hoc from every event branch without storing dirty state. Rejected: too fragile and hard to audit as the TUI grows.

### Redraw only after handling an event and only when something visible changed

The main event loop should stop drawing unconditionally at the top of every iteration.

Normative direction:

- preserve one initial startup draw so the first frame appears promptly
- after startup, wait for app events and redraw notifications rather than drawing before every wait
- draw only when the current dirty state indicates a visible change or when a full invalidation is pending
- if an event changes only non-visual counters used for diagnostics, do not mark the UI dirty
- if multiple events arrive before the next draw, allow their dirty regions to merge into one draw pass

This removes the most obvious source of needless redraw churn while preserving event-driven behavior.

**Alternative considered:** keep pre-event drawing but skip every Nth frame heuristically. Rejected: heuristics are less truthful and less maintainable than explicit draw requests and dirty gating.

### Coalesce redraw bursts and optionally clamp frame rate

Themion should avoid one redraw per micro-event when many visible changes arrive in quick succession.

Normative direction:

- coalesce multiple redraw requests that occur before the next actual draw into a single draw
- for rapidly streaming or animated paths, optionally clamp redraw frequency to a bounded maximum cadence instead of drawing on every incoming update
- a Codex-like bounded cadence around 120 FPS is a reasonable upper bound, but Themion may choose a lower ceiling if that better matches its simpler UI and terminal workload
- the chosen cadence must be documented as a redraw-notification limit, not as a promise of sustained frame rate

This brings burst behavior under control without sacrificing responsiveness.

**Alternative considered:** never clamp because terminal diffing is already efficient. Rejected: coalescing and modest rate limiting protect the app from self-inflicted churn during streaming or timer-heavy periods.

### Mark dirty regions according to visible impact

Event handling should mark the smallest practical UI region set that needs rerendering.

Normative direction:

- conversation transcript mutations mark `conversation` dirty
- input edits, cursor-affecting paste, and history navigation mark `input` dirty
- busy/idle/activity changes, workflow/statusline changes, and rate-limit/model info changes mark `status` dirty
- transcript review open/close or overlay scroll changes mark `overlay` dirty, and when they affect the underlying visible composition they may also mark `conversation`
- layout-affecting changes such as terminal resize, input height change, or mode transitions may escalate to full invalidation
- event paths that mutate state through helper methods should ideally not have to remember redraw policy separately; the helper should either mark dirtiness or return that information

This region model is intentionally coarse. The goal is to avoid needless full redraw triggers and keep future rendering choices explicit.

**Alternative considered:** track dirty cells or exact rectangles from the start. Rejected: too much complexity for a first implementation and unnecessary if Ratatui diffing remains in place.

### Preserve Ratatui diffing by making region rendering stable

When a redraw happens, Themion should continue using Ratatui's normal draw/buffer diff path, but the render functions should avoid needless churn in unchanged regions.

Normative direction:

- keep using `terminal.draw(|f| draw(f, &app))`
- factor `draw` into region-oriented helpers so conversation, input, status, and overlay rendering are easier to reason about independently
- ensure helpers derive output only from the state relevant to that region as much as practical
- avoid regenerating decorative or animated text unless that region intentionally changed
- preserve stable string/layout generation for unchanged regions so Ratatui can keep terminal-level diffs small

This lets Ratatui keep doing terminal-level diffs while Themion becomes more disciplined about when and why a frame changes.

**Alternative considered:** skip rendering of non-dirty widgets inside one frame while still rebuilding other widgets ad hoc. Rejected: potentially workable later, but first the app should establish clear region ownership and draw gating.

### Gate tick-driven animation and status changes behind visible need

The 150 ms tick is currently a reliable wake source even when the user is idle.

Normative direction:

- keep the tick for runtime needs that still require it, but do not treat every tick as a redraw reason
- only request a frame on tick when a visible spinner frame, elapsed indicator, or other user-facing status text actually changes
- if the app is fully idle and the statusline does not need animation, the tick should update internal metrics without forcing a draw
- if future work wants different animation cadence than the tick cadence, separate those concerns explicitly
- if delayed UI expiry is needed, such as an expiring hint or cooldown display, prefer scheduled frame requests at the relevant deadline rather than unconditional periodic redraw

This is likely to reduce idle redraw churn materially without changing core runtime structure.

**Alternative considered:** remove the tick entirely. Rejected: the tick has other uses and removing it is a larger behavior change than required for redraw optimization.

### Make redraw reasons observable in `/debug runtime`

The existing runtime debug surface should help confirm whether redraw optimization is working.

Normative direction:

- extend the TUI activity metrics to distinguish event wakeups from draw requests and actual draw submissions
- add counters for coalescing and skipped-clean behavior where practical, for example `event_wakeups`, `draw_requested`, `draw_executed`, and `draw_skipped_clean`
- optionally track delayed/coalesced requests and dirty-region counts so developers can see whether churn is mostly conversation, input, status, or overlay-driven
- keep the wording clear that these are Themion runtime activity counters, not OS-level paint metrics

This gives the implementation a built-in way to validate whether redraw gating is reducing work in real sessions.

**Alternative considered:** optimize redraws without changing diagnostics. Rejected: hard to validate and easy to regress.

### Keep terminal resize and overlay transitions correct by allowing full invalidation

Correctness should win over over-aggressive partial optimization.

Normative direction:

- terminal resize events should mark the whole UI dirty
- opening or closing the transcript review overlay may use full invalidation if that is the simplest correct approach
- any event path that cannot safely determine a smaller dirty set should be allowed to fall back to full redraw
- the design should treat full invalidation as a valid escape hatch, not as failure

This keeps the optimization robust and easier to land incrementally.

**Alternative considered:** require every event to compute a minimal dirty subset with no fallback. Rejected: too brittle and likely to introduce redraw bugs.

### Implementation guide from `codex-rs`

The preferred implementation direction for Themion should explicitly borrow these lessons from `../codex/codex-rs/tui`:

- use a cloneable redraw-request handle rather than direct draw calls from many code paths
- coalesce many redraw requests before notifying the main loop
- treat redraw as event-driven work, not a fixed step of every loop iteration
- let state-changing handlers report whether they actually changed visible state
- schedule delayed frames for time-based UI expiration or animation rather than using redraw-every-tick as the default
- keep terminal diffing in Ratatui rather than implementing a bespoke patch renderer first

Themion does not need to copy Codex structure exactly, but the resulting behavior should be comparable:

- visible state changes request frames
- idle wakeups do not automatically redraw
- bursty updates collapse into bounded redraw work

**Alternative considered:** copy Codex module boundaries and API names exactly. Rejected: Themion should match the idea, not cargo-cult the exact file layout.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/tui.rs` | Add dirty-region state, redraw gating, region-aware invalidation helpers, and event-path dirty marking for conversation/input/status/overlay changes. |
| `crates/themion-cli/src/tui.rs` or a nearby helper module | Add a lightweight frame-request scheduler inspired by `codex-rs` so redraw requests can be coalesced before draw execution. |
| `crates/themion-cli/src/tui.rs` | Restructure the main TUI loop so drawing is event-driven and request-driven rather than unconditional at the top of every loop iteration. |
| `crates/themion-cli/src/tui.rs` | Refactor rendering into region-oriented helpers as needed so unchanged sections stay stable and future partial-update work is clearer. |
| `crates/themion-cli/src/tui.rs` | Extend runtime activity/debug counters to distinguish wakeups, redraw requests, executed draws, coalesced draws, and skipped-clean iterations. |
| `docs/architecture.md` | Update the TUI mode description so it reflects dirty-gated, request-driven redraws instead of implying unconditional redraw on every loop iteration. |
| `docs/engine-runtime.md` | Document redraw gating and clarify how tick wakeups, redraw requests, and actual visible redraws differ in the CLI runtime. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Edge Cases

- the app receives frequent ticks while fully idle → verify: idle runtime counters continue to update, but redraw execution stays low when no visible text changes.
- assistant streaming appends text rapidly → verify: conversation updates still appear promptly and redraw requests coalesce without dropping visible chunks.
- the user types in the input box while no other region changes → verify: input-driven redraws are requested promptly and the cursor stays correct.
- the statusline spinner is active during an agent turn → verify: status redraws continue at the intended cadence without forcing unrelated state churn.
- transcript review overlay opens, scrolls, and closes → verify: overlay visibility and navigation remain visually correct even if these transitions use full invalidation.
- the terminal is resized → verify: the next draw fully reflows layout and no stale content remains on screen.
- a command updates internal debug counters but produces no visible UI change → verify: the event path can leave the UI clean and skip drawing safely.
- multiple dirty regions are marked before the next draw → verify: one draw clears the combined dirty state and renders a correct frame.
- a future event path forgets to mark a needed region dirty → verify: fallback full invalidation remains available for correctness-sensitive transitions and tests cover common paths.
- rapid redraw requests arrive faster than the chosen frame cap → verify: notifications are coalesced or clamped rather than causing one draw per request.

## Migration

This is an internal TUI rendering optimization with no user data or config migration.

Expected rollout shape:

- land frame-request scheduling and dirty-state tracking first
- preserve one initial startup draw but remove unconditional steady-state redraw
- keep the existing UI layout and transcript model intact
- allow conservative full invalidation on complex transitions initially
- refine dirty-region precision later without changing the user-facing redraw contract silently

No SQLite schema changes are required.

## Testing

- start Themion in TUI mode and leave it idle for several seconds → verify: the UI remains correct while executed draw counts stay materially lower than tick wake counts.
- run `/debug runtime` before and after the redraw changes → verify: the command exposes separate wakeup, draw-request, and draw-execution counters that show skipped-clean or coalesced iterations.
- type, paste, and navigate input history → verify: input editing remains correct and redraws occur when the input region changes.
- stream a normal assistant response with visible chunking → verify: new assistant text appears promptly and transcript rendering matches current behavior.
- trigger bursts of streamed chunks or rapid input events → verify: multiple redraw requests coalesce into bounded draw execution without losing visible updates.
- open, scroll, and close transcript review → verify: overlay rendering remains correct and underlying content is restored cleanly.
- resize the terminal during idle and during streaming → verify: layout reflows correctly with no stale or torn regions.
- trigger login, status, tool-call, and remote-event updates → verify: conversation and status regions redraw when their visible content changes.
- run `cargo check -p themion-cli` after implementation → verify: the redraw changes compile cleanly in the default configuration.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: redraw gating also compiles cleanly in the Stylos-enabled configuration.

## Implementation checklist

- [x] add a lightweight frame-request scheduler so redraw requests can be coalesced before draw execution
- [x] add explicit dirty-region tracking to TUI app state
- [x] preserve an initial startup draw but stop unconditional steady-state redraws
- [x] mark dirty regions from conversation, input, status, overlay, and resize event paths
- [x] gate tick-driven redraws so idle ticks can remain visually clean
- [x] optionally clamp redraw notifications to a bounded maximum cadence during bursty activity
- [x] refactor draw code into region-oriented helpers as needed for stability and maintainability
- [x] extend `/debug runtime` counters with redraw gating and coalescing visibility
- [x] update `docs/architecture.md` and `docs/engine-runtime.md`
- [x] update `docs/README.md` with the new PRD entry


## Implementation notes

The implemented slice landed with these concrete behaviors:

- `crates/themion-cli/src/tui.rs` now uses a request-driven redraw scheduler instead of unconditional steady-state redraw at the top of every loop iteration
- the TUI tracks coarse dirty UI regions and only executes `terminal.draw(...)` when visible state actually changed
- redraw requests are coalesced through a lightweight scheduler before they wake the draw path
- tick wakeups still occur, but idle ticks no longer automatically imply a redraw unless pending/status text changed
- `/debug runtime` now distinguishes executed draws, draw requests, and skipped-clean redraw attempts in both recent-window and lifetime activity output
- docs now describe the CLI redraw path as dirty-gated and request-driven rather than unconditional
