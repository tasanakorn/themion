# PRD-092: Slash Command for Source-Kind-Scoped Unified-Search Indexing

- **Status:** Implemented
- **Version:** v0.59.1
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-03

## Implementation status

Landed in `v0.59.1`.

## Summary

- PRD-091 introduced generalized unified-search indexing with source-kind-scoped rebuild support in the core/tool layer, but the human-facing slash command still only supports whole-project refresh and full rebuild.
- Add a slash-command and matching command-path improvement so a human can rebuild unified-search indexes for one specific source kind such as `memory` without reindexing every supported source kind in the project.
- When indexing one source kind, process the newest source records first so recent edits become searchable sooner during manual maintenance or restart validation.
- Keep the existing `/unified-search index` and `/unified-search index full` behavior unchanged for whole-project maintenance.
- Make source-kind selection explicit and bounded to the same supported values already used by `unified_search` and `unified_search_rebuild`.

## Goals

- Let a human trigger unified-search indexing for one specific source kind from the TUI slash-command surface.
- Keep source-kind-scoped indexing aligned with the already shipped core rebuild capability.
- Prioritize recently updated source records first during a rebuild so manual validation sees the most recent content indexed earliest.
- Preserve the existing whole-project slash-command behavior for callers who do not specify a source kind.
- Keep the command syntax compact, explicit, and easy to discover from built-in help.
- Preserve runtime ownership of indexing behavior and keep the TUI limited to command intake and display.
- Keep the allowed source-kind values consistent with the generalized unified-search system.

## Non-goals

- No redesign of the underlying unified-search indexing schema, chunking rules, or aggregation behavior from PRD-091.
- No arbitrary multi-filter indexing syntax beyond selecting one explicit source kind in this PRD.
- No requirement to add source-record-id-targeted rebuild commands in this slice.
- No change to `unified_search` query semantics.
- No automatic background indexing policy changes beyond exposing the narrower manual trigger.
- No requirement to expose source-kind-scoped rebuild progress as a separate long-lived runtime dashboard in this PRD.
- No truncation or best-effort partial indexing mode; recency-first ordering changes processing priority, not completion requirements.

## Background & Motivation

### Current state

PRD-091 shipped generalized unified-search indexing across `memory`, `chat_message`, `tool_call`, and `tool_result`.

Today:

- the core rebuild path already supports an optional `source_kind` filter
- the `unified_search_rebuild` tool already accepts optional `source_kind`
- the TUI slash-command surface only supports `/unified-search index` and `/unified-search index full`
- the headless `--command unified-search index` path only accepts `--full`
- built-in slash-command help text only describes whole-project indexing
- the current unified-search input collection path gathers eligible rows but does not document a recency-first indexing requirement for rebuild order

This means the runtime already knows how to rebuild a single source kind, but a human using the normal slash-command maintenance path cannot request that narrower scope directly.

### Why this matters now

Generalized indexing now spans multiple source domains with very different corpus sizes.

A human debugging or validating one source domain often needs to:

- rebuild only `memory` after creating or editing Project Memory nodes
- rebuild only `chat_message` when validating transcript indexing
- rebuild only `tool_call` or `tool_result` when verifying tool-record coverage
- avoid the extra time and noise of reprocessing every indexed source kind in the project
- see the most recent edits indexed first during restart testing or partial-progress observation

The core capability already exists, so the remaining product gap is the human-facing maintenance surface and clear indexing-priority behavior.

**Alternative considered:** require humans to use only the model-facing `unified_search_rebuild` tool for source-kind-scoped rebuilds. Rejected: PRD-091 explicitly established a human-invocable slash-command/runtime maintenance path, and manual maintenance should not require going through an agent tool call when the user wants a direct command.

## Design

### 1. Add explicit source-kind syntax to the slash command

Themion should extend the unified-search slash-command surface to accept one explicit source kind.

Required behavior:

- `/unified-search index` keeps its current meaning: refresh all eligible unified-search source kinds for the current project
- `/unified-search index full` keeps its current meaning: rebuild all eligible unified-search source kinds for the current project
- add a source-kind-scoped form for incremental refresh
- add a source-kind-scoped form for full rebuild
- source-kind selection is explicit rather than inferred from free-form text

Implementation-ready command shape:

- `/unified-search index <source_kind>`
- `/unified-search index full <source_kind>`

Supported `<source_kind>` values in this PRD:

- `memory`
- `chat_message`
- `tool_call`
- `tool_result`

Invalid or unsupported source kinds must produce a clear usage or validation error rather than silently falling back to whole-project indexing.

### 2. Keep runtime-owned command execution and narrow TUI responsibility

