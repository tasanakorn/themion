# PRD-093: Remove `history_search` and Make `unified_search` the Canonical Search Tool

- **Status:** Implemented
- **Version:** v0.59.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-03

## Implementation status

Landed in `v0.59.1`.

## Summary

- Themion currently exposes both `history_search` and `unified_search`, which creates overlapping retrieval surfaces and weakens the product expectation that one generalized search tool should be the canonical path.
- Remove `history_search` from the tool surface, including its legacy alias `search_history`, and make `unified_search` the single canonical search tool for project-scoped retrieval.
- Preserve `history_recall` as the chronological retrieval tool for ordered transcript playback or session reconstruction rather than text search.
- Require `unified_search` to cover the practical project-scoped history lookup cases that `history_search` currently serves, using explicit `source_kinds` and `mode` rather than a second dedicated search tool.
- Update tool descriptions, prompt guidance, docs, tests, and any command/help surfaces so callers stop seeing or depending on `history_search`.

## Goals

- Remove `history_search` and `search_history` from the model-facing tool surface.
- Make `unified_search` the canonical project-scoped search entry point.
- Preserve exact transcript lookup capability through `unified_search mode=fts` for indexed `chat_message` content.
- Keep chronological transcript retrieval available through `history_recall`.
- Reduce tool-choice ambiguity in prompt/tool guidance by giving search one canonical tool.
- Align the tool surface with the generalized multi-source retrieval direction established by PRD-091.

## Non-goals

- No removal of `history_recall` in this PRD.
- No requirement to make `unified_search` a chronological replay tool; ordered recall remains a separate capability.
- No requirement to preserve the legacy `history_search` response shape if `unified_search` already provides the needed product behavior with a better shared schema.
- No cross-project semantic broadening beyond the explicit project-scoping rules already defined for `unified_search`.
- No requirement in this PRD to remove source-of-truth history storage; only the model-facing search tool surface changes.
- No requirement to keep `history_search` or `search_history` as deprecated aliases once the replacement path is ready.

## Background & Motivation

### Current state

Themion now has two different search-style tools that can touch transcript-like content.

Today:

- `history_search` performs text search across stored conversation history
- the tool implementation also accepts the legacy alias `search_history`
- `history_recall` retrieves earlier messages chronologically
- `unified_search` is the newer generalized retrieval surface spanning multiple source kinds
- current tool descriptions and defaults still allow the older history-search surface to remain visible
- callers can reasonably become uncertain about when to use `history_search` versus `unified_search`

That ambiguity works against the product goal of one clear generalized search entry point.

### Why this matters now

When multiple search tools remain visible, the model and the human lose a clean mental model:

- transcript lookup may use an older narrow tool even when generalized retrieval is preferred
- prompt guidance must explain tool choice instead of pointing to one canonical search path
- maintenance and documentation have to describe overlapping capabilities
- future search improvements become harder to centralize because one older tool remains on the surface

If the product direction is that `unified_search` should be the general retrieval tool, the exposed tool surface should reflect that directly.

**Alternative considered:** keep `history_search` and merely reword its description to discourage use. Rejected: it preserves ambiguity and leaves the old tool callable even though the product intent is to consolidate on one canonical search path.

## Design

### 1. Remove `history_search` and `search_history` from the exposed tool catalog

Themion should stop exposing the legacy history-search tool family as callable tools.

Required behavior:

- `history_search` is removed from the tool list presented to the model
- the legacy alias `search_history` is also removed from callable tool handling rather than silently preserved
- prompt/tool guidance should no longer mention `history_search` as an available retrieval path
- user-facing documentation should no longer describe `history_search` as part of the active tool surface
- if there are tool-registration tests, snapshots, or inspection outputs, they should reflect the removal

This is a product-surface removal, not merely a guidance preference.

### 2. Make `unified_search` the single canonical search tool

General text retrieval should go through `unified_search`.

Required behavior:

- search guidance should direct callers to `unified_search` for project-scoped retrieval
- exact search over indexed transcript-like content should use `unified_search mode=fts`
- broader retrieval across memory and chat should use `unified_search` defaults and explicit `source_kinds` when needed
- tool-call and tool-result search should remain explicit opt-in as defined by PRD-091 rather than part of the default search surface
- no other active model-facing search tool should be documented as the preferred alternative for general project retrieval

Required default semantics carried forward:

- omitted `source_kinds` means the default human-oriented source kinds: `memory` and `chat_message`
- callers may explicitly request `tool_call` and `tool_result` when they need tool-record search

