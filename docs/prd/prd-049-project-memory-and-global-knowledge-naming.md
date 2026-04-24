# PRD-049: Project Memory and Global Knowledge Naming for Durable Knowledge Tools

- **Status:** Implemented
- **Version:** v0.30.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-24

## Summary

- Themion's durable knowledge feature is currently exposed mostly as `memory`, with docs describing a long-term memory knowledge base.
- `memory` is a valid agent-system umbrella term, but by itself it can be ambiguous to LLMs: it may sound like chat transcript recall, user preference storage, or generic autobiographical memory.
- Rename the user-facing concept to **Project Memory** and consistently describe it as a durable long-term knowledge base.
- Promote the `[GLOBAL]` magic project selector in user-facing language as **Global Knowledge**, while keeping the exact `[GLOBAL]` selector for tool/API compatibility.
- Global Knowledge is not a separate system; it is the explicit cross-project context inside Project Memory, selected with `project_dir="[GLOBAL]"`.
- Keep the storage model from PRD-046: project-scoped graph nodes, typed edges, hashtags, and an explicit virtual global context.
- Prefer prompt/tool-description clarity first; only rename tool namespaces if the compatibility cost is acceptable.

## Goals

- Make the durable knowledge system easier for LLMs and operators to recognize as intentional reusable project knowledge, not ordinary chat history.
- Establish **Project Memory** as the primary user-facing/product name for project-scoped durable knowledge.
- Establish **Global Knowledge** as the user-facing name for knowledge stored in the virtual `[GLOBAL]` context.
- Preserve the exact `[GLOBAL]` magic selector as the machine-facing project context token.
- Clarify that Global Knowledge is an explicitly selected context within Project Memory, not a separate database or competing tool family.
- Clarify that Themion's Project Memory is mostly **semantic memory**: facts, decisions, troubleshooting records, components, files, observations, and durable relationships.
- Keep **knowledge base** and **knowledge graph** as descriptive terms for the content model and linked representation, not as competing top-level product names.
- Improve tool and prompt descriptions so models know when to create or search Project Memory versus session history or board notes.
- Define conservative promotion rules so reusable facts can move to Global Knowledge without polluting it with project-local details.
- Define a compatibility-safe migration path for any optional future tool namespace rename.

## Non-goals

- No change to the PRD-046 storage schema solely for naming.
- No removal of project-directory scoping.
- No removal or reinterpretation of the exact `[GLOBAL]` selector.
- No automatic inclusion of Global Knowledge in project-scoped searches unless a later PRD explicitly changes retrieval behavior.
- No replacement of history recall/search tools; transcript history remains separate from Project Memory.
- No replacement of board notes; task coordination remains separate from durable knowledge capture.
- No requirement to rename SQLite table names such as `memory_nodes` in the first implementation step.
- No broad ontology, embedding, or inference redesign.

## Background & Motivation

### Current state

PRD-046 introduced a lightweight long-term memory knowledge base backed by SQLite. It stores typed graph nodes, typed directed edges, hashtags, and a `project_dir` context. The implemented tool family is named `memory_*`, and docs already try to clarify that these tools are for durable reusable knowledge, not transcript logging or task tracking.

The model is intentionally knowledge-base-like:

- `fact`, `decision`, `troubleshooting`, `component`, `file`, `concept`, `person`, and `observation` node types are preferred
- narrative `memory` records are allowed only when no more specific type fits
- hashtags organize retrieval
- graph edges link related durable knowledge
- omitted `project_dir` defaults to the current project
- exact `[GLOBAL]` selects a virtual shared context that is not a filesystem path

That behavior is sound, but the primary label `memory` can be underspecified.

### Naming research and LLM recognition risk

Current LLM/agent terminology generally uses `memory` as the umbrella term for persistent cross-session recall. Frameworks and research commonly distinguish:

