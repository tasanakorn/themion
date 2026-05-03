# PRD-096: Automatic Append-Only Chat Message Indexing and Idle-Only Background Embedding

- **Status:** Draft
- **Version:** >v0.59.2 +minor
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-03

## Summary

- Themion already persists chat history durably and already has generalized unified-search indexing, but new `chat_message` rows still rely too much on manual indexing maintenance.
- Make new indexable `chat_message` rows register themselves automatically in `unified_search_documents` as part of normal transcript persistence.
- Keep that append-time step lightweight: create or refresh the document row immediately, but defer chunking and embedding to background work.
- Run deferred embedding only from the CLI-owned background runtime and only when all local agents are idle.
- Keep `agent_messages` as the source of truth and keep `unified_search_rebuild` as the repair and backfill path rather than the normal path for newly appended chat messages.

## Goals

- Make newly appended indexable `chat_message` rows enter the unified-search pipeline automatically.
- Keep transcript persistence fast by separating document registration from chunking and embedding.
- Reuse the existing generalized unified-search data model instead of creating a second chat-specific indexing system.
- Keep scheduling and all-agents-idle policy in runtime/app-state ownership, not in the TUI.
- Ensure pending work survives process restarts through durable index state.
- Preserve manual rebuild and refresh paths for repair, migration, and historical backfill.

## Non-goals

- No redesign of `agent_messages` as the source-of-truth transcript store.
- No new chat-only semantic schema parallel to `unified_search_documents` and `unified_search_chunks`.
- No synchronous embedding generation on the transcript write path.
- No TUI-owned scheduler, queue, or idleness policy.
- No requirement to treat `tool` rows or currently excluded empty assistant carrier rows as chat-message inputs.
- No requirement in this PRD to guarantee that all pending chat-message embeddings finish before shutdown.
- No removal of manual `unified_search_rebuild` or related maintenance tools.

## Background & Motivation

### Current state

PRD-091 established generalized unified search across `memory`, `chat_message`, `tool_call`, and `tool_result`. PRD-092 added source-kind-scoped maintenance commands. The active product direction is therefore already clear:

- `agent_messages` remains the durable transcript source of truth
- `unified_search_documents` and `unified_search_chunks` are derived searchable artifacts
- `chat_message` is already a supported unified-search source kind
- the CLI runtime already has a dedicated background Tokio runtime domain for lower-priority maintenance work
- the runtime already has hub-owned local-agent activity truth that can determine whether all local agents are idle

What remains weak is the default ingestion path for new chat content. Newly appended chat messages can exist durably in `agent_messages` while still waiting for a manual refresh or rebuild before they appear in the generalized index.

### Why this matters now

That gap creates three practical product problems:

- search freshness for recent transcript content depends too much on manual operator action
- large rebuilds waste effort rediscovering recent append-only rows that could have been registered incrementally
- embedding work is easy to put in the wrong place unless the runtime explicitly owns the policy

The transcript domain is unusually well suited to incremental indexing because new chat rows are normally appended, not edited in place. That makes it reasonable to register each new indexable row into `unified_search_documents` immediately, mark it pending, and let the background runtime finish chunking and embedding only during all-idle periods.

**Alternative considered:** make each transcript write also generate embeddings immediately. Rejected: it would turn ordinary transcript persistence into a provider- and compute-sensitive hot path.

## Design

### 1. Append-time document registration is required for new indexable chat messages

Themion must treat new indexable `chat_message` rows as automatic unified-search ingestion events.

Required behavior:

- after a new indexable `chat_message` row is persisted to `agent_messages`, Themion must create or refresh the corresponding `unified_search_documents` row without waiting for manual rebuild
- this append-time step must reuse the existing normalized unified-search identity for `chat_message`, not invent a second chat-specific identity scheme
- the append-time step must be idempotent so that automatic follow-up and later rebuilds converge on the same document row
- the append-time step must not depend on the TUI being active; headless and non-interactive runtime paths should still register new indexable chat rows

For this PRD, the normal indexable chat-message set remains the same as today:

- `user` rows are eligible
- eligible non-empty `assistant` rows are eligible
- `tool` rows are not reclassified as `chat_message`
- currently excluded empty assistant carrier rows remain excluded

This PRD intentionally keeps indexability policy aligned with the existing generalized search system rather than expanding it.

### 2. Keep one unified derived-index pipeline and split it into two stages

This feature must extend the existing generalized unified-search pipeline rather than create a separate chat-only indexing architecture.

Required two-stage behavior:

1. **Stage A: append-time registration**
   - source row is already persisted in `agent_messages`
   - runtime decides whether the row is indexable as `chat_message`
   - runtime upserts the matching `unified_search_documents` row
   - the document row is left in a non-ready state until chunking and embedding complete

