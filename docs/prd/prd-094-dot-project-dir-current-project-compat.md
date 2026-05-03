# PRD-094: Treat `project_dir="."` as Current Project Scope Fallback in Targeted Project-Scoped Tools

- **Status:** Implemented
- **Version:** v0.59.2
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-03

## Goals

- Modify the behavior of the already implemented targeted project-scoped tools so `project_dir="."` is treated as the current project scope.
- Add a runtime fallback for Studio-style model calls that send `"."` when they mean the current project.
- Preserve the canonical model-facing contract: omit `project_dir` for the current project, pass an explicit absolute project path, or use `[GLOBAL]` only where a tool already supports it.
- Keep `"."` as compatibility behavior only, not part of the advertised tool schema contract.
- Make the targeted implemented tools behave consistently instead of returning empty results or indexing the wrong scope because literal `"."` does not match the current absolute project path.

## Non-goals

- Do not add `"."` to tool schema descriptions as a documented canonical input.
- Do not change the meaning of `[GLOBAL]`.
- Do not reinterpret arbitrary relative paths other than the exact compatibility token `"."`.
- Do not change durable storage format for `project_dir` values.
- Do not redesign project scoping beyond this fallback.
- Do not claim that every project-scoped tool in the product uses one shared resolver today when the current implementation does not.

## Background & Motivation

Some already implemented project-scoped tools currently use absolute project paths internally or default omitted `project_dir` to the current project. However, Studio-style model behavior can still send `project_dir="."` as shorthand for “current project.”

Today that value can fail because:

- the tool input path treats `"."` as a literal project scope string
- stored project-scoped rows use the concrete absolute project path
- the lookup or rebuild therefore misses data that actually belongs to the current project, or indexes under the wrong scope

This PRD changes behavior for the targeted implemented tools so that when `"."` is detected, the tool/runtime resolution layer resolves it to the current project’s absolute path before normal scoped operations continue.

**Alternative considered:** require callers to stop sending `"."` and rely only on schema wording. Rejected: the current project intent is obvious, and this compatibility fallback is low risk and improves robustness for model-driven callers.

## Design

### Behavior requirement

When a targeted implemented project-scoped tool receives `project_dir="."`, it must treat that value as a fallback spelling for the current project scope.

Required behavior:

- detect the exact token `"."`
- resolve it to the current project’s absolute path before project-scoped lookup, creation, listing, or rebuild logic runs
- keep omitted `project_dir` behavior unchanged
- keep explicit absolute paths unchanged
- keep `[GLOBAL]` unchanged where the tool already supports it
- do not reinterpret other relative paths such as `".."` or `"foo/bar"`

### Tool contract requirement

The model-facing tool schema contract must remain canonical and strict.

Required behavior:

- tool schema descriptions must not mention `project_dir="."` as a public or preferred input form
- schema wording should continue to direct callers to omit `project_dir` for the current project
- schema wording should continue to use explicit absolute project paths when a specific project must be named
- schema wording may mention `[GLOBAL]` only for tools that actually support it

This keeps the fallback as runtime compatibility behavior for model mistakes or Studio-style shorthand, not as part of the advertised API surface.

### Affected implemented tools

This PRD targets the concrete already implemented tool handlers in `crates/themion-core/src/tools.rs` that currently accept `project_dir`.

Target implemented tools:

- `memory_create_node`
- `unified_search`
- `memory_list_hashtags`
- `unified_search_rebuild`

These tools do not all use the same code path today:

- `memory_create_node`, `unified_search`, and `memory_list_hashtags` currently call `resolve_memory_project_dir(...)`
- `unified_search_rebuild` currently resolves `project_dir` inline in its own handler instead of using `resolve_memory_project_dir(...)`

This PRD requires these targeted tools to converge on equivalent `"."` fallback behavior, but it does not assume that convergence already exists.

### Implementation layer and function ownership

This behavior should be implemented in the tool/runtime resolution layer, not in the storage/query normalization layer.

Required ownership decision:

- primary implementation point: `crates/themion-core/src/tools.rs`
- primary function to change: `resolve_memory_project_dir(args: &Value, ctx: &ToolCtx) -> String`
- additional targeted handler change: `unified_search_rebuild` should stop resolving `project_dir` inline and should use the same resolver behavior

Reasoning:

- `resolve_memory_project_dir(...)` already owns translation from model-provided tool arguments into the concrete project scope string for several targeted tools
- it already applies current-project defaulting when `project_dir` is omitted
- adding the exact-token `"."` fallback here keeps the behavior close to tool input handling
- `unified_search_rebuild` is currently an exception and should be brought into the same tool-layer resolution behavior
- this keeps `memory.rs` focused on storage/query normalization and validation instead of making low-level memory helpers depend on Studio-style tool-call compatibility behavior

Required implementation shape:

- if `args["project_dir"]` is absent, keep returning `ctx.project_dir` as today
- if `args["project_dir"] == "."`, return `ctx.project_dir` as an absolute path string
- if `args["project_dir"] == "[GLOBAL]"`, preserve `[GLOBAL]` unchanged for tools that support it
- otherwise, pass through the provided string unchanged so downstream validation and existing semantics continue to work

### Runtime target path

The expected runtime path after implementation is:

- tool call enters `crates/themion-core/src/tools.rs`
- `resolve_memory_project_dir(...)` resolves omitted or `"."` current-project intent to `ctx.project_dir`
- `memory_create_node`, `unified_search`, `memory_list_hashtags`, and `unified_search_rebuild` all use equivalent tool-layer resolution behavior before calling downstream operations
- `crates/themion-core/src/memory.rs` continues to normalize and validate already-resolved scope strings without becoming the owner of `"."` fallback behavior

### Versioning intent

This PRD targets **v0.59.2**.

Reasoning:

- the workspace PRD stream already documents PRD-092 and PRD-093 at `v0.59.1`
- this PRD is a behavior correction for already implemented tools rather than a new top-level capability
- the change is backward-compatible and should improve robustness for existing model/tool callers
- a patch release is the appropriate default unless the implementation expands beyond this compatibility fix

**Alternative considered:** target `v0.60.0`. Rejected for now: the described behavior is a narrow compatibility fix, not a feature-sized product expansion.

### Implementation boundary

The behavior change should happen at tool/runtime scope resolution, not in storage.

Required behavior:

- do not migrate stored rows
- do not persist literal `"."` as a new canonical project scope
- resolve the fallback before the underlying scoped operation runs
- make the targeted tools consistent even if one of them currently resolves `project_dir` inline

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Update `resolve_memory_project_dir(...)` so `project_dir="."` resolves to the current project absolute path before scoped tool execution, while keeping schema text canonical and not advertising `"."`. |
| `crates/themion-core/src/tools.rs` tool handlers | Ensure `memory_create_node`, `unified_search`, and `memory_list_hashtags` continue to route through `resolve_memory_project_dir(...)`, and update `unified_search_rebuild` to use equivalent resolver behavior instead of its current inline handling. |
| `crates/themion-core/src/memory.rs` | Keep existing normalization/validation behavior for already-resolved scope strings; do not make `memory.rs` the owner of Studio-style `"."` fallback. |
| tests for `resolve_memory_project_dir(...)`, `unified_search_rebuild`, and affected tool behavior | Add or update regression coverage proving that omitted `project_dir`, explicit current-project absolute path, and `project_dir="."` produce equivalent current-project behavior where applicable. |
| `docs/README.md` | Keep PRD index/status aligned when implementation lands. |

## Edge Cases

- call `memory_create_node` with omitted `project_dir` → verify: node is stored in the current project scope.
- call `memory_create_node` with the explicit absolute current project path → verify: node is stored in the same current project scope.
- call `memory_create_node` with `project_dir="."` → verify: node is stored in the current project scope, not under literal `"."`.
- call `unified_search` with omitted `project_dir` → verify: current-project results are returned.
- call `unified_search` with the explicit absolute current project path → verify: the same results are returned.
- call `unified_search` with `project_dir="."` → verify: behavior matches omitted-current-project lookup.
- call `memory_list_hashtags` with omitted `project_dir` → verify: current-project hashtags are returned.
- call `memory_list_hashtags` with the explicit absolute current project path → verify: the same hashtags are returned.
- call `memory_list_hashtags` with `project_dir="."` → verify: current-project hashtags are returned.
- call `unified_search_rebuild` with omitted `project_dir` → verify: rebuild targets the current project scope.
- call `unified_search_rebuild` with the explicit absolute current project path → verify: rebuild targets the same scope.
- call `unified_search_rebuild` with `project_dir="."` → verify: rebuild targets the current project scope rather than literal dot scope.
- call with `project_dir="[GLOBAL]"` on a tool that supports it → verify: Global Knowledge behavior remains unchanged.
- call with an explicit non-current absolute project path → verify: the explicit path is preserved.
- call with a relative path other than `"."` → verify: this PRD does not reinterpret it.
- inspect persisted storage after fallback use → verify: storage still uses concrete project scope values rather than persisting `"."` as canonical scope.

