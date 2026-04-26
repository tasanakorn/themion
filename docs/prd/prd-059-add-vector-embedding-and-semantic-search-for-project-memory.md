# PRD-059: Add Vector Embedding and Semantic Search for Project Memory

- **Status:** Proposed
- **Version:** v0.37.0
- **Scope:** phased delivery: Phase 1 spike artifact and evaluation, Phase 2 production integration planning for `themion-core`/`themion-cli`, later optimization follow-ons if warranted
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Project Memory currently supports keyword, hashtag, node-type, and graph-link retrieval, but it does not support vector or semantic search.
- This makes recall weaker when query wording differs from the stored wording even when the underlying concept is the same.
- The product outcome of this PRD is additive semantic retrieval for Project Memory: agents should be able to find semantically related knowledge without losing current exact-search behavior.
- The product outcome is intentionally delivered in multiple phases because the feature is valuable, but the technical shape should be validated before changing shipped storage, lifecycle, and tool-surface behavior.
- Phase 1 is a spike, not the first production integration: use a separate temporary artifact to evaluate local embeddings, storage shape, and bounded similarity ranking before wiring anything into the main codebase.
- Phase 2 is the first production integration into `themion-core` and `themion-cli` if the Phase 1 spike shows that the local-first approach is useful and practical.
- Later phases remain available for scale or operational follow-on work such as `sqlite-vec`, async embedding lifecycle, or broader model comparisons if earlier phases show clear pressure.

## Goals

- Deliver a shipped Project Memory feature that adds explicit semantic retrieval while preserving current exact-search behavior.
- Keep the product requirement clear even though delivery is phased: the PRD is complete only when shipped Project Memory supports additive semantic retrieval with predictable scoping and inspectable behavior.
- Use Phase 1 to validate whether local embeddings improve recall for paraphrased or differently worded queries on realistic Project Memory-like data.
- Use Phase 1 to measure the practical tradeoffs of local embedding generation, vector storage shape, and in-process similarity ranking before production integration.
- Preserve existing keyword, hashtag, node-type, and graph-link retrieval behavior in the shipped product while the spike is being evaluated.
- Preserve current project scoping and explicit `[GLOBAL]` selection semantics as part of both the spike evaluation and the eventual shipped feature.
- Produce a concrete recommendation for a production Phase 2 implementation that fits Themion's local-first SQLite architecture.

## Non-goals

- No production integration into `memory_search` or another shipped Project Memory tool surface in Phase 1.
- No requirement to modify the main Project Memory storage schema in `themion-core` during the spike.
- No requirement to introduce a remote hosted vector database or remote embedding service.
- No requirement to embed every historical transcript or board note.
- No broad retrieval-augmented generation redesign across all context sources.
- No automatic silent cross-project retrieval changes; project scoping rules should remain explicit.
- No requirement to adopt `sqlite-vec` in Phase 1.
- No requirement to implement asynchronous embedding generation in Phase 1.
- No requirement to support multiple local embedding engines in the first production implementation.

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

The current search path is simple and inspectable: it uses FTS when available, otherwise plain SQL filtering, then orders by `updated_at_ms`. There is no existing embedding table, vector index, or background indexing runtime in the current shipped design.

### Product outcome this PRD is targeting

The product requirement is not merely "add embeddings." The product requirement is that Project Memory becomes better at finding relevant knowledge when wording differs, while remaining local, predictable, and compatible with current exact retrieval.

That outcome matters because Project Memory stores durable facts, decisions, troubleshooting notes, files, components, conventions, and reusable observations that are often naturally paraphrased.

Examples:

- a stored node says `provider responses backend drops field X under rate limits`
- a later query asks about `missing response metadata from provider when throttled`

Or:

- a stored troubleshooting note says `partial redraw leaves stale statusline on resize`
- a later query asks about `screen artifacts after terminal resize`

Keyword search may miss or under-rank these relationships even though they are semantically close.

Local embeddings plus SQLite-friendly similarity search provide the additive path this PRD wants to use to reach that product outcome. For a lightweight local knowledge base, that is a better fit than assuming a hosted vector service or remote embedding dependency.

This PRD is therefore structured in phases not because the final product outcome is unclear, but because the technical route should be validated before it is committed to the main codebase.

**Alternative considered:** rely only on better hashtags and manual linking. Rejected: hashtags and graph edges remain valuable, but they depend on prior curation and exact labeling. Semantic retrieval helps when the wording gap itself is the problem.

### Why the work is phased

The feature goal is clear, but the technical shape has multiple important decisions: embedding engine choice, storage representation, ranking path, lifecycle cost, and operational complexity.

Trying to refine all of those directly in shipped code in one step would create unnecessary risk. This PRD therefore separates the work into phases:

- Phase 1 validates the technical approach in an isolated spike artifact
- Phase 2 uses the validated findings to implement the first shipped semantic-retrieval feature in the main codebase
- later phases refine scale, lifecycle, or ranking strategy only if earlier phases reveal clear pressure

That phased structure keeps the product requirement stable while allowing the technical solution to be proven before production integration.

**Alternative considered:** implement the simplest production path directly in Phase 1. Rejected: even a simple production integration would still commit the main codebase to product-surface, persistence, and lifecycle decisions before the team has measured whether the approach is worthwhile.

### External research summary informing the spike

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

This research supports a concrete Phase 1 spike path:

- local ONNX-backed embeddings through `fastembed`
- vectors stored in ordinary SQLite rows or blobs inside a separate evaluation artifact
- app-side similarity ranking over a bounded candidate set after applying project/filter-like constraints in the prototype

**Alternative considered:** start implementation directly with `sqlite-vec`. Rejected: the simpler baseline may already be sufficient for Project Memory scale and is easier to evaluate before deciding whether production integration needs vector-extension complexity.

### Model refinement for Phase 1

Additional model-specific research further narrows the likely Phase 1 choice.

What `fastembed` supports directly today:

- `EmbeddingModel::BGEM3` is supported directly and maps to `BAAI/bge-m3` with 1024-dimensional dense embeddings.
- `EmbeddingModel::BGESmallENV15` and `EmbeddingModel::BGESmallENV15Q` are supported directly and map to `BAAI/bge-small-en-v1.5` with 384-dimensional embeddings.
- `bge-micro-v2` is not a built-in `fastembed` enum model today, so it would require a user-defined or custom-model loading path.
- There is no built-in `bge-m3-tiny` or equivalent smaller `bge-m3` variant exposed in the `fastembed` Rust API.

Implications for Phase 1:

- `bge-m3` is technically attractive for multilingual retrieval, but it is substantially heavier for local-first CPU use: larger model assets, 1024-dimensional vectors, more storage per node, and more likely memory and startup pressure.
- `bge-micro-v2` is attractive on size, but using it immediately would add integration uncertainty because the work would be testing custom model loading at the same time as semantic-search architecture.
- `bge-small-en-v1.5` sits in the middle and is the more practical default: directly supported, smaller than `bge-m3`, and simpler to adopt in a first Rust/`fastembed` spike.

This means the Phase 1 model guidance for this PRD is:

- do not use `bge-m3` as the default Phase 1 model unless multilingual retrieval is a hard requirement from the beginning
- do not use `bge-micro-v2` as the default Phase 1 model because custom-model loading would add a second source of uncertainty
- prioritize `BGESmallENV15` and `BGESmallENV15Q` together so the spike can measure the quality-versus-size tradeoff directly instead of assuming it

**Alternative considered:** use `BGEM3` first because it is the most featureful BGE-family model supported by `fastembed`. Rejected: its size and dimensionality make it a poor default for a first local-only spike, and the simple dense `fastembed` path would not exercise BGE-M3's broader sparse or hybrid capabilities anyway.

## Design

### Design principles

- Keep semantic search additive and explicit rather than replacing exact search.
- Prefer a local embedded design that matches Themion's SQLite-first architecture.
- Keep embeddings local and avoid mandatory remote services in this PRD.
- Respect current project scoping and explicit `[GLOBAL]` selection semantics.
- Make retrieval observable enough that agents can see whether results came from semantic matching.
- Bound operational complexity so the first version remains practical.
- Preserve a product-oriented multi-phase roadmap while keeping the product outcome stable across phases.

### 1. Product behavior and PRD completion state

Themion should eventually add an explicit semantic retrieval path for Project Memory.

The target shipped product behavior should be:

- agents can retrieve relevant Project Memory nodes even when query wording differs from stored wording
- existing exact-search, hashtag, node-type, and graph-link retrieval continue to work
- semantic retrieval is opt-in or otherwise clearly signaled rather than silently changing all retrieval behavior
- results remain scoped by the same current-project and `[GLOBAL]` rules as existing Project Memory tools
- the retrieval mode remains inspectable enough that users and agents can understand why results were returned

Acceptable eventual API and tool shapes include:

- extending `memory_search` with an explicit semantic mode or ranking mode
- adding a dedicated semantic-search tool such as `memory_semantic_search`
- supporting hybrid retrieval that combines keyword filtering with vector ranking when explicitly requested

The important product requirement is additive semantic retrieval with predictable scoping and observability, not one specific tool name.

This PRD is complete only when that shipped product behavior exists in the main codebase. Completing Phase 1 alone does not complete the PRD; it only validates or rejects a technical route toward that product outcome.

**Alternative considered:** make all memory search semantic by default. Rejected: that would make retrieval less predictable, harder to debug, and more difficult to validate against existing workflows.

