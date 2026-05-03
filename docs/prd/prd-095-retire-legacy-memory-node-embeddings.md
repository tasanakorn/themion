# PRD-095: Retire the Legacy Memory-Node Embedding Table and Direct Semantic Search Path

- **Status:** Implemented
- **Version:** v0.60.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-03

## Summary

- Themion now ships generalized unified search from PRD-091, but the older Project Memory-only semantic storage table `memory_node_embeddings` and its direct query/write logic still remain in `crates/themion-core/src/memory.rs`.
- Retire the legacy `memory_node_embeddings` table and remove the direct memory-only semantic path centered on `MemoryStore::search_nodes_semantic(...)` so `unified_search` becomes the only canonical semantic retrieval path for Project Memory.
- Remove the concrete legacy helper functions and dispatch branches that exist only to support the old table, rather than leaving dormant dead code behind.
- Keep ordinary Project Memory storage, graph operations, hashtags, and exact/structured memory lookup intact; this PRD removes only obsolete semantic duplication.
- Require newly created or updated memory nodes to trigger generalized unified-search index refresh from the memory write path via an immediate runtime-owned follow-up step, so semantic freshness does not regress when the old table and old semantic query path are removed.

## Goals

- Remove the legacy semantic storage table `memory_node_embeddings` from Themion's active long-term design.
- Remove direct Project Memory-only semantic retrieval that bypasses the generalized unified-search index.
- Make `unified_search` the single canonical semantic retrieval path for Project Memory as intended by PRD-091.
- Eliminate duplicate semantic indexing/storage responsibilities for the same memory-node content.
- Preserve normal Project Memory CRUD, graph, hashtag, and exact-search behavior.
- Ensure newly created or updated memory nodes become semantically searchable through the canonical generalized index from the memory create/update write path rather than through later manual-only rebuilds.
- Resolve the remaining legacy semantic code into one clear end state instead of leaving dead functions, unused schema setup, or stale helper logic in the codebase.

## Non-goals

- No removal of Project Memory itself or its exact/structured search behavior.
- No redesign of generalized unified-search chunking, aggregation, or source-kind semantics beyond what PRD-091 already established.
- No requirement in this PRD to remove the generalized unified-search semantic tables `unified_search_documents` or `unified_search_chunks`.
- No requirement to add new search source kinds.
- No requirement to preserve direct memory-only semantic search as a compatibility surface after consolidation.
- No requirement to introduce remote vector infrastructure or hosted embedding services.
- No TUI-specific behavior redesign beyond any help/docs/runtime messaging updates needed to reflect the canonical search path.

## Background & Motivation

### Current state

PRD-059 introduced Project Memory semantic retrieval with one embedding row per memory node. PRD-091 later introduced generalized chunked semantic search and explicitly positioned the old memory-only embedding table as a short-lived transitional artifact to retire after the generalized path was verified.

Today, the legacy semantic path still exists in `crates/themion-core/src/memory.rs` through both schema and runtime logic:

- schema setup for the table `memory_node_embeddings`
- create/update write calls through `write_node_embedding_now(...)`
- semantic dispatch through `MemoryStore::search_nodes(...)` into `MemoryStore::search_nodes_semantic(...)`
- semantic candidate lookup through `semantic_candidates_for_query(...)`
- legacy-only refresh and maintenance helpers including `index_pending_embeddings(...)`, `pending_embedding_candidates(...)`, `write_embedding_row(...)`, `write_embedding_values(...)`, and `remove_stale_embeddings_without_nodes(...)`
- shared text-construction helper `embedding_input_from_parts(...)` that currently serves the old per-node embedding path and legacy maintenance flow

At the same time:

- generalized `unified_search mode="semantic"` does not use `memory_node_embeddings`
- generalized `unified_search mode="semantic"` reads from `unified_search_documents` and `unified_search_chunks`
- the generalized unified-search index is maintained through explicit rebuild/refresh paths
- the repository therefore keeps two semantic representations for memory content even though only one is the intended long-term product direction

That split creates ambiguity about which semantic retrieval surface is authoritative and whether new memory content is expected to become searchable through direct node embeddings or through the generalized index.

### Why this matters now

Leaving both paths alive has several costs:

- the codebase continues to maintain semantic data in two places for Project Memory
- new contributors can mistake the old memory-node embedding table for an active canonical feature rather than a transitional remnant
- semantic behavior can diverge between direct memory search and generalized unified search
- live behavior becomes confusing when a newly created memory node gets a legacy embedding immediately but does not necessarily appear in generalized semantic `unified_search` until the generalized index is refreshed
- the repository currently carries concrete dead-code risk: once callers move away from the old path, helper functions and schema setup can linger as unused or misleading maintenance baggage
- PRD-091's intended consolidation remains incomplete while obsolete logic continues to look product-relevant

