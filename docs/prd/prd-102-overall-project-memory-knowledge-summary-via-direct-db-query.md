# PRD-102: Overall Project Memory and Knowledge Summary via Direct Database Query

- **Status:** Implemented
- **Version:** v0.62.0
- **Scope:** `themion-web`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-04

## Summary

- Themion stores Project Memory in SQLite, but `themion-web` still lacks a browser-visible overview of what durable knowledge actually exists.
- Add one read-only knowledge summary page in `themion-web` that queries the active `system.db` file directly instead of requiring new core runtime APIs.
- The first implementation should answer a small fixed set of operator questions clearly: how much memory exists, what kinds it contains, how it is tagged, how connected it is, where it is scoped, and whether it has been updated recently.
- Keep the feature local-first and read-only, with explicit visibility into the exact database path and summary freshness.
- Leave detailed node browsing, memory editing, and generic database inspection to later PRDs.

## Goals

- Show an overall Project Memory and knowledge summary in `themion-web` by reading the active SQLite database file directly.
- Provide a browser-visible at-a-glance answer to the operator questions:
  - how many memory nodes exist
  - what `node_type` categories dominate
  - which hashtags are used most
  - how connected the graph is
  - whether knowledge is project-local or concentrated in `[GLOBAL]`
  - whether knowledge capture appears recent or stale
- Keep the page read-only and truthful about the exact source of summarized data.
- Preserve the repository architecture rule that the browser surface observes persisted truth and does not become the owner of memory semantics, mutation policy, or runtime orchestration.
- Keep the first implementation self-contained in `themion-web` unless a later approved PRD explicitly widens scope.

## Non-goals

- No requirement in this PRD to implement in-browser Project Memory editing, deletion, or creation.
- No requirement in this PRD to expose raw SQL execution or a generic database browser.
- No requirement in this PRD to add new `memory_*` tools or change existing tool contracts.
- No requirement in this PRD to add new runtime snapshot APIs to `themion-core` or `themion-cli` for this first slice.
- No requirement in this PRD to replace `unified_search`, `memory_open_graph`, `memory_get_node`, or other existing retrieval surfaces.
- No requirement in this PRD to summarize chat history, tool-call history, or other non-memory domains beyond what is necessary to identify the active knowledge source.
- No requirement in this PRD to deliver browser-native agent chat, memory authoring, or graph-editing workflows.
- No requirement in this PRD to claim internet-facing hardening or multi-user access control.

## Background & Motivation

### Current state

Themion already stores durable Project Memory knowledge in canonical SQLite tables:

- `memory_nodes`
- `memory_node_hashtags`
- `memory_edges`

Related derived indexing tables such as `unified_search_documents` and `unified_search_chunks` also exist, but they are not the canonical source of truth for Project Memory itself.

`themion-web` now exists as a separate local web surface and dedicated binary, but the currently landed implementation is terminal-focused. It does not yet provide a browser-visible summary of the knowledge base already persisted in the active database.

### Why this matters now

A high-level knowledge summary is one of the clearest useful browser monitoring views because it answers practical questions without requiring the operator to compose tool calls or inspect SQLite manually.

Today, learning whether a project has rich memory coverage or sparse disconnected knowledge often requires several manual steps, for example:

- `memory_list_hashtags`
- `memory_open_graph`
- `unified_search`
- direct SQLite inspection

That is powerful but slower than a dedicated summary surface. The operator should be able to open `themion-web` and immediately understand the overall shape of the knowledge base.

**Alternative considered:** add a new runtime-owned summary API in `themion-core` and have `themion-web` consume that. Rejected for this first slice: direct database querying is the requested product direction, and it keeps the web feature aligned with the current `themion-web` strategy of consuming existing external state without adding new adapters to existing crates.

## Design

### 1. Add one dedicated knowledge-summary page to `themion-web`

`themion-web` should gain a dedicated memory or knowledge page focused on overall Project Memory summary rather than a generic placeholder dashboard.

Required behavior:

- the web UI should expose a dedicated summary page reachable from the normal navigation flow
- the page should present aggregate knowledge information by default rather than raw records
- the page should clearly state that it summarizes Project Memory from the active SQLite database
- the page should render a meaningful empty state when no Project Memory rows exist yet

