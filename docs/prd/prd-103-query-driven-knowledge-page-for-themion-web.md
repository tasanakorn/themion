# PRD-103: Query-Driven Knowledge Page for Themion Web

- **Status:** Implemented
- **Version:** v0.63.0
- **Scope:** `themion-web`, `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-04

## Summary

- `themion-web` already has a read-only knowledge summary page, but it still stops at aggregate state instead of letting the operator ask follow-up questions from the browser.
- Add a query workspace to the knowledge page so the operator can search and inspect stored knowledge directly from the web UI.
- Make shared `unified_search` execution the canonical browser-query backend by routing web requests through one reusable themion-core query interface instead of creating a separate web-only search model.
- Keep the current summary page as the landing view, then let operators pivot from summary cards into prefilled queries and read-only result drill-downs.
- Introduce a typed core query request/response path so the tool surface and web surface call the same search implementation.
- Keep this slice focused on retrieval and inspection; memory editing, arbitrary SQL, and a full graph-management surface remain out of scope.

## Implementation notes

- Landed with a browser query workspace in `themion-web` that reuses shared typed `themion-core` unified-search execution.
- The shipped web UI keeps the summary and query views as direct-linkable knowledge destinations instead of one always-visible two-pane layout.
- Summary-to-query pivots landed for summary tables and keep query context in browser URL state.
- Advanced structured filters, source-scope selection, degraded/error handling, and source-aware result rendering are all present in the first implementation.

## Goals

- Extend the `themion-web` knowledge page from a fixed summary view into a query-capable browser surface.
- Let an operator run targeted knowledge queries from the web UI without needing terminal tool calls.
- Reuse `unified_search` logic and semantics through one shared themion-core interface so browser query execution stays aligned with Themion's canonical retrieval surface while still allowing a memory-first web preset.
- Preserve the existing overall summary page as the default entry state and add query workflows alongside it.
- Support a compact browser workflow where summary cards, filters, results, and result drill-down all live on one knowledge page.
- Keep the browser surface read-only and focused on retrieval, inspection, and drill-down rather than knowledge mutation.
- Preserve clear ownership boundaries so the web surface projects runtime or persisted truth instead of inventing a second search model.

## Non-goals

- No requirement in this PRD to add in-browser Project Memory creation, editing, deletion, or edge mutation.
- No requirement to expose arbitrary SQL execution or a generic database-table browser.
- No requirement to duplicate all `memory_*` tools one-for-one as separate browser pages.
- No requirement to replace the existing summary sections from PRD-102.
- No requirement to invent browser-only search semantics that differ from `unified_search` when the same query concept already exists there.
- No requirement to make the browser the canonical owner of indexing policy, embedding refresh policy, or search ranking.
- No requirement in this PRD to add remote multi-user auth, internet-facing hardening, or a public search API for third-party clients.
- No requirement to ship a full node-graph explorer, graph editor, or multi-page knowledge-management UI in this slice.

## Background & Motivation

### Current state

PRD-102 added a browser-visible overall Project Memory and knowledge summary page in `themion-web`. That page is useful for answering broad questions such as how much knowledge exists, how it is distributed, and whether it appears recent.

However, the current page is still only a single state view. It does not let the operator ask the next natural questions, for example:

- what memories mention a specific topic
- which chat messages and memory nodes match a phrase
- which hashtags or node types are most relevant to a search term
- what exact records are behind the summary counts
- how a query behaves across `memory` and `chat_message` together

Themion already has a canonical query surface for this kind of retrieval through `unified_search`, with explicit project scope, source-kind selection, retrieval modes such as `fts`, `semantic`, and `hybrid`, and optional structured filters such as hashtags, node type, relation type, and linked-node targeting.

### Why this matters now

A summary-only knowledge page is a useful landing page but an incomplete operational workflow. Once an operator notices an interesting distribution or stale area in the summary, the next step is usually to query into the underlying knowledge directly.

If the browser adds a separate bespoke query model that does not match `unified_search`, users may get inconsistent results, inconsistent filtering behavior, or duplicated implementation logic. That would weaken the product expectation that Themion has one canonical search model.

**Alternative considered:** add browser query features by issuing a new web-only set of SQLite queries unrelated to `unified_search`. Rejected: the user explicitly prefers shared logic, and the product should not create a second competing search definition when a canonical retrieval surface already exists.

## Design

### 1. Keep one knowledge page with a two-pane workflow

The knowledge page should remain useful at a glance while also supporting active querying.

Required behavior:

- the current knowledge summary remains the default landing state of the page
- the page should present a stable two-pane workflow:
  - a left or top query workspace for composing and revising searches
  - a main result area that can show summary state before the first query and query results after submission
- the operator should not need to navigate to a separate route just to run a basic knowledge query
- the summary and query areas should feel like one coherent product surface, with the summary helping the operator decide what to query next
- the query UI should support repeated use during one browser session without feeling like a one-shot form submission
- the page should still render a meaningful state when no knowledge rows exist, including an empty query result state that does not look like an error

Implementation-ready UX contract:

- before the first query, the main content area should continue showing the PRD-102 summary sections
- after a query runs, the result area should switch to a results view while leaving the query form visible for refinement
- the page should include a simple control to return from results to the summary view without losing recent query state

### 2. Support summary-to-query pivots directly from the existing cards

The summary should become the starting point for targeted inspection instead of staying isolated from the query flow.

Required behavior:

- key summary sections should expose lightweight actions such as `Search`, `Show related`, or `Use as filter` where a sensible query can be derived
- clicking a hashtag in the summary should prefill a query constrained by that hashtag rather than forcing the operator to type it manually
- clicking a node type, relation type, or project scope bucket in the summary should prefill the corresponding structured filter when that filter is supported by the canonical query logic
- recent-activity rows may expose a `Find related` or equivalent action that launches a query using the selected node identity or text context when that is supported cleanly
- these pivots should stay read-only and query-oriented; they must not imply edit capabilities

**Alternative considered:** keep the summary as a purely static dashboard and require all queries to start from a blank form. Rejected: that would miss the most useful workflow improvement for a summary-first page.

### 3. Make shared `unified_search` execution the canonical browser search behavior

The browser knowledge query surface should reuse the existing Themion search execution model instead of inventing a parallel one.

Required behavior:

- browser knowledge search should share the same core execution semantics as `unified_search`
- the query inputs should map cleanly to the canonical `unified_search` concepts:
  - `query`
  - `project_dir`
  - `source_kinds`
  - `mode`
  - `limit`
  - `hashtags`
  - `hashtag_match`
  - `node_type`
  - `relation_type`
  - `linked_node_id`
- the browser should preserve the same meaning of omitted `source_kinds` as `unified_search` when omission is used, but the knowledge page does not need to use omission as its initial preset
- result meaning should stay aligned with `unified_search`, including source-aware results and explicit score/ranking interpretation where exposed
- if a browser interaction needs a new retrieval behavior that `unified_search` does not support well, that gap should be identified explicitly instead of papered over with browser-only execution semantics

Current themion-core state:

- `themion-core` already exposes reusable lower-level pieces of `unified_search` behavior, but not yet one single browser-ready public search function
- exact/FTS source-row retrieval is already available through `DbHandle::unified_search_rows(...)`
- semantic retrieval is already available through `MemoryStore::unified_search_semantic(...)`
- shared result structs such as `UnifiedSearchMode`, `UnifiedSearchResult`, and `UnifiedSearchResponse` already live in `themion-core`
- the full canonical merge/orchestration behavior is still assembled inside the `unified_search` tool execution path in `crates/themion-core/src/tools.rs`
- `themion-web` therefore cannot currently reuse one stable core-level `unified_search` interface directly without either calling tool execution machinery or reimplementing the merge path itself

Required themion-core interface change:

- `themion-core` should expose one reusable public query entry point for canonical unified search behavior, for example a runtime-owned function or service method that accepts a typed query struct and returns `UnifiedSearchResponse`
- that public entry point should own the same orchestration that the tool currently owns:
  - argument normalization and defaults
  - omitted-`source_kinds` handling
  - FTS retrieval
  - semantic retrieval when available
  - hybrid merge behavior
  - unavailable-source-kind reporting
  - result shaping into the canonical `UnifiedSearchResponse`
- the `unified_search` tool should become a thin adapter over that same core entry point rather than remaining the only place where the canonical behavior is assembled
- `themion-web` should call that same core entry point through a web adapter rather than reimplementing the tool branch logic

Implementation-ready ownership decision:

- keep the canonical search implementation in `themion-core`, not in `themion-web` and not in `tools.rs` as tool-only logic
- introduce a typed query/request struct in `themion-core` for browser and tool callers to share
- keep model-facing tool schema parsing in `tools.rs`, but move the actual unified-search execution plan into a reusable core function or service
- do not make `themion-web` depend on model-tool JSON argument handling just to execute canonical search

Suggested interface shape:

- add a typed request struct in `themion-core`, for example `UnifiedSearchQuery`, with caller-facing fields for:
  - `query`
  - `project_dir`
  - `source_kinds` as an optional list so omitted-source-kind default behavior remains representable
  - `mode`
  - `limit`
  - `hashtags`
  - `hashtag_match`
  - `node_type`
  - `relation_type`
  - `linked_node_id`
- add one public execution entry point in `themion-core`, for example `DbHandle::unified_search(query: UnifiedSearchQuery) -> Result<UnifiedSearchResponse>` or an equivalent runtime-owned service method
- keep the request type typed and Rust-native; do not make web or tool callers pass generic JSON blobs into core search execution
- if argument normalization needs a distinct phase, it may use a second internal normalized struct, but the public entry point should still be one stable typed interface

Required contract of the public core interface:

- it must accept both explicit and omitted `source_kinds` without losing the difference between the two
- it must apply the same default limit and retrieval-mode behavior that the canonical tool currently applies
- it must return the same degraded-mode and unavailable-source-kind signals regardless of whether the caller is a tool or the web UI
- it must be usable without any dependence on model/tool registration, tool schemas, or transcript/runtime prompt assembly code
- it must remain read-only with respect to knowledge/query execution; indexing and rebuild remain separate capabilities

Web-preset rule:

- the knowledge page should default its initial source-kind selection to explicit `memory` rather than relying on omitted `source_kinds`
- this memory-first preset is a product choice for the browser knowledge page, not a change to the canonical meaning of omitted `source_kinds` in core/tool search execution
- the web UI may let the operator expand scope to `chat_message` or mixed-source search, but that broader scope is opt-in rather than the default knowledge-page submission state
- once the web surface builds a typed request, execution should still run through the same shared core unified-search path as tool callers

Recommended Rust shape:

- public caller-facing request type:

  ```rust
  pub struct UnifiedSearchQuery {
      pub query: String,
      pub project_dir: Option<String>,
      pub source_kinds: Option<Vec<UnifiedSearchSourceKind>>,
      pub mode: Option<UnifiedSearchMode>,
      pub limit: Option<u32>,
      pub hashtags: Vec<String>,
      pub hashtag_match: Option<HashtagMatch>,
      pub node_type: Option<String>,
      pub relation_type: Option<String>,
      pub linked_node_id: Option<String>,
  }
  ```

- recommended typed enum for source kinds instead of free-form strings:

  ```rust
  pub enum UnifiedSearchSourceKind {
      Memory,
      ChatMessage,
      ToolCall,
      ToolResult,
  }
  ```

- recommended execution entry point:

  ```rust
  impl DbHandle {
      pub fn unified_search(&self, query: UnifiedSearchQuery) -> anyhow::Result<UnifiedSearchResponse>;
  }
  ```

- recommended internal normalization shape:

  ```rust
  struct NormalizedUnifiedSearchQuery {
      query: String,
      project_dir: String,
      source_kinds: Vec<UnifiedSearchSourceKind>,
      source_kinds_were_omitted: bool,
      mode: UnifiedSearchMode,
      limit: u32,
      hashtags: Vec<String>,
      hashtag_match: HashtagMatch,
      node_type: Option<String>,
      relation_type: Option<String>,
      linked_node_id: Option<String>,
  }
  ```

Normalization guidance:

- the public `UnifiedSearchQuery` should preserve caller intent, including the difference between omitted and explicit values
- normalization should resolve defaults once, inside `themion-core`, before any FTS, semantic, or hybrid execution begins
- `project_dir: Option<String>` should allow callers to omit project scope and rely on the same current-project default behavior the tool already provides when a runtime context is available
- `mode: Option<UnifiedSearchMode>` and `limit: Option<u32>` should preserve omission at the public boundary and become concrete only during normalization
- `source_kinds: Option<Vec<UnifiedSearchSourceKind>>` should preserve the important difference between omitted source kinds and an explicit provided list
- normalization should reject invalid empty-string or structurally unusable values before retrieval work begins

Recommended conversion boundary:

- `tools.rs` should convert tool JSON arguments into `UnifiedSearchQuery`, then call the shared core entry point
- `themion-web` should convert form state or HTTP parameters into `UnifiedSearchQuery`, then call the same shared core entry point
- if a `TryFrom`-style conversion helper improves validation clarity, use it at the surface boundary, but keep execution defaults and canonical behavior owned by the unified-search core implementation rather than duplicated in both callers

**Alternative considered:** let `themion-web` stitch together `DbHandle::unified_search_rows(...)` plus `MemoryStore::unified_search_semantic(...)` on its own. Rejected: that would duplicate the exact merge/default/degradation logic this PRD is supposed to centralize.

### 4. Define one compact browser query form with progressive disclosure

The browser query form should reflect the typed themion-core interface closely enough that there is a simple one-to-one mapping from UI state into the shared core request shape. In particular, the UI should preserve omission versus explicit selection for `source_kinds`, `mode`, and `limit` where that distinction matters to canonical core behavior, while still using an explicit memory-first preset for the default knowledge-page search scope.

The operator should be able to compose useful knowledge queries without raw JSON or terminal syntax.

Required behavior:

- the knowledge page should expose a query form with the following always-visible controls:
  - free-text query input
  - source-kind selection
  - retrieval-mode selector
  - result-limit selector
  - submit action
- the page should expose additional structured filters in an expandable `Advanced filters` section:
  - project scope
  - hashtags
  - hashtag match mode
  - node type
  - relation type
  - linked node id
- the UI should show the effective query shape clearly enough that the operator can understand what was searched
- the page should support re-running the same query after changing one option without losing the rest of the form state
- the page should support clearing filters independently from clearing the whole query text

Required default behavior:

- the default knowledge-page source scope should be explicit `memory` so the browser starts from Project Memory rather than broad transcript search
- the UI should make broader scopes such as `chat_message` or mixed-source search available, but those remain opt-in choices
- when the operator intentionally clears source-kind selection to use canonical omission behavior, the request should preserve omitted `source_kinds` semantics instead of silently expanding them in the browser
- the default mode should be `fts` unless the canonical search product direction changes elsewhere first
- the default result limit should match the canonical default currently used by `unified_search`
- if semantic or hybrid modes are unavailable in the current build or data state, the UI should report that clearly rather than pretending those modes ran normally

### 5. Return source-aware results with clear grouping and drill-down

The browser query result should present meaningful Themion objects rather than raw rows.

Required behavior:

- result rows should stay source-aware in the same way `unified_search` results are source-aware
- each result should identify what kind of item matched, for example `memory`, `chat_message`, `tool_call`, or `tool_result` when those source kinds are in scope
- the result list should show a primary snippet or summary that explains why the item matched
- when canonical search logic provides scores or score kinds, the browser may display them in a compact operator-friendly way without pretending to provide a different ranking definition
- result rendering should favor scannability and drill-down over raw schema dumps
- if the query returns no matches, the page should show a clear no-results state that preserves the submitted query context

Implementation-ready result layout:

- each result row should include a compact header with source-kind badge, source-specific title/label, and core metadata
- each result row should include one primary snippet and may expose additional detail behind an expand action
- the result list should support a simple grouped or filtered view by source kind when mixed-source results are present
- the first implementation may keep grouping lightweight, for example tabs or inline chips with counts, rather than a complex faceted explorer

Implementation-ready source-specific presentation:

- memory results should surface title, node type, hashtags, and snippet when available
- chat-message results should surface speaker or agent context plus snippet when available
- tool-call and tool-result results should remain secondary and explicit opt-in as they already are in the canonical search model, not part of the browser default if the canonical default excludes them

### 6. Add lightweight follow-up actions from results

The browser query surface should help the operator continue inspection without leaving context.

Required behavior:

- the operator should be able to click or expand a result to see more detail about the matched object
- if a memory result maps to a specific node id or a linked-node context, the page should expose that identity clearly
- if a result includes hashtags, node type, relation type, or source-kind metadata, the UI should make it easy to launch a follow-up query from that metadata
- a memory result should support a direct follow-up action to query by `linked_node_id` when that produces a meaningful related-items workflow
- follow-up actions should stay read-only and query-oriented rather than crossing into edit workflows
- the first implementation may keep drill-down compact and local to the page rather than introducing a full multi-page knowledge explorer

**Alternative considered:** keep result rows static and require the operator to manually copy identifiers into another surface for deeper inspection. Rejected: that would make the browser query page much less useful as an operational tool.

### 7. Preserve direct-summary and canonical-query ownership boundaries

The summary and query portions may use different underlying data-access paths, but they must still remain honest about which layer owns each behavior.

The page now combines two different knowledge access patterns and must keep them conceptually clean.

Required behavior:

- the overall summary sections from PRD-102 may continue to read canonical memory tables directly if that remains the accepted design for the summary portion
- the interactive query portion should prefer shared canonical query logic from `unified_search` rather than duplicating search behavior in separate direct SQL code paths, even when the browser intentionally starts from a narrower memory-first preset
- the page must label or structure these capabilities clearly enough that operators can tell the difference between summary counts and query results
- the browser must not become the owner of indexing truth, embedding refresh rules, or search ranking policy
- if the canonical search path reports unavailable source kinds or degraded mode behavior, the web surface should preserve that information rather than hiding it
- the browser should not translate a failed canonical query into silently different query semantics just to keep the UI looking smooth

### 8. Define explicit loading, degraded-mode, and empty-state behavior

The browser query surface should be truthful when search capability is limited.

Required behavior:

- the page should show a visible loading state while a query is executing and prevent ambiguous duplicate submissions
- if the database is readable but contains no knowledge content, the page should show a valid empty query state rather than an error
- if the canonical search logic cannot serve a requested mode or source-kind combination, the UI should show the limitation explicitly
- if semantic search is unavailable because indexing or feature support is absent, the browser should explain that clearly and keep exact-search workflows usable when possible
- if the shared search path returns an execution error, the page should show an operator-facing failure state with enough detail to distinguish query failure from zero matches
- if the summary page is healthy but interactive query execution is unavailable, the UI should degrade gracefully instead of taking down the whole knowledge page
- if a later query succeeds after an earlier failure, the UI should replace the stale error state with the new result state cleanly

### 9. Keep the first browser query scope focused on retrieval, not full graph tooling

This PRD should improve the knowledge page materially without turning it into a full knowledge-management suite.

Required behavior:

- the first browser query surface should prioritize searching and inspecting existing knowledge over graph editing or advanced administration
- if graph-neighborhood or linked-node views are added in this PRD, they should be framed as drill-downs from query results rather than a separate graph editor
- arbitrary combinations of every `memory_*` tool are not required in the first browser iteration
- a future PRD may add richer graph navigation, memory-node detail pages, or authoring flows after the query foundation is proven

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-103-query-driven-knowledge-page-for-themion-web.md` | Define the product requirement for evolving the knowledge page from summary-only into a query-capable browser surface that reuses canonical `unified_search` behavior. |
| `docs/README.md` | Add the PRD-103 entry in sorted order with Proposed status and target version. |
| `crates/themion-web/src/` | Add the knowledge-page query workspace, summary-to-query pivots, web handlers/adapters, typed request mapping into the shared core query interface, empty/error/degraded states, and result presentation. |
| `crates/themion-core/src/` | Add one reusable public unified-search interface plus typed request struct(s) that own canonical query defaults, merge behavior, degraded-mode reporting, and `UnifiedSearchResponse` shaping for both tool and web callers. |
| `crates/themion-web/README.md` | Update the web-surface documentation when the query-driven knowledge page lands so the browser feature set matches reality. |
| relevant search/runtime docs | Update active docs if implementation changes the documented ownership path or search reuse guidance for `unified_search`. |