2. **Stage B: deferred background embedding**
   - runtime later discovers pending chat-message documents from durable state
   - reusable core indexing code computes normalized text, chunks, and embeddings
   - document state becomes `ready` only after the corresponding chunk rows are written successfully

Required constraints:

- `agent_messages` remains authoritative source data
- append-time registration must be lightweight compared with full chunking and embedding
- chunking and embedding must not block transcript writes or foreground agent-turn progress
- the index state must represent not-ready work honestly instead of pretending the document is semantically ready immediately

### 3. Durable pending state is the source of truth for unfinished work

The automatic pipeline must remain correct even if the process restarts or an in-memory wake signal is lost.

Required behavior:

- unfinished chat-message embedding work must be discoverable from durable `unified_search_documents` state
- append-time registration should therefore leave the document in a durable state such as `pending` or equivalent non-ready state already recognized by the generalized index model
- background workers may use in-memory wakeups or nudges for responsiveness, but must not depend on them for correctness
- on restart, the runtime must be able to rediscover pending `chat_message` documents and resume work later during an eligible idle window

Implementation-ready direction:

- treat the document row, keyed by the existing normalized `(source_kind, source_id, project_dir)` identity, as the durable coordination point
- prefer upsert semantics for append-time registration
- keep rebuild logic and append-time registration convergent so both paths repair the same document row rather than competing with duplicate rows

### 4. Background embedding belongs to the CLI runtime, reusable indexing belongs to core

This feature must follow the repository layering rules.

Ownership requirements:

- `themion-core` owns reusable indexing logic, source-to-document projection logic, chunking helpers, and durable index state operations
- `themion-cli` runtime modules own scheduling, wakeup, and all-agents-idle gating for background execution
- `tui.rs` and `tui_runner.rs` must remain observers or intent forwarders only; they must not become the owner of indexing policy, worker lifecycle, or idleness decisions
- headless and TUI-visible modes should consume the same runtime-owned scheduling behavior rather than reconstructing separate policies

This is intentionally the same ownership shape already required elsewhere in the repository: if the system decided when background indexing may run, that decision belongs outside the TUI.

### 5. All-agents-idle gating is the explicit scheduling rule

Deferred chat-message embedding must run only when every local agent in the current process is idle.

Required behavior:

- if any local agent is active, busy, or mid-turn, background embedding for pending `chat_message` documents must not start new work
- when runtime-owned activity truth transitions to all agents idle, pending chat-message embedding becomes eligible automatically
- if active work resumes while background embedding is in progress, runtime policy must yield in a bounded way so active work keeps priority
- the all-agents-idle decision must come from hub/app-state-owned runtime truth shared across TUI, headless, and Stylos-adjacent runtime paths

This PRD intentionally chooses the stricter policy the user requested, even though more permissive background parallelism could be technically possible.

**Alternative considered:** permit embedding while some agents remain active if separate runtime threads are available. Rejected: the desired product rule is explicit all-agents-idle gating, and that rule is simpler to reason about operationally.

### 6. Failure, retry, and rebuild behavior must converge on the same rows

The automatic pipeline must fail safely without compromising transcript storage.

Required behavior:

- if append-time registration succeeds but background embedding has not yet run, the document remains durably pending
- if background embedding fails for one document, the failure must be represented explicitly in durable index state so later retry or rebuild can repair it
- transcript source rows in `agent_messages` remain authoritative regardless of indexing success or failure
- manual `unified_search_rebuild` remains the repair and backfill path for broader recovery, historical catch-up, or migration
- automatic registration, retry, and rebuild must all converge on the same normalized document identity so they do not create duplicate rows for one source message

Implementation-ready direction:

- use the existing document uniqueness contract from generalized unified search
- keep failure state explicit through the existing embedding-state model rather than inventing sidecar tracking tables unless the implementation proves that is required
- prefer incremental retry/drain behavior over one giant all-or-nothing embedding pass

### 7. Chat-message append-only semantics should guide the steady-state design

This PRD is based on the product expectation that chat messages are append-only in normal operation.

Required interpretation:

- the normal path assumes new chat rows are appended and then left unchanged
- the automatic pipeline is optimized for that append-only steady state
- unusual in-place edits, if they exist now or are introduced later, do not redefine the core design; they should be handled by the same idempotent document-upsert plus rebuild/repair model rather than by adding a second mutation-heavy indexing policy here

This keeps the design DRY: one generalized document identity, one append-time registration rule, one deferred embedding path, one rebuild path.

### 8. Active docs must describe the split between registration and embedding

Because this PRD changes the expected freshness path for transcript search, docs must describe the new behavior clearly.

Required behavior:

- active docs should state that new indexable `chat_message` rows automatically create or refresh their `unified_search_documents` row
- docs should distinguish immediate append-time registration from deferred idle-only chunking and embedding
- docs should describe the all-agents-idle rule as runtime/app-state-owned scheduling behavior
- docs should preserve manual rebuild as the explicit backfill and repair path

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core` transcript persistence and unified-search indexing helpers | Add a lightweight append-time `chat_message` document upsert path that reuses the existing normalized unified-search identity and existing durable embedding-state model. |
| `crates/themion-core` generalized indexing code | Reuse or extend existing source-text extraction, chunking, and embedding helpers so deferred chat-message work uses the same derived-index pipeline as rebuilds instead of a second chat-only implementation. |
| `crates/themion-cli/src/app_state.rs` and adjacent runtime modules | Own background scheduling, wakeup, and all-agents-idle gating for pending `chat_message` embedding work. |
| CLI runtime topology / background runtime wiring | Reuse the existing background Tokio runtime domain for deferred embedding execution rather than performing embeddings on the foreground turn path. |
| runtime-owned activity snapshot path | Provide the all-agents-idle truth that gates whether the background worker may begin or continue pending work. |
| `docs/engine-runtime.md` | Document append-time document registration for `chat_message`, deferred background embedding, and the all-agents-idle scheduling rule. |
| `docs/README.md` | Track this PRD in the docs index and update status/version notes when implemented. |

## Edge Cases

- append a new `user` message while agents are busy → verify: the matching `unified_search_documents` row is created promptly and remains pending until an all-idle window.
- append a new eligible `assistant` message while agents are busy → verify: registration happens without blocking the turn and embedding is deferred.
- append a `tool` row or excluded empty assistant carrier row → verify: existing indexability rules still apply and no unintended `chat_message` document row is created.
- process restarts with pending chat-message rows → verify: later all-idle runtime windows rediscover and resume unfinished work from durable state.
- manual rebuild overlaps with rows already auto-registered → verify: both paths converge on the same normalized document rows without duplication.
- active work resumes during an idle-time embedding drain → verify: runtime policy yields so active work remains higher priority and unfinished documents remain recoverable.
- one background embedding attempt fails → verify: failure is visible in durable state and later retry or rebuild can repair it.
- run headless mode or TUI mode → verify: the same runtime-owned all-agents-idle rule governs background embedding in both surfaces.

## Migration

This PRD changes the default ingestion path for future chat messages but does not make historical backfill implicit.

Required rollout behavior:

- newly appended indexable `chat_message` rows should use the automatic append-time registration path immediately once implemented
- previously stored historical rows may still require manual or startup-triggered rebuild/backfill until processed
- source-of-truth transcript storage must remain correct regardless of index freshness
- any schema/state change needed to represent durable non-ready chat-message documents should be introduced in a way that remains compatible with rebuild-based recovery when practical

## Testing

- append a new `user` message in a normal session → verify: a `source_kind="chat_message"` document row is created automatically without manual rebuild.
- append a new eligible `assistant` message in a normal session → verify: the document row is created automatically and remains non-ready until background embedding finishes.
- append a new message during active work → verify: transcript persistence is not blocked on chunking or embedding.
- allow the process to become all-idle with pending chat-message rows present → verify: background runtime work generates chunks/embeddings and transitions those rows to `ready`.
- keep at least one local agent busy while others are idle → verify: background chat-message embedding does not begin new work until all local agents are idle.
- restart with pending chat-message rows in durable state, then later allow idle time → verify: pending rows resume without manual rebuild.
- run `unified_search_rebuild` for `chat_message` after some recent rows were auto-registered → verify: rebuild converges without duplicate rows or broken source linkage.
- append excluded transcript rows such as `tool` or empty assistant carrier rows → verify: automatic registration still respects the current indexability filter.
- simulate one background embedding failure → verify: failure state is durable and later retry or rebuild can recover it.
- run `cargo check -p themion-core` after implementation → verify: default core build stays clean.
- run `cargo check -p themion-core --all-features` after implementation → verify: all-features core build stays clean.
- run `cargo check -p themion-cli` after implementation → verify: default CLI build stays clean.
- run `cargo check -p themion-cli --features stylos` after implementation if touched runtime code is feature-adjacent → verify: relevant feature-enabled CLI build stays clean.
- run `cargo check -p themion-cli --all-features` after implementation → verify: all-features CLI build stays clean.

## Implementation checklist

- [ ] identify the authoritative transcript append path for persisted chat rows
- [ ] add one lightweight append-time `chat_message` document upsert step at that path
- [ ] keep append-time registration idempotent by existing normalized unified-search identity
- [ ] preserve the current chat-message indexability filter
- [ ] represent non-ready chat-message documents durably through the existing embedding-state model
- [ ] add or reuse pending-document discovery for incremental background `chat_message` embedding
- [ ] schedule deferred embedding on the CLI background Tokio runtime domain
- [ ] gate background work on runtime-owned all-agents-idle truth
- [ ] ensure pending work resumes from durable state after restart
- [ ] keep rebuild and automatic follow-up convergent on the same document rows
- [ ] update active runtime/indexing docs to reflect the new default pipeline
