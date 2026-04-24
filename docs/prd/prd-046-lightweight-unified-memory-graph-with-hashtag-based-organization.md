# PRD-046: Lightweight Long-Term Memory Knowledge Base with Hashtag-Based Organization

- **Status:** Implemented
- **Version:** v0.29.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-23

## Summary

- Themion should add a lightweight long-term memory knowledge base for reusable facts, relationships, decisions, and evolving project knowledge.
- Use one graph-backed model where knowledge-base entries, entities, concepts, files, tasks, decisions, observations, and occasional narrative memory records are all first-class graph nodes.
- Replace Stele-style hierarchical scopes with hashtag-based organization and retrieval.
- Keep the system lightweight: SQLite-backed, tool-driven, and simple enough for routine agent use during coding sessions.
- The write path should make it easy to store durable knowledge as structured graph nodes and relationships, while still allowing lightweight narrative records when structure is not yet known.
- The read path should let agents retrieve by hashtags, keywords, node identity, direct graph links, and explicit project-directory context from one unified backing store.
- Memory should default to the current `project_dir`, while still supporting explicit cross-project access and a `[GLOBAL]` magic project selector for shared, non-repository-specific knowledge.
- Implemented as an additive SQLite-backed long-term memory knowledge base in `themion-core`, including KB-first tool semantics, current-project default scoping, explicit cross-`project_dir` selection, and `[GLOBAL]` virtual shared-memory selector semantics.

## Goals

- Define a lightweight long-term memory knowledge base for Themion, backed by one graph model.
- Make durable knowledge entries first-class graph nodes instead of storing narrative memory and graph entities in separate subsystems.
- Remove the need for a separate scope concept in the proposed design.
- Use hashtags as the primary organizational and retrieval label system.
- Scope memory by `project_dir` by default, with explicit cross-project access when requested and a `[GLOBAL]` virtual shared project directory for reusable knowledge that is not tied to one checkout.
- Preserve a simple enough model that the feature remains practical for coding-agent workflows rather than becoming a heavy standalone knowledge platform.
- Outline a tool surface that emphasizes knowledge-base construction and retrieval, while still supporting freeform long-term memory capture when useful.
- Identify how the proposed unified design fits Themion's current architecture, SQLite usage, and tool-driven runtime.

## Non-goals

- No commitment yet to remote multi-user synchronization semantics beyond current Themion local/runtime patterns; cross-`project_dir` support is local database selection/filtering, not network sync.
- No vector embedding search or semantic retrieval layer in this first design.
- No attempt to replicate all of Stele's current MCP surface exactly.
- No introduction of a general-purpose ontology engine, schema language, or inference system.
- No replacement of existing session history tools in the same first step.
- No migration plan from an already shipped Themion memory graph feature, because no such feature exists today.

## Background & Motivation

### Current state

Themion already has several durable storage concepts, but none of them is a lightweight shared memory graph:

- persistent SQLite-backed session history in `themion-core`
- durable board notes for asynchronous work coordination
- project-scoped history recall/search tools
- no current first-class user-facing memory/KG subsystem for reusable facts, relationships, and evolving project knowledge

The repository architecture places reusable runtime logic, tool contracts, and SQLite-backed durable behavior in `themion-core`, while user-facing local orchestration and TUI behavior live in `themion-cli`. That split fits a future memory feature well: the durable model and query/write tools belong in core, while any future UI affordances belong in CLI.

### Research note from Stele

Stele provides a useful reference because it already ships both flat memory and a knowledge graph. Its design is explicit:

- flat memory stores note-like items such as decisions, conventions, troubleshooting notes, and references
- the knowledge graph stores entities, observations, and directed relations
- both systems share a scope model, but they remain separate storage concepts and separate tool families

That separation is clean, but it also creates duplication pressure. The same real-world concept may need to exist both as a memory document and as a graph entity. Facts can be duplicated between flat note content and graph observations. Retrieval logic and user mental models are split across two stores.

This PRD starts from the user's preferred direction: long-term memory should behave as a knowledge base, not as a separate note bucket plus a graph. Durable knowledge should be graph-native from the beginning.

**Alternative considered:** copy Stele's two-system model directly. Rejected: the main motivation of this proposal is to avoid the duplication and split mental model that come from separate memory and KG stores.

### Why hashtags instead of scopes