## Edge Cases

- the knowledge page summary loads but the shared query path fails to initialize → verify: the summary remains visible and the query area shows a clear degraded state.
- the operator submits a query with no explicit source kinds after intentionally clearing the memory-first preset → verify: the browser uses the same default meaning as omitted `source_kinds` in `unified_search`.
- the operator requests `semantic` or `hybrid` mode in a build or dataset that cannot support it → verify: the page reports the limitation clearly instead of silently falling back without explanation.
- the operator searches across `memory` and `chat_message` for one phrase → verify: browser results are consistent in scope and source labeling with the canonical `unified_search` behavior.
- the operator includes explicit tool-related source kinds → verify: tool-call and tool-result matches remain opt-in and clearly labeled.
- the query returns no results while the summary shows plenty of knowledge → verify: the page shows a true no-results state rather than implying the database is empty.
- the query returns matches from several source kinds → verify: result rendering stays readable and source-aware instead of collapsing different object types into one generic row shape.
- the operator reruns a query after changing only mode or limit → verify: prior form state is preserved sensibly and remaps to the shared typed request without dropping other filters or resetting the chosen source scope.
- the operator launches a query from a summary hashtag or node-type pivot → verify: the corresponding filter is prefilled correctly, remains editable, and maps cleanly into the shared typed core request.
- the shared `unified_search` path reports unavailable source kinds → verify: the web UI surfaces that degraded-mode information explicitly.
- a previous query failed and the next query succeeds → verify: the result area recovers cleanly without leaving stale failure messaging behind.

