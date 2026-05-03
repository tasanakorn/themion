# PRD-091: Generalized Multi-Source Chunked Unified Search

- **Status:** Implemented
- **Version:** v0.59.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-03

## Summary

- Themion currently ships semantic search only for Project Memory, and the current implementation stores one embedding per memory node rather than supporting chunked retrieval or search across multiple source domains.
- Add a generalized local embedding index that supports chunked semantic retrieval across Project Memory, chat messages, tool calls, and tool results.
- Replace `memory_search` with one new generalized search tool named `unified_search` that can search one or many source kinds in one request while preserving explicit project scoping.
- The unified tool supports `fts`, `semantic`, and `hybrid` retrieval modes rather than being named after only one ranking method.
- Use a practical chunk schema with `chunk_index`, `char_start`, and `char_len` so snippets, highlighting, rebuilds, and stale-update behavior stay deterministic and inspectable.
- When vector search matches multiple chunks from the same underlying source object, collapse them into one source-level result using the best chunk as the primary snippet and a simple bounded score aggregation rule.
- Define explicit rebuild, targeted rebuild, and incremental stale-refresh behavior so embeddings can be regenerated safely and predictably from source-of-truth tables.

## Goals

- Extend search beyond Project Memory into a generalized multi-source retrieval capability.
- Support chunked embeddings so long content can be searched semantically without requiring one whole source record to fit naturally into one embedding input.
- Allow one query to search any explicit subset of supported source kinds rather than forcing client-side fan-out and merging.
- Preserve current explicit project scoping semantics so generalized search can target one project or explicit global knowledge paths without silent cross-project expansion.
- Keep structured metadata filtering and semantic ranking as complementary layers rather than forcing all semantics into embedding text.
- Return source-aware, inspectable results that identify what kind of item matched and where the match came from.
- Ensure multiple chunk matches for one source object aggregate into one stable source-level result rather than noisy duplicates.
- Define practical chunk metadata that supports deterministic rebuilds, snippet extraction, and stale-update replacement without unnecessary complexity.
- Replace the memory-only semantic tool surface immediately with one canonical generalized search tool.
- Define how embeddings are created, refreshed, rebuilt, and invalidated when source data changes.
- Preserve the architecture rule that runtime-owned data remains the source of truth and the embedding index is a derived searchable artifact.

## Non-goals

- No remote hosted vector database or mandatory remote embedding service.
- No requirement to preserve `memory_search` as a compatibility surface after `unified_search` lands.
- No requirement to expose raw chunk rows as the default user-facing result type.
- No attempt to make vectors replace normal structured filtering such as `project_dir`, source kind, session identity, or source-specific type filters.
- No silent merging of unrelated projects or implicit cross-project retrieval when the caller did not request it.
- No commitment in this PRD to a complex learned reranker or provider-specific ranking pipeline.
- No requirement to index every ephemeral transcript fragment or every internal runtime event.
- No requirement to guarantee perfectly synchronous immediate embedding refresh on every write path; a bounded async refresh path is acceptable if stale-state handling is explicit.
- No requirement to store token offsets in the first implementation if character offsets and lengths provide sufficient deterministic chunk positioning.
- No requirement to expose every matching chunk separately in the primary result list when they belong to the same source object.

## Background & Motivation

### Current state

PRD-059 added feature-gated semantic retrieval for Project Memory, but the current implementation is intentionally narrow.

Today:

- semantic retrieval is limited to Project Memory
- the shipped table shape stores one embedding row per memory node
- semantic search joins `memory_nodes` directly to one embedding row per node
- the tool surface exposes explicit `fts` and `semantic` modes for `memory_search`
- there is no generalized embedding document model for chat history, tool calls, or tool results
- there is no chunk-capable schema for storing multiple embeddings per source record

That is sufficient for node-level Project Memory lookup, but it does not support the more natural cross-source queries that arise during real agent work.

Examples of the desired use cases include:

- search `memory` and `chat_message` for one project in one request
- search `chat_message` and `tool_result` for one project in one request
- search long tool results semantically even when the important content appears in only one portion of a larger payload

### Why a dedicated generalized design is needed

A memory-only one-vector-per-node table does not scale cleanly to the new requirement.

The product need is broader than "add more embeddings":

