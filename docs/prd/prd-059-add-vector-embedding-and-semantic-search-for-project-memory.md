# PRD-059: Add Vector Embedding and Semantic Search for Project Memory

- **Status:** Proposed
- **Version:** v0.37.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Project Memory currently supports keyword, hashtag, node-type, and graph-link retrieval, but it does not support vector or semantic search.
- This makes recall weaker when the query wording differs from the stored wording even if the underlying concept is the same.
- This PRD now proposes implementing Phase 1 only: local embeddings through `fastembed`, vectors stored in ordinary SQLite rows or blobs, and in-process similarity ranking.
- The Phase 1 goal is to ship an additive, explicit semantic retrieval path without taking on SQLite vector-extension complexity yet.
- Keep current search behavior and filters; semantic retrieval should be additive, inspectable, and bounded rather than replacing exact search.
- Defer `sqlite-vec`, broader model comparisons, and asynchronous embedding lifecycle work unless Phase 1 measurements show clear pressure.

## Goals

- Implement a practical Phase 1 semantic retrieval path for Project Memory using local embeddings and SQLite-friendly storage.
- Improve recall when agent queries and stored knowledge use different wording for the same concept.
- Keep the current keyword and graph-based search paths available and predictable.
- Make semantic retrieval explicit enough that users and agents can understand when it is being used.
- Preserve current project scoping and explicit `[GLOBAL]` selection semantics.
- Keep the first implementation simple enough to ship and validate before adding indexed or extension-based vector search.

## Non-goals

- No comparison in this PRD between remote hosted embedding providers and local embedding engines.
- No requirement in this PRD to support multiple local embedding engines in the first implementation.
- No replacement of existing `memory_search` keyword/hashtag filtering in the first step.
- No requirement to introduce a remote hosted vector database or remote embedding service.
- No requirement to adopt `sqlite-vec` in Phase 1.
- No requirement to implement asynchronous embedding generation in Phase 1.
- No requirement to embed every historical transcript or board note.
- No broad retrieval-augmented generation redesign across all context sources.
- No automatic silent cross-project retrieval changes; project scoping rules should remain explicit.

## Background & Motivation

### Current state

PRD-046 introduced Project Memory as a lightweight graph-backed durable knowledge base with node types, hashtags, typed links, and current-project default scoping. PRD-049 clarified the naming as Project Memory and Global Knowledge, reinforcing that the feature is durable semantic knowledge rather than transcript recall.

Today, Project Memory retrieval is strong when one of these is true:

- the query contains the same or similar keywords as the stored node
- the relevant hashtags are known
- the relevant node type is known
- the caller already knows a nearby related node and can navigate via graph links

But recall is weaker when the agent remembers the concept, symptom, or intent without matching the original wording. This is the gap semantic search should address.

Current implementation constraints also matter. Project Memory is stored in SQLite tables in `themion-core`:

- `memory_nodes`
- `memory_node_hashtags`
- `memory_edges`
- optional FTS5 table `memory_nodes_fts`

The current search path is simple and inspectable: it uses FTS when available, otherwise plain SQL filtering, then orders by `updated_at_ms`. There is no existing embedding table, vector index, or background indexing runtime in this slice.

### Why semantic retrieval is needed

Project Memory is intended to store durable facts, decisions, troubleshooting notes, files, components, conventions, and reusable observations. Many of those are naturally paraphrased.

Examples:

- a stored node says `provider responses backend drops field X under rate limits`
- a later query asks about `missing response metadata from provider when throttled`

Or:

- a stored troubleshooting note says `partial redraw leaves stale statusline on resize`
- a later query asks about `screen artifacts after terminal resize`

Keyword search may miss or under-rank these relationships even though they are semantically close.

Local embeddings plus SQLite-friendly similarity search provide the additive path this PRD wants to validate. For a lightweight local knowledge base, that is a better fit than assuming a hosted vector service or remote embedding dependency.

**Alternative considered:** rely only on better hashtags and manual linking. Rejected: hashtags and graph edges remain valuable, but they depend on prior curation and exact labeling. Semantic retrieval helps when the wording gap itself is the problem.

### Why the scope is intentionally narrowed

The earlier draft left too many variables open at once: remote versus local embeddings, multiple engine families, and multiple storage/index strategies. That makes spike results harder to interpret.

This PRD now uses the spike conclusions to narrow implementation scope further:

- embeddings must be generated locally
- search must remain SQLite-friendly and local-first
- Phase 1 should use ordinary SQLite storage and app-side similarity ranking
- Phase 1 should defer SQLite vector-extension complexity unless measurements later justify it

That narrower scope better matches Themion's lightweight local architecture and makes the first implementation easier to ship, inspect, and validate.