- **long-term memory** — persistence beyond one conversation/session
- **semantic memory** — stable facts, rules, decisions, preferences, and reusable knowledge
- **episodic memory** — time-bound events, incidents, task histories, and past interactions
- **knowledge base** — curated factual repository
- **knowledge graph** — linked entity/relationship representation

Themion's feature mostly matches long-term semantic memory implemented as a lightweight knowledge-base graph. Therefore `memory` is not wrong, but using only `memory` risks the LLM treating it as a generic recall store rather than a durable project knowledge system.

The requested naming direction is to reduce that ambiguity. A better user-facing term should make the model ask: "Is this durable reusable project knowledge?" rather than "Is this something from memory?"

**Alternative considered:** rename the feature entirely to `knowledge_base`. Rejected: that underplays the agent-native write/recall behavior and creates a colder static-document mental model. `Project Memory` preserves agent familiarity while making the durable project scope explicit.

### Why `[GLOBAL]` should become Global Knowledge in user-facing language

The current exact selector `[GLOBAL]` is a useful machine token because it is explicit, short, and hard to confuse with a real path. However, it is not a complete user-facing name. Operators and models need a conceptual label for what belongs there.

**Global Knowledge** is the clearest name for the virtual shared context:

- it avoids implying a real filesystem directory
- it distinguishes global reusable knowledge from current-project memory
- it works naturally in prompt guidance: "Use Global Knowledge for cross-project facts or reusable preferences"
- it preserves `[GLOBAL]` as the exact selector the tools already understand

Global Knowledge should still be presented as part of the same Project Memory system. The distinction is contextual, not architectural: ordinary omitted `project_dir` means current-project Project Memory, while exact `project_dir="[GLOBAL]"` means Global Knowledge.

**Alternative considered:** call `[GLOBAL]` "Global Memory". Rejected: it is acceptable, but less precise than Global Knowledge for the intended contents: reusable facts, conventions, preferences, and troubleshooting patterns that are not tied to one checkout.

### Risk of over-promoting project details

A global context is useful only if it stays broadly applicable. If agents store ordinary project details in Global Knowledge, later work in unrelated repositories may retrieve irrelevant or misleading facts. The naming itself should therefore make Global Knowledge feel like a higher bar than project-local recall.

The safest default is conservative:

- when information is specific to one repository, component, file, branch, bug, PRD, local command, or local decision, keep it in current-project Project Memory
- when information is user-level, provider-level, tool/runtime-level, language/framework-general, or clearly reusable across multiple projects, consider Global Knowledge
- when unsure, prefer Project Memory and promote later only after cross-project usefulness is clear

**Alternative considered:** allow agents to freely globalize any durable knowledge and rely on hashtags for filtering. Rejected: hashtags help retrieval but do not prevent cross-project leakage of irrelevant details.

## Design

### Rename the user-facing concept to Project Memory

The primary user-facing name should be **Project Memory**.

Normative direction:

- docs should introduce the feature as "Project Memory, Themion's long-term durable knowledge base"
- model-facing guidance should say Project Memory is for durable reusable project knowledge that should outlive the current session
- keep using "long-term memory" as the technical agent-memory category
- use "knowledge base" to describe the curated durable content
- use "knowledge graph" or "graph-backed" to describe the linked representation when relationships matter
- avoid presenting plain `memory` as the full user-facing name without additional context

This keeps alignment with agent-memory conventions while reducing ambiguity for LLM tool selection.

**Alternative considered:** keep all terminology exactly as PRD-046. Rejected: PRD-046 is behaviorally correct, but this naming clarification should improve model recognition and operator understanding without changing the storage model.

### Promote `[GLOBAL]` as Global Knowledge while preserving the magic selector

The exact string `[GLOBAL]` should remain the machine-facing selector for the virtual shared memory context. The user-facing name for that context should be **Global Knowledge**.

Normative direction:

- describe `[GLOBAL]` as "Global Knowledge" in docs and prompt/tool descriptions
- keep the exact selector `[GLOBAL]` in tool inputs and examples
- continue to state that `[GLOBAL]` is virtual and is not a filesystem path
- present Global Knowledge as an explicitly selected context inside Project Memory, not as a separate database, storage engine, or tool family
- use Global Knowledge only for reusable, cross-project information such as general user preferences, provider behavior notes, coding conventions, and troubleshooting patterns
- keep project-specific knowledge in the current project context by default
- when unsure, prefer current-project Project Memory; promote to Global Knowledge only when cross-project usefulness is clear
- do not silently merge Global Knowledge into project-scoped search results unless a future PRD explicitly chooses that behavior

This separates the human/model concept from the exact API token.

**Alternative considered:** replace `[GLOBAL]` with a string such as `global` or `global_knowledge`. Rejected: `[GLOBAL]` is already implemented and its bracketed shape makes the magic-token nature explicit.

### Promotion rules for Global Knowledge

The implementation should add prompt and documentation guidance for deciding whether durable information belongs in current-project Project Memory or Global Knowledge.

Normative direction:

- store in Project Memory by default when information is tied to the current repository, project directory, file path, component, branch, PRD, local incident, or local decision
- store in Global Knowledge only when information is reusable across multiple projects or independent of any single checkout
- valid Global Knowledge candidates include user preferences, durable coding conventions, provider quirks, tool/runtime behavior, recurring troubleshooting patterns, and cross-project architecture lessons
- avoid Global Knowledge for one-off bugs, local task state, branch-specific decisions, unverified speculation, or details whose usefulness depends on a specific file path in the current repo
- if a fact begins as project-local but later proves reusable, create or update a Global Knowledge node intentionally rather than silently changing default storage behavior

Suggested model-facing wording:

```text
Project Memory stores durable knowledge for the current project by default. Use project_dir="[GLOBAL]" only for Global Knowledge: reusable cross-project facts, preferences, conventions, provider/tool behavior, or troubleshooting patterns. When unsure, keep knowledge project-local and promote later only when cross-project usefulness is clear.
```

**Alternative considered:** make `[GLOBAL]` the default for all user preferences and lessons learned. Rejected: even preferences and lessons can be project-specific, so global writes should stay intentional.

### Examples for destination choice

Docs and model-facing guidance should include a compact destination table so agents can generalize from examples.

| Information | Destination |
| --- | --- |
| This repo uses `scripts/bump_version.py` for version bumps. | Project Memory |
| `crates/themion-cli/src/tui.rs` owns TUI statusline rendering. | Project Memory |
| PRD-049 chose Project Memory as the user-facing name for this repository's durable knowledge feature. | Project Memory |
| The user prefers concise final summaries by default. | Global Knowledge, if the preference is intended across projects |
| OpenAI Responses backend behavior that applies to all projects using that provider. | Global Knowledge |
| A Rust warning source should be fixed in touched scope when practical. | Global Knowledge if this is the user's general convention; otherwise project instructions |
| A one-off bug investigation in the current branch. | Project Memory or board note, depending whether it is durable knowledge or active work |
| An unfinished delegated task. | Board note, not Project Memory or Global Knowledge |
| Exact wording from an old chat turn. | History search/recall, not Project Memory |

### Search behavior stays explicit

This naming change should not blur retrieval boundaries.

Normative direction:

- omitted `project_dir` on memory search continues to mean the current project context only
- exact `project_dir="[GLOBAL]"` searches Global Knowledge only
- project searches do not silently include Global Knowledge
- if a later workflow wants current-project plus Global Knowledge retrieval, it should add an explicit mode or separate tool behavior rather than changing omitted-argument semantics silently

This prevents Global Knowledge from becoming invisible ambient prompt context and keeps retrieval predictable.

**Alternative considered:** include Global Knowledge in every project-scoped memory search by default. Rejected: automatic inclusion would make global entries harder to govern and could surprise users with unrelated cross-project facts.

### Keep tool namespace compatibility, but strengthen descriptions