Stele uses hierarchical scopes as a primary partitioning and retrieval concept. Themion already has a strong project-directory boundary for session history and board behavior, and the requested design explicitly prefers hashtags instead of scopes.

Hashtags provide a lighter-weight organizational model:

- easier for humans and agents to add opportunistically
- flexible enough for multiple perspectives at once
- less rigid than hierarchical scope trees
- better aligned with a "lightweight memory" feature than a formal namespace design

The cost is weaker hard partitioning semantics, so the feature design should treat hashtags as labels for retrieval and organization rather than as strict security boundaries.

**Alternative considered:** keep both scopes and hashtags. Rejected: that would preserve more complexity than the requested lightweight model needs and would reintroduce duplicate organization concepts.

## Design

### Unified graph model

The proposed feature should use one durable graph-backed data model where every stored knowledge-base item is a node.

Normative direction:

- represent durable facts, concepts, files, components, decisions, tasks, observations, and relationships as knowledge-base graph data
- allow narrative long-term memory records as nodes, but do not make note capture the primary product model
- represent traditional graph entities as nodes in the same node table
- represent relationships between any two nodes through typed edges
- allow any node to carry descriptive text while still participating in the graph

This means a fact such as "build uses temporary workaround for issue X" should be reusable knowledge in the graph. It may be represented as a decision/fact node, linked directly to the build system node, the issue node, and the affected file or component nodes.

**Alternative considered:** use one storage engine but still keep separate memory-node and entity-node APIs. Rejected: that keeps the same conceptual split even if the tables are merged underneath.

### Minimal node shape

The node model should stay intentionally small.

A proposed first-step node shape:

- `id`
- `project_dir` — normalized project context; omitted tool arguments default to the current session project, and `[GLOBAL]` denotes the virtual shared project context
- `node_type` — e.g. `concept`, `component`, `person`, `file`, `task`, `decision`, `fact`, `observation`, `troubleshooting`, `memory`
- `title` or `name`
- `content` — optional descriptive/body text
- `hashtags` — zero or more tags such as `#auth`, `#contract`, `#todo`
- `created_at_ms`
- `updated_at_ms`
- optional lightweight metadata JSON for future-proofing without over-designing the first schema

A proposed first-step edge shape:

- `id`
- `from_node_id`
- `to_node_id`
- `relation_type` — e.g. `depends_on`, `mentions`, `owned_by`, `blocks`, `documents`, `relates_to`
- `created_at_ms`
- optional lightweight metadata JSON

This is enough to support a lightweight knowledge base, named entities, durable facts, occasional narrative memory records, and graph links while staying SQLite-friendly.

**Alternative considered:** start with a richer property graph and arbitrary per-edge/per-node typed fields. Rejected: too heavy for the lightweight first version and likely to increase tool and migration complexity.

### Long-term memory is a knowledge base, not a note side channel

The key design rule is that Themion's long-term memory should be a knowledge base first. Narrative notes are allowed, but they are one node type among many rather than the center of the design.

Examples:

- a reusable fact is a `fact` or `observation` node linked to the relevant component, file, person, task, or decision
- a decision is a `decision` node with rationale text and edges to affected components or files
- a troubleshooting record is a `troubleshooting` node linked to the symptom, component, fix, and relevant files
- a person node can link to ownership, preference, or responsibility facts
- a file node can link to contracts, gotchas, decisions, and tasks

This avoids the Stele-style split where narrative material lives in one system and structural knowledge lives in another, while also avoiding a product model that feels like plain note storage.

**Alternative considered:** keep observations as a child table attached only to entity nodes. Rejected: reusable knowledge should be first-class retrievable/linkable graph data, not only attached annotation rows.

### Hashtags replace scope as the main organization primitive

Instead of hierarchical scopes, the unified graph should use hashtags as its primary lightweight organizational system.

Normative direction:

- hashtags are flat labels such as `#rust`, `#stylos`, `#provider`, `#todo`, `#breaking`
- a node may have many hashtags
- retrieval tools should support any-match and all-match filtering by hashtags
- hashtags are for categorization, retrieval, and lightweight clustering, not strict namespace enforcement
- existing Themion concepts such as current project directory remain runtime context, but they are not exposed as a memory-graph scope hierarchy

This matches the user's requested model while keeping storage simple.

