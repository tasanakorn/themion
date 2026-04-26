# PRD-059: Add Vector Embedding and Semantic Search for Project Memory

- **Status:** Proposed
- **Version:** v0.37.0
- **Scope:** phased delivery: Phase 1 spike artifact and evaluation, Phase 2 production integration planning for `themion-core`/`themion-cli`, later optimization follow-ons if warranted
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Project Memory currently supports keyword, hashtag, node-type, and graph-link retrieval, but not semantic retrieval when the query wording differs from the stored wording.
- Themion should add additive semantic retrieval for Project Memory without replacing current exact-search behavior.
- This PRD proposes starting Phase 1 now as an isolated spike to validate local embeddings, bounded ranking, and practical cost before changing shipped storage or tool surfaces.
- If the spike is successful, Phase 2 becomes the first production integration into `themion-core` and `themion-cli`.
- Later phases remain optional follow-on work for scale, indexing, or lifecycle improvements.

## Goals

- Deliver a shipped Project Memory feature that adds explicit semantic retrieval while preserving current exact-search behavior.
- Keep the product requirement clear even though delivery is phased: the PRD is complete only when shipped Project Memory supports additive semantic retrieval with predictable scoping and inspectable behavior.
- Start Phase 1 now to validate whether local embeddings improve recall for paraphrased or differently worded queries on realistic Project Memory-like data.
- Measure the practical tradeoffs of local embedding generation, vector storage shape, and in-process similarity ranking before production integration.
- Preserve current project scoping and explicit `[GLOBAL]` selection semantics in both the spike and the eventual shipped feature.
- Produce a concrete recommendation for a Phase 2 implementation that fits Themion's local-first SQLite architecture.

## Non-goals

- No production integration into `memory_search` or another shipped Project Memory tool surface in Phase 1.
- No requirement to modify the main Project Memory storage schema in `themion-core` during the spike.
- No remote hosted vector database or remote embedding service.
- No embedding of every historical transcript or board note.
- No broad retrieval-augmented generation redesign across all context sources.
- No silent cross-project retrieval changes; project scoping rules should remain explicit.
- No requirement to adopt `sqlite-vec`, async embedding generation, or multiple local embedding engines in Phase 1.

## Background & Motivation

### Current state

PRD-046 introduced Project Memory as a lightweight graph-backed durable knowledge base with node types, hashtags, typed links, and current-project default scoping. PRD-049 clarified the naming as Project Memory and Global Knowledge, reinforcing that the feature is durable semantic knowledge rather than transcript recall.

Today, Project Memory retrieval is strong when one of these is true:

- the query contains the same or similar keywords as the stored node
- the relevant hashtags are known
- the relevant node type is known
- the caller already knows a nearby related node and can navigate via graph links

Recall is weaker when the agent remembers the concept, symptom, or intent without matching the original wording. That is the gap semantic search should address.

Current shipped storage and search are intentionally simple and inspectable:

- SQLite tables: `memory_nodes`, `memory_node_hashtags`, `memory_edges`
- optional FTS5 table: `memory_nodes_fts`
- current search path: FTS when available, otherwise plain SQL filtering, then ordering by `updated_at_ms`

There is no existing embedding table, vector index, or background indexing runtime.

### Product outcome

The requirement is not merely to add embeddings. The requirement is to make Project Memory better at finding relevant knowledge when wording differs, while remaining local, predictable, and compatible with current exact retrieval.

Examples:

- stored: `provider responses backend drops field X under rate limits`
- query: `missing response metadata from provider when throttled`
- stored: `partial redraw leaves stale statusline on resize`
- query: `screen artifacts after terminal resize`

Keyword search may miss or under-rank these relationships even when they are semantically close. Local embeddings plus SQLite-friendly similarity search are the additive path this PRD proposes.

**Alternative considered:** rely only on better hashtags and manual linking. Rejected: those remain valuable, but they depend on prior curation and exact labeling. Semantic retrieval helps when the wording gap itself is the problem.

### Why this work is phased

