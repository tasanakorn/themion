# PRD-059: Add Vector Embedding and Semantic Search for Project Memory

- **Status:** Partially implemented (Phase 1 complete; Phase 2 ready to start)
- **Version:** v0.37.0
- **Scope:** phased delivery: Phase 1 spike artifact and evaluation plus Phase 2 feature-flagged production integration for `themion-core`/`themion-cli`, with later optimization follow-ons if warranted
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Project Memory currently supports keyword, hashtag, node-type, and graph-link retrieval, but not semantic retrieval when the query wording differs from the stored wording.
- Themion should add additive semantic retrieval for Project Memory without replacing current exact-search behavior.
- Phase 1 is complete as an isolated spike to validate local embeddings, bounded ranking, and practical cost before changing shipped storage or tool surfaces, with direct measurements captured for `BGESmallENV15`, `BGESmallENV15Q`, `bge-micro-v2`, and `BGEM3`. The current spike recommendation is to use `bge-micro-v2` as the first production implementation target for Phase 2.
- If the spike is successful, Phase 2 becomes the first feature-flagged production integration into `themion-core` and `themion-cli`, starting with `bge-micro-v2`.
- Later phases remain optional follow-on work for scale, indexing, or lifecycle improvements.

## Goals

- Deliver a shipped Project Memory feature that adds explicit semantic retrieval while preserving current exact-search behavior.
- Keep the product requirement clear even though delivery is phased: the PRD is complete only when shipped Project Memory supports additive semantic retrieval with predictable scoping and inspectable behavior.
- Start Phase 1 now to validate whether local embeddings improve recall for paraphrased or differently worded queries on realistic Project Memory-like data.
- Measure the practical tradeoffs of local embedding generation, vector storage shape, and in-process similarity ranking before production integration.
- Preserve current project scoping and explicit `[GLOBAL]` selection semantics in both the spike and the eventual shipped feature.
- Produce a concrete recommendation for a Phase 2 implementation that fits Themion's local-first SQLite architecture.

## Non-goals

- No always-on production integration into `memory_search` or another shipped Project Memory tool surface in Phase 1.
- No requirement to modify the main Project Memory storage schema in `themion-core` during the spike.
- No remote hosted vector database or remote embedding service.
- No embedding of every historical transcript or board note.
- No broad retrieval-augmented generation redesign across all context sources.
- No silent cross-project retrieval changes; project scoping rules should remain explicit.
- No requirement to adopt `sqlite-vec` or multiple local embedding engines in Phase 1.

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

The product goal is clear, and the repository has now landed Phase 1. The technical shape still needs validation: embedding engine choice, storage representation, ranking path, lifecycle cost, and operational complexity.

This PRD therefore separates the work into phases:

- Phase 1 is complete as an isolated spike artifact.
- Phase 2 now starts from those validated findings for the first shipped semantic-retrieval feature.
- Later phases refine scale, lifecycle, or ranking only if earlier phases show clear pressure.

**Alternative considered:** implement the simplest production path directly in Phase 1. Rejected: that would commit the main codebase to persistence, lifecycle, and tool-surface decisions before the team knows whether the approach is worthwhile.

### Phase 1 recommendation basis

Current research is sufficient to start the Phase 1 spike without pretending the production design is already settled.

Recommended Phase 1 defaults:

- embedding library: `fastembed`
- initial evaluation set: `BGESmallENV15`, `BGESmallENV15Q`, `bge-micro-v2`, and `BGEM3`, with the current Phase 1 recommendation targeting `bge-micro-v2` for the first production implementation
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

The shipped API shape should extend the existing `memory_search` surface with an explicit retrieval mode rather than introducing a separate primary search tool.

Required retrieval modes for the shipped path:

- explicit `fts` mode to preserve current full-text search behavior
- explicit `semantic` mode to request embedding/vector ranking
- optional later hybrid behavior only if it stays clearly inspectable

This PRD is complete only when that shipped product behavior exists in the main codebase. Completing Phase 1 alone does not complete the PRD.

