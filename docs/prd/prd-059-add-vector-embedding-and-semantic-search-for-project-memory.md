# PRD-059: Add Vector Embedding and Semantic Search for Project Memory

- **Status:** Draft
- **Version:** >v0.36.0 +minor
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Project Memory currently supports keyword, hashtag, node-type, and graph-link retrieval, but it does not support vector or semantic search.
- This makes recall weaker when the query wording differs from the stored wording even if the underlying concept is the same.
- Themion should add an embedding-backed retrieval path for Project Memory so agents can find semantically related knowledge, not just literal keyword matches.
- Keep current search behavior and filters; semantic retrieval should be additive, inspectable, and bounded rather than replacing exact search.
- Start with a lightweight embedded vector approach that fits Themion's SQLite-backed design and can evolve later if needed.

## Goals

- Add semantic retrieval for Project Memory using embeddings.
- Improve recall when agent queries and stored knowledge use different wording for the same concept.
- Keep the current keyword and graph-based search paths available and predictable.
- Fit the implementation into Themion's existing local durable-storage model without requiring an external vector database.
- Make semantic retrieval explicit enough that users and agents can understand when it is being used.

## Non-goals

- No replacement of existing `memory_search` keyword/hashtag filtering in the first step.
- No requirement to introduce a remote hosted vector database.
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

### Why semantic retrieval is needed

Project Memory is intended to store durable facts, decisions, troubleshooting notes, files, components, conventions, and reusable observations. Many of those are naturally paraphrased.

Examples:

- a stored node says `provider responses backend drops field X under rate limits`
- a later query asks about `missing response metadata from provider when throttled`

Or:

- a stored troubleshooting note says `partial redraw leaves stale statusline on resize`
- a later query asks about `screen artifacts after terminal resize`

Keyword search may miss or under-rank these relationships even though they are semantically close.

Vector embeddings provide a practical additive path: convert node text and query text into dense vectors, then retrieve nearby nodes by similarity. For a lightweight local knowledge base, this is often enough to unlock better recall without requiring a full ontology or inference system.

**Alternative considered:** rely only on better hashtags and manual linking. Rejected: hashtags and graph edges remain valuable, but they depend on prior curation and exact labeling. Semantic retrieval helps when the wording gap itself is the problem.

## Design

### Design principles

- Keep semantic search additive and explicit rather than replacing exact search.
- Prefer a local embedded design that matches Themion's SQLite-first architecture.
- Respect current project scoping and explicit `[GLOBAL]` selection semantics.
- Make retrieval observable enough that agents can see whether results came from semantic matching.
- Bound operational complexity so the first version remains practical.

### 1. Add embedding-backed retrieval for Project Memory nodes

Themion should support generating embeddings for Project Memory nodes and querying those embeddings for nearest-neighbor style retrieval.

The first version should cover node fields that best represent durable knowledge, such as:

- title
- content
- hashtags rendered into text form when useful
- optionally lightweight structured context such as node type

This embedding-backed retrieval should be available for ordinary project-scoped Project Memory and explicit `[GLOBAL]` searches under the same scoping rules as existing memory search.

**Alternative considered:** embed only titles. Rejected: titles are often too short to capture troubleshooting details, rationale, and nuanced facts.

### 2. Keep semantic search additive to exact search

Semantic retrieval should not silently replace existing `memory_search` behavior.

Acceptable first-step patterns include:

- extend `memory_search` with an explicit semantic mode or ranking mode
- add a dedicated semantic-search tool such as `memory_semantic_search`
- support hybrid retrieval that combines keyword filtering with vector ranking when explicitly requested

The important behavior is:

- existing keyword/hashtag usage remains valid
- semantic retrieval is opt-in or otherwise clearly signaled
- results can still be filtered by `project_dir`, hashtags, node type, or linked-node constraints when that combination is practical

**Alternative considered:** make all memory search semantic by default. Rejected: that would make retrieval less predictable, harder to debug, and more difficult to validate against existing workflows.

### 3. Use an embedded/local vector storage approach

Themion should prefer an embedded/local implementation that fits its current architecture rather than introducing a mandatory external vector service.

Reasonable first-step approaches include:

- storing embedding vectors in SQLite-adjacent tables and computing similarity in-process
- using a lightweight local vector index library if it fits the dependency and operational budget
- precomputing and caching node embeddings with explicit invalidation on node update

The first implementation does not need to optimize for million-node scale. It should optimize for correctness, inspectability, and good enough performance for a local coding-agent memory graph.

**Alternative considered:** require a dedicated vector database from the start. Rejected: that adds deployment and dependency overhead inconsistent with Themion's lightweight local-first design.

### 4. Define embedding lifecycle and invalidation behavior