### 2. Phase 1 delivery slice: isolated spike artifact

Phase 1 is a spike, not production integration.

Phase 1 should include:

- one local embedding engine family: `fastembed`
- one supported starting model pair: `BGESmallENV15` and `BGESmallENV15Q`
- local embedding generation only
- SQLite-backed storage in ordinary rows or blobs only inside the spike artifact
- local query execution only
- an isolated prototype that evaluates semantic retrieval on realistic Project Memory-like data without changing shipped tool surfaces

The Phase 1 goal is not to build the final highest-scale vector architecture. The Phase 1 goal is to gather evidence about quality, latency, startup cost, runtime cost, and storage shape so production integration decisions are informed rather than guessed.

Phase 1 is successful when it produces:

- a separate temporary artifact used to evaluate embedding generation and bounded similarity ranking
- documented latency and resource observations
- documented retrieval-quality observations on representative queries
- a recommendation on whether Phase 2 should integrate the simple baseline into the main codebase or pivot toward another design such as `sqlite-vec`

The spike artifact should be clearly isolated from shipped product paths. It may live in a temporary script, experiment, or other intentionally non-production location, but the artifact choice should stay lightweight and avoid unnecessary architecture churn.

**Alternative considered:** keep this PRD at exploration-only status without any concrete spike shape. Rejected: the narrowed Phase 1 design is now specific enough to support a useful experiment while still avoiding premature production commitments.

### 3. Phase 1 spike constraints

Phase 1 should use `fastembed`.

The default recommendation for Phase 1 is:

- use `EmbeddingModel::BGESmallENV15` as the quality-first anchor
- use `EmbeddingModel::BGESmallENV15Q` as the footprint-first anchor
- keep the spike structured so one concrete default can be recommended for Phase 2 without redoing the whole evaluation
- optionally compare against `AllMiniLML6V2` later only if spike results are ambiguous and one public-benchmark anchor is needed
- do not start with `BGEM3`
- do not start with `bge-micro-v2`

Phase 1 storage and query rules:

- store vectors as `f32` little-endian blobs or another simple, explicitly documented equivalent inside the spike
- L2-normalize vectors before ranking so cosine similarity reduces to a dot product at query time
- keep scoping and explicit filters in front of vector ranking so the candidate set remains bounded
- rank candidates in process rather than requiring SQLite vector extension support
- keep the prototype inspectable enough that debugging can confirm what text was embedded and how ranking was derived

Phase 1 spike expectations:

- define one stable text-serialization format for embedding input and use it consistently during the experiment
- evaluate representative create, update, and query flows without wiring them into shipped Project Memory lifecycle behavior yet
- define how sample or exported Project Memory-like data is prepared for the experiment
- keep missing-embedding or partial-coverage behavior explicit in evaluation results

**Alternative considered:** compare several local embedding engines or storage or index paths immediately. Rejected: that introduces too many variables before the team knows whether the simpler local architecture is viable.

### 4. Phase 2 production integration

If Phase 1 shows useful retrieval quality at acceptable local cost, Phase 2 should be the first production integration into `themion-core` and `themion-cli`.

Phase 2 would define and implement:

- the shipped Project Memory semantic retrieval surface
- the production storage or schema changes if needed
- lifecycle behavior for create, update, and backfill
- inspectable result presentation
- fallback or degraded behavior when embeddings are missing or unavailable

Phase 2 is successful when users and agents can use the shipped Project Memory semantic-retrieval path in normal product flows while exact retrieval, scoping rules, and inspectability remain intact.

Phase 2 should preserve the product behavior defined earlier in this PRD rather than turning the spike artifact directly into a permanent product surface without review.

**Alternative considered:** treat the Phase 1 artifact as the production implementation with only minor cleanup. Rejected: spike code and production behavior should be reviewed separately so architecture, schema, and user-facing semantics remain intentional.

### 5. Later phases and follow-on directions

This PRD should retain later phases explicitly so the product outcome does not collapse into a single implementation tactic.

Potential later-phase directions include:

- `sqlite-vec` or another SQLite-native vector path if Phase 2 query latency or scale becomes limiting
- asynchronous or deferred embedding lifecycle work if synchronous updates prove too expensive in production
- broader model comparisons if Phase 1 or Phase 2 quality or resource tradeoffs remain unclear
- more hybrid ranking strategies if exact-plus-semantic retrieval needs refinement after real usage

These later phases are follow-on scale and optimization work. They should not block Phase 1 experimentation or the first eventual production delivery.

**Alternative considered:** fold all future scale and optimization work into the Phase 1 proposal. Rejected: that would blur the product requirement and make the evaluation step harder to execute clearly.

## Changes by Component

### Phase 1 expected changes