The first implementation should prefer compatibility-safe wording changes over a breaking tool rename.

Normative direction:

- keep existing `memory_*` tool names unless the implementation deliberately ships a compatibility window with aliases
- rewrite tool descriptions to start from Project Memory intent, for example: "Create a durable Project Memory knowledge-base node..."
- explicitly contrast Project Memory with transcript history and board notes in the tool guidance
- mention Global Knowledge in `project_dir` parameter descriptions: exact `[GLOBAL]` means the virtual Global Knowledge context
- prefer specific node types over `memory`; describe `memory` node type as a fallback narrative record

This is likely enough to improve LLM recognition without breaking existing calls.

**Alternative considered:** immediately rename all tools to `project_memory_*`. Rejected for the first step because tool names are part of the model-visible and code-visible API. A rename is possible, but should be staged with aliases and documentation rather than done as a silent break.

### Optional future alias path for `project_memory_*`

If tool-name clarity remains a problem, Themion may add `project_memory_*` aliases that call the same underlying implementations as `memory_*`.

Normative direction for a future alias step:

- add aliases such as `project_memory_create_node`, `project_memory_search`, and `project_memory_link_nodes`
- keep `memory_*` as backward-compatible aliases for at least one release
- prefer `project_memory_*` in new docs and prompt examples if aliases are introduced
- avoid duplicating storage logic; aliases should route to the same tool handlers
- clearly document whether both namespaces are permanent or whether `memory_*` is deprecated

This keeps the PRD compatible with a gradual migration while not requiring an immediate breaking rename.

**Alternative considered:** keep only `memory_*` forever. Rejected as a firm commitment: the current user concern is specifically about LLM recognition, so aliasing or renaming should remain available if prompt-description changes are not enough.

### Clarify semantic versus episodic content

Themion should describe Project Memory as mostly semantic memory.

Normative direction:

- semantic Project Memory includes facts, decisions, troubleshooting records, component notes, file contracts, conventions, and relationships
- episodic records may exist for incidents, task histories, or session outcomes, but should not become routine transcript logging
- ordinary conversation history stays in session history tools
- ordinary delegated work and follow-up coordination stays in board notes
- narrative `node_type=memory` remains a fallback, not the default for durable knowledge

This taxonomy helps the LLM decide whether to write a durable node, search history, or create/update a board note.

**Alternative considered:** avoid academic memory terminology entirely. Rejected: semantic/episodic distinctions are common in agent-memory research and are useful as internal guidance, as long as docs remain practical.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Update `memory_*` tool descriptions and `project_dir` parameter text to use Project Memory and Global Knowledge terminology. |
| `crates/themion-core/src/agent.rs` or prompt-guidance source | Update built-in model guidance so Project Memory means durable long-term knowledge, while `[GLOBAL]` means Global Knowledge. |
| `crates/themion-core/src/memory.rs` | Keep existing storage behavior; rename internal comments or user-facing errors only if they are model/operator visible. |
| `docs/architecture.md` | Rename user-facing description from generic long-term memory knowledge base to Project Memory where appropriate, while keeping technical storage details. |
| `docs/engine-runtime.md` | Update the long-term memory tools section to introduce Project Memory and Global Knowledge. |
| `docs/prd/prd-046-lightweight-unified-memory-graph-with-hashtag-based-organization.md` | Do not rewrite historical design except for implementation-status notes if needed; treat this PRD as the naming successor. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Edge Cases