- multiple source domains must be searchable together
- long content must be chunkable
- hard filters such as project scope and source kind must remain explicit and efficient
- chunk matches must aggregate back to useful source-level results
- embedding state must be rebuildable and refreshable when source data changes
- chunk position metadata must be strong enough to support snippets, highlighting, deterministic rebuilds, and old-chunk replacement
- repeated chunk hits for one object must not flood the result list with duplicates of the same underlying source
- the tool surface should be optimized around one canonical generalized query path rather than preserving a rare-use memory-only semantic tool

This is better served by a generalized derived index for embedded documents and chunks than by continuing to stretch the current memory-only table shape.

**Alternative considered:** keep separate embedding tables and separate search tools for each source type. Rejected: that would force routine client-side fan-out and merging for exactly the multi-source queries this PRD is meant to support.

## Design

### 1. Supported source kinds and canonical query contract

This PRD is implementation-ready with the following concrete source kinds and search contract decisions.

Supported source kinds in the first generalized implementation:

- `memory`
- `chat_message`
- `tool_call`
- `tool_result`

The first implementation should replace the memory-only semantic tool surface with one generalized tool.

Implementation-ready query contract decisions:

- the canonical generalized tool name is `unified_search`
- `unified_search` accepts an explicit `source_kinds` array
- `source_kinds` values are limited to `memory`, `chat_message`, `tool_call`, and `tool_result`
- omitted `source_kinds` means the default human-oriented source kinds allowed by the generalized search surface: `memory` and `chat_message`
- `project_dir` remains an explicit required scope for project-local search, with `"[GLOBAL]"` continuing to mean Global Knowledge where relevant
- retrieval mode remains explicit, using `fts`, `semantic`, and `hybrid`
- result payloads must be source-aware and aggregated to source-object results rather than raw chunk rows
- `memory_search` is removed when `unified_search` lands rather than kept as an alias or compatibility bridge

Implementation-ready `unified_search` tool shape:

- `query: string`
- `project_dir: string`
- `source_kinds: string[]`
- `mode: "fts" | "semantic" | "hybrid"`
- `limit: integer`

Implementation-ready default decisions:

- omitted `source_kinds` means the default human-oriented generalized source kinds: `memory` and `chat_message`
- omitted `limit` defaults to `10`
- `limit` maximum is `50`
- `mode=fts` searches only sources that have exact-search support in the generalized surface
- `mode=semantic` searches only sources that have semantic index coverage
- `mode=hybrid` combines exact and semantic ranking only for sources that support both in the current build; unsupported sources degrade explicitly

Implementation-ready shared result-shape decision:

- `unified_search` returns one shared top-level result schema for `fts`, `semantic`, and `hybrid`
- `score` is always present in result rows
- `score_kind` is always present and is one of `fts`, `semantic`, or `hybrid`
- `primary_snippet`, `snippet`, `source_kind`, `source_id`, and `project_dir` are always present in result rows
- `supporting_snippets[]` may be empty but is always present

Result shape requirements:

- `source_kind`
- `source_id`
- `project_dir`
- `score`
- `score_kind`
- `snippet`
- source-specific summary fields needed to make the result actionable

**Alternative considered:** keep `memory_search` alongside `unified_search`. Rejected: the old tool is rare-use and would become obsolete immediately; keeping it would add unnecessary long-term surface area and prompt complexity.

### 2. Introduce a generalized embedded-source model

Themion should treat semantic indexing as a derived index over multiple source domains rather than as a Project Memory-only storage feature.

Required behavior:

- each indexed source record must have one normalized source identity that records what it is and where it came from
- the embedding index must remain derived from source-of-truth domain tables rather than becoming the canonical owner of source content
- source-specific metadata should stay in the original domain tables unless a small duplicated projection is required for retrieval efficiency or snippet generation

A normalized source identity must include:

- `source_kind`
- `source_id`
- `project_dir`
- optional parent context such as `session_id`, turn identity, or tool invocation identity when relevant
- source update time used to determine staleness and reindex need

Normalized source identity for first-implementation source kinds:

- `memory` → one Project Memory node
- `chat_message` → one stored chat/history message row
- `tool_call` → one stored tool-call record
- `tool_result` → one stored tool-result record

Implementation-ready source identity decision:

- use the existing primary key from the source-of-truth table as `source_id`
- do not invent a second public-facing id namespace for generalized search results in v1

### 3. Storage schema and practical chunk metadata