**Alternative considered:** infer pseudo-scopes from hashtags like `#team/foo`. Rejected: that would drift back toward hidden scope semantics instead of embracing a simpler label system.

### Project-directory context and `[GLOBAL]` memory

The memory knowledge base should support cross-project access without reintroducing Stele-style scopes. The runtime already has a current `project_dir`; memory tools should use that as the default context so ordinary project knowledge stays attached to the checkout where it was created.

Normative direction:

- every memory node should record a `project_dir` context or equivalent normalized project key
- when the caller omits project selection, tools operate on the current session's `project_dir`
- tools may accept an explicit `project_dir` selector for cross-project search, lookup, creation, and linking when the user intentionally requests it
- the exact magic selector `[GLOBAL]` represents a virtual shared project directory for knowledge that should be available across projects, such as general user preferences, reusable troubleshooting patterns, or provider behavior notes
- `[GLOBAL]` is not a filesystem path and must not be resolved with `cd`, canonicalization, or path existence checks
- project-specific searches should not silently include `[GLOBAL]` unless the tool contract explicitly says so; preferred first behavior is an explicit selector so retrieval boundaries stay predictable
- links should normally stay within one project context, but links involving `[GLOBAL]` nodes may be allowed for reusable concepts that intentionally bridge global and project-specific knowledge

This is a project-context selector, not a hierarchical memory scope system. Hashtags remain the primary organization primitive inside each selected context.

**Alternative considered:** store all memory in one unqualified graph and rely only on hashtags like `#project_themion`. Rejected: cross-project recall needs a reliable default boundary, and relying only on tags would make accidental leakage between projects too easy.

**Alternative considered:** treat `[GLOBAL]` as a real directory. Rejected: it is a virtual shared project context and should not depend on any local filesystem layout.

### Tool surface should describe one memory graph, not two systems

The user specifically wants the feature and its tools to behave as a long-term memory knowledge base rather than as separate note storage and knowledge graph behavior. The tool surface should therefore present one descriptive model rather than separate "memory tools" and "KG tools" with overlapping concepts.

A proposed initial tool family. Tools should default to the current session `project_dir`; where project selection is exposed, the exact selector `[GLOBAL]` means the virtual shared project directory.

- `memory_create_node`
  - create any knowledge-base node, including concepts, files, decisions, facts, observations, and occasional narrative memory nodes; default to the current `project_dir`, with explicit `project_dir`/`[GLOBAL]` support for intentional cross-project writes
- `memory_update_node`
  - edit title, content, type, hashtags, metadata
- `memory_link_nodes`
  - create typed edges between nodes
- `memory_unlink_nodes`
  - remove specific edges
- `memory_get_node`
  - retrieve one node with its content, hashtags, immediate links, and project context
- `memory_search`
  - search by keywords, hashtags, node type, optional relation filters, and optional `project_dir` selector; omitted selector searches the current project by default, while `[GLOBAL]` searches the shared virtual project context
- `memory_open_graph`
  - open a neighborhood around one or more nodes
- `memory_delete_node`
  - delete a node and its directly owned link rows
- `memory_list_hashtags`
  - inspect commonly used hashtags

The tool names above are placeholders, but the important design property is unified semantics: one set of tools for one long-term memory knowledge-base model.

**Alternative considered:** use separate `memory_*` and `graph_*` tool families for discoverability. Rejected: the proposal should communicate that the graph is the memory system, not a separate subsystem.

### Two authoring modes: knowledge-first and lightweight capture

Although storage is unified, authoring should remain easy.

Proposed interaction modes:

1. Knowledge-first
   - create explicit concept, component, file, task, decision, fact, or observation nodes
   - link nodes with typed relationships when the user or agent understands the structure
2. Lightweight capture
   - create a narrative long-term memory node with title, content, hashtags, and optional type when the structure is not yet known
   - refine it later by changing type or adding graph links

This keeps the feature lightweight in practice while preserving the product expectation that the destination is a knowledge base. Users and agents can start with quick capture, but the preferred steady state is reusable graph-shaped knowledge.

**Alternative considered:** require explicit graph structure on every write. Rejected: too much friction for quick capture and contrary to the lightweight goal.

### Retrieval should combine text, hashtags, and local graph expansion

The system should support three retrieval styles from one store:

- keyword/text search over titles and content
- hashtag filtering
- graph-neighborhood expansion from one or more anchor nodes
- project-context selection, defaulting to current `project_dir` and supporting `[GLOBAL]` for shared knowledge