- a model wants to store a project-specific file contract → verify: guidance points it to Project Memory in the current project context, not Global Knowledge.
- a model wants to store a reusable user preference or provider behavior note → verify: guidance points it to Global Knowledge using exact `project_dir="[GLOBAL]"`.
- a model is unsure whether a lesson is reusable beyond this repository → verify: guidance says to keep it in Project Memory until cross-project usefulness is clear.
- a model wants to recall old chat wording from the current session → verify: guidance points it to history tools, not Project Memory.
- a model wants to coordinate unfinished work → verify: guidance points it to board notes, not Project Memory.
- a model wants to store an incident or task outcome → verify: it can use a specific node type such as `task`, `troubleshooting`, or `observation`, and only uses `memory` when narrative capture is genuinely the best fit.
- project search omits `project_dir` → verify: behavior still searches the current project only and does not silently include Global Knowledge.
- search explicitly sets `project_dir="[GLOBAL]"` → verify: docs describe this as searching Global Knowledge only.
- a future workflow wants current-project plus Global Knowledge search → verify: it uses an explicit mode or separate behavior rather than changing omitted `project_dir` semantics silently.
- future `project_memory_*` aliases are introduced → verify: both aliases and original tools share implementation and do not create duplicate storage paths.

## Migration

This PRD should be implemented as a naming and prompt-description migration first.

Expected rollout shape:

- no database migration
- no schema rename required
- no change to stored `project_dir` values
- no change to the exact `[GLOBAL]` selector
- update docs and model-facing tool descriptions to use Project Memory and Global Knowledge
- add conservative promotion guidance: when unsure, keep durable knowledge project-local and promote to Global Knowledge only when cross-project usefulness is clear
- keep `memory_*` tools initially for compatibility
- optionally add `project_memory_*` aliases in a later or expanded implementation if empirical use shows the namespace remains confusing

Because the proposal changes model-facing terminology and user-facing docs, it is release-worthy as a minor version even if storage behavior remains unchanged.

## Testing

- inspect generated tool definitions → verify: `memory_*` descriptions describe Project Memory as durable long-term knowledge, not generic memory or transcript history.
- inspect the `project_dir` parameter descriptions → verify: exact `[GLOBAL]` is documented as Global Knowledge and as a virtual non-filesystem context.
- ask the model to store a durable project decision → verify: prompt/tool guidance makes Project Memory the obvious destination.
- ask the model to store a cross-project reusable preference → verify: prompt/tool guidance makes Global Knowledge with `project_dir="[GLOBAL]"` the obvious destination.
- ask the model to store an ambiguous lesson learned → verify: guidance prefers Project Memory until cross-project usefulness is clear.
- ask the model to search current-project memory without `project_dir` → verify: Global Knowledge is not silently included.
- ask the model to search Global Knowledge → verify: guidance requires explicit `project_dir="[GLOBAL]"`.
- ask the model to search current conversation history → verify: guidance still points to history tools rather than Project Memory.
- run `cargo check -p themion-core` after implementation → verify: default core build compiles cleanly.
- run `cargo check -p themion-cli` after implementation → verify: CLI integration still compiles cleanly.

## Implementation notes

Implemented in v0.30.0. The first implementation keeps the `memory_*` tool namespace for compatibility and defers `project_memory_*` aliases. It updates model-facing guidance, tool descriptions, and docs so Project Memory is the user-facing feature name and exact `project_dir="[GLOBAL]"` is Global Knowledge. Storage behavior, the exact `[GLOBAL]` selector, and current-project-only omitted `project_dir` search semantics remain unchanged.

## Implementation checklist

- [x] update model-facing memory guidance to use Project Memory and Global Knowledge terminology
- [x] update `memory_*` tool descriptions and `project_dir` parameter descriptions
- [x] add Global Knowledge promotion guidance, including "when unsure, keep it project-local"
- [x] keep exact `[GLOBAL]` selector behavior unchanged
- [x] keep omitted `project_dir` search behavior current-project-only
- [x] decide whether to add `project_memory_*` aliases now or defer them
- [x] update `docs/architecture.md` and `docs/engine-runtime.md`
- [x] update `docs/README.md` with this PRD entry
- [x] decide and apply the repository version bump if implementing this PRD
- [x] check `Cargo.lock` after any version change
- [x] run `cargo check -p themion-core`
- [x] run `cargo check -p themion-cli`
