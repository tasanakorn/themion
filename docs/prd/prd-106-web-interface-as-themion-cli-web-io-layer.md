# PRD-106: Web Interface as a `themion-cli --web` I/O Layer

- **Status:** In progress
- **Version:** v0.65.0
- **Scope:** `themion-cli`, `themion-core`, selected reusable web assets, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-07

## Summary

- Themion should stop treating the web interface as a separate product runtime.
- Add a web interface mode to `themion-cli`, enabled by `--web`.
- The web surface should be another optional I/O layer over the same CLI-owned bootstrap and app-state/runtime.
- Deliver this work in phases so the first slice proves clean runtime hierarchy before richer browser features land.
- Absorb the needed browser UI, transport, and PTY behavior into `themion-cli`, and do not duplicate bootstrap, workflow, or app-state logic in a separate web runtime.
- Converge on one shared websocket connection that multiplexes multiple agents and multiple terminal sessions instead of separate per-feature realtime channels.

## Goals

- Make the web interface a first-party mode of `themion-cli` instead of a separate runtime product.
- Start the local browser surface with `themion-cli --web`.
- Keep runtime ownership in `themion-cli` bootstrap, app-state, and orchestration layers.
- Treat TUI, headless, and web as optional surfaces, not as the core runtime owner.
- Deliver the migration in small steps, with the first step focused on runtime hierarchy and ownership clarity.
- Absorb needed browser implementation into `themion-cli` without preserving the wrong ownership boundaries or a second long-term web codepath.
- Keep PTY/browser-shell support possible without making PTY the owner of the overall runtime.
- Use one browser realtime transport for both agent-facing and terminal-facing events so multi-agent and multi-terminal sessions share one CLI-owned websocket path.

## Non-goals

- Do not keep a second long-term runtime owner for browser agent, roster, workflow, or session truth.
- Do not move runtime policy into the browser UI.
- Do not require the TUI and web UI to render the same widgets or share presentation code.
- Do not rewrite `themion-core` provider/tool behavior just to serve the web surface.
- Do not treat `themion-web` as an ongoing target architecture; remaining code there is migration source material only until it is absorbed or retired.
- Do not make PTY or shell transport the canonical owner of agent runtime state.

## Background & Motivation

### Current state

Themion already has the local runtime pieces that matter for a web interface in `themion-cli`: config loading, project resolution, session/bootstrap wiring, app-state ownership, agent roster coordination, watchdog/board behavior, and surface-facing runtime snapshots.

At the same time, existing web work introduced a separate `themion-web` binary and, in some areas, a separate web-owned runtime path. That shape duplicates behavior that the CLI runtime already owns.

The repository architecture guidance is clear that TUI is only an I/O layer and that system decisions belong in runtime/app-state. The same rule should apply to the web surface.

### Why this matters now

A browser interface is still useful, but it should plug into the existing product instead of becoming a parallel product with its own runtime truth.

This change reduces duplication, lowers drift risk, removes the obsolete standalone web target, and keeps one source of truth for local agents, sessions, workflow, board routing, incoming-prompt policy, and related runtime decisions.

A phased plan is important because the biggest risk is not visual UI work. The main risk is landing a web startup path that quietly creates another runtime owner or keeps `themion-web` alive as a parallel product shape. The first phase should therefore prove the ownership model before broader browser features are migrated.

**Alternative considered:** continue building the web experience primarily inside a separate `themion-web` runtime and only share small helpers later. Rejected: that keeps the same architectural split that this PRD is intended to remove.

## Design

### 1. Runtime hierarchy starts from bootstrap and app-state

The important requirement here is runtime hierarchy, not request flow.

The core runtime shape should start from process entry, then bootstrap, then app-state/runtime ownership. TUI, headless, and web are only optional surfaces on top of that core.

Required runtime hierarchy:

```text
themion process
└─ entrypoint
   └─ crates/themion-cli/src/main.rs
      └─ bootstrap
         ├─ config / args / project resolution
         ├─ database / session setup
         ├─ app-state creation
         ├─ runtime/orchestrator setup
         └─ optional surface activation
            ├─ TUI
            ├─ headless
            └─ web (`--web`)
```

The key meaning is:

- `main.rs` and bootstrap start the system
- app-state/runtime ownership is created before any surface is attached
- TUI, headless, and web are optional consumers or adapters over that runtime
- any one of those surfaces may be omitted
- the runtime must still have a clear ownership shape even if no TUI or no web layer exists

Required ownership rule:

- do not make TUI, headless, or web the thing that creates or owns the canonical runtime truth

### 2. Expected ownership tree under bootstrap

After bootstrap, the next important layer is the shared runtime owner.

Required ownership tree:

```text
themion process
└─ entrypoint + bootstrap
   └─ shared CLI app-state / runtime owner
      ├─ config/project/session/bootstrap state
      ├─ local agent roster ownership
      ├─ workflow / board / watchdog / incoming-prompt policy
      ├─ runtime snapshots and intent handling
      ├─ runtime domains / task groups as needed
      │  ├─ core
      │  ├─ network
      │  ├─ background
      │  ├─ optional web-serving tasks/runtime
      │  └─ optional PTY service tasks/runtime
      └─ optional surfaces
         ├─ TUI surface
         ├─ headless surface
         └─ web surface
```

Important meaning:

- the shared CLI app-state/runtime owner is the core local owner
- runtime domains and service tasks live under that owner
- optional surfaces also live under that owner
- web-serving work may have its own executor/tasks if useful, but it is still a subordinate branch
- PTY may have its own service/runtime branch if useful, but it is still a subordinate branch
- surfaces are optional; they are not the root of the runtime tree

### 3. Disallowed hierarchy

This PRD is specifically trying to avoid any hierarchy where the web path becomes its own core runtime.

Disallowed shape:

```text
themion process
├─ entrypoint + bootstrap for CLI runtime
│  └─ CLI app-state/runtime owner
└─ separate web bootstrap
   └─ separate web app-state/runtime owner
```

Also disallowed:

```text
themion process
└─ web surface
   └─ creates its own runtime truth first
      └─ later calls into themion-cli or themion-core
```

In both bad shapes, the optional surface stops being optional and starts becoming a second core owner. That is what this PRD must prevent.

### 4. Add `--web` mode to `themion-cli`

`themion-cli` should gain a `--web` startup mode that activates the web surface from the existing CLI bootstrap path.

Required behavior:

- running `themion-cli --web` starts the browser-facing local server
- `--web` is only one optional surface mode alongside existing TUI and headless entry paths
- startup should reuse the same CLI-owned config, project-dir, auth, database, and runtime bootstrap rules unless a web-specific difference is explicitly required
- the web mode should report bind/startup errors through the normal CLI-facing error path
- if needed, web bind configuration may use a CLI flag, config, or environment variable, but ownership still stays in `themion-cli`

### 5. Treat web as another optional I/O surface over CLI-owned runtime state

The browser must not become a second runtime owner.

Required behavior:

- the web surface reads and mutates runtime state through `themion-cli` app-state/runtime interfaces
- agent roster truth, workflow truth, board-routing policy, incoming-prompt admission, watchdog behavior, and runtime status snapshots stay CLI-owned
- the browser sends intents and receives snapshots/events in the same ownership pattern expected for optional surfaces
- if the system decided something, that decision must remain outside the browser UI layer
- the web transport may use HTTP for simple snapshots, but realtime browser traffic should converge on one shared CLI-owned websocket
- that websocket should multiplex multiple agents and multiple terminal sessions over one connection rather than opening separate per-agent or per-terminal sockets
- transport choices must not change ownership boundaries

### 5A. Implement the shared websocket as a routed multiplexer

The single websocket should be simple in shape but explicit in routing. The connection is shared. The streams inside it are identified.

Required behavior:

- the browser opens one websocket to the CLI-owned web surface for realtime traffic
- each websocket message carries a stable envelope with at least: message kind, target domain, target id, and payload
- `target domain` distinguishes at minimum agent traffic, terminal traffic, and shared runtime events
- `target id` identifies the concrete stream inside that domain, for example one agent id, one terminal session id, or one broadcast/runtime channel name
- the server owns routing from the envelope into the correct runtime or PTY branch; the browser does not guess internal task topology
- the websocket protocol should support both request-style client messages and pushed server events without opening extra sockets
- the same envelope family should work for one active agent/session and for many, so the protocol does not need a redesign when the UI adds tabs or split views

Recommended message shape:

```text
{
  kind,        // subscribe | unsubscribe | input | resize | event | snapshot | ack | error
  domain,      // agent | terminal | runtime
  target_id,   // agent_id, terminal_session_id, or runtime channel key
  request_id?, // client correlation when useful
  payload
}
```

Important meaning:

- `kind` describes what the message is doing
- `domain` says which subsystem should receive or interpret it
- `target_id` says which stream inside that subsystem it belongs to
- `payload` stays domain-specific so agent input, terminal bytes, resize events, and runtime notifications do not need separate socket types