If `unified_search` is the canonical semantic retrieval path, the repository should make that true in both storage and execution behavior.

**Alternative considered:** keep `memory_node_embeddings` indefinitely as an internal fast path for memory-only semantic search. Rejected: it preserves duplicate semantic indexing logic, weakens the PRD-091 canonical-path intent, and keeps ambiguous product semantics around when memory content becomes searchable.

## Design

### 1. Remove the legacy semantic storage table `memory_node_embeddings`

Themion should stop treating the table `memory_node_embeddings` as an active semantic storage dependency.

Required behavior:

- remove writes to `memory_node_embeddings` from memory-node create/update flows
- remove semantic query logic that depends on `memory_node_embeddings`
- remove active schema initialization and active maintenance behavior for `memory_node_embeddings`
- keep the generalized unified-search embedding tables `unified_search_documents` and `unified_search_chunks` as the only active semantic storage path for Project Memory retrieval
- if rollout must temporarily tolerate an already-existing physical `memory_node_embeddings` table in older databases, treat that as unread legacy state only rather than active product behavior

This makes the generalized derived index the only semantic representation that matters for Project Memory search.

### 2. Remove the direct memory-only semantic function path from `MemoryStore`

`MemoryStore::search_nodes(...)` should no longer expose a separate semantic retrieval path for Project Memory.

Required behavior:

- remove `MemoryStore::search_nodes_semantic(...)`
- remove the semantic/hybrid dispatch branch inside `MemoryStore::search_nodes(...)` that routes `SearchNodesArgs` into `search_nodes_semantic(...)`
- preserve Project Memory exact/structured lookup behavior for `fts` mode and non-semantic filtering
- make the public semantic expectation clear: semantic memory retrieval should go through `unified_search`, not through a second direct memory-store path

Implementation-ready direction:

- `search_nodes(...)` should remain for exact/structured memory lookup responsibilities that still belong to Project Memory internals
- semantic and hybrid memory retrieval should no longer be implemented as an alternate memory-store query contract
- if any internal callers still request semantic mode through `search_nodes(...)`, migrate them to `unified_search` or make the contract reject unsupported semantic mode explicitly rather than silently keeping the old path alive

**Alternative considered:** keep the semantic branch but internally proxy it to generalized unified-search rows. Rejected for now: that would keep two overlapping semantic entry points alive and preserve avoidable ambiguity about the canonical API surface.

### 3. Remove the concrete legacy helper functions instead of leaving dead code behind

This cleanup should remove the old implementation pieces explicitly, not merely disconnect them from the last caller.

Legacy functions and helpers to remove when this PRD is implemented:

- `write_node_embedding_now(...)`
- `index_pending_embeddings(...)`
- `pending_embedding_candidates(...)`
- `search_nodes_semantic(...)`
- `semantic_candidates_for_query(...)`
- `write_embedding_row(...)`
- `write_embedding_values(...)`
- `remove_stale_embeddings_without_nodes(...)`
- any legacy-only structs or enums used solely by these helpers, such as pending-embedding maintenance state for `memory_node_embeddings`

Shared-helper requirement:

- if `embedding_input_from_parts(...)` remains useful for the generalized unified-search memory indexing path, keep it and repoint it clearly as generalized-index support code
- if it is no longer needed after legacy cleanup, remove it rather than leaving an orphaned helper behind

Dead-code resolution requirement:

- do not leave behind unreachable semantic branches, unused maintenance entry points, legacy-only result structs, or dormant schema constants whose only purpose was the retired memory-node embedding path
- if one helper is kept for a remaining active caller, its name, placement, and surrounding comments should make that surviving purpose clear

### 4. Make generalized indexing the only path that determines semantic searchability for memory nodes

Once the legacy table is retired, semantic searchability of memory nodes should depend only on the generalized index.

Required behavior:

- the generalized index becomes the sole semantic source of truth for memory retrieval
- memory-node create/update behavior must no longer imply that semantic searchability comes from direct per-node embedding writes
- new or updated memory content should become semantically searchable through the generalized index lifecycle defined by PRD-091 and PRD-092
- product behavior around search freshness should be explicit and consistent with the generalized indexing model rather than split across old and new mechanisms

Implementation-ready requirement:

- `create_node(...)` and `update_node(...)` must rewire from `write_node_embedding_now(...)` to the generalized unified-search indexing path for the affected `source_kind="memory"` record
- newly created or updated memory nodes must cause the corresponding generalized unified-search document/chunk state to be created or refreshed as part of the memory write workflow, rather than waiting for a separate manual rebuild as the normal path
- the canonical product behavior is: the memory write remains authoritative, then the runtime immediately performs a tightly coupled follow-up generalized-index refresh for that one affected `source_kind="memory"` record; this should not depend on a later manual rebuild or background maintenance pass as the normal path
- generalized index failure handling must be explicit: canonical `memory_nodes` writes must remain authoritative, and any indexing failure must surface as generalized-index stale/pending/failed state rather than reviving the old table
- manual or bulk `unified_search_rebuild` remains a repair/rebuild path, not the normal required path for newly created or updated memory nodes to become semantically searchable

This PRD therefore resolves the freshness question directly: rewiring memory create/update into the generalized index path is required, not optional.

### 5. Align active docs and guidance with the canonical path

The repository should stop documenting or implying that a direct memory-only semantic path is part of the active intended design.

Required behavior:

- active docs should describe generalized unified search as the semantic retrieval path for Project Memory
- any active guidance that still presents Project Memory semantic search as a separate memory-only capability should be updated
- PRD and implementation notes should reflect that PRD-091's transitional cleanup has landed once this work is implemented
- if PRD-059 remains as historical documentation, it may remain historical, but active docs should not mislead readers about the current canonical path

### 6. Make migration and cleanup behavior explicit

Removing the old table should be a deliberate cleanup, not a silent half-removal. This PRD is implementation-ready on the product side: the expected user-visible outcome is fixed even if the exact internal sequencing is handled by the runtime implementation.

Required behavior:

- remove active schema initialization or active dependency on `memory_node_embeddings` once the implementation no longer needs it
- define whether rollout drops `memory_node_embeddings` eagerly or leaves harmless orphaned historical rows until an explicit schema cleanup runs
- preserve source-of-truth memory-node data during the cleanup
- ensure generalized unified-search rebuild remains sufficient to reconstruct semantic retrieval state for memory content after migration
- update tests so they verify generalized semantic behavior rather than legacy memory-only semantic behavior

Implementation-ready migration preference:

- the repository should prefer a clear removal path over an indefinite dormant table
- if immediate destructive schema cleanup is risky, one release may tolerate the dormant physical table as unread legacy data only while all active reads, writes, and maintenance helpers are removed, followed by explicit table removal in the same implementation slice if practical or the next narrowly scoped cleanup slice if that constraint is documented clearly

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/memory.rs` | Remove `memory_node_embeddings` schema/setup and the direct legacy semantic functions `search_nodes_semantic(...)`, `write_node_embedding_now(...)`, `index_pending_embeddings(...)`, `pending_embedding_candidates(...)`, `semantic_candidates_for_query(...)`, `write_embedding_row(...)`, `write_embedding_values(...)`, and `remove_stale_embeddings_without_nodes(...)`, while keeping Project Memory logic focused on source-of-truth node storage plus generalized-search integration boundaries. |
| `crates/themion-core/src/memory.rs` shared helpers and legacy structs | Remove legacy-only helper types/constants/structs that become dead after the old table and old semantic path are removed; keep `embedding_input_from_parts(...)` only if an active generalized-index caller still needs it. |
| `crates/themion-core/src/tools.rs` | Keep `unified_search` as the semantic retrieval surface and remove any active routing assumptions that depend on legacy direct memory semantic search. |
| generalized indexing path in `crates/themion-core/src/memory.rs` | Ensure Project Memory semantic searchability is derived from the generalized unified-search index only, and explicitly rewire `create_node(...)` / `update_node(...)` so the affected `source_kind="memory"` document/chunks are created or refreshed from the memory write path. |
| tests for memory search and unified search | Remove or rewrite tests that assume `memory_node_embeddings`, direct `search_nodes_semantic(...)` behavior, or legacy pending-embedding maintenance helpers, and add coverage proving that memory semantic retrieval works through `unified_search` alone. |
| active docs in `docs/` | Update search and memory docs so they no longer imply that `memory_node_embeddings` or direct memory-only semantic search is an active canonical path. |
| `docs/prd/prd-091-generalized-multi-source-chunked-semantic-search.md` and `docs/README.md` | When implemented, update status notes so the transitional-artifact cleanup is reflected accurately. |

## Edge Cases

- create a new memory node after legacy removal → verify: normal node creation still works and the corresponding generalized unified-search document/chunks are created or refreshed from the write path without relying on manual rebuild.
- update an existing memory node after legacy removal → verify: exact/structured reads reflect the edit immediately, and the affected generalized unified-search document/chunks are refreshed from the write path rather than through legacy per-node embedding writes.
- run `unified_search mode="semantic"` for `source_kinds=["memory"]` after indexing → verify: Project Memory semantic retrieval still works without `memory_node_embeddings`.
- inspect the active schema after migration → verify: the product no longer depends on `memory_node_embeddings` for active search behavior.
- inspect code paths after migration → verify: there is no direct `search_nodes_semantic(...)` path left that joins `memory_nodes` to legacy embeddings.
- inspect the compiled core code after migration → verify: legacy helper functions for `memory_node_embeddings` are removed rather than left unused.
- rebuild the generalized unified-search index from source-of-truth data only → verify: memory semantic retrieval can be fully regenerated without the old table.
- search exact memory content with `fts` behavior after cleanup → verify: non-semantic memory search still works.
- request semantic memory retrieval through an old internal path after cleanup → verify: the call is migrated to `unified_search` or rejected clearly rather than silently using obsolete logic.
- start with an existing database that still contains `memory_node_embeddings` rows from an older build → verify: rollout either removes them safely or ignores them without affecting canonical search results.

## Migration

This PRD removes obsolete semantic duplication, not Project Memory source-of-truth data.

Required rollout behavior:

- preserve `memory_nodes`, edges, hashtags, and other source-of-truth memory data
- remove active reads, writes, dispatch, and maintenance helpers for `memory_node_embeddings`
- rewire `create_node(...)` and `update_node(...)` so they update the generalized unified-search index for the affected memory record as the canonical semantic freshness path
- make generalized unified-search rebuild sufficient to recreate semantic search state for memory content
- update active docs and tests in the same implementation slice so the visible product behavior matches the code
- if a transitional release keeps the old physical table briefly for safety, it must be treated as unread legacy state only, not as part of the supported active design

## Testing

- create a Project Memory node and run semantic `unified_search` for `source_kinds=["memory"]` after the normal write flow and its immediate follow-up indexing step complete → verify: the node is returned through generalized semantic search without requiring a separate manual rebuild and without any legacy table dependency.
- update a Project Memory node and rerun semantic `unified_search` after the normal write flow and its immediate follow-up indexing step complete → verify: results reflect the updated memory content through the refreshed generalized index without requiring a separate manual rebuild.
- inspect the code or schema after implementation → verify: direct writes to `memory_node_embeddings` are gone.
- inspect semantic memory query paths after implementation → verify: there is no active `search_nodes_semantic(...)` path using the legacy table.
- inspect the compiled code or source after implementation → verify: legacy helper functions for the old table are removed rather than left as dead code.
- run exact/structured memory lookup after cleanup → verify: Project Memory non-semantic behavior is unchanged.
- run a generalized unified-search rebuild from source-of-truth data only → verify: semantic retrieval for memory content can be fully reconstructed.
- start with a database created by an older build that contains legacy memory-node embedding rows → verify: the new build handles rollout safely and canonical search behavior does not depend on those rows.
- inspect active docs after implementation → verify: they describe `unified_search` as the semantic retrieval path for memory content.
- run `cargo check -p themion-core` after implementation → verify: default core build stays clean.
- run `cargo check -p themion-core --all-features` after implementation → verify: all-features core build stays clean.
- run `cargo check -p themion-cli` after implementation if touched docs/help/runtime surfaces cross into CLI code → verify: default CLI build stays clean.
- run `cargo check -p themion-cli --all-features` after implementation if touched docs/help/runtime surfaces cross into CLI code → verify: all-features CLI build stays clean.

## Implementation checklist

- [ ] remove active writes to `memory_node_embeddings` from `create_node(...)` and `update_node(...)`
- [ ] remove `MemoryStore::search_nodes_semantic(...)`
- [ ] remove the semantic/hybrid dispatch branch from `MemoryStore::search_nodes(...)` or replace it with an explicit unsupported-mode outcome for direct memory-store search
- [ ] remove `index_pending_embeddings(...)`
- [ ] remove `pending_embedding_candidates(...)`
- [ ] remove `semantic_candidates_for_query(...)`
- [ ] remove `write_node_embedding_now(...)`, `write_embedding_row(...)`, `write_embedding_values(...)`, and `remove_stale_embeddings_without_nodes(...)`
- [ ] remove any legacy-only structs, enums, constants, or schema strings that become dead after the old table path is removed
- [ ] keep or remove `embedding_input_from_parts(...)` based on whether the generalized index still actively uses it
- [ ] ensure semantic memory retrieval uses `unified_search` only
- [ ] rewire `create_node(...)` and `update_node(...)` so they create or refresh the corresponding generalized unified-search document/chunk rows for `source_kind="memory"`
- [ ] define the exact failure/reporting behavior when generalized indexing triggered by memory writes fails
- [ ] update or remove tests that assume legacy memory-node embeddings or legacy pending-embedding maintenance
- [ ] add regression coverage for memory semantic retrieval through `unified_search`
- [ ] update active docs to remove legacy memory-only semantic-path guidance
- [ ] update PRD/index status notes when implementation lands