| Component / file area | Change |
| --- | --- |
| temporary spike artifact location (for example `scripts/`, `experiments/`, or another intentionally non-production path) | Add a lightweight isolated prototype for embedding generation, vector storage, and bounded semantic ranking over representative Project Memory-like data. |
| sample or evaluation data preparation | Define the representative corpus shape and query set used to evaluate paraphrase recall, scoping behavior, and runtime or storage tradeoffs. |
| docs and PRD notes | Document the spike setup, measured results, recommended production direction, and the criteria for moving to Phase 2 integration. |
| `crates/themion-core/src/` and `crates/themion-cli/src/` | No shipped semantic-search integration in Phase 1; production integration is deferred to Phase 2 after the spike recommendation. |

### Phase 2 expected changes

| Component / file area | Change |
| --- | --- |
| `crates/themion-core/src/` Project Memory storage and query code | Add the production embedding storage and query path chosen from Phase 1 findings while preserving existing exact retrieval behavior. |
| `crates/themion-core/src/` tool layer for Project Memory | Add or extend the explicit shipped semantic-retrieval surface while preserving current exact-search behavior. |
| `crates/themion-core/src/` provider or integration support | Add the production local embedding integration and lifecycle support selected from the Phase 1 recommendation. |
| `crates/themion-cli/src/` user-facing wiring and presentation | Expose semantic retrieval results clearly enough that the shipped mode is explicit and inspectable. |
| docs and PRD notes | Document the implemented shipped behavior, lifecycle behavior, storage shape, and any measured validation notes that materially affect later-phase decisions. |

## Edge Cases

- Sample or exported nodes may have little or no embed-worthy text. Phase 1 should define whether those items are skipped or represented with a reduced text shape rather than silently producing meaningless vectors.
- The spike corpus may not perfectly match future production corpus scale. Phase 1 results should call out representativeness limits rather than overstating confidence.
- If local embedding initialization fails or model assets are unavailable, the spike should record the failure mode clearly rather than hiding it behind fallback behavior.
- If title-only, content-only, or hashtag-heavy cases behave differently, the evaluation should call out the impact of the chosen text-serialization format.
- If the two candidate models produce materially different ranking quality or runtime or storage costs, the Phase 2 recommendation should state that explicitly rather than forcing an early default by assumption.
- If later phases introduce new indexing or lifecycle mechanics, those changes should preserve the same product behavior rather than silently changing scoping or retrieval semantics.

## Migration

- No shipped product migration is required in Phase 1 because the spike should not change the main Project Memory storage or tool surface.
- Phase 1 should, however, document what migration questions Phase 2 will need to answer, such as schema additions, backfill approach, and degraded behavior when embeddings are missing.
- Phase 2 should define the production migration shape for storage, backfill, and degraded behavior if shipped semantic retrieval is introduced.
- If the spike uses exported or copied Project Memory-like data, that data-preparation path should be documented well enough that results can be repeated.
- Later phases should evolve the internal implementation without breaking the explicit semantic-retrieval product contract established by this PRD.

## Testing

### Phase 1 spike validation

- run the spike on representative Project Memory-like nodes with paraphrased but semantically related wording → verify: semantic ranking surfaces relevant nodes that exact-only retrieval would miss or rank lower
- compare exact retrieval expectations against the same corpus → verify: the spike demonstrates additive value rather than redefining what exact retrieval already does well
- evaluate spike queries under project-like and `[GLOBAL]`-like scopes → verify: the prototype preserves the intended scoping semantics during ranking
- run Phase 1 with `BGESmallENV15` and `BGESmallENV15Q` on the same corpus → verify: the quality-versus-size tradeoff is measured directly rather than assumed
- measure embedding generation, cold-start cost, warm query latency, and storage or runtime impact → verify: Phase 2 recommendations are backed by concrete observations
- inspect the prototype's embedded text serialization and ranking behavior → verify: the experiment remains understandable and reproducible
- simulate missing model assets, initialization failure, or partially prepared corpora → verify: failure modes are explicit in the spike results

### Phase 2 shipped-feature validation

- use the shipped semantic retrieval path on Project Memory nodes with paraphrased but semantically related wording → verify: relevant nodes are returned while existing exact retrieval remains available
- use existing exact keyword or hashtag retrieval on the same corpus → verify: current exact-search behavior remains predictable and preserved
- query shipped semantic retrieval within one project and with `project_dir="[GLOBAL]"` → verify: scoping semantics match existing Project Memory boundaries
- create and update nodes that affect the chosen embedded text shape → verify: the production embedding lifecycle behaves consistently with the shipped serialization contract
- simulate missing model assets, initialization failure, or partially embedded corpora → verify: degraded behavior stays explicit and exact retrieval remains usable

## Implementation checklist

### Phase 1 checklist

Completing this checklist does not complete the full PRD. It completes only the spike needed to validate the technical approach before Phase 2 production integration.

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