Semantic retrieval depends on keeping stored embeddings in sync with node content.

The implementation should define a clear lifecycle:

- create embedding when a node is created if the node has embed-worthy text
- refresh embedding when title/content/hashtags or other embedded fields change
- remove or invalidate embedding when a node is deleted
- tolerate nodes that do not yet have embeddings during rollout or partial backfill

If embedding generation fails temporarily, keyword search should continue to work and the system should degrade gracefully.

**Alternative considered:** batch-generate embeddings only out of band. Rejected: acceptable as a later optimization, but too weak as the only path because newly created knowledge would remain undiscoverable semantically until a separate job runs.

### 5. Preserve Project Memory scoping semantics

Semantic search should obey the same context boundaries as existing Project Memory retrieval.

Normative direction:

- omitted `project_dir` continues to mean the current project only
- exact `project_dir="[GLOBAL]"` searches Global Knowledge only
- project search does not silently include Global Knowledge
- any future combined current-project-plus-global mode should be explicit rather than implicit

This keeps semantic retrieval from becoming a hidden cross-project leak path.

**Alternative considered:** search all projects semantically by default because similarity benefits from a larger corpus. Rejected: wider recall is not worth surprising scope expansion.

### 6. Surface semantic match information clearly

When semantic retrieval returns results, the caller should be able to understand that semantic matching occurred.

Useful first-step result metadata may include:

- similarity score or normalized rank
- indicator that the result came from semantic matching or hybrid ranking
- brief matched text snippet or node fields used for ranking when practical

This is important both for debugging and for helping agents decide whether a retrieved node is likely relevant.

**Alternative considered:** hide ranking details and return only nodes. Rejected: lack of observability makes quality tuning and trust much harder.

## Changes by Component

| Component / file area | Change |
| --- | --- |
| `crates/themion-core/src/` Project Memory storage and tool logic | Add embedding storage/index support and semantic retrieval logic for Project Memory nodes. |
| `crates/themion-core/src/` memory create/update/delete flows | Generate, refresh, or invalidate node embeddings as node content changes. |
| `crates/themion-core/src/` tool definitions and prompt-visible tool descriptions | Expose explicit semantic-search behavior or mode while preserving current keyword search semantics. |
| `crates/themion-cli/src/` any user-visible memory result rendering | If semantic metadata is shown, keep ranking/similarity display compact and understandable. |
| `docs/architecture.md`, `docs/engine-runtime.md`, `docs/README.md`, this PRD | Document additive semantic retrieval, embedding lifecycle, scoping rules, and any new or changed tool behavior. |

## Edge Cases

- Very short nodes may produce low-value embeddings; the system should still allow exact search to carry those cases.
- Some node types may have structured text where naive embedding input loses signal; the implementation should use a stable text serialization strategy.
- Embedding generation may fail because of provider/config issues; Project Memory should remain usable through exact search and graph navigation.
- Older nodes may not have embeddings immediately after rollout; search should tolerate mixed indexed/unindexed state.
- Semantic search can return plausible but wrong neighbors; bounded result counts and visible ranking metadata should help reduce over-trust.
- Global Knowledge may contain broad reusable facts that semantically resemble project-local facts; explicit `project_dir` boundaries remain necessary.

## Migration

- Existing Project Memory nodes may initially lack embeddings.
- The rollout should support either lazy-on-read generation, on-write generation for new/updated nodes plus background backfill for old nodes, or another bounded backfill strategy.
- Existing keyword and graph search behavior should continue to work during and after rollout.
- Any schema addition for embeddings should be backward-compatible with databases that predate semantic search.

## Testing

- create two semantically related nodes with different wording and search using only one phrasing → verify: semantic retrieval returns the related node even when keyword overlap is weak
- run exact `memory_search` without semantic mode → verify: existing keyword/hashtag behavior remains unchanged
- update a node's content after embedding generation → verify: subsequent semantic retrieval reflects the updated content
- delete a node with an embedding → verify: semantic retrieval no longer returns the deleted node
- search with `project_dir` omitted versus `project_dir="[GLOBAL]"` → verify: semantic retrieval respects the same scope boundaries as normal Project Memory search
- run semantic search while embedding generation is unavailable → verify: the system fails gracefully without breaking exact Project Memory operations

## Implementation checklist

- [ ] choose the first-step tool/API shape for semantic retrieval while preserving existing exact search behavior
- [ ] add embedding persistence or index support for Project Memory nodes
- [ ] define stable text input used to generate node embeddings
- [ ] implement embedding create/update/delete lifecycle behavior
- [ ] expose semantic match metadata in results where useful
- [ ] document semantic retrieval behavior, scope boundaries, and rollout/backfill expectations