**Alternative considered:** compare local and remote embedding engines in the same implementation slice. Rejected: that would mix deployment-model decisions with storage/query-shape decisions and make the implementation harder to reason about.

### External research summary informing Phase 1

Current external research points to a relatively clear shortlist for a Rust terminal app.

Local embedding candidates:

- `fastembed` is the strongest Phase 1 candidate because it offers a Rust-first API, uses ONNX Runtime underneath, supports local model download/caching, and avoids the heavier `libtorch` path.
- direct `ort` integration is the lower-level alternative when Themion needs tighter control over ONNX Runtime behavior, model packaging, or execution settings.
- `rust-bert` can produce sentence embeddings, but its usual `libtorch` dependency shape is materially heavier and less attractive for a lightweight terminal app.
- `candle` is interesting as a Rust-native inference building block, but it is less turnkey for this specific semantic-search slice.

SQLite-friendly search candidates:

- plain SQLite storage plus app-side cosine similarity is the simplest baseline and is likely viable for modest corpora.
- `sqlite-vec` is the main SQLite-native follow-on candidate for local vector search, but it adds native extension and build/runtime packaging complexity.
- `sqlite-vss` should not be the default path for new work because it is not the actively preferred successor path and brings heavier Faiss/C++ operational complexity.
- SQLite `vec1` is promising but too early for this PRD's default implementation plan.
- `sqlite-vector` is license-sensitive and should not be the default recommendation for this repository.

This research supports a concrete Phase 1 implementation path:

- local ONNX-backed embeddings through `fastembed`
- vectors stored in ordinary SQLite rows or blobs
- app-side similarity ranking over a bounded candidate set after applying project/filter constraints

**Alternative considered:** start implementation directly with `sqlite-vec`. Rejected: the simpler baseline may already be sufficient for Project Memory scale and is easier to ship and maintain first.

### Model refinement: Phase 1 should begin with `BGESmallENV15` and `BGESmallENV15Q`

Additional model-specific research further narrows the likely Phase 1 choice.

What `fastembed` supports directly today:

- `EmbeddingModel::BGEM3` is supported directly and maps to `BAAI/bge-m3` with 1024-dimensional dense embeddings.
- `EmbeddingModel::BGESmallENV15` and `EmbeddingModel::BGESmallENV15Q` are supported directly and map to `BAAI/bge-small-en-v1.5` with 384-dimensional embeddings.
- `bge-micro-v2` is not a built-in `fastembed` enum model today, so it would require a user-defined/custom-model loading path.
- There is no built-in `bge-m3-tiny` or equivalent smaller `bge-m3` variant exposed in the `fastembed` Rust API.

Implications for Phase 1:

- `bge-m3` is technically attractive for multilingual retrieval, but it is substantially heavier for local-first CPU use: larger model assets, 1024-dimensional vectors, more storage per node, and more likely memory/startup pressure.
- `bge-micro-v2` is attractive on size, but using it immediately would add integration uncertainty because the work would be testing custom model loading at the same time as semantic-search architecture.
- `bge-small-en-v1.5` sits in the middle and is the more practical default: directly supported, smaller than `bge-m3`, and simpler to adopt in a first Rust/`fastembed` implementation.

This means the Phase 1 model guidance for this PRD is:

- do not use `bge-m3` as the default Phase 1 model unless multilingual retrieval is a hard requirement from the beginning
- do not use `bge-micro-v2` as the default Phase 1 model because custom-model loading would add a second source of uncertainty
- prioritize `BGESmallENV15` and `BGESmallENV15Q` together so the implementation and validation can measure the quality-versus-size tradeoff directly instead of assuming it

**Alternative considered:** use `BGEM3` first because it is the most featureful BGE-family model supported by `fastembed`. Rejected: its size and dimensionality make it a poor default for a first local-only implementation, and the simple dense `fastembed` path would not exercise BGE-M3's broader sparse/hybrid capabilities anyway.

## Design

### Design principles

- Keep semantic search additive and explicit rather than replacing exact search.
- Prefer a local embedded design that matches Themion's SQLite-first architecture.
- Keep embeddings local and avoid mandatory remote services in this PRD.
- Respect current project scoping and explicit `[GLOBAL]` selection semantics.
- Make retrieval observable enough that agents can see whether results came from semantic matching.
- Bound operational complexity so the first version remains practical.
- Prefer the simplest shippable local approach first; require stronger evidence before adding native SQLite vector-extension complexity.

### 1. Implement a bounded Phase 1 semantic retrieval slice

Themion should implement Phase 1 directly rather than leaving this PRD at spike-only guidance.

Phase 1 should include:

- one local embedding engine family: `fastembed`
- one supported starting model pair: `BGESmallENV15` and `BGESmallENV15Q`
- local embedding generation only
- SQLite-backed storage in ordinary rows or blobs only
- local query execution only
- explicit semantic retrieval surface added alongside existing exact search

The Phase 1 goal is not to build the final highest-scale vector architecture. The goal is to ship a simple, inspectable semantic retrieval capability that proves useful within Themion's existing local Project Memory design.

Expected outputs from the implementation:

- a persisted embedding storage path for Project Memory nodes
- a semantic retrieval query path that ranks bounded candidates in process
- measured latency/resource observations documented in the PRD or follow-on docs/status notes
- a recommendation on whether later work should remain on the simple baseline or move to `sqlite-vec`

**Alternative considered:** keep this PRD at exploration-only status until every Phase 2 comparison is complete. Rejected: the narrowed design is now specific enough to support a useful first implementation.

### 2. Fix the Phase 1 embedding engine and model pair

Phase 1 should use `fastembed`.

The default recommendation for Phase 1 is:

- use `EmbeddingModel::BGESmallENV15` as the quality-first anchor
- use `EmbeddingModel::BGESmallENV15Q` as the footprint-first anchor
- keep the implementation structured so one concrete default can be configured or chosen without redesigning the storage shape
- optionally compare against `AllMiniLML6V2` later only if Phase 1 results are ambiguous and one public-benchmark anchor is needed
- do not start with `BGEM3`
- do not start with `bge-micro-v2`

This keeps the implementation variable set small while still honoring the model research captured earlier in the PRD.

**Alternative considered:** compare several local embedding engines immediately. Rejected: that introduces too many variables before the team knows whether the simpler local architecture is viable in production.

### 3. Store embeddings in simple SQLite rows and rank in process

Phase 1 should use the baseline local storage/query shape directly.

Normative Phase 1 storage/query rules:

- store vectors as `f32` little-endian blobs
- L2-normalize vectors at insert/update time so cosine similarity reduces to a dot product at query time
- keep project scoping and any explicit filters in front of vector ranking so the app-side candidate set remains bounded
- rank candidates in process rather than requiring SQLite vector extension support
- keep the implementation inspectable enough that debugging can confirm what text was embedded and how ranking was derived

If later measurements show query latency or scaling pressure, Phase 2 may introduce `sqlite-vec` against the same data model or a closely related one.

**Alternative considered:** start directly with `sqlite-vec` and skip the plain SQLite baseline. Rejected: the simpler baseline may already be sufficient for Project Memory scale and would be easier to ship and maintain.

### 4. Define a stable embedding text shape

Phase 1 should define one stable text-serialization format for embedding input and use it consistently for create, update, backfill, and query preparation where relevant.

The exact serialization may still be refined during implementation, but it should be explicit and stable enough that:

- embeddings are reproducible for the same node content
- changes to title/content/hashtags or other selected fields clearly trigger re-embedding
- evaluation results are interpretable because the embedded text shape is not drifting silently

The initial embedded text should prefer durable semantic content already present in Project Memory, such as:

- node title
- node content
- hashtags when they add meaningful retrieval context

**Alternative considered:** embed only raw content with no stable formatting contract. Rejected: that makes later evaluation and migration harder because retrieval quality would depend on an implicit, potentially drifting text shape.

### 5. Keep semantic retrieval additive to exact search

Semantic retrieval should not silently replace existing `memory_search` behavior.

Acceptable Phase 1 patterns include:

- extend `memory_search` with an explicit semantic mode or ranking mode
- add a dedicated semantic-search tool such as `memory_semantic_search`
- support hybrid retrieval that combines keyword filtering with vector ranking when explicitly requested

The important behavior is:

- existing keyword/hashtag usage remains valid
- semantic retrieval is opt-in or otherwise clearly signaled
- results can still be filtered by `project_dir`, hashtags, node type, or linked-node constraints when that combination is practical
- the realistic deployed shape should be treated as hybrid retrieval, not dense-only retrieval in isolation

**Alternative considered:** make all memory search semantic by default. Rejected: that would make retrieval less predictable, harder to debug, and more difficult to validate against existing workflows.

### 6. Preserve Project Memory scoping semantics

Semantic search should obey the same context boundaries as existing Project Memory retrieval.

Normative direction:

- omitted `project_dir` continues to mean the current project only
- exact `project_dir="[GLOBAL]"` searches Global Knowledge only
- project search does not silently include Global Knowledge
- any future combined current-project-plus-global mode should be explicit rather than implicit

This keeps semantic retrieval from becoming a hidden cross-project leak path.