A likely first-step SQLite implementation:

- node table for durable records including project context
- node-hashtag join table
- edge table
- FTS5 virtual table over node title/content

This keeps the retrieval model practical and aligns with existing Themion SQLite patterns.

**Alternative considered:** graph traversal only, without FTS. Rejected: users and agents will still need ordinary text search for knowledge-base entries and narrative long-term memory records.

### Relationship to existing Themion history

This feature should complement, not replace, current session history tools.

Normative direction:

- session history remains the transcript/log of what happened in a conversation
- the unified memory graph is Themion's long-term memory knowledge base for distilled durable knowledge that should outlive one session and be reused later
- history tools stay useful for reconstructing prior turns
- memory graph tools become the place to store reusable project knowledge, decisions, links, and graph structure intentionally
- cross-project retrieval is explicit: omitted project selection means current project, while `[GLOBAL]` is reserved for intentionally shared non-project-specific knowledge

This distinction helps avoid turning the memory graph into an uncurated copy of every conversation message.

**Alternative considered:** auto-ingest all assistant and user messages into the graph. Rejected: too noisy for a lightweight usable memory feature and likely to destroy signal quality.

### Placement in Themion architecture

The feature fits the current repo structure naturally.

Proposed placement:

- `themion-core`
  - schema, SQLite queries, data model, tool definitions, graph retrieval logic
- `themion-cli`
  - optional future commands, TUI affordances, or onboarding/help text
- `docs/`
  - architecture and tool documentation

This follows the repository rule that reusable runtime behavior and durable storage live in core, while UI behavior stays in CLI.

**Alternative considered:** implement the first version entirely in CLI because it is user-facing. Rejected: the durable model and tool surface are reusable runtime features and belong in core.

## Changes by Component

| File / Area | Proposed change |
| ----------- | --------------- |
| `crates/themion-core` | Add a new SQLite-backed unified memory-graph subsystem with nodes, project context, hashtags, edges, and FTS-backed retrieval. |
| `crates/themion-core` | Add a coherent tool family for creating nodes, linking nodes, searching memory, opening graph neighborhoods, listing hashtags, and selecting current-project, explicit-project, or `[GLOBAL]` memory contexts. |
| `crates/themion-core` | Define result payloads that make knowledge-base content, entity metadata, facts, observations, and graph links easy for the model to understand and reuse. |
| `crates/themion-cli` | Optionally add future UX affordances or help text for the memory graph, but keep first implementation dependency-light. |
| `docs/architecture.md` | Document the role of the unified memory graph relative to history, board notes, and the rest of the runtime. |
| `docs/engine-runtime.md` | Document the new tool contracts and storage model once implemented. |
| `docs/README.md` | Add this PRD entry. |

## Edge Cases

- a user stores a quick captured knowledge item with hashtags but no links → verify: it remains a valid node retrievable by text and hashtags without requiring graph structure immediately.
- two nodes have near-duplicate titles but different hashtags and links → verify: search results expose enough metadata to distinguish them.
- a node has no hashtags → verify: it can still be found by direct id lookup or text search.
- a hashtag becomes widely overused such as `#todo` → verify: search still supports combining hashtags with keywords or node type filters to narrow results.
- a knowledge-base node links to many entities → verify: neighborhood reads remain bounded and do not dump the whole graph accidentally.
- deleting one node that has many links → verify: linked edge rows are removed consistently without leaving dangling references.
- two agents create similar memory nodes independently → verify: the first version tolerates duplicates and does not require aggressive deduplication logic.
- hashtags contain punctuation or mixed case → verify: normalization rules are defined clearly enough to avoid duplicate tag variants such as `#Rust` vs `#rust` when implemented.
- a tool call omits project selection → verify: it uses the current session `project_dir` by default.
- a tool call uses `[GLOBAL]` as the project selector → verify: the node/search operates on the virtual shared project context without trying to resolve `[GLOBAL]` as a filesystem path.
- a project-specific search is run while global knowledge exists → verify: `[GLOBAL]` rows are not silently mixed in unless the chosen tool contract explicitly requests that behavior.
- a cross-project search names another `project_dir` → verify: results are filtered to that explicit project context and do not require changing the process working directory.
- a node is mostly structural with almost no content → verify: the model still supports entity-style nodes cleanly.
- a node is mostly narrative text with one or two links → verify: the model still supports lightweight long-term memory capture cleanly without making note storage the dominant model.