## Migration

This is an additive improvement to the existing `themion-web` knowledge page.

Expected rollout behavior:

- the current summary view from PRD-102 remains available as the default knowledge-page entry state
- the knowledge page gains interactive read-only query capability layered on top of the existing summary functionality
- no Project Memory schema migration is required solely for the browser query UI
- implementation should add a reusable themion-core unified-search interface and refactor the existing tool path plus the new web adapter to call it, while preserving canonical search behavior rather than changing it silently
- existing terminal and tool-driven query workflows remain valid after the browser page gains query support

## Testing

- open the knowledge page with existing data but do not run a query → verify: the current summary remains visible as the page's default state and the query form defaults to explicit `memory` scope.
- click a summary hashtag or node-type pivot → verify: the query workspace opens or updates with the correct prefilled filter.
- submit a browser query with the default browser source scope → verify: the request is sent as explicit `memory` scope and results stay limited to Project Memory content unless the operator broadens scope.
- submit a browser query scoped to `memory` only → verify: memory results are shown with source-aware metadata and snippets.
- intentionally clear or disable explicit source-kind selection and then submit a browser query → verify: results match the canonical omitted-`source_kinds` behavior and the request reaches core as omitted `source_kinds`, not as a silently expanded explicit list.
- submit a browser query scoped to `chat_message` only with `mode=fts` → verify: transcript-like matches align with canonical `unified_search` behavior.
- submit a browser query using advanced filters such as hashtags, node type, relation type, or linked node id → verify: the effective query and results reflect the canonical filter semantics and the same typed core request path used by the tool surface.
- submit a browser query using `semantic` and `hybrid` where supported → verify: mode selection, result labeling, and degraded-mode reporting behave truthfully.
- submit a query that returns no matches → verify: the page shows a no-results state that preserves the query context.
- trigger a shared-search error or unavailable-source-kind condition → verify: the browser surfaces the failure or degradation clearly instead of silently changing semantics.
- use a follow-up click or metadata action from a result row → verify: the page launches a read-only drill-down or follow-up query without losing context.
- run `cargo check -p themion-core` after implementation if shared search logic is refactored → verify: the core crate still builds cleanly in its default configuration.
- run `cargo check -p themion-core --all-features` after implementation if shared search logic is refactored → verify: the core crate still builds cleanly across feature combinations.
- run `cargo check -p themion-web` after implementation → verify: the web crate builds cleanly in its default configuration.
- run `cargo check --all-features -p themion-web` after implementation → verify: the web crate builds cleanly across feature combinations.

## Implementation checklist

- [x] define the knowledge-page query UX and shipped summary/result state transitions
- [x] add summary-to-query pivot actions for the most useful existing summary sections
- [x] define the public typed themion-core unified-search request shape, including exact Rust field types and omitted versus explicit `source_kinds`
- [x] define the normalization boundary between the public request type and internal normalized query handling
- [x] add a public themion-core unified-search interface that returns canonical `UnifiedSearchResponse` from that typed request
- [x] refactor the existing `unified_search` tool path to parse tool arguments, build the typed request, and call that shared core interface
- [x] implement browser query handlers/adapters that map UI state into the same typed request and call the same shared core interface
- [x] make the default web knowledge-page query preset explicit `memory` scope while preserving an operator path to broader or omitted source-kind search
- [x] implement advanced filters for canonical structured query fields
- [x] implement source-aware result rendering and no-results/degraded/error states
- [ ] add lightweight result drill-down and follow-up query actions from result metadata
- [x] preserve PRD-102 summary behavior while layering query workflows on top
- [x] update web and search documentation when the feature lands
- [x] validate touched crates in default and all-features configurations