Canonical replacement guidance:

- former `history_search(query=..., session_id omitted)` usage should usually become `unified_search(query=..., project_dir=<current>, source_kinds=["chat_message"], mode="fts")`
- former `history_search(query=..., session_id="*")` project-wide usage should also become `unified_search(..., source_kinds=["chat_message"], mode="fts")`
- broader project recall that previously mixed memory-like and transcript-like intent should become omitted `source_kinds` or `source_kinds=["memory","chat_message"]`

**Alternative considered:** replace `history_search` with a second transcript-only exact-search tool. Rejected: it recreates the same ambiguity under a different name instead of consolidating search into `unified_search`.

### 3. Preserve `history_recall` for chronological retrieval only

Removing `history_search` should not remove the separate need for ordered message recall.

Required behavior:

- `history_recall` remains available
- `history_recall` is described as chronological message retrieval, not general search
- prompt guidance should treat `history_recall` as the right tool when the caller needs earlier messages in order, recent transcript windows, or session reconstruction
- callers should not be told to simulate chronological recall through `unified_search`
- removal of `history_search` must not change the current `history_recall` parameter contract unless a separate PRD later requires that change

This keeps the distinction clear:

- `unified_search` = search/retrieval
- `history_recall` = ordered recall

### 4. Require `unified_search` to cover the practical former `history_search` use cases

The tool removal is only acceptable if the replacement path is adequate for the common product need.

Required behavior:

- project-scoped exact transcript lookup that previously depended on `history_search` should be achievable through `unified_search` against `chat_message`
- search results should remain source-aware so callers can tell when a hit came from `chat_message` versus `memory`
- when transcript search support is unavailable in a given retrieval mode, the response must degrade explicitly rather than silently pretending coverage
- documentation and prompt guidance should show transcript-oriented examples using `unified_search`
- active search guidance should not require callers to reason about session UUID filters merely to replace common project-local `history_search` usage

Implementation-ready expectations:

- a caller who previously wanted transcript text search should now use `unified_search` with `source_kinds=["chat_message"]` and `mode="fts"` when exact text lookup is desired
- a caller who wants broader project recall should use omitted `source_kinds` or `source_kinds=["memory","chat_message"]`
- chronological replay remains with `history_recall`
- transcript-search migration in this PRD is project-scoped; it does not require `unified_search` to reproduce one exact legacy `session_id=<uuid>` filter path unless implementation review finds that active product behavior still depends on it and the PRD/docs must be amended

**Alternative considered:** keep `history_search` only for exact one-session lookup. Rejected: it preserves a second search tool for a narrower subset of the same domain and weakens the canonical-search goal.

### 5. Update tool descriptions and prompt guidance to avoid legacy steering

The tool catalog and instruction text should reinforce the new product shape.

Required behavior:

- `unified_search` description should make clear that it is the canonical generalized search tool
- `unified_search` parameter wording for omitted `source_kinds` should state the human-oriented default explicitly
- `history_recall` description should stay narrow and literal so it is not mistaken for the removed search tool
- any system/instruction/docs text that currently suggests `history_search` for project recall should be rewritten to reference `unified_search` or `history_recall` as appropriate
- any examples that previously encouraged transcript lookup through `history_search` should be rewritten with `unified_search`

Example wording direction:

- `unified_search`: “Search indexed project content. Omit `source_kinds` for default human-oriented kinds: `memory` and `chat_message`.”
- `history_recall`: “Retrieve earlier conversation messages chronologically.”

### 6. Remove old dispatch paths and keep runtime behavior explicit

The implementation should not leave a half-removed tool path behind.

Required behavior:

- remove `history_search` from tool registration and exported tool schemas
- remove the `history_search` and `search_history` dispatch arms from tool execution handling
- if exact-search logic used by `history_search` remains useful, it should be reached only through supported current surfaces such as `unified_search`
- if there are internal tests or helper names that still describe active behavior as `history_search`, rename or update them so the shipped product surface is not misleading

This keeps the runtime honest about the active product contract.

### 7. Migration and compatibility behavior

The repository should remove the old tool cleanly rather than leaving a vague partially supported state.

Required behavior:

- remove `history_search` in the same implementation slice that updates tool descriptions, docs, and guidance
- update active docs, tests, and prompt guidance in the same implementation slice
- if any code paths internally depended on `history_search`, migrate them to `unified_search` or remove the dependency
- if there are historical docs or PRD notes describing `history_search` as active behavior, update the relevant active docs so they match the shipped tool surface
- no long-lived deprecated alias is required by this PRD