**Alternative considered:** separate websocket endpoints for agents and terminals. Rejected: that would reintroduce feature silos, duplicate transport/session logic, and make multi-pane browser state harder to coordinate.

### 5B. Keep subscription and fan-out owned by the server

The browser may show many agents and many terminal sessions at once, but the CLI runtime should still control which streams are active and how events are fanned out.

Required behavior:

- the browser explicitly subscribes and unsubscribes to agent streams, terminal streams, and shared runtime streams over the same websocket
- the server keeps the authoritative subscription map for that websocket connection
- multiple browser widgets may depend on the same underlying stream, but the client should not need duplicate backend subscriptions just because the page has multiple views
- server-pushed events should include enough routing metadata for the browser to place each event into the correct agent panel, terminal tab, or shared timeline
- if one agent stream becomes idle, blocked, or closes, that state change should affect only the relevant subscribed stream rather than the whole websocket
- terminal attach, detach, close, and resize are stream-scoped operations and should not disturb unrelated agent streams on the same connection

Implementation guidance:

- use runtime-owned registries keyed by agent id and terminal session id
- keep per-connection subscription state in the web layer, but keep agent truth and terminal truth in the existing runtime/service owners
- prefer one websocket writer task per browser connection that serializes multiplexed outbound messages in arrival order

### 6. Phase the delivery so runtime hierarchy is proven first

This PRD should land in multiple steps.


#### Phase 1: simple web startup under existing bootstrap

The first implementation slice should be intentionally small. Its purpose is to prove that `themion-cli --web` activates a web surface under the existing bootstrap and app-state/runtime owner.

Required behavior:

- `themion-cli --web` starts a simple local web server from the CLI entry path
- the startup path reuses the same bootstrap and app-state/runtime creation path rather than spinning up a second core runtime tree
- the implementation makes it easy to inspect where web-serving tasks sit under the shared runtime owner
- the first page may be minimal, for example a health/status page or a simple runtime summary page
- Phase 1 does not need agent chat, shell, knowledge query, or full websocket interactivity yet
- the main success condition is architectural cleanliness, not feature completeness

Implementation status note:

- Phase 1 startup wiring has landed in `themion-cli`
- `themion-cli --web` now starts a minimal local web surface from the existing CLI bootstrap path
- the current landed slice spans Phase 1 startup plus a small Phase 2 runtime-observing browser surface
- `/` now serves a Leptos SPA/WASM browser shell from `themion-cli --web` instead of the earlier inline summary page JavaScript
- the SPA opens one shared CLI-owned websocket at `/api/ws` during startup and reuses that one connection for agent subscription/input and terminal-list subscription
- browser-side page state, websocket handling, and UI updates for the SPA entry path are now Rust/WASM-owned instead of handwritten page-local JavaScript
- the SPA still reads runtime-owned snapshots from `/api/status`, while `/api/agents` and `/api/transcript` remain available as runtime-owned JSON endpoints during the migration
- `/transcript`, `/agents`, and `/shell` still exist as migration-era routes, but new browser product direction now centers on the SPA entry route served by `themion-cli`
- the generated SPA browser assets are embedded and served by `crates/themion-cli/src/web_assets.rs`, keeping the server/runtime owner in `themion-cli`
- richer browser interaction, broader route migration, and a more complete websocket/UI surface remain future phases

Recommended Phase 1 review checklist:

- can a reviewer point to one bootstrap path before surface activation
- can a reviewer point to one shared app-state/runtime owner before surface activation
- can a reviewer show that web is attached after that owner exists
- can a reviewer show that the system could omit TUI, headless, or web without changing who owns runtime truth

Phase 1 should answer one question clearly: is web only an optional surface under the existing runtime tree?

#### Phase 2: runtime-observing browser surface

After Phase 1 proves clean startup ownership, the next slice may add browser features that observe CLI-owned runtime state.

Examples:

- runtime status page
- agent roster display
- read-only session or transcript snapshots
- simple event streaming from runtime-owned snapshots

Required behavior:

- any visible runtime state in the browser must come from the shared CLI-owned runtime owner
- the browser must not rebuild a second roster/bootstrap path just to render the page
- if multiple browser pages need the same truth, they should consume one runtime-owned snapshot source rather than each page reconstructing state separately

#### Phase 3: interactive browser intents

After runtime observation is clean, later work may add browser-submitted actions.

Examples:

- agent prompt submission
- board-oriented actions
- shell/PTY interaction
- knowledge-page routing migrated under CLI-owned web mode
- multiplexed realtime updates for multiple agents and multiple browser terminal sessions over one websocket

Required behavior:

- browser actions enter the same runtime ownership boundary as other optional surfaces
- per-feature transport details must not bypass CLI-owned runtime policy
- one shared websocket should carry multiplexed realtime traffic for browser agent interaction and browser terminal interaction
- websocket messages should include stable routing metadata so the CLI-owned runtime can distinguish agent streams, terminal streams, and future browser event types without adding separate socket types
- PTY remains an adapter or service branch, not the owner of agent/session/runtime truth
- if a browser action would require web code to decide runtime policy locally, that design should be rejected and moved back into the shared runtime owner

**Alternative considered:** migrate all current `themion-web` features in one large step. Rejected: that would make it too easy to preserve accidental duplicate runtime ownership while moving fast.

### 7. Absorb needed `themion-web` behavior into `themion-cli` without preserving the old ownership split

Existing browser work is still valuable only as migration source material. The intended end state is that browser behavior lives in or under `themion-cli`.

Required behavior:

- browser UI components, routes, websocket envelope ideas, and PTY/browser-shell adapters may be migrated into `themion-cli` where they are still needed
- the migrated transport should converge on one websocket protocol owned by `themion-cli`, not separate websocket stacks for agent UI and terminal UI
- the shared protocol should keep one envelope family and one subscription model even if browser pages later render the streams in different ways
- do not keep a shared long-term split where browser behavior continues to live primarily in `themion-web` while `themion-cli` hosts another copy
- code moved from `themion-web` must be reshaped around the shared CLI-owned runtime interfaces instead of keeping a web-owned roster/bootstrap layer
- direct database-backed read-only views may remain valid where they intentionally inspect persisted state, but interactive runtime behavior must not invent separate truth outside shared CLI app-state
- remove duplicated bootstrap flows, runtime ownership, and browser transport ownership from `themion-web` once CLI-owned replacements exist

**Alternative considered:** preserve the current web-owned agent runtime and merely re-host it under `themion-cli`. Rejected: that changes the binary name but keeps the wrong architecture.

### 8. Keep PTY and browser-shell support as adapters, not as the runtime owner

PTY may remain a distinct implementation concern.

Required behavior:

- browser shell/PTY support may run through its own adapter or service layer
- terminal session ids must be stable enough for websocket routing, tab restore, and reconnect within the running process
- PTY lifecycle and terminal transport can stay separate from agent-turn execution details where that separation is useful
- PTY should not become the owner of overall runtime bootstrap, agent roster truth, or workflow policy
- shell access and agent interaction should share one browser websocket plumbing layer when useful, while each still consumes runtime-owned truth from the correct layer
- if PTY needs its own executor/runtime domain, that executor is still a service branch under the shared runtime owner, not a second core runtime owner

### 9. Keep CLI/web/TUI layering explicit

This product direction should follow the repository layering guidance.

Required behavior:

- `themion-core` keeps reusable agent/runtime/provider/tool behavior
- `themion-cli` keeps bootstrap, app-state ownership, startup wiring, local runtime orchestration, and surface coordination
- TUI, headless, and web stay optional surface layers
- the web layer in or under `themion-cli` stays presentation plus transport glue
- TUI-specific rendering must not become the source of truth for web behavior, but web and TUI should both observe the same runtime-owned state where the concepts overlap
- if reusable surface-neutral helpers emerge, prefer placing them behind CLI/runtime-owned interfaces instead of rebuilding logic separately in each surface

Recommended mental model:

```text
bootstrap + app-state/runtime owner first
optional surfaces second
optional service branches under the same owner
```

### 10. Retire `themion-web` as the target and finish migration into CLI-owned web mode

The change should be staged cleanly.

Required behavior:

- new implementation work for browser interaction should target the `themion-cli --web` direction
- existing `themion-web` behavior should be treated only as migration source material and reference, not as an implementation target
- docs should clearly state that the intended product direction is CLI-owned web mode and that `themion-web` is obsolete
- follow-up implementation may migrate in slices, for example startup first, then runtime-backed browser observation, then interactive browser features
- any temporary remaining standalone `themion-web` code should be documented as obsolete migration residue, not as a supported parallel path

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-105-lite-web-agent-surface-for-themion-web.md` | Mark the old separate-`themion-web` direction canceled and point to the replacement PRD. |
| `docs/prd/prd-106-web-interface-as-themion-cli-web-io-layer.md` | Define the new CLI-owned phased web-interface product direction. |
| `docs/README.md` | Update the PRD table to show PRD-105 as canceled and add PRD-106 as the active replacement. |
| `crates/themion-cli/src/main.rs` | Add `--web` startup wiring in follow-up implementation, starting with a simple Phase 1 web runtime path. |
| `crates/themion-cli/src/web.rs` | Phase 1 minimal web surface: bind/startup, health page, and shared-runtime status page. |
| `crates/themion-cli/src/` bootstrap/app-state/runtime modules | Remain the core runtime owner that later browser phases consume. |
| `crates/themion-cli/src/` web-facing modules | Add or absorb browser transport, routing, and presentation glue during phased implementation. |
| `crates/themion-web/` | Treat as obsolete migration source material to absorb or retire; do not continue targeting it as a product runtime. |

## Edge Cases

- start `themion-cli --web` with the same local runtime features disabled or unavailable as normal CLI startup → verify: web mode reports the same underlying readiness constraints clearly.
- implement only Phase 1 simple startup first → verify: the result still proves that web is optional and not a second core runtime owner.
- run without TUI and without web → verify: bootstrap and app-state/runtime ownership still remain the clear core shape.
- connect both TUI-oriented and web-oriented surfaces to the same runtime-owned state in later phases → verify: they observe the same agent roster and runtime truth instead of diverging.
- subscribe to several agent panels and terminal tabs at once → verify: one websocket can carry interleaved traffic without losing stream identity or forcing feature-specific reconnects.
- run browser shell/PTY features while agent turns are also active in later phases → verify: PTY activity stays a subordinate service concern and does not become the owner of agent scheduling or runtime policy.
- migrate only part of the old `themion-web` surface at first → verify: docs still make it clear that `themion-web` is obsolete and that `themion-cli --web` is the only intended target.

## Migration

This PRD changes product direction, not only implementation detail.

- PRD-105 becomes canceled as the long-term design basis
- future browser work should target `themion-cli --web`
- future realtime browser work should target one multiplexed CLI-owned websocket instead of feature-specific parallel socket paths
- implementation should proceed in phases, starting with a minimal startup slice that proves bootstrap/app-state ownership comes first and web is only optional
- existing `themion-web` code may be migrated into `themion-cli` or retired in follow-up implementation work
- during migration, docs should distinguish obsolete residue from the intended final product shape

## Testing

- implement Phase 1 `themion-cli --web` startup → verify: the web server starts from the same bootstrap/app-state/runtime path and does not create a second core owner.
- review runtime hierarchy in Phase 1 code → verify: bootstrap and app-state/runtime ownership are clearly established before optional surface activation.
- run a configuration with one optional surface omitted → verify: the ownership tree remains the same.
- implement later runtime-observing browser pages → verify: browser-visible state comes from the shared CLI-owned runtime owner.
- implement later interactive browser features → verify: browser actions enter the same runtime ownership boundary as other optional surfaces.
- review the shared websocket design → verify: one connection can multiplex multiple agents and multiple terminal sessions without creating separate runtime owners or per-feature socket silos.
- exercise mixed traffic on one websocket → verify: agent prompts, agent events, terminal bytes, resize events, and terminal lifecycle updates stay correctly routed by `domain` and `target_id`.
- review PTY/browser-shell design → verify: PTY stays a service branch and not the owner of overall runtime truth.
- review docs after migration planning → verify: PRD-105 is clearly canceled and PRD-106 is clearly the phased replacement direction.

## Implementation checklist

- [x] add Phase 1 `--web` mode to `themion-cli` with simple startup and a minimal page
- [x] make bootstrap and app-state/runtime ownership explicit before optional surface activation
- [x] expose the runtime snapshots later browser phases will need for read-only observation (`/api/status`, `/api/agents`, `/api/transcript`)
- [x] add Phase 2 runtime-observing browser views without rebuilding a second runtime owner (`/`, `/agents`, `/transcript`)
- [x] add Phase 3 interactive browser intents through CLI-owned runtime paths
- [ ] migrate needed `themion-web` browser assets/behavior into `themion-cli`, converging on one shared websocket that multiplexes multiple agents and multiple terminal sessions, and retire the old web-owned path
- [x] keep PTY/browser-shell support as subordinate adapter/service logic rather than overall runtime ownership
- [x] update docs to reflect the phased CLI-owned web direction and any temporary migration state