The TUI should remain only a command-intake surface.

Required behavior:

- the TUI parses the command and forwards an intent that includes `full` plus an optional `source_kind`
- runtime/app-state code remains responsible for busy checks, status text, background task launch, and result reporting
- the actual indexing call continues to use the shared runtime/core rebuild path that already supports `source_kind`
- the TUI must not grow its own indexing logic or source-kind-specific policy beyond bounded command parsing and help text

This preserves the repository layering rule that the TUI is a surface and runtime/app-state owns behavior.

### 3. Prefer recency-first processing order within the requested indexing scope

When a human triggers unified-search indexing, Themion should process the newest eligible source objects first within the selected scope.

Required behavior:

- when `source_kind` is specified, candidate source objects for that source kind should be processed in descending recency order
- when no `source_kind` is specified, each source-kind-specific collection path should still prefer descending recency within its own candidate set before indexing work is written
- for v1, recency means the same freshness timestamp already used by the index record: `source_updated_at_ms`
- the effective processing order should therefore prefer higher `source_updated_at_ms` values before lower ones
- this is a prioritization rule only; the rebuild or refresh must still attempt the full eligible scope unless it fails or is interrupted
- reports and query semantics remain unchanged; only indexing order becomes explicitly specified

Why this is required:

- recent memory or transcript edits are usually the first thing a human wants to validate
- restart testing often needs confirmation that the newest objects became queryable before older backlog completes
- if indexing is interrupted or observed mid-flight, the most relevant recent objects are more likely to be available first

Current implementation note: the existing `collect_unified_search_inputs(...)` path in `crates/themion-core/src/memory.rs` gathers eligible source records but does not currently document or enforce a recency-first ordering contract for the final collected set. This PRD makes that ordering explicit.

**Alternative considered:** preserve database-natural or source-enumeration order. Rejected: it is less useful for manual maintenance and restart-validation workflows, because it gives no product guarantee that the most recently changed content becomes searchable first.

### 4. Align the human command path with the existing core rebuild contract

The human-facing command should reuse the existing capability rather than inventing a separate implementation path.

Required behavior:

- the runtime command shape should carry optional `source_kind`
- the background indexing task should call the same `rebuild_unified_search_index(project_dir, source_kind, full)` path already used by the model-facing tool and headless path
- returned reports should continue to be scoped and inspectable
- transcript/status wording should mention the selected source kind when one is provided

Example user-visible status text:

- `refreshing generalized unified-search index for source kind 'memory' in this project…`
- `rebuilding generalized unified-search index for source kind 'tool_result' in this project…`

**Alternative considered:** add a second source-kind-specific slash-command family such as `/unified-search index-memory`. Rejected: one command family with one bounded optional source-kind argument is more compact and matches the generalized design of PRD-091.

### 5. Extend the headless command path in the same command family

The standalone command surface should keep pace with the slash-command improvement.

Required behavior:

- `themion --command unified-search index [--full]` keeps its current meaning
- add one bounded source-kind selector to the same command family for direct shell use
- the shell form should use the same source-kind enum values as the slash command and tool contract
- invalid combinations should fail clearly

Implementation-ready command shape:

- `themion --command unified-search index [--full] [--source-kind <source_kind>] [--dir PATH]`

This keeps human-invocable maintenance consistent across interactive and non-interactive CLI usage.

### 6. Update discoverability and guidance docs

Because this change alters the documented maintenance surface, the repository docs should reflect it directly.

Required behavior:

- slash-command help text should list the new source-kind-scoped forms
- CLI usage/help text should list the new `--source-kind` argument
- docs that describe unified-search maintenance should explain that whole-project indexing remains available and that one source kind may now be rebuilt explicitly
- docs should also state that indexing is recency-first by `source_updated_at_ms DESC` within the requested scope
- the allowed source-kind values should be spelled out in at least one user-facing doc location

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Extend `/unified-search index` parsing and help text to support one explicit `<source_kind>` and `full <source_kind>` forms while keeping the TUI limited to input parsing and display. |
| `crates/themion-cli/src/app_runtime.rs` or equivalent runtime-command definitions | Extend the runtime command intent shape so unified-search indexing carries an optional `source_kind`. |
| `crates/themion-cli/src/app_state.rs` | Preserve busy checks and runtime-owned background execution, and update indexing status text/report handling to include optional source-kind scope. |
| `crates/themion-cli/src/main.rs` | Extend `--command unified-search index` parsing to accept one bounded `--source-kind <source_kind>` argument alongside `--full`. |
| `crates/themion-cli/src/headless_runner.rs` | Forward the optional `source_kind` through the existing headless unified-search index execution path. |
| `crates/themion-core/src/memory.rs` | Make unified-search input collection or pre-write ordering explicitly prefer `source_updated_at_ms DESC` within the requested scope before indexing documents. |
| `crates/themion-core/src/tools.rs` | Keep the existing `unified_search_rebuild` source-kind contract aligned with the human-facing command wording if any schema/help wording needs clarification. |
| `docs/engine-runtime.md` | Update unified-search maintenance documentation to include source-kind-scoped slash-command and headless command behavior plus recency-first indexing order. |
| `docs/README.md` | Add this PRD to the index and later reflect status/version when it lands. |

