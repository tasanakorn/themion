# PRD-097: Patch the Retired Memory-Node Embedding Runtime Mismatch

- **Status:** Implemented
- **Version:** v0.60.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-04

## Summary

- Themion already decided to retire the legacy `memory_node_embeddings` path, but current runtime code still calls that retired path when `semantic-memory` is enabled.
- Patch the bug by making the implementation match the already-established product direction: no active reads or writes to `memory_node_embeddings`, and no direct memory-only semantic search path.
- Keep ordinary Project Memory CRUD, graph links, hashtags, and exact lookup behavior unchanged.
- Prefer the smallest code-level refinement that stops the runtime error now and leaves generalized `unified_search` as the only canonical semantic path.
- Do not reopen the old table or revive the deprecated storage design as a compatibility workaround.

## Goals

- Eliminate the runtime error `no such table: memory_node_embeddings` during Project Memory create/update flows.
- Make the compiled `semantic-memory` feature path consistent with the retirement intent already documented in PRD-091 and PRD-095.
- Remove or disable stale runtime calls that still depend on `memory_node_embeddings` after schema initialization drops it.
- Keep Project Memory source-of-truth node storage authoritative and working regardless of semantic indexing state.
- Preserve the canonical product expectation that semantic memory retrieval goes through `unified_search` rather than a second direct memory-store semantic path.

## Non-goals

- No restoration of `memory_node_embeddings` as an active schema dependency.
- No redesign of generalized indexing architecture beyond what PRD-091 and PRD-095 already decided.
- No new semantic search capability beyond restoring correctness of the intended current path.
- No broad refactor of unrelated memory or search code.
- No TUI-specific behavior changes except any small user-facing degradation/error wording needed to reflect the patched behavior accurately.

## Background & Motivation

### Current state

PRD-091 established generalized `unified_search` as the canonical search surface and recorded that Project Memory should no longer keep a separate `memory_node_embeddings` semantic storage path. PRD-095 then made the retirement explicit and marked it implemented.

The codebase is currently in a mismatched intermediate state:

- schema initialization drops `memory_node_embeddings`
- generalized `unified_search` uses `unified_search_documents` and `unified_search_chunks`
- but `MemoryStore` still contains feature-gated helper paths that read and write `memory_node_embeddings`
- `memory_create_node` still reaches those helpers indirectly through `MemoryStore::create_node(...)`

That mismatch is sufficient to break ordinary Project Memory writes in `semantic-memory` builds even though the table was intentionally retired.

### Why this matters now

This is not a new product-direction question. The repository already decided the desired end state. The current problem is that implementation cleanup is incomplete in a way that causes live runtime failure.

That failure matters because:

- creating or updating Project Memory nodes can fail in builds that enable `semantic-memory`
- the failure happens on a normal write path, not only on an obscure maintenance path
- the current behavior contradicts implemented PRD notes and makes the retirement look incomplete or unreliable
- reviving the old table would move the codebase farther away from the documented canonical design rather than closer to it

**Alternative considered:** recreate `memory_node_embeddings` temporarily so old runtime code keeps working. Rejected: that preserves the deprecated design, increases semantic duplication again, and conflicts with PRD-095's retirement intent.

## Design

### 1. Treat the bug as an implementation mismatch, not as a request to restore the legacy table

The patch should align runtime behavior with the already accepted design instead of weakening that design.

Required behavior:

- builds that enable `semantic-memory` must no longer require `memory_node_embeddings` to exist for normal Project Memory operations
- schema initialization may continue to treat `memory_node_embeddings` as retired legacy state
- no new code should be added that recreates, refreshes, or queries the retired table as part of active product behavior

This keeps the implementation moving toward the existing canonical architecture instead of reopening the deprecated branch.

### 2. Remove or disable direct embedding writes from memory-node create/update paths

`MemoryStore::create_node(...)` and `MemoryStore::update_node(...)` must not call legacy embedding-write helpers that target `memory_node_embeddings`.

Required behavior:

- creating a memory node must succeed without attempting a direct write to `memory_node_embeddings`
- updating a memory node must succeed without attempting a direct write to `memory_node_embeddings`
- source-of-truth writes to `memory_nodes`, hashtags, and related graph data remain authoritative even if generalized indexing is not refreshed immediately by this patch
- if the repository already has a supported generalized per-record refresh path for `source_kind="memory"`, this patch may call that path instead of simply removing the legacy call
- if that generalized per-record refresh path is not yet ready or is too risky for a patch-sized fix, the patch should prefer explicit non-use of the legacy path over silently reviving the old behavior

Patch-sizing requirement:

- prefer the smallest code-level change that restores correctness on the memory write path
- a patch release may temporarily leave freshness to existing generalized rebuild/refresh behavior if that is the safest way to stop the runtime failure without reintroducing retired storage

**Alternative considered:** keep the old direct write and catch the missing-table error silently. Rejected: it hides a known stale path instead of removing the dependency and would still preserve misleading dead behavior.

### 3. Remove or clearly reject direct memory-only semantic search paths that still depend on the retired table

The implementation must not leave active semantic query paths that still assume `memory_node_embeddings` exists.