## Migration

No database migration is required.

Rollout requirements:

- add the `"."` fallback in `resolve_memory_project_dir(...)`
- update `unified_search_rebuild` to use the shared resolver behavior instead of inline project-dir handling
- preserve current canonical schema wording
- do not advertise `"."` in tool schema descriptions
- add regression coverage for fallback behavior

## Testing

- test `resolve_memory_project_dir(...)` with omitted `project_dir` → verify: it returns the current project absolute path from `ctx.project_dir`.
- test `resolve_memory_project_dir(...)` with `project_dir="."` → verify: it returns the same current project absolute path.
- test `resolve_memory_project_dir(...)` with an explicit absolute path → verify: it returns that path unchanged.
- test `resolve_memory_project_dir(...)` with `project_dir="[GLOBAL]"` → verify: it returns `[GLOBAL]` unchanged for downstream handling.
- call `memory_create_node` with omitted `project_dir` in a project context → verify: the created node uses the current project scope.
- call `memory_create_node` with the explicit absolute current project path → verify: the created node uses the same project scope.
- call `memory_create_node` with `project_dir="."` → verify: the created node uses the same project scope.
- call `unified_search` with omitted `project_dir` against known current-project content → verify: current-project results are returned.
- call `unified_search` with the explicit absolute current project path → verify: the same results are returned.
- call `unified_search` with `project_dir="."` → verify: the same results are returned.
- call `memory_list_hashtags` with omitted `project_dir` → verify: current-project hashtags are returned.
- call `memory_list_hashtags` with the explicit absolute current project path → verify: the same hashtags are returned.
- call `memory_list_hashtags` with `project_dir="."` → verify: the same hashtags are returned.
- call `unified_search_rebuild` with omitted `project_dir` → verify: rebuild targets the current project scope.
- call `unified_search_rebuild` with the explicit absolute current project path → verify: rebuild targets the same scope.
- call `unified_search_rebuild` with `project_dir="."` → verify: it acts on current-project scope.
- inspect affected tool schemas after implementation → verify: schema descriptions do not advertise `"."`.
- run `cargo check -p themion-core` after implementation → verify: default core build stays clean.
- run `cargo check -p themion-core --all-features` after implementation → verify: all-features core build stays clean.
- run `cargo check -p themion-cli` after implementation if touched code crosses crate boundaries → verify: default CLI build stays clean.
- run `cargo check -p themion-cli --all-features` after implementation if touched code crosses crate boundaries → verify: all-features CLI build stays clean.

## Implementation checklist

- [x] update `resolve_memory_project_dir(...)` in `crates/themion-core/src/tools.rs`
- [x] add exact-token fallback from `"."` to the current project absolute path
- [x] preserve omitted-current-project behavior unchanged
- [x] preserve explicit absolute-path behavior unchanged
- [x] preserve `[GLOBAL]` behavior unchanged where supported
- [x] avoid reinterpreting non-dot relative paths
- [x] ensure `memory_create_node`, `unified_search`, and `memory_list_hashtags` use the shared resolver path
- [x] update `unified_search_rebuild` to use the same resolver behavior instead of inline `project_dir` handling
- [x] keep tool schema descriptions canonical and do not advertise `"."`
- [x] add regression coverage for omitted scope, explicit current-project absolute path, and `"."` equivalence
- [x] validate touched builds in the required default and all-features configurations

## Implementation notes

- Landed in `v0.59.2`.
- `resolve_memory_project_dir(...)` in `crates/themion-core/src/tools.rs` now treats exact `project_dir="."` as the current project path from `ctx.project_dir`.
- `unified_search_rebuild` now uses the same tool-layer resolver behavior instead of inline current-project fallback logic.
- Regression coverage was added in `crates/themion-core/tests/memory_tools.rs` for omitted `project_dir`, explicit current-project paths, and `project_dir="."` across the targeted tools.