### 2. Query the active SQLite database file directly and read-only

The first implementation should read the database file directly from `themion-web`.

Required behavior:

- `themion-web` should open the active Themion SQLite database file directly for read-only querying
- default path resolution should follow the documented repository behavior:
  - `$XDG_DATA_HOME/themion/system.db`
  - `~/.local/share/themion/system.db` when `XDG_DATA_HOME` is unset
- if a custom database path is later supported for `themion-web`, the UI must still show the actual resolved path being summarized
- the summary feature must not require write access and must not mutate the database
- the implementation should treat live SQLite tables as source of truth rather than copying the data into a second browser-owned store

### 3. Standardize the exact first-page summary sections

The first page should answer a fixed set of questions with a stable summary layout.

Required behavior:

- the page should include a top-level overview section with:
  - total Project Memory node count
  - total edge count
  - count of distinct hashtags
  - most recent `updated_at_ms` value across `memory_nodes`, rendered as a readable timestamp
- the page should include a node-type section showing counts grouped by `node_type`
- the page should include a hashtag section showing the top hashtags by usage count, with a bounded list rather than an unbounded dump
- the page should include a relation section showing counts grouped by `relation_type`
- the page should include a scope section showing counts grouped by `project_dir`, including an explicit `[GLOBAL]` bucket when present
- the page should include a graph-shape section showing at least:
  - nodes with at least one edge
  - nodes with no edges
  - average or ratio context that helps the operator judge whether the graph is sparse or linked
- the page should include a recent-activity section showing a bounded list or compact histogram-style summary of recently updated memory nodes

Implementation-ready contract:

- the first implementation should bound the top-hashtag list and recent-activity list explicitly rather than rendering an arbitrarily large result set
- the first implementation should use canonical Project Memory tables for all main counts:
  - `memory_nodes`
  - `memory_node_hashtags`
  - `memory_edges`

### 4. Make source identity and freshness explicit in the UI

The operator should be able to tell exactly what data was summarized and when.

Required behavior:

- the summary page should show the resolved database path
- the page should show when the summary payload was generated or refreshed
- if the summary view can be refreshed manually, the UI should acknowledge the refresh time without implying a background subscription model
- if the database file is missing, unreadable, or incompatible with the expected schema, the page should show a clear operator-facing error state rather than silently rendering zeros
- if the database is readable but contains no memory rows, the page should show an empty-state summary rather than an error state

### 5. Distinguish canonical memory truth from derived search/index data

The page must avoid confusing Project Memory with derived indexing state.

Required behavior:

- the main knowledge summary must be derived from canonical memory tables, not from `unified_search_*` tables
- if the UI later chooses to mention unified-search indexing status, that information must be visually secondary and labeled as derived index state
- stale or empty derived indexing tables must not cause canonical memory counts to disappear or become misleading

### 6. Keep the product optimized for overview legibility, not full browsing

The first product need is a broad overview that is easy to scan quickly.

Required behavior:

- prefer compact cards, grouped summary sections, and bounded tables over long raw listings
- the page should stay useful for both sparse and dense knowledge bases
- if drill-down links are added, they should remain secondary and should not turn this PRD into a full browser memory explorer
- low-data projects should still render clear explanatory text instead of a visually broken dashboard

### 7. Preserve web-surface ownership boundaries

This summary feature must respect repository layering guidance.

Required behavior:

- `themion-web` may read and summarize persisted memory data, but it must not become the owner of memory mutation rules, graph-maintenance rules, or indexing policy
- this PRD must not add write paths into Project Memory tables
- if a later feature would require memory mutation, node editing, graph repair, or runtime-owned summary semantics, that belongs in a later PRD rather than being folded silently into this one

### 8. Leave room for later knowledge-inspection follow-up work

This PRD should stay specific without overstating later browser knowledge features.

Required behavior:

- later PRDs may add individual node browsing, hashtag drill-downs, graph-neighborhood views, or unified-search-backed exploration
- later PRDs may add broader database/config inspection pages in `themion-web`
- the first landed summary should not be described as a complete browser knowledge-management interface

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-102-overall-project-memory-knowledge-summary-via-direct-db-query.md` | Define the product requirement for a read-only browser-visible overall Project Memory summary sourced directly from the SQLite database file. |
| `docs/README.md` | Add the PRD-102 entry in sorted order and reflect implementation-ready status/version. |
| `crates/themion-web/src/` | Add database-path resolution, read-only summary query logic, summary-state error handling, and the browser UI page for overall Project Memory/knowledge summary presentation. |
| `crates/themion-web/README.md` | Update the documented crate scope when the summary view lands so the web monitoring surface reflects actual behavior. |
| relevant architecture/docs files if implementation lands | Update web-surface and monitoring docs so they describe the new summary view and its direct-database-query ownership model accurately. |

## Edge Cases

- the active database file does not exist yet → verify: the UI reports a missing database state clearly and includes the attempted path.
- the database exists but contains no `memory_nodes` rows → verify: the UI shows a valid empty-state summary rather than an error.
- the database contains only `[GLOBAL]` knowledge and no current-project knowledge → verify: the scope section makes that distribution explicit.
- the database contains many hashtags but only a few dominant ones → verify: the hashtag section shows a bounded top list without flooding the page.
- many memory nodes have no edges → verify: the graph-shape section shows sparsity explicitly rather than implying strong linkage.
- the derived unified-search tables are stale, empty, or absent while canonical memory tables are populated → verify: canonical knowledge counts still render correctly.
- the database file is locked by a running Themion process → verify: read-only summary queries still behave safely or fail with a clear operator-facing message.
- the schema is older or partially missing expected memory tables → verify: the page reports an incompatible-schema state clearly instead of silently rendering misleading zeros.

## Migration

This is an additive browser monitoring capability and should not require a database migration.

Expected rollout behavior:

- existing TUI, headless, and current `themion-web` terminal workflows remain valid
- `themion-web` gains an additional read-only knowledge summary surface
- no Project Memory data rewrite or backfill is required for the first summary view
- absent or empty databases should still produce predictable UI behavior without requiring the web surface to initialize or repair the database itself

## Testing

- start `themion-web` against a normal local Themion database with existing memory data → verify: the browser UI shows overview, node-type, hashtag, relation, scope, graph-shape, and recent-activity sections sourced from the database.
- start `themion-web` when the resolved database path is missing → verify: the summary page shows a clear missing-database state including the attempted path.
- populate memory data across multiple `node_type` values, hashtags, and relation types → verify: grouped summary sections reflect the expected counts.
- populate both project-local and `[GLOBAL]` memory nodes → verify: the scope section distinguishes them clearly.
- populate memory nodes with and without edges → verify: the graph-shape section reflects connected versus unconnected nodes accurately.
- simulate stale or empty `unified_search_*` tables with populated canonical memory tables → verify: the summary still reflects the canonical memory state correctly.
- run `cargo check -p themion-web` after implementation → verify: the touched crate builds cleanly in its default configuration.
- run `cargo check --all-features -p themion-web` after implementation → verify: the touched crate builds cleanly across feature combinations.

## Implementation checklist

- [x] add a dedicated Project Memory/knowledge summary page to `themion-web`
- [x] resolve the active SQLite database path and show the actual summarized path in the UI
- [x] implement direct read-only summary queries over canonical Project Memory tables
- [x] implement the fixed first-page sections: overview, node types, hashtags, relations, scope, graph shape, and recent activity
- [x] distinguish missing database, incompatible schema, empty knowledge base, and ordinary populated states clearly
- [x] keep the implementation read-only and local-first without introducing new runtime ownership paths
- [x] update relevant docs when implementation lands

## Implementation notes

- Landed in `v0.62.0`.
- Implemented the knowledge summary inside `themion-web` without adding new APIs to `themion-core` or `themion-cli`.
- The summary reads the active SQLite database directly, using documented default `system.db` path resolution and optional `THEMION_WEB_DB_PATH` override support for the web binary.
- The first shipped page includes overview, node-type, hashtag, relation, scope, graph-shape, and recent-activity sections.
- Canonical counts come from `memory_nodes`, `memory_node_hashtags`, and `memory_edges`; derived `unified_search_*` tables are not used for the main summary.
- The UI distinguishes missing database, incompatible schema, query-error, empty knowledge base, and populated knowledge states.