Required behavior:

- no live semantic or hybrid memory-search branch should query `memory_node_embeddings`
- if direct memory-store semantic retrieval is still reachable through `MemoryStore::search_nodes(...)`, the patch should either remove that branch or reject unsupported semantic mode clearly
- callers should be guided toward `unified_search` as the active semantic retrieval path for Project Memory

Patch-sizing guidance:

- a clear unsupported-mode response is acceptable for a patch if full internal migration to a generalized per-record refresh path is not yet ready
- silent fallback to obsolete legacy logic is not acceptable

### 4. Keep semantic freshness semantics explicit during the patch window

Stopping the runtime failure must not create false claims about semantic freshness.

Required behavior:

- if this patch removes immediate legacy embedding writes before a replacement immediate generalized refresh path is available, the code and docs must not imply that create/update alone guarantees immediate semantic availability
- ordinary exact/structured Project Memory behavior remains available immediately because `memory_nodes` remains the source of truth
- semantic freshness should be described in terms of the generalized index lifecycle that actually exists in the code after the patch

This PRD intentionally allows a patch-sized stabilization step that favors correctness over premature completeness, as long as the behavior is explicit.

### 5. Keep the fix consistent across feature configurations

The bug is exposed specifically in `semantic-memory` builds, so the patch must be validated there rather than only in default builds.

Required behavior:

- default builds must still compile cleanly
- `semantic-memory` builds must compile cleanly and must not retain live references that make normal Project Memory writes fail at runtime
- all-features builds for touched crates must still compile cleanly

## Changes by Component

| Component | Change |
| --- | --- |
| `docs/prd/prd-097-patch-retired-memory-node-embedding-runtime-mismatch.md` | Define the patch-level refinement of the already-approved retirement path and record that the goal is to remove the runtime mismatch rather than restore the old table. |
| `crates/themion-core/src/memory.rs` | Remove or disable remaining live create/update/search paths that depend on `memory_node_embeddings`, or replace them with the smallest safe canonical-path behavior. |
| `crates/themion-core/src/tools.rs` | Keep tool-layer behavior aligned with the patched Project Memory semantics, especially for `memory_create_node` and any semantic-search-facing guidance. |
| tests for Project Memory and unified search behavior | Add or update regression coverage proving that memory create/update no longer depends on `memory_node_embeddings` and that semantic callers are routed to supported current behavior only. |
| active docs in `docs/` if needed | Clarify any behavior wording that currently overstates immediate semantic freshness or implies the retired table is still active. |

## Edge Cases

- a user creates a Project Memory node in a build with `semantic-memory` enabled and no legacy table present → verify: the write succeeds without touching `memory_node_embeddings`.
- a user updates an existing Project Memory node in the same build → verify: the update succeeds without touching the retired table.
- an old database still physically contains `memory_node_embeddings` from an earlier build → verify: active behavior does not depend on it.
- a caller requests semantic memory retrieval through an old direct path → verify: the request is rejected clearly or migrated to supported `unified_search` behavior rather than using obsolete table-backed logic.
- a default build without `semantic-memory` is compiled after the cleanup → verify: no new feature-gating regressions are introduced.

## Migration

This patch should be treated as completion of a previously intended cleanup step rather than a new architectural migration.

Expected rollout behavior:

- existing databases may still contain historical legacy rows or may already have had the legacy table dropped
- active runtime behavior must succeed in both cases without relying on the legacy table
- generalized `unified_search_rebuild` remains the repair/backfill path for canonical semantic state
- the patch must not require users to recreate or manually repair the old legacy table

## Testing

- create a Project Memory node in a `semantic-memory` build against a database without `memory_node_embeddings` → verify: node creation succeeds and no missing-table runtime error occurs.
- update a Project Memory node in a `semantic-memory` build against the same database → verify: node update succeeds and no missing-table runtime error occurs.
- exercise any remaining direct semantic-memory-store entry point after the patch → verify: it no longer queries `memory_node_embeddings` and either uses supported current behavior or fails clearly.
- run `cargo check -p themion-core` after the code change → verify: default feature build still compiles.
- run `cargo check -p themion-core --features semantic-memory` after the code change → verify: feature-enabled build compiles cleanly.
- run `cargo check -p themion-core --all-features` after the code change → verify: all-feature core build compiles cleanly.
- if `themion-cli` is touched, run `cargo check -p themion-cli` after the code change → verify: default CLI build still compiles.
- if `themion-cli` is touched, run `cargo check -p themion-cli --features semantic-memory` after the code change → verify: feature-enabled CLI build compiles cleanly.
- if `themion-cli` is touched, run `cargo check -p themion-cli --all-features` after the code change → verify: all-feature CLI build compiles cleanly.

## Implementation checklist

- [x] remove or disable legacy create/update writes to `memory_node_embeddings`
- [x] remove or clearly reject any remaining direct semantic search path that depends on `memory_node_embeddings`
- [x] add regression coverage for the missing-table runtime failure scenario
- [x] update any active docs that would otherwise imply the retired table still participates in canonical behavior
- [x] validate touched crates in default, relevant feature-on, and all-features configurations