**Alternative considered:** search all projects semantically by default because similarity benefits from a larger corpus. Rejected: wider recall is not worth surprising scope expansion.

### 7. Define embedding lifecycle expectations for Phase 1

Phase 1 should establish a clear consistency contract for embeddings even if later versions change the mechanics.

Normative Phase 1 lifecycle expectations:

- create or refresh embeddings synchronously when a node is created if the node has embed-worthy text
- refresh embeddings synchronously when title/content/hashtags or other chosen embedded fields change
- define explicit behavior for backfill and partially embedded corpora
- keep semantic retrieval behavior explicit when embeddings are missing or stale

If Phase 1 measurements show synchronous embedding is too expensive, later work may move the lifecycle to async or deferred generation, but that should not block the first bounded implementation.

**Alternative considered:** defer all lifecycle semantics until after semantic search ships. Rejected: retrieval correctness depends on a clear sync contract from the beginning.

## Changes by Component

| Component / file area | Change |
| --- | --- |
| `crates/themion-core/src/` Project Memory storage and query code | Add Phase 1 embedding storage, stable embedding text serialization, synchronous create/update refresh behavior, and in-process semantic ranking over filtered candidates. |
| `crates/themion-core/src/` tool layer for Project Memory | Add or extend the explicit semantic retrieval tool surface while preserving existing exact-search behavior. |
| `crates/themion-core/src/` provider/integration support | Add local embedding integration through `fastembed` and support the Phase 1 model pair. |
| `crates/themion-cli/src/` user-facing wiring and presentation | Expose semantic retrieval results clearly enough that the mode is explicit and the results remain inspectable. |
| docs and PRD notes | Document the Phase 1-only scope, lifecycle behavior, storage shape, and any measured validation notes that materially affect follow-on decisions. |

## Edge Cases

- Some nodes may have little or no embed-worthy text. Phase 1 should define whether those nodes are skipped or embedded from a reduced text shape rather than silently producing meaningless vectors.
- Older corpora may contain nodes without embeddings until backfill runs. Semantic retrieval should remain explicit about partial coverage rather than pretending all nodes are ranked semantically.
- If local embedding initialization fails or model assets are unavailable, exact Project Memory retrieval should continue to work and the degraded semantic state should be understandable.
- If hashtag-only or title-only edits occur, the re-embedding rule should still be consistent with the declared embedded text shape.
- If Phase 1 uses a configurable default model inside the supported pair, stored metadata should make it possible to tell which model produced a given embedding set.

## Migration

- Add the Phase 1 embedding storage in a way that coexists with the current Project Memory tables and preserves existing exact retrieval behavior.
- Existing nodes may require a backfill path before semantic retrieval has full coverage.
- Backfill may be synchronous utility-driven, startup-triggered, or explicit tooling, but the chosen path should keep partial-coverage behavior understandable.
- If semantic retrieval is unavailable because embeddings are missing or model initialization failed, the tool surface should degrade to exact retrieval or a clearly signaled partial mode rather than failing opaquely.

## Testing

- create Project Memory nodes with paraphrased but semantically related wording → verify: explicit semantic retrieval returns relevant nodes that exact-only retrieval misses or ranks lower
- run Phase 1 with `BGESmallENV15` and `BGESmallENV15Q` on the same corpus → verify: the quality-versus-size tradeoff is measured directly rather than assumed
- create and update nodes that affect title/content/hashtags → verify: embeddings refresh consistently according to the declared serialization contract
- query semantic retrieval within one project and with `project_dir="[GLOBAL]"` → verify: scoping semantics match existing Project Memory boundaries
- store normalized vectors as plain SQLite blobs and rank in-process → verify: the simplest baseline remains correct and measurable
- simulate missing model assets, initialization failure, or partially embedded corpora → verify: degraded behavior stays explicit and exact retrieval remains usable

## Implementation checklist

- [ ] add Phase 1 local embedding integration through `fastembed`
- [ ] support `BGESmallENV15` and `BGESmallENV15Q` as the initial model pair
- [ ] define one stable text-serialization format for embedding input
- [ ] add SQLite-backed embedding storage using L2-normalized `f32` little-endian blobs
- [ ] refresh embeddings on node create/update according to the Phase 1 lifecycle contract
- [ ] add explicit semantic retrieval alongside existing exact search behavior
- [ ] preserve Project Memory scoping and filter semantics in semantic retrieval
- [ ] define backfill and partial-coverage behavior clearly enough for Phase 1
- [ ] measure and record create/update latency, cold-start cost, warm query latency, and storage/runtime impact
- [ ] document the implemented Phase 1 scope and any follow-on pressure toward `sqlite-vec` or async lifecycle work