The product goal is clear, and the repository should now begin Phase 1. The technical shape still needs validation: embedding engine choice, storage representation, ranking path, lifecycle cost, and operational complexity.

This PRD therefore separates the work into phases:

- Phase 1 starts now as an isolated spike artifact.
- Phase 2 uses validated findings for the first shipped semantic-retrieval feature.
- Later phases refine scale, lifecycle, or ranking only if earlier phases show clear pressure.

**Alternative considered:** implement the simplest production path directly in Phase 1. Rejected: that would commit the main codebase to persistence, lifecycle, and tool-surface decisions before the team knows whether the approach is worthwhile.

### Phase 1 recommendation basis

Current research is sufficient to start the Phase 1 spike without pretending the production design is already settled.

Recommended Phase 1 defaults:

- embedding library: `fastembed`
- starting model pair: `BGESmallENV15` and `BGESmallENV15Q`
- storage baseline: ordinary SQLite rows or blobs in an isolated artifact
- ranking baseline: app-side similarity ranking over a bounded candidate set after scoping/filtering

This keeps the spike lightweight, local-first, and compatible with Themion's existing architecture.

**Alternative considered:** start directly with `sqlite-vec`. Rejected: the simpler baseline may already be sufficient at Project Memory scale and is easier to evaluate before adopting native extension complexity.

## Design

### Design principles

- Keep semantic search additive and explicit rather than replacing exact search.
- Prefer a local embedded design that matches Themion's SQLite-first architecture.
- Keep embeddings local and avoid mandatory remote services.
- Respect current project scoping and explicit `[GLOBAL]` selection semantics.
- Keep retrieval inspectable enough that users and agents can understand why results were returned.
- Bound operational complexity so the first version remains practical.

### 1. Product behavior and PRD completion state

Themion should eventually add an explicit semantic retrieval path for Project Memory.

The target shipped product behavior should be:

- agents can retrieve relevant Project Memory nodes even when query wording differs from stored wording
- existing exact-search, hashtag, node-type, and graph-link retrieval continue to work
- semantic retrieval is opt-in or otherwise clearly signaled rather than silently changing all retrieval behavior
- results remain scoped by the same current-project and `[GLOBAL]` rules as existing Project Memory tools
- the retrieval mode remains inspectable enough that users and agents can understand why results were returned

Acceptable eventual API shapes include:

- extending `memory_search` with an explicit semantic mode or ranking mode
- adding a dedicated semantic-search tool such as `memory_semantic_search`
- supporting explicitly requested hybrid retrieval that combines keyword filtering with vector ranking

This PRD is complete only when that shipped product behavior exists in the main codebase. Completing Phase 1 alone does not complete the PRD.

**Alternative considered:** make all memory search semantic by default. Rejected: that would make retrieval less predictable, harder to debug, and more difficult to validate against existing workflows.

### 2. Phase 1 delivery slice: isolated spike artifact

Phase 1 is a spike, not production integration, and this PRD proposes starting it now.

Phase 1 should include:

- `fastembed`
- `BGESmallENV15` and `BGESmallENV15Q`
- local embedding generation only
- SQLite-backed storage in ordinary rows or blobs only inside the spike artifact
- local query execution only
- an isolated prototype that evaluates semantic retrieval on realistic Project Memory-like data without changing shipped tool surfaces

Phase 1 is successful when it produces:

- a separate temporary artifact for embedding generation and bounded similarity ranking
- documented latency and resource observations
- documented retrieval-quality observations on representative queries
- a recommendation on whether Phase 2 should integrate the simple baseline or pivot toward another design such as `sqlite-vec`

The spike artifact should remain clearly isolated from shipped product paths and should avoid unnecessary architecture churn.

**Alternative considered:** keep this PRD at exploration-only status without any concrete spike shape. Rejected: the narrowed Phase 1 design is now specific enough to support immediate experimentation while still avoiding premature production commitments.

### 3. Phase 1 spike constraints

The default Phase 1 model guidance is:

- use `EmbeddingModel::BGESmallENV15` as the quality-first anchor
- use `EmbeddingModel::BGESmallENV15Q` as the footprint-first anchor
- do not start with `BGEM3`
- do not start with `bge-micro-v2`
- optionally compare against `AllMiniLML6V2` later only if results are ambiguous and a benchmark anchor is needed

Phase 1 storage and query rules:

- store vectors as `f32` little-endian blobs or another simple, explicitly documented equivalent
- L2-normalize vectors before ranking so cosine similarity reduces to a dot product at query time
- apply scoping and explicit filters before vector ranking so the candidate set remains bounded
- rank candidates in process rather than requiring SQLite vector extension support
- define one stable text-serialization format for embedding input and use it consistently during the experiment
- keep missing-embedding or partial-coverage behavior explicit in evaluation results

**Alternative considered:** compare several local embedding engines or index paths immediately. Rejected: that introduces too many variables before the team knows whether the simpler local architecture is viable.

### 4. Phase 2 production integration

If Phase 1 shows useful retrieval quality at acceptable local cost, Phase 2 should be the first production integration into `themion-core` and `themion-cli`.

Phase 2 would define and implement:

- the shipped Project Memory semantic retrieval surface
- the production storage or schema changes if needed
- lifecycle behavior for create, update, and backfill
- inspectable result presentation
- fallback or degraded behavior when embeddings are missing or unavailable

Phase 2 should preserve the product behavior defined earlier in this PRD rather than turning the spike artifact directly into a permanent product surface without review.

**Alternative considered:** treat the Phase 1 artifact as the production implementation with only minor cleanup. Rejected: spike code and production behavior should be reviewed separately so architecture, schema, and user-facing semantics remain intentional.

### 5. Later phases and follow-on directions

Potential later-phase directions include:

- `sqlite-vec` or another SQLite-native vector path if Phase 2 query latency or scale becomes limiting
- asynchronous or deferred embedding lifecycle work if synchronous updates prove too expensive in production
- broader model comparisons if Phase 1 or Phase 2 quality or resource tradeoffs remain unclear
- more hybrid ranking strategies if exact-plus-semantic retrieval needs refinement after real usage

These follow-ons should not block Phase 1 experimentation or the first eventual production delivery.

**Alternative considered:** fold all future scale and optimization work into the Phase 1 proposal. Rejected: that would blur the product requirement and make the evaluation step harder to execute clearly.

## Changes by Component

| Component / file area | Change |
| --- | --- |
| temporary spike artifact location such as `scripts/` or another intentionally non-production path | Add a lightweight isolated prototype for embedding generation, vector storage, and bounded semantic ranking over representative Project Memory-like data. |
| sample or evaluation data preparation | Define the representative corpus shape and query set used to evaluate paraphrase recall, scoping behavior, and runtime or storage tradeoffs. |
| docs and PRD notes | Document the spike setup, measured results, recommendation for Phase 2, and criteria for moving to production integration. |
| `crates/themion-core/src/` Project Memory storage and query code | Phase 2: add the production embedding storage and query path chosen from Phase 1 findings while preserving existing exact retrieval behavior. |
| `crates/themion-core/src/` tool layer for Project Memory | Phase 2: add or extend the explicit shipped semantic-retrieval surface while preserving current exact-search behavior. |
| `crates/themion-core/src/` provider or integration support | Phase 2: add the production local embedding integration and lifecycle support selected from the Phase 1 recommendation. |
| `crates/themion-cli/src/` user-facing wiring and presentation | Phase 2: expose semantic retrieval results clearly enough that the shipped mode is explicit and inspectable. |

## Edge Cases

- Some nodes may have little or no embed-worthy text. Phase 1 should define whether those items are skipped or represented with a reduced text shape rather than silently producing meaningless vectors.
- The spike corpus may not perfectly match future production corpus scale. Phase 1 results should call out representativeness limits rather than overstating confidence.
- If local embedding initialization fails or model assets are unavailable, the spike should record the failure mode clearly rather than hiding it behind fallback behavior.
- If title-only, content-only, or hashtag-heavy cases behave differently, the evaluation should call out the impact of the chosen text-serialization format.
- If the two candidate models produce materially different ranking quality or runtime or storage costs, the Phase 2 recommendation should state that explicitly rather than forcing an early default by assumption.
- If later phases introduce new indexing or lifecycle mechanics, those changes should preserve the same product behavior rather than silently changing scoping or retrieval semantics.