**Alternative considered:** make all memory search semantic by default. Rejected: that would make retrieval less predictable, harder to debug, and more difficult to validate against existing workflows.

### 2. Phase 1 delivery slice: isolated spike artifact

Phase 1 is a landed spike, not production integration.

Phase 1 should include:

- `fastembed`
- evaluation coverage for `BGESmallENV15`, `BGESmallENV15Q`, `bge-micro-v2`, and `BGEM3`
- a documented recommendation to take `bge-micro-v2` forward as the first production implementation target unless later Phase 2 constraints overturn it
- spike design notes for a feature-flagged production rollout rather than an always-on replacement
- local embedding generation only
- SQLite-backed storage in ordinary rows or blobs only inside the spike artifact
- local query execution only
- a dedicated Tokio runtime direction for future background index generation and refresh work
- an isolated prototype that evaluates semantic retrieval on realistic Project Memory-like data without changing shipped tool surfaces

Phase 1 is successful when it produces:

- a separate temporary artifact for embedding generation and bounded similarity ranking
- documented latency and resource observations
- documented retrieval-quality observations on representative queries
- a recommendation on whether Phase 2 should integrate the simple baseline or pivot toward another design such as `sqlite-vec`

The spike artifact should remain clearly isolated from shipped product paths and should avoid unnecessary architecture churn.

**Alternative considered:** keep this PRD at exploration-only status without any concrete spike shape. Rejected: the landed Phase 1 artifact is specific enough to support repeatable experimentation while still avoiding premature production commitments.

### 3. Phase 1 spike constraints

The current Phase 1 model guidance is:

- keep `EmbeddingModel::BGESmallENV15` as a balanced comparison anchor
- keep `EmbeddingModel::BGESmallENV15Q` as a reduced-download comparison anchor
- use `EmbeddingModel::BGESmallZHV15` (`bge-micro-v2`) as the current first production implementation target for Phase 2 based on the measured Phase 1 tradeoffs
- keep `EmbeddingModel::BGEM3` as a high-cost comparison anchor rather than a default production target unless later quality evidence justifies it

Phase 1 storage and query rules:

- store vectors as `f32` little-endian blobs or another simple, explicitly documented equivalent
- L2-normalize vectors before ranking so cosine similarity reduces to a dot product at query time
- apply scoping and explicit filters before vector ranking so the candidate set remains bounded
- rank candidates in process rather than requiring SQLite vector extension support
- define one stable text-serialization format for embedding input and use it consistently during the experiment
- keep missing-embedding or partial-coverage behavior explicit in evaluation results

**Alternative considered:** compare several local embedding engines or index paths immediately. Rejected: that introduces too many variables before the team knows whether the simpler local architecture is viable.

### 4. Phase 2 production integration

Phase 1 has shown useful retrieval quality at acceptable local spike cost, so Phase 2 should now be the first production integration into `themion-core` and `themion-cli`.

Phase 2 would define and implement:

- a feature-flagged shipped Project Memory semantic retrieval surface
- an extension of the existing `memory_search` tool with an explicit search mode that can request `fts` or `semantic` retrieval
- the production storage or schema changes needed for embedding rows and index bookkeeping
- lifecycle behavior for create, update, pending refresh, and backfill
- background embedding generation on a dedicated Tokio runtime rather than the interactive agent execution pool
- a slash command to trigger generation of missing or pending embedding indexes and a full regeneration path
- a CLI command that allows shell-driven indexing or regeneration outside the tool surface for rare maintenance workflows
- inspectable result presentation
- fallback or degraded behavior when embeddings are missing or unavailable

Phase 2 should preserve the product behavior defined earlier in this PRD rather than turning the spike artifact directly into a permanent product surface without review.

**Alternative considered:** treat the Phase 1 artifact as the production implementation with only minor cleanup. Rejected: spike code and production behavior should be reviewed separately so architecture, schema, and user-facing semantics remain intentional.

### 5. Later phases and follow-on directions

Potential later-phase directions include:

- `sqlite-vec` or another SQLite-native vector path if Phase 2 query latency or scale becomes limiting
- broader model comparisons if Phase 1 or Phase 2 quality or resource tradeoffs remain unclear
- more hybrid ranking strategies if exact-plus-semantic retrieval needs refinement after real usage
- additional indexing automation once the feature-flagged background runtime and command surfaces prove stable

These follow-ons should not block Phase 1 experimentation or the first eventual production delivery.

**Alternative considered:** fold all future scale and optimization work into the Phase 1 proposal. Rejected: that would blur the product requirement and make the evaluation step harder to execute clearly.

## Changes by Component

| Component / file area | Change |
| --- | --- |
| temporary spike artifact location such as `scripts/` or another intentionally non-production path | Add a lightweight isolated prototype for embedding generation, vector storage, and bounded semantic ranking over representative Project Memory-like data. |
| sample or evaluation data preparation | Define the representative corpus shape and query set used to evaluate paraphrase recall, scoping behavior, and runtime or storage tradeoffs. |
| docs and PRD notes | Document the spike setup, measured results, recommendation for Phase 2, and criteria for moving to production integration. |
| `crates/themion-core/src/` Project Memory storage and query code | Phase 2: add feature-flagged embedding storage, pending-index bookkeeping, and the production semantic query path while preserving existing exact retrieval behavior. |
| `crates/themion-core/src/` tool layer for Project Memory | Phase 2: extend the existing `memory_search` tool with an explicit `fts` vs `semantic` search mode while preserving current exact-search semantics by default. |
| `crates/themion-core/src/` provider or integration support | Phase 2: add the production local embedding integration, dedicated background indexing runtime hooks, and lifecycle support selected from the Phase 1 recommendation. |
| `crates/themion-cli/src/` slash-command handling and app wiring | Phase 2: add user-triggered commands to generate missing or pending indexes and to force full regeneration through the running app. |
| `crates/themion-cli/src/` standalone CLI command surface | Phase 2: add a shell-invokable maintenance command for rare indexing or regeneration workflows outside the tool surface. |
| `crates/themion-cli/src/` user-facing wiring and presentation | Phase 2: expose semantic retrieval results and indexing status clearly enough that the shipped mode is explicit and inspectable. |

## Edge Cases

- Some nodes may have little or no embed-worthy text. Phase 1 should define whether those items are skipped or represented with a reduced text shape rather than silently producing meaningless vectors.
- The spike corpus may not perfectly match future production corpus scale. Phase 1 results should call out representativeness limits rather than overstating confidence.
- If local embedding initialization fails or model assets are unavailable, the spike should record the failure mode clearly rather than hiding it behind fallback behavior.
- If title-only, content-only, or hashtag-heavy cases behave differently, the evaluation should call out the impact of the chosen text-serialization format.
- If the two candidate models produce materially different ranking quality or runtime or storage costs, the Phase 2 recommendation should state that explicitly rather than forcing an early default by assumption.
- If semantic mode is requested while embeddings are missing, stale, or still pending regeneration, degraded behavior should stay explicit and should not silently masquerade as semantic success.
- Slash-command and CLI-triggered regeneration should avoid blocking the interactive agent loop on long indexing work by routing the work onto the dedicated background runtime.
- If later phases introduce new indexing or lifecycle mechanics, those changes should preserve the same product behavior rather than silently changing scoping or retrieval semantics.

## Migration

- No shipped product migration is required in Phase 1 because the spike should not change the main Project Memory storage or tool surface.
- Phase 1 should document what migration questions Phase 2 will need to answer, such as feature-flag gating, schema additions, backfill approach, and degraded behavior when embeddings are missing.
- Phase 2 should define the production migration shape for storage, backfill, pending-index state, and degraded behavior if shipped semantic retrieval is introduced.
- If the spike uses exported or copied Project Memory-like data, that data-preparation path should be documented well enough that results can be repeated.

## Testing