The semantic storage layer should move from one-vector-per-record assumptions to one-to-many chunk storage.

Implementation-ready schema direction:

- one source-document table for normalized source identity and staleness tracking
- one chunk table for embedded text segments and embedding vectors

Required source-document-table fields:

- stable document id
- `source_kind`
- `source_id`
- `project_dir`
- nullable source-specific context ids such as `session_id`, turn id, and tool invocation id
- `source_updated_at_ms`
- `chunking_version`
- `embedding_model`
- `embedding_state` as one of `ready`, `stale`, `pending`, `failed`, `skipped`
- `last_indexed_at_ms`
- nullable `last_error`

Required chunk-table fields:

- stable chunk id or stable composite key
- source-document foreign key
- `chunk_index`
- `char_start`
- `char_len`
- `chunk_text`
- nullable `token_start`
- nullable `token_len`
- `embedding_model`
- `embedding_dim`
- `embedding_blob`
- `source_updated_at_ms`
- `indexed_at_ms`

Implementation-ready practical decisions:

- `chunk_index`, `char_start`, and `char_len` are required baseline metadata
- character offsets are the canonical stored position unit in the first implementation
- token offsets are optional and should be stored only when the implementation already computes them meaningfully
- `chunk_text` should be stored in the chunk row in the first implementation because it simplifies retrieval, snippet generation, debugging, and validation
- chunk rows are unique per `(source_document_id, embedding_model, chunk_index)`
- source-document rows are unique per normalized source identity

Implementation-ready schema constraint decision:

- uniqueness for normalized source identity is `(source_kind, source_id, project_dir)`
- source-specific context ids are stored for retrieval and filtering, not for identity uniqueness in v1

### 4. Deterministic chunking rules

Chunking behavior must be deterministic enough that unchanged content reproduces the same logical chunk layout.

Implementation-ready chunking requirements:

- chunking operates on normalized source text extracted from the source-of-truth record
- chunking should be token-budget-aware if the embedding client already uses token accounting; otherwise a bounded character-based fallback is acceptable
- chunk boundaries should be stable for unchanged source text and unchanged chunking version
- each chunk must map back to a contiguous range in the normalized source text via `char_start` and `char_len`
- if chunking logic changes materially, increment `chunking_version` and mark affected documents stale

Implementation-ready first-implementation rule:

- use one fixed chunker configuration across all supported source kinds in v1
- prefer token-based chunking when the embedding integration already has reliable token counting
- otherwise use the following character-based fallback configuration:
  - `chunk_len = 1200` characters
  - `chunk_overlap = 200` characters
- do not introduce source-kind-specific chunker policies in v1 unless one source kind is impossible to index correctly without a narrow documented exception

### 5. Keep normal fields for filtering and vectors for ranking

Generalized search should combine structured filters and vector similarity instead of asking the embedding text to carry all retrieval meaning.

Required behavior:

- normal fields such as `project_dir`, `source_kind`, timestamps, source-specific ids, and source-specific categories must remain queryable as structured filters
- structured filters must narrow the candidate set before vector ranking
- semantic similarity must rank candidates within the filtered set
- future exact-match or hybrid ranking must compose with the same filter model rather than requiring separate scoping logic

Concrete first-implementation filtering behavior:

- filter by `project_dir`
- filter by `source_kinds`
- do not expose additional source-specific filter fields in `unified_search` in v1
- keep `tool_call` and `tool_result` indexed and searchable only through explicit opt-in via `source_kinds` rather than the omitted-`source_kinds` default
- keep deeper source-specific filtering for future work or source-specific tools

### 6. Aggregate multi-chunk hits to one source-level result per source object

Chunk-level retrieval should be an internal indexing and ranking mechanism, not the default final result shape.

Required behavior:

- the search pipeline may score chunk rows internally
- the primary result surface must aggregate chunk hits back to useful source-level results by default
- the result must identify the source kind, source identity, and enough snippet/context information to explain the match
- when multiple chunks from the same source record match, the result surface must return one source-aware result for that source object rather than multiple duplicate source-object rows
- snippet extraction should prefer stored `chunk_text` and positional metadata rather than re-deriving approximate boundaries at query time

Implementation-ready aggregation rule:

- grouping key is normalized source object identity, effectively `(source_kind, source_id, project_dir)`
- top-level results contain at most one row per grouped source object
- the highest-scoring chunk for that source object becomes the primary snippet and the baseline score source
- up to 2 additional supporting snippets from distinct non-identical chunks of the same source object may be included when they materially help explain the match
- the first implementation should rank the source object primarily by its highest-scoring chunk
- the first implementation may apply only a small bounded boost for additional strong chunks from the same source object
- additional same-object chunk hits must not create multiple ranked rows or unbounded score inflation

Concrete first-implementation score aggregation rule:

- `source_score = best_chunk_score + bonus_1 + bonus_2`
- `bonus_1` is the lesser of `0.10 * best_chunk_score` and the second-best chunk score contribution
- `bonus_2` is the lesser of `0.05 * best_chunk_score` and the third-best chunk score contribution
- if fewer than 2 additional qualifying chunks exist, missing bonus terms are `0`
- only chunks from the same grouped source object may contribute to these bonuses

Qualifying supporting chunk rule:

- a supporting chunk must be distinct from the best chunk by `chunk_index`
- a supporting chunk should not be included if its similarity score is less than `0.50 * best_chunk_score`
- only the top 3 chunks total per grouped source object participate in v1 aggregation

This keeps the rule concrete, bounded, and implementation-ready while avoiding long-object domination.

### 7. Source-aware result payloads

A multi-source search surface is only useful if the caller can tell what each result represents.

Required behavior:

- each result must identify its `source_kind`
- each result must include stable source identity fields
- each result must include a score and `score_kind` for all modes
- each result must include a source-appropriate title, summary, or snippet projection
- source-specific metadata should be included only as needed to make the result actionable and inspectable
- when multiple chunks from the same source object matched, the payload should make it clear which snippet is primary and which additional snippets, if any, are supporting evidence for the same object

Implementation-ready source payload expectations:

- `memory` → node title, node type, hashtags, primary snippet
- `chat_message` → agent/speaker label if available, session context, primary snippet
- `tool_call` → tool name, invocation context, primary snippet
- `tool_result` → tool name, invocation context, primary snippet

Implementation-ready payload field decisions:

- include `primary_snippet` and `supporting_snippets[]` in the generalized result shape
- `supporting_snippets[]` maximum length is `2`
- `supporting_snippets[]` entries may include `char_start` and `char_len`
- top-level result `snippet` should alias or equal `primary_snippet` for compatibility with simpler consumers

### 8. Project scoping and source eligibility

The intended common queries are project-scoped. The design should reflect that directly.

Required behavior:

- generalized search must accept explicit `project_dir`
- source types indexed into the generalized embedding system must either carry `project_dir` directly or be mappable to one deterministically during indexing
- the system must not silently broaden retrieval beyond the requested project scope
- explicit global knowledge semantics such as `project_dir = "[GLOBAL]"` must remain compatible with source kinds where global scope is meaningful

Implementation-ready source eligibility rule:

- a source kind may participate in the generalized semantic index only after its indexing path can derive a deterministic `project_dir`
- if a source kind is compiled in but not currently indexable for a requested scope, the search response must degrade explicitly rather than pretending that source kind participated normally

Implementation-ready degraded-response rule:

- the generalized search result should include an `unavailable_source_kinds` list when requested source kinds could not participate normally
- unavailable source kinds must not silently disappear from the effective query semantics

### 9. Exact-search and hybrid-search behavior

The unified tool supports `fts`, `semantic`, and `hybrid` modes. The first implementation must define exact-search and hybrid behavior concretely enough to ship.

Implementation-ready exact-search decision:

- generalized `fts` mode should use available source-of-truth exact-search capability where one already exists or can be implemented cheaply in the same storage layer
- if one supported source kind does not yet have generalized exact-search support, it must appear in `unavailable_source_kinds` for `mode=fts`

Implementation-ready hybrid-search rule:

- hybrid ranking in v1 is a simple merge of the semantic result set and the exact-search result set within the same `project_dir` and `source_kinds` filter
- group by the same top-level source identity `(source_kind, source_id, project_dir)` before final ranking
- if a source object appears in both semantic and exact-search candidate sets, boost its final score above semantic-only or exact-only candidates with similar base scores
- if a source object appears in only one candidate set, it remains eligible for final results
- v1 hybrid ranking must stay simple and inspectable; do not introduce a learned reranker

Implementation-ready hybrid score rule:

- start from semantic `source_score` when semantic evidence exists
- add a fixed hybrid exact-match bonus when the same grouped source object also appears in the exact-search candidate set
- use `exact_match_bonus = 0.15 * semantic_source_score` when both signals exist
- when only exact-match evidence exists and no semantic score exists, assign an exact-only baseline score using exact-search rank order converted into a monotonic descending score band
- for exact-only rows in v1, compute `score = 0.30 - (rank_index * 0.01)` with a floor of `0.05`, where `rank_index` starts at `0` for the best exact-only candidate within the filtered exact-search result set
- exact-only rows use `score_kind = "fts"`
- rows with both exact and semantic evidence use `score_kind = "hybrid"`
- semantic-only rows in hybrid mode retain `score_kind = "semantic"`

### 10. Explicit embedding lifecycle, rebuild, and update behavior

The generalized embedding index should have documented rules for initial indexing, incremental refresh, targeted rebuilds, and full rebuilds.

Required behavior:

- the system must support creating index rows for newly seen source records
- the system must support updating embeddings when source content or relevant metadata changes
- the system must support deleting obsolete chunk rows when a source record is deleted, becomes non-indexable, or produces fewer chunks after rechunking
- the system must support a full rebuild that discards derived embedding rows and regenerates them from source-of-truth tables
- the system must support targeted rebuilds scoped by `project_dir`, `source_kind`, or explicit source identity when practical
- rebuild/update logic must be deterministic enough that repeated indexing of unchanged input produces the same logical chunk ordering and chunk identities
- rebuild/update logic must preserve or deliberately regenerate chunk positional metadata in a consistent way

Implementation-ready lifecycle triggers:

- source record created → create or schedule a new source-document row and chunk embeddings
- source record updated → mark the source-document row `stale` and regenerate its chunk rows
- source record deleted → remove or tombstone the derived source-document row and its chunk rows
- source record becomes non-indexable for scope reasons → mark `skipped` and remove active chunk rows from queryable search state
- embedding model change, chunking-version change, or schema version change → mark affected source-document rows `stale` for rebuild

Implementation-ready update semantics:

- source freshness is judged from source-of-truth timestamps or equivalent deterministic change markers
- reindexing one source record must replace that source record's prior chunk rows atomically enough that search does not mix incompatible old and new chunk sets for the same source record
- failed embedding updates must leave canonical source data untouched and record explicit `failed` or `stale` index state
- unchanged source records must not be re-embedded unnecessarily during ordinary incremental refresh
- if a source edit changes chunk boundaries, old chunk rows for that source must be removed rather than kept alongside the new chunk layout

Implementation-ready maintenance surfaces:

- one explicit full rebuild path for generalized index regeneration
- one targeted rebuild path for one project, one source kind, or one source record
- one incremental stale-refresh path that scans for stale or missing source-document rows and updates only those

Recommended operational behavior:

- maintenance paths may be CLI/runtime-owned rather than exposed as model-facing tools in the first implementation
- the first implementation must include an explicit CLI/runtime maintenance command path for humans, replacing or superseding the old `/semantic-memory index` flow with generalized unified-search index rebuild/refresh commands
- if maintenance is exposed to tools later, keep the contract compact and explicit

Implementation-ready rebuild semantics:

- full rebuild may clear derived generalized embedding tables and repopulate them from source-of-truth records
- targeted rebuild for one source object should fully replace that object's source-document and chunk rows
- incremental stale refresh should skip `ready` rows whose source freshness marker and indexing parameters still match

### 11. Migration and rollout

The current Project Memory embedding table is not the right long-term base for this generalized chunked design.

Required behavior:

- the migration path may discard existing embedding rows and rebuild them
- source-of-truth domain data must remain intact during migration
- the generalized embedding schema should be introduced in a way that allows full regeneration from source-of-truth tables
- migration should not pretend old one-vector-per-node rows can preserve all information needed for chunked indexing
- ordinary exact or non-semantic search must remain usable even when the generalized search index is empty, partial, or rebuilding
- `memory_search` must be removed as part of landing `unified_search`, not in a later cleanup phase

Implementation-ready rollout sequence:

1. create the new generalized source-document and chunk-embedding tables
2. keep ordinary source-of-truth memory/history/tool tables unchanged
3. register indexing extractors for `memory`, `chat_message`, `tool_call`, and `tool_result`
4. implement full rebuild plus incremental stale-refresh
5. validate same-object multi-chunk aggregation on representative long records
6. add `unified_search` and wire it to the new index
7. remove `memory_search` in the same implementation slice
8. update tool docs/prompt guidance to use `unified_search`, including memory-only searches via `source_kinds=["memory"]` and explicit tool-record searches via `source_kinds=["tool_call"]` or `source_kinds=["tool_result"]`
9. retire the old memory-only embedding table after the generalized path is verified, or leave it only as a short-lived transitional artifact with a removal plan

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/memory.rs` | Replace the memory-only one-row-per-node semantic storage dependency with a generalized source-document and chunk-embedding model, while preserving Project Memory as one supported source kind. |
| `crates/themion-core/src/db.rs` and related history storage modules | Expose the source records and metadata needed to index chat messages, tool calls, and tool results into the generalized semantic index with explicit project scoping, source freshness markers, and practical chunk-position metadata. |
| `crates/themion-core/src/tools.rs` | Remove `memory_search` and add the `unified_search` contract with explicit `source_kinds`, project scope, retrieval mode, source-aware result payloads, and omitted-`source_kinds` default behavior that excludes tool noise unless explicitly requested. |
| `crates/themion-core/src/agent.rs` | Remove prompt/tool guidance that prefers `memory_search` and align tool exposure metadata with `unified_search` as the canonical generalized retrieval surface. |
| `crates/themion-core/src/` search query path | Add source-object grouping and score aggregation so multiple matching chunks from the same source object become one top-level result with one primary snippet and optional supporting evidence. |
| `crates/themion-cli/src/` runtime or command wiring | Add explicit maintenance/reindex paths for full rebuild, targeted rebuild, stale refresh, and index progress inspection without making those flows part of ordinary query behavior, including a human-invocable slash-command/runtime command path that replaces or supersedes the old `/semantic-memory index` flow. |
| docs (`docs/architecture.md`, `docs/engine-runtime.md`, relevant search docs) | Document the generalized search index, supported source kinds, project-scope rules, result semantics, practical chunk metadata, same-object chunk aggregation, exact/hybrid behavior, and the replacement of `memory_search` by `unified_search`. |
| `docs/README.md` | Reflect this PRD's status/version and later implementation status when the work lands. |

## Edge Cases

- a source record is too long for one embedding input → verify: indexing splits it into multiple deterministic chunks rather than failing back to one giant embedding string.
- several chunks from the same source record match strongly → verify: the result surface aggregates them to one source-level result rather than duplicating the source noisily.
- two high-scoring chunks from one source object outrank one high-scoring chunk from another object → verify: the first object appears once with an aggregated score rather than occupying multiple result rows.
- a caller searches `memory + chat_message` within one project → verify: both source kinds participate in one query and all returned results remain within that project.
- a caller searches `chat_message + tool_result` within one project → verify: both source kinds participate in one query and source labels remain clear in the returned results.
- one source kind lacks meaningful content for embedding → verify: indexing and retrieval handle the absence explicitly without corrupting the rest of the source record.
- a source record changes after indexing → verify: chunk rows for that source are detected as stale and regenerated from the updated source.
- a source record shrinks after re-editing and now produces fewer chunks → verify: obsolete old chunk rows are removed rather than lingering in search.
- an updated source record causes chunk boundaries to shift → verify: new `char_start` and `char_len` values match the regenerated chunk layout and old chunk rows are removed.
- embedding generation fails for one source record → verify: the source record is marked failed or stale, prior canonical data remains intact, and the rest of the index continues to work.
- a full rebuild runs after a model or chunking-version change → verify: old derived chunk rows can be discarded and regenerated deterministically.
- a source kind cannot yet be mapped to `project_dir` deterministically → verify: implementation either blocks indexing for that source kind or records a clear scoped degraded behavior rather than silently broadening scope.
- unified search is requested for a source kind that is disabled, unsupported, or not yet indexed → verify: degraded behavior is explicit and the response does not pretend the missing source participated normally.
- hybrid mode is requested for a source kind with semantic coverage but no exact-search coverage → verify: that source kind is reported in `unavailable_source_kinds` for the exact/hybrid portion rather than silently treated as fully hybrid-capable.
- a former memory-only use case calls for search after the migration → verify: the same need is satisfied through `unified_search` with `source_kinds=["memory"]`, not through `memory_search`.
- an exact-only row appears in hybrid mode → verify: it receives the documented exact-only score band and `score_kind="fts"` while remaining in the shared result schema.
- exact-match or hybrid retrieval is extended later → verify: the same `source_kinds` and structured filter semantics still apply.

## Migration

This PRD introduces a new generalized derived search index and treats existing embeddings as rebuildable artifacts.

Required migration shape:

- keep source-of-truth domain tables unchanged except for any source metadata exposure needed for indexing
- introduce new generalized source-document and chunk-embedding tables rather than stretching the current memory-only table shape indefinitely
- allow old Project Memory embedding rows to be discarded and regenerated into the new schema
- provide an explicit full rebuild path and incremental stale-refresh path for generalized embeddings
- preserve ordinary exact-search and non-semantic behaviors while the generalized search index is absent, partially built, or rebuilding
- when generalized retrieval switches to the new index, primary results must already use source-object aggregation rather than exposing raw duplicate chunk matches
- remove `memory_search` as part of the same feature landing rather than keeping a compatibility bridge

## Testing

- index one Project Memory node with content long enough to require chunking → verify: multiple chunk rows are created for one source record and retrieval still returns one memory result through `unified_search`.
- index one chat message and one tool result in the same project → verify: both appear in the generalized source-document index with correct `source_kind` and `project_dir` metadata.
- search with `source_kinds=["memory","chat_message"]` and one project scope → verify: results may come from either source kind but never from another project.
- search with omitted `source_kinds` and one project scope → verify: default results come from `memory` and `chat_message`, not `tool_call` or `tool_result`.
- search with `source_kinds=["chat_message","tool_result"]` and one project scope → verify: results may come from either source kind and result payloads identify which source kind matched.
- search a long tool result where only one chunk is relevant → verify: the source-level result is returned with a useful supporting snippet from the matching chunk.
- search a source object where several chunks match well → verify: one top-level result is returned for that object, the best chunk becomes the primary snippet, and optional supporting snippets remain attached to that one object result.
- search two source objects where one object has multiple good chunk matches → verify: the object with repeated matches receives one aggregated rank position rather than occupying multiple positions in the result list.
- run `mode=hybrid` where one grouped source object appears in both exact and semantic candidates → verify: the merged result receives the documented hybrid bonus and remains one top-level object result.
- run `mode=hybrid` where a result appears only in the exact-search candidate set → verify: it receives the documented exact-only score band and `score_kind="fts"` in the shared result schema.
- run a former memory-only semantic query after the migration → verify: it succeeds through `unified_search` with `source_kinds=["memory"]` and `memory_search` no longer exists.
- create a new supported source record after the index already exists → verify: the record is indexed through the documented create/update flow and becomes searchable without requiring a full rebuild.
- update an indexed source record → verify: stale chunk rows are regenerated, obsolete prior chunk rows are removed, and new search results reflect the updated content.
- update an indexed source record so chunk boundaries move → verify: regenerated chunks have correct `char_start` and `char_len` values and no mixed old/new chunk layout remains searchable.
- delete or de-scope an indexed source record → verify: derived source-document and chunk rows are removed or excluded consistently from future search.
- trigger a targeted rebuild for one project or one source kind → verify: only the requested slice is regenerated and other indexed slices remain available.
- rebuild the generalized search index from source-of-truth data → verify: prior embedding rows may be discarded and regenerated without loss of canonical memory/history/tool data.
- change the configured embedding model or chunking version → verify: affected source-document rows become stale and are regenerated under the new indexing parameters.
- request search for a source kind that is unsupported in the current build or not yet indexed → verify: degraded behavior is explicit and exact-search behavior remains available where supported.
- run `cargo check -p themion-core` after implementation → verify: default core build compiles with the generalized schema, same-object aggregation, and source-aware search contract.
- run `cargo check -p themion-core --all-features` after implementation → verify: all-features core build still compiles with the semantic index enabled.
- run the generalized slash-command/runtime maintenance path after implementation → verify: `/unified-search index` and `/unified-search index full` trigger the generalized index rebuild/refresh flow and report scoped results clearly.
- run `cargo check -p themion-cli` and `cargo check -p themion-cli --all-features` after implementation if CLI reindex or presentation paths change → verify: default and feature-enabled CLI builds stay clean.

## Implementation checklist

- [ ] add a generalized semantic source model covering `memory`, `chat_message`, `tool_call`, and `tool_result`
- [ ] add a generalized source-document table for normalized source identity, staleness tracking, and embedding lifecycle state
- [ ] add a generalized chunk-embedding table that supports multiple chunks per source record and model
- [ ] require `chunk_index`, `char_start`, and `char_len` as baseline chunk metadata, and add token offsets only when they are meaningfully available
- [ ] define one canonical chunking path and a documented `chunking_version`
- [ ] use `chunk_len=1200` and `chunk_overlap=200` for the character-based fallback chunker
- [ ] preserve structured filtering by `project_dir` and `source_kind` before semantic ranking
- [ ] remove `memory_search` and add the `unified_search` contract with explicit `source_kinds`, `mode`, bounded `limit`, and omitted-`source_kinds` default behavior that excludes `tool_call` and `tool_result` unless explicitly requested
- [ ] return one shared result schema across `fts`, `semantic`, and `hybrid`, including `score` and `score_kind`
- [ ] aggregate same-object multi-chunk hits into one top-level source result with one primary snippet and optional bounded supporting evidence
- [ ] implement the documented bounded source-score aggregation rule based on best chunk plus capped support bonus
- [ ] implement the documented hybrid ranking rule, exact-only score band, and degraded-response behavior for unavailable source kinds
- [ ] add indexing/regeneration support for Project Memory, chat messages, tool calls, and tool results where project scoping is reliable
- [ ] define stale-detection and refresh rules for create, update, delete, model-change, and chunking-version-change events
- [ ] provide an explicit full rebuild path, targeted rebuild path, and incremental stale-refresh path
- [ ] ensure source-level reindex replacement removes obsolete old chunk rows for the same source record
- [ ] keep degraded behavior explicit when some source kinds are unavailable or not yet indexed
- [ ] add or replace the human-facing slash-command/runtime maintenance path for generalized index rebuild/refresh
- [ ] update architecture/runtime/docs guidance so the generalized search behavior is documented accurately

## Appendix: implementation decisions captured explicitly

This PRD is implementation-ready because it resolves the following concrete decisions:

- first supported source kinds are exactly `memory`, `chat_message`, `tool_call`, and `tool_result`
- the new canonical tool name is `unified_search`
- `memory_search` is removed when `unified_search` lands
- omitted `source_kinds` means the default human-oriented generalized source kinds available to that search surface: `memory` and `chat_message`
- generalized search exposes `mode`, `limit`, and explicit `project_dir`
- `limit` defaults to `10` and caps at `50`
- `unified_search` returns one shared result schema for all three modes
- baseline chunk metadata is `chunk_index`, `char_start`, `char_len`, and stored `chunk_text`
- character offsets are canonical in v1; token offsets are optional
- source identity uniqueness is `(source_kind, source_id, project_dir)` in v1
- top-level results are grouped by `(source_kind, source_id, project_dir)`
- best chunk supplies the primary snippet and baseline score
- additional same-object chunk contribution is bounded to at most 2 supporting chunks with explicit capped bonus rules
- hybrid mode uses a simple explicit merge with exact-match bonus rather than a learned reranker
- exact-only rows in hybrid mode use the documented descending score band with `score_kind="fts"`
- character-based fallback chunking uses `1200` characters with `200` characters overlap
- generalized results include `primary_snippet`, `supporting_snippets[]`, `score_kind`, and `unavailable_source_kinds` when needed
- full rebuild, targeted rebuild, and incremental stale-refresh are all required in the first implementation
- embeddings remain derived artifacts and may be discarded and rebuilt from source-of-truth tables


## Implementation notes

- Landed in `v0.59.0`, with a later follow-up refinement to make omitted `source_kinds` default to the human-oriented kinds `memory` and `chat_message` rather than including tool records.
- `memory_search` was replaced by `unified_search`.
- Generalized derived index tables now exist for normalized source documents and chunk embeddings.
- Supported source kinds are `memory`, `chat_message`, `tool_call`, and `tool_result`.
- `unified_search` supports `fts`, `semantic`, and `hybrid` modes with one shared result schema, source-aware labels, primary snippets, and bounded supporting snippets.
- A rebuild/refresh maintenance path is available via `unified_search_rebuild` and the generalized `/unified-search index` slash-command/runtime flow.