## Migration

- No shipped product migration is required in Phase 1 because the spike should not change the main Project Memory storage or tool surface.
- Phase 1 should document what migration questions Phase 2 will need to answer, such as schema additions, backfill approach, and degraded behavior when embeddings are missing.
- Phase 2 should define the production migration shape for storage, backfill, and degraded behavior if shipped semantic retrieval is introduced.
- If the spike uses exported or copied Project Memory-like data, that data-preparation path should be documented well enough that results can be repeated.

## Testing

- run the Phase 1 spike on representative Project Memory-like nodes with paraphrased but semantically related wording → verify: semantic ranking surfaces relevant nodes that exact-only retrieval would miss or rank lower
- compare exact retrieval expectations against the same corpus → verify: the spike demonstrates additive value rather than redefining what exact retrieval already does well
- evaluate spike queries under project-like and `[GLOBAL]`-like scopes → verify: the prototype preserves the intended scoping semantics during ranking
- run Phase 1 with `BGESmallENV15` and `BGESmallENV15Q` on the same corpus → verify: the quality-versus-size tradeoff is measured directly rather than assumed
- measure embedding generation, cold-start cost, warm query latency, and storage or runtime impact → verify: the Phase 2 recommendation is backed by concrete observations
- inspect the prototype's embedded text serialization and ranking behavior → verify: the experiment remains understandable and reproducible
- simulate missing model assets, initialization failure, or partially prepared corpora → verify: failure modes are explicit in spike results
- use the shipped semantic retrieval path on Project Memory nodes with paraphrased but semantically related wording → verify: relevant nodes are returned while existing exact retrieval remains available
- use existing exact keyword or hashtag retrieval on the same corpus → verify: current exact-search behavior remains predictable and preserved
- query shipped semantic retrieval within one project and with `project_dir="[GLOBAL]"` → verify: scoping semantics match existing Project Memory boundaries
- create and update nodes that affect the chosen embedded text shape → verify: the production embedding lifecycle behaves consistently with the shipped serialization contract
- simulate missing model assets, initialization failure, or partially embedded corpora → verify: degraded behavior stays explicit and exact retrieval remains usable

## Implementation checklist

Completing the Phase 1 items does not complete the full PRD. It completes only the spike needed to validate the technical approach before Phase 2 production integration.

### Phase 1 checklist

- [ ] choose and create a lightweight isolated Phase 1 spike artifact outside shipped Project Memory paths
- [ ] define a representative Project Memory-like evaluation corpus and query set
- [ ] add local embedding integration through `fastembed` in the spike artifact
- [ ] evaluate `BGESmallENV15` and `BGESmallENV15Q` as the initial model pair
- [ ] define one stable text-serialization format for embedding input during the experiment
- [ ] store vectors in a simple documented shape and rank bounded candidates in process
- [ ] preserve project-like scoping and filter semantics in the evaluation logic
- [ ] document partial-coverage and failure-mode behavior clearly enough for Phase 1 results
- [ ] measure and record create, update, and query latency, cold-start cost, and storage or runtime impact
- [ ] document the Phase 1 recommendation for whether and how to proceed to Phase 2 production integration

### Phase 2 checklist

- [ ] define the shipped semantic-retrieval product surface for Project Memory while preserving existing exact retrieval
- [ ] add the production local embedding integration chosen from Phase 1 findings
- [ ] define and implement the production storage and query shape for embeddings
- [ ] refresh or backfill embeddings according to the chosen production lifecycle contract
- [ ] preserve Project Memory scoping and filter semantics in shipped semantic retrieval
- [ ] define degraded behavior clearly enough for partially embedded or temporarily unavailable semantic retrieval
- [ ] document the implemented shipped behavior and any follow-on pressure toward later-phase indexing or lifecycle work