- run the Phase 1 spike on representative Project Memory-like nodes with paraphrased but semantically related wording → verify: semantic ranking surfaces relevant nodes that exact-only retrieval would miss or rank lower
- compare exact retrieval expectations against the same corpus → verify: the spike demonstrates additive value rather than redefining what exact retrieval already does well
- evaluate spike queries under project-like and `[GLOBAL]`-like scopes → verify: the prototype preserves the intended scoping semantics during ranking
- run Phase 1 across `BGESmallENV15`, `BGESmallENV15Q`, `bge-micro-v2`, and `BGEM3` on the same corpus → verify: the model tradeoffs and the first production implementation recommendation are measured directly rather than assumed
- measure embedding generation, cold-start cost, warm query latency, and storage or runtime impact in the isolated spike → verify: the Phase 2 recommendation is backed by concrete observations
- inspect the prototype's embedded text serialization and ranking behavior → verify: the experiment remains understandable and reproducible
- simulate missing model assets, initialization failure, or partially prepared corpora → verify: failure modes are explicit in spike results
- use the feature-flagged `memory_search` path in `semantic` mode on Project Memory nodes with paraphrased but semantically related wording → verify: relevant nodes are returned while existing exact retrieval remains available
- use the same shipped `memory_search` path in explicit `fts` mode on the same corpus → verify: current full-text search behavior remains predictable and preserved
- query shipped semantic retrieval within one project and with `project_dir="[GLOBAL]"` → verify: scoping semantics match existing Project Memory boundaries
- create and update nodes that affect the chosen embedded text shape → verify: the production embedding lifecycle records missing or pending indexing work consistently with the shipped serialization contract
- trigger missing or pending indexing work from the slash command and the standalone CLI command → verify: both surfaces enqueue or run the expected background work without blocking normal interactive search handling
- simulate missing model assets, initialization failure, or partially embedded corpora → verify: degraded behavior stays explicit and exact retrieval remains usable

## Implementation checklist

Completing the Phase 1 items does not complete the full PRD. It completes only the spike needed to validate the technical approach before Phase 2 production integration.

### Phase 1 checklist

- [x] choose and create a lightweight isolated Phase 1 spike artifact outside shipped Project Memory paths
- [x] define a representative Project Memory-like evaluation corpus and query set
- [x] add local embedding integration through `fastembed` in the spike artifact
- [x] evaluate `BGESmallENV15`, `BGESmallENV15Q`, `bge-micro-v2`, and `BGEM3` on the same spike corpus
- [x] use the Phase 1 measurements to recommend `bge-micro-v2` as the first production implementation target
- [x] define one stable text-serialization format for embedding input during the experiment
- [x] store vectors in a simple documented shape and rank bounded candidates in process
- [x] preserve project-like scoping and filter semantics in the evaluation logic
- [x] document partial-coverage and failure-mode behavior clearly enough for Phase 1 results
- [x] document the Phase 1 recommendation for whether and how to proceed to Phase 2 production integration

### Phase 2 checklist

- [ ] gate the shipped semantic retrieval path behind an explicit feature flag while preserving existing exact retrieval when the feature is off
- [ ] extend the existing `memory_search` tool with explicit `fts` and `semantic` retrieval modes
- [ ] add the production local embedding integration chosen from Phase 1 findings
- [ ] define and implement the production storage, pending-index tracking, and query shape for embeddings
- [ ] refresh or backfill embeddings according to the chosen production lifecycle contract using a dedicated background Tokio runtime
- [ ] measure and record create, update, query, cold-start, and storage/runtime impact for the production lifecycle rather than only the isolated spike runs
- [ ] add slash-command support to trigger missing or pending indexing work and full regeneration from the running app
- [ ] add a standalone CLI maintenance command for rare shell-driven indexing or full regeneration workflows
- [ ] preserve Project Memory scoping and filter semantics in shipped semantic retrieval
- [ ] define degraded behavior clearly enough for partially embedded or temporarily unavailable semantic retrieval
- [ ] document the implemented shipped behavior and any follow-on pressure toward later-phase indexing or lifecycle work

## Phase 1 implementation notes