## Edge Cases

- run `/unified-search index memory` → verify: only `memory` documents are refreshed for the current project.
- run `/unified-search index full tool_result` → verify: only `tool_result` documents are rebuilt for the current project.
- run `/unified-search index` with no source kind → verify: current whole-project behavior remains unchanged.
- run `/unified-search index full` with no source kind → verify: current whole-project full rebuild behavior remains unchanged.
- run `/unified-search index nonsense` → verify: the command fails with a clear source-kind validation or usage message.
- run `/unified-search index memory extra` → verify: extra trailing tokens fail clearly rather than being ignored.
- run the command while the app is busy → verify: existing busy-guard behavior still applies.
- run the headless command with `--source-kind chat_message` → verify: the selected source kind is forwarded to the shared rebuild path.
- run a source-kind-scoped rebuild where that source kind currently has zero indexable records in the project → verify: the command completes with a clear zero-work scoped report instead of pretending the whole project was indexed.
- inspect a partially completed or interrupted rebuild → verify: the most recent eligible rows in the requested scope were attempted before older rows.
- rebuild a mixed-scope project with omitted `source_kind` → verify: each source collection path still prefers newer source objects ahead of older ones within that source kind.

## Migration

This is a maintenance-surface expansion with no schema migration.

Required rollout behavior:

- keep existing whole-project slash-command and headless-command forms working unchanged
- add source-kind-scoped forms in the same command family rather than replacing existing commands
- adopt recency-first processing order without changing the final set of indexed documents expected from a successful full run
- update user-facing docs/help so humans discover the narrower rebuild capability without needing source-code knowledge

## Testing

- run `/unified-search index` → verify: whole-project refresh still works as before.
- run `/unified-search index full` → verify: whole-project full rebuild still works as before.
- run `/unified-search index memory` → verify: the rebuild report is scoped to `memory` and other source kinds are not rebuilt.
- run `/unified-search index full tool_call` → verify: the rebuild report is scoped to `tool_call`.
- run `/unified-search index invalid_kind` → verify: the user gets a clear validation or usage error.
- run `themion --command unified-search index --source-kind memory` → verify: headless command parsing accepts the scoped form and returns a scoped JSON report.
- run `themion --command unified-search index --full --source-kind tool_result` → verify: full scoped rebuild works from the shell command path too.
- populate multiple eligible rows with different `source_updated_at_ms` values in one source kind, run scoped indexing, and inspect the resulting attempt order or observable partial-progress state → verify: newer rows are processed before older rows.
- interrupt a larger scoped rebuild after some work completes → verify: the already indexed subset skews toward the newest eligible source objects.
- run `cargo check -p themion-cli` after implementation → verify: default CLI build stays clean.
- run `cargo check -p themion-cli --features stylos` after implementation if Stylos-gated CLI code is touched nearby → verify: relevant feature-enabled CLI build stays clean.
- run `cargo check -p themion-cli --all-features` after implementation → verify: all-features CLI build stays clean.
- run `cargo check -p themion-core` and `cargo check -p themion-core --all-features` after implementation if shared tool/runtime contracts are touched → verify: touched core paths still compile in default and all-feature builds.

## Implementation checklist

- [ ] add one optional `source_kind` field to the unified-search indexing runtime command intent
- [ ] extend TUI slash-command parsing to accept `/unified-search index <source_kind>` and `/unified-search index full <source_kind>`
- [ ] validate `<source_kind>` against the generalized supported source kinds and reject invalid values clearly
- [ ] preserve existing whole-project command behavior when `source_kind` is omitted
- [ ] extend headless `--command unified-search index` parsing to accept `--source-kind <source_kind>`
- [ ] forward optional `source_kind` through the headless and interactive runtime paths to `rebuild_unified_search_index`
- [ ] make scoped and unscoped unified-search indexing prefer `source_updated_at_ms DESC` within each candidate source set
- [ ] update user-facing status/help text to mention scoped indexing when relevant
- [ ] update docs that describe unified-search maintenance behavior
- [ ] validate the touched CLI build configurations and any touched core build configurations