## Migration

Because this feature does not yet exist in Themion, initial rollout is additive.

Expected rollout shape:

- add the unified memory-graph schema alongside current history and board tables
- include project-context storage for memory nodes, defaulting to the current session `project_dir` for new writes
- reserve `[GLOBAL]` as a virtual shared project context rather than a real filesystem path
- expose the new tools without changing existing history or board contracts
- keep current transcript history and board workflows intact
- avoid introducing scope migration because the proposed feature does not use scopes

If a later implementation imports data from Stele or another system, that should be handled by a separate migration/import PRD rather than folded into the initial Themion feature.

## Testing

- create a knowledge-base node with hashtags and descriptive text → verify: it is retrievable by keyword and hashtag search.
- create entity-style nodes and link them with a typed relation → verify: the relation is returned by direct node lookup and neighborhood open operations.
- create an observation/fact node and link it to an entity node → verify: one unified graph can express both durable knowledge content and structural links without needing a second store.
- search with one hashtag → verify: nodes with that hashtag are returned regardless of whether they are narrative, factual, or entity-like.
- search with multiple hashtags in all-match mode → verify: only nodes carrying every requested hashtag are returned.
- create a node without specifying project selection → verify: it is stored under the current session `project_dir`.
- create/search with `project_dir` set to `[GLOBAL]` → verify: the virtual shared context is used and no filesystem path resolution is attempted.
- search an explicit non-current `project_dir` → verify: cross-project results are returned only for that selected project context.
- open the graph around one node with many neighbors → verify: the response is bounded and returns a predictable local neighborhood rather than an unbounded dump.
- delete a linked node → verify: its edges are cleaned up and later reads do not return dangling links.
- run `cargo check -p themion-core` after implementation → verify: the unified memory-graph feature compiles cleanly in core.
- run `cargo check -p themion-cli` after implementation → verify: CLI wiring and docs-related code paths still compile cleanly.
- exercise the tools in a real session after implementation → verify: the model can build and retrieve long-term memory knowledge-base entries and graph relations through one coherent tool surface.

## Implementation notes

Implemented in this repository as an additive core feature:

- `crates/themion-core/src/memory.rs` defines the SQLite schema, persistence/query logic, node/edge result shapes, project-context storage, `[GLOBAL]` selector support, hashtag normalization, and bounded graph expansion.
- `crates/themion-core/src/db.rs` initializes the memory graph tables and FTS table alongside existing history and board tables.
- `crates/themion-core/src/tools.rs` exposes the unified `memory_*` tool family with KB-first descriptions, current-project defaults, explicit `project_dir` selection, `[GLOBAL]` shared-memory selection, and an `observation` default for lightweight capture.
- `crates/themion-core/tests/memory_tools.rs` covers knowledge-base node creation, hashtag/keyword retrieval, entity-style linking, graph neighborhood reads, edge cleanup on delete, all-match hashtag filtering, current-project defaults, `[GLOBAL]` partitioning, and project-scoped hashtag listing.
- `docs/architecture.md` and `docs/engine-runtime.md` document the shipped storage model and tool contracts with KB-first wording.

## Implementation checklist

- [x] design the SQLite schema for unified nodes, hashtags, edges, and FTS support
- [x] add project-context storage/query semantics with current `project_dir` as the default
- [x] add `[GLOBAL]` as a magic virtual shared project selector that is not treated as a filesystem path
- [x] update memory tool schemas/handlers for explicit cross-`project_dir` selection where appropriate
- [x] correct tool descriptions and create-node semantics so the tool family is KB-first rather than note-like-memory-first
- [x] decide bounded response shapes for direct lookup, search, and neighborhood expansion
- [x] define hashtag normalization and matching rules
- [x] update tests/examples from note-like memory examples to KB-style fact/observation/decision/concept examples
- [x] run final validation after KB-first and project-context corrections land
- [x] add core persistence and query logic in `themion-core`
- [x] add initial tool schemas and runtime handlers in `themion-core`
- [x] update `docs/architecture.md` and `docs/engine-runtime.md` so shipped behavior is documented as a long-term memory knowledge base
- [x] update `docs/README.md` with Implemented status only after KB-first semantics, docs, and validation are complete