Implementation-ready migration note:

- older transcripts may still mention `history_search`, but that historical chat content does not justify keeping the tool active
- the active tool list, active docs, and prompt guidance are the authoritative post-change surfaces

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Remove the `history_search` schema, remove the `search_history` alias handling, and update `unified_search` plus `history_recall` descriptions so the active search/recall split is explicit. |
| `crates/themion-core/src/agent.rs` and other prompt/instruction assembly paths | Remove guidance that mentions `history_search` and replace it with `unified_search` and `history_recall` guidance as appropriate. |
| `crates/themion-core/src/db.rs` or related query paths | Ensure any search logic that existed only to serve `history_search` is either no longer exposed or is reused through `unified_search` where still product-relevant. |
| `docs/engine-runtime.md` | Update active tool-surface documentation and search guidance to remove `history_search` and explain the `unified_search` versus `history_recall` split. |
| `docs/architecture.md` or other active search/tool docs | Remove `history_search` references where they describe active behavior. |
| tests/tool snapshots/inspection outputs | Update expectations so the active tool surface no longer includes `history_search` or `search_history`. |
| `docs/README.md` | Add this PRD and later reflect implementation status when the change lands. |

## Edge Cases

- ask for transcript text lookup after `history_search` removal → verify: `unified_search` with `source_kinds=["chat_message"]` and `mode="fts"` covers the need.
- ask for broad project recall with omitted `source_kinds` → verify: results come from `memory` and `chat_message`, not tool noise.
- ask for earlier chronological transcript context rather than text search → verify: `history_recall` remains the correct tool and stays available.
- inspect the active tool list after the change → verify: neither `history_search` nor `search_history` is exposed.
- inspect prompt or docs guidance after the change → verify: they no longer steer callers toward `history_search`.
- inspect runtime dispatch after the change → verify: there is no callable `history_search` or `search_history` path left behind.
- run a retrieval mode where one requested source kind cannot participate → verify: `unavailable_source_kinds` or equivalent degradation reporting remains explicit.
- observe older stored transcripts that mention `history_search` → verify: historical content may remain searchable as text, but it does not reactivate the removed tool surface.

## Migration

This is a tool-surface consolidation change with no database migration required by itself.

Required rollout behavior:

- remove `history_search` and `search_history` in the same implementation slice that updates tool descriptions and guidance
- preserve `history_recall` for ordered message retrieval
- preserve transcript search capability through `unified_search` for indexed `chat_message` content
- update active docs so the visible product surface matches the implementation
- treat older transcript mentions of `history_search` as historical text only, not as a compatibility requirement

## Testing

- inspect the registered tool list after implementation → verify: `history_search` is absent, `search_history` is absent, and `unified_search` remains present.
- run a former `history_search`-style exact transcript lookup through `unified_search` with `source_kinds=["chat_message"]` and `mode="fts"` → verify: matching transcript results are returned.
- run `unified_search` with omitted `source_kinds` → verify: default results come from `memory` and `chat_message` only.
- run `history_recall` after the removal → verify: chronological retrieval still works.
- inspect prompt/instruction text or prompt-assembly tests after implementation → verify: `history_search` is no longer mentioned as an available search tool.
- inspect tool-dispatch tests or direct tool invocation handling after implementation → verify: `history_search` and `search_history` are rejected because they are no longer registered or handled.
- run `cargo check -p themion-core` after implementation → verify: default core build stays clean.
- run `cargo check -p themion-core --all-features` after implementation → verify: all-features core build stays clean.
- run `cargo check -p themion-cli` after implementation if active docs/help/runtime wiring there is touched → verify: default CLI build stays clean.
- run `cargo check -p themion-cli --all-features` after implementation if CLI-visible tool/help surfaces are touched → verify: all-features CLI build stays clean.

## Implementation checklist

- [ ] remove `history_search` from tool registration and exported tool schemas
- [ ] remove the `search_history` alias and any remaining dispatch handling for the removed tool family
- [ ] update `unified_search` description and parameter wording so it is the canonical search tool
- [ ] ensure omitted `source_kinds` wording matches the human-oriented default behavior
- [ ] preserve `history_recall` and keep its description focused on chronological recall
- [ ] remove or rewrite prompt/instruction text that references `history_search`
- [ ] update active docs and tool-surface expectations to reflect the consolidated search surface
- [ ] validate the touched core and CLI build configurations