- Reproducible runner command for the current recommended model: `PRD059_EMBEDDING_MODEL='BGE-Micro-v2' rust-script scripts/prd059_phase1_spike.rs --artifact-dir tmp/prd059-phase1-bge-micro-refresh`
- Fresh rerun for that command completed successfully with `/usr/bin/time` reporting `MAXRSS_KB=177680` and `ELAPSED=0:10.01`; the resulting `project_plus_global` summary reported semantic `avg_query_ms=5.581317`, exact `avg_query_ms=0.0668076`, `sqlite_bytes=53248`, and embedding dimension `512`.
- Phase 1 recommendation for Phase 2: ship a feature-flagged production path that starts with `bge-micro-v2`, extends the existing `memory_search` tool with explicit `fts` and `semantic` modes, and keeps index generation off the interactive path by routing it onto a dedicated background Tokio runtime.
- Expected maintenance surfaces from the Phase 1 recommendation: a slash command for generating missing or pending indexes, a slash command for full regeneration, and a standalone CLI maintenance command for rare shell-driven indexing runs.
- Observed Phase 1 degraded/failure behavior so far: the spike fails explicitly when model initialization or asset loading fails, and semantic results depend on fully generated local artifacts rather than silently falling back to an unmarked semantic approximation.
- Landed spike runner: `scripts/prd059_phase1_spike.rs`
- Landed evaluation corpus and usage notes: `docs/prd/phase1/`
- The spike remains intentionally isolated from shipped `themion-core` Project Memory tool surfaces.
- Additional Phase 1 comparison runs were captured for `bge-micro-v2` and `BGEM3`, and the current spike recommendation is to take `bge-micro-v2` forward as the first production implementation target.
- Phase 1 is complete enough to start implementation. The remaining open work in this PRD is now all Phase 2 production integration: explicit `memory_search` retrieval modes, dedicated background indexing runtime, slash/CLI regeneration commands, and production-lifecycle measurements.

## Appendix: technical note on Phase 1 findings so far

Current measured comparison runs from the isolated ONNX-based `fastembed` spike show a clearer resource tradeoff than the initial model-size assumptions alone suggested.

| Model            | Embedding dim | Avg semantic query latency (ms) | Model-cache disk usage | Memory usage                  | Notes                                                                  |
| :--------------- | ------------: | ------------------------------: | ---------------------: | :---------------------------- | :--------------------------------------------------------------------- |
| `BGESmallENV15`  |           384 |                            8.05 |                 128 MB | ~216 MiB peak RSS            | Balanced baseline; matched the others on the small corpus retrieval metrics. |
| `BGESmallENV15Q` |           384 |                           21.38 |                  65 MB | ~293 MiB peak RSS            | Smaller disk footprint than `BGESmallENV15`, but higher measured peak RSS and slower latency in this environment. |
| `bge-micro-v2`   |           512 |                            5.37 |                  91 MB | ~173 MiB peak RSS            | Fastest measured semantic latency among the compared models in this run. |
| `BGEM3`          |          1024 |                           86.17 |                 2.2 GB | ~1.70 GiB peak RSS           | Clear cost outlier in cache size and runtime memory, even when retrieval metrics matched on the small corpus. |

Table notes:

- Latency values above use the `project_plus_global` semantic `avg_query_ms` measurements from the refreshed local spike reruns so the table compares one like-for-like scenario across all models.
- Memory usage values above are peak RSS measurements captured by rerunning `scripts/prd059_phase1_spike.rs` under `/usr/bin/time -f %M`, then converting reported KiB to approximate MiB/GiB for readability.
- All four measured models reached the same recall-at-5 and MRR-at-5 on the current small evaluation corpus, so the practical difference in this appendix is primarily runtime and storage cost rather than observed retrieval-quality separation so far.
- These findings support taking `bge-micro-v2` forward as the first production implementation target for Phase 2, while keeping `BGESmallENV15` and `BGESmallENV15Q` as useful comparison anchors and `BGEM3` as a measured high-cost reference point.
