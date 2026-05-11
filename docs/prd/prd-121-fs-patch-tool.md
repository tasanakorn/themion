# PRD-121: Add an `fs_patch` Tool for Targeted Text-File Edits

- **Status:** Implemented
- **Version:** v0.74.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-11

## Summary

- Themion already has `fs_write_file`, but it replaces the whole file contents.
- Whole-file writes are a poor fit for small code edits. They can overwrite unrelated content and cost many extra tokens.
- Add a new `fs_patch` tool for targeted edits to existing text-based files.
- Implement `fs_patch` with the `mpatch` crate in strict exact mode and wrap it with Themion-side validation and safety guards.
- Keep `fs_write_file` for new files and intentional whole-file replacement, and teach `fs_patch` as the normal tool for small edits.

## Goals

- Add one model-facing tool for targeted edits to existing text-based files.
- Reduce accidental unrelated changes during small code edits.
- Reduce token cost compared with full-file rewrite flows.
- Give the model one simple canonical edit path for localized changes.
- Keep `fs_write_file` for file creation and true whole-file replacement.
- Keep patch behavior inside Themion instead of depending on host `patch` commands.

## Non-goals

- Do not remove `fs_write_file`.
- Do not patch binary files.
- Do not add file create, delete, or rename behavior in the first `fs_patch` slice.
- Do not require TUI or Web UI code to understand patch syntax.
- Do not expose several competing patch formats in the tool contract.
- Do not enable fuzzy best-effort patching in the first slice.

## Background & Motivation

### Current state

Themion currently exposes `fs_write_file` for file writes. Its contract is explicit: it replaces the entire target file contents.

That is useful when the model should create a new file or rewrite a full file deliberately. It is a poor fit for small code edits. In practice, whole-file writes have two recurring costs:

- they can introduce unrelated changes when the regenerated full file does not exactly preserve untouched content
- they require the model to send large file bodies even when only one small region should change

This creates both correctness risk and token waste.

### Why this matters now

Themion already prefers compact tool surfaces and low prompt overhead. A targeted patch tool fits both goals.

The `mpatch` crate is a good implementation base because it already parses patch text and applies multi-file edits inside Rust. However, `mpatch` also supports fuzzy matching and broader patch behaviors than Themion wants in a first safe slice. For `fs_patch` v1, safety is more important than aggressive patch recovery. The first version should use `mpatch` parsing and application in strict exact mode so stale, ambiguous, or out-of-scope patch requests fail clearly.

## Design

### 1. Add one canonical `fs_patch` tool

Themion should add one new model-facing tool named `fs_patch`.

Required schema:

```json
{
  "name": "fs_patch",
  "description": "Apply a targeted patch to existing text files.",
  "parameters": {
    "type": "object",
    "properties": {
      "patch": {
        "type": "string",
        "description": "Unified-diff patch text to apply."
      },
      "reason": {
        "type": "string",
        "description": "Optional reason."
      }
    },
    "required": ["patch"]
  }
}
```

Required behavior:

- accept one patch payload as text
- patch one or more existing text files
- change only the regions described by the patch
- fail clearly when the patch cannot be applied safely
- return a compact result that identifies changed paths and rejected paths
- keep the schema short and explicit

The first implementation must not add parallel modes such as separate search/replace fields, ad hoc JSON patch objects, or shell-command patch strings.

### 2. Support unified diff text only in the first slice

The first `fs_patch` contract should accept unified diff text only.

Required behavior:

- accept unified-diff file headers and hunks as the canonical input shape
- accept patch text wrapped in a markdown ```diff code block because that is common model output
- allow optional leading `diff --git` lines when the patch otherwise uses unified-diff headers
- require `---` and `+++` file header lines for each target file
- reject conflict-marker-only input in the first slice because it does not carry file paths cleanly
- reject patch text that cannot be parsed as supported unified diff

This keeps the first tool surface simple and matches the multi-file use case.

### 3. Scope the first slice to existing regular text files only

The first `fs_patch` slice should stay narrow.

Required behavior:

- patch existing regular text files only
- reject missing-file targets
- reject file creation through `fs_patch` in this first slice
- reject file deletion through `fs_patch` in this first slice
- reject rename or move behavior through `fs_patch` in this first slice
- reject symlinks, directories, devices, sockets, and other non-regular files
- reject binary targets or clearly non-text patch requests
- leave file creation to `fs_write_file`

This keeps the first version focused on the main need: safe localized edits to files that already exist.

### 4. Normalize and validate target paths before apply

Themion should validate patch target paths before reading or writing files.

Required behavior:

- accept unified-diff header paths only; do not add a second path source in the tool schema
- strip a leading `a/` or `b/` prefix before validation
- normalize every target to one canonical workspace-relative path
- reject absolute paths
- reject `..` traversal
- reject empty paths
- reject old/new header pairs that imply rename-style path mismatch in this first slice
- reject the same normalized target path appearing more than once in one `fs_patch` call

This gives the tool one clear workspace path model and avoids ambiguous or unsafe patch targeting.

### 5. Use `mpatch` as the primary implementation library in strict exact mode

Implementation should use `mpatch` as the primary crate/library for patch parsing and application.

Required behavior:

- depend on `mpatch = "1.4.4"` or the implementation-time current compatible release if a later patch version is selected during landing
- use `mpatch` as the normal implementation path
- do not shell out to external `patch` as the standard execution path
- configure application in strict non-fuzzy mode for the first slice
- use `ApplyOptions::exact()` or the equivalent exact-match setting in the selected compatible `mpatch` release
- explicitly reject broader `mpatch` behaviors that are out of scope for v1, including conflict-marker-only input, file creation, deletion-on-empty, duplicate targets, and partial success
- if `mpatch` cannot support one required first-slice behavior cleanly, stop and document the blocker instead of silently broadening the contract

This keeps patch behavior inside Themion's Rust runtime and aligns the first tool version with the safety goal.

### 6. Make patch application atomic across the whole request

The first implementation should treat one `fs_patch` call as one atomic patch transaction.

Required behavior:

- validate the whole payload before writing any file
- if any target, hunk, or validation step fails, leave all files unchanged
- do not allow partial success in the first slice
- if patch context does not match current file content, fail clearly instead of guessing
- the tool must not silently rewrite untouched file regions

This is more important than maximizing patch acceptance. The model should be able to trust that one rejected file means no file was changed.

### 7. Restrict the first slice to UTF-8 text and explicit size limits

The first `fs_patch` slice should use simple, conservative content rules.

Required behavior:

- support UTF-8 text files only in v1
- reject targets containing NUL bytes
- reject targets that are not valid UTF-8
- allow a patch result to produce an empty file
- do not interpret empty final content as file deletion
- apply Themion-side limits before patch application

Suggested starting limits:

- patch text size `<= 256 KiB`
- target files per call `<= 32`
- total hunks per call `<= 256`
- per-file output size bounded, for example `<= 1 MiB` unless the repository already has a shared tighter file-content limit

These rules keep the first version deterministic and reduce runtime and prompt risk.

### 8. Preserve file content exactly enough to keep line endings stable

The first `fs_patch` slice should stay strict about content matching while avoiding surprising line-ending rewrites.

Required behavior:

- patch matching should be exact against the current file content
- preserve the existing file line-ending style when practical
- do not add a special fuzzy LF/CRLF normalization layer in v1 unless the selected `mpatch` behavior requires one documented rule
- if line-ending differences prevent an exact apply, reject the patch clearly

This keeps the v1 contract simple while avoiding hidden normalization behavior.

### 9. Define one compact result shape

The first implementation should return one compact structured result.

Required result fields:

- `ok: boolean`
- `entity: "file_patch"`
- `operation: "apply"`
- `changed_paths: string[]`
- `rejected_paths: string[]`
- `message: string`

Required behavior:

- full success returns `ok: true` and an empty `rejected_paths`
- any rejected path returns `ok: false`
- because v1 is atomic, any rejected patch must leave `changed_paths` empty
- `message` should stay short and summarize success or failure
- if the patch fails before file targeting is known, `changed_paths` and `rejected_paths` may both be empty
- validation failures should return the relevant normalized target path in `rejected_paths` when it is known

This keeps the tool result easy for models to inspect without reading large prose.

### 10. Keep `fs_patch` and `fs_write_file` as distinct file-edit tools

Themion should keep both tools, but each should have a clear role.

Required behavior:

- `fs_write_file` remains the tool for new files and intentional whole-file replacement
- `fs_patch` becomes the preferred tool for small or localized edits to existing text files
- prompt and tool guidance should teach this distinction directly
- tool descriptions should make the difference obvious enough that models do not need long extra explanation

The intended rule is simple:

- use `fs_write_file` when the whole file should be written
- use `fs_patch` when only part of an existing text file should change

### 11. Keep patch logic in the core runtime/tool layer

This is a runtime tool capability, not a UI feature.

Required behavior:

- schema and patch-application logic belong in `themion-core`
- `themion-cli` should only expose and render the tool through existing runtime paths
- TUI and Web UI must not implement their own patch parser or patch policy
- text/binary checks and patch-safety rules should live with the tool implementation

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Add the `fs_patch` tool schema, dispatch path, validation, and compact result shape. Keep `fs_write_file` as a separate whole-file write tool. |
| `crates/themion-core` file/tool implementation layer | Add unified-diff parsing and strict exact patch application for existing regular UTF-8 text files, using `mpatch` in exact mode with Themion-side validation and atomic whole-request behavior. |
| `crates/themion-core/Cargo.toml` | Add the `mpatch` dependency. |
| `crates/themion-core` tests | Add focused tests for parse success, exact-apply success, stale-context failure, missing-file rejection, create/delete rejection, duplicate-target rejection, binary rejection, special-file rejection, and atomic multi-file failure behavior. |
| `crates/themion-cli` transcript/tool display path | Surface the new tool through existing runtime output without adding UI-owned patch logic. |
| `docs/tool-design-and-implementation-guide.md` | Update guidance if needed so file-edit tools clearly distinguish whole-file write from targeted patch application. |
| `docs/engine-runtime.md` | Update tool/runtime guidance so `fs_patch` is the preferred path for small edits to existing text files. |
| `docs/README.md` | Track this PRD and later update its status/version when implemented. |

## Implementation Notes

Implemented in v0.74.0. Themion now exposes `fs_patch` from `crates/themion-core/src/tools.rs` with a compact `{ patch, reason? }` schema and a compact `file_patch` result. The landed implementation uses `mpatch` in exact mode, accepts raw unified diffs and markdown-wrapped ```diff blocks, validates and normalizes workspace-relative target paths, rejects conflict-marker input plus create/delete/rename requests, preserves CRLF style for supported files, and applies multi-file requests atomically by validating everything before write and rolling back earlier writes if a later write fails. `fs_write_file` remains the whole-file write and file-creation path, the TUI now labels `fs_patch` tool calls, and runtime/docs guidance now teaches `fs_patch` as the preferred tool for localized edits to existing text files.

## Edge Cases

- patch targets one existing text file with a small exact match → verify: only the requested region changes.
- patch text is wrapped in a markdown ```diff block → verify: the tool still parses and applies it.
- patch includes optional `diff --git` lines plus valid unified-diff headers → verify: the tool accepts it.
- patch text is conflict markers only → verify: the tool rejects it.
- patch targets a missing file → verify: the tool fails clearly.
- patch tries to create a new file → verify: the tool rejects the request and guidance still points to `fs_write_file` for creation.
- patch tries to delete a file → verify: the tool rejects it.
- patch makes an existing file empty → verify: the tool allows the empty file result and does not treat it as deletion.
- patch old/new headers imply a rename or mismatched paths → verify: the tool rejects it.
- patch includes the same normalized target path more than once → verify: the tool rejects it before writing any file.
- patch context does not match current file contents → verify: the tool fails clearly instead of guessing.
- patch includes multiple file edits and one file fails → verify: the tool returns `ok: false` and leaves all files unchanged.
- patch targets a binary file or invalid UTF-8 content → verify: the tool rejects the request.
- patch targets a symlink or other special file → verify: the tool rejects the request.
- patch exceeds configured size or file-count limits → verify: the tool rejects the request with a clear validation error.
- caller wants to replace the entire file contents → verify: guidance points to `fs_write_file` as the better tool.
- caller wants to change one line in a large file → verify: guidance points to `fs_patch` as the better tool.

## Migration

No database migration is required.

This is an additive tool-surface change. Existing `fs_write_file` callers can keep working, but model-facing guidance should shift normal small-edit behavior toward `fs_patch` after implementation lands.

Minor-version scope is appropriate because this adds a new user-visible editing capability.

## Testing

- inspect generated tool schemas → verify: `fs_patch` appears with the exact `patch` + optional `reason` parameter shape.
- inspect generated tool schemas → verify: `fs_write_file` remains present and still describes whole-file replacement.
- apply a one-line unified diff to an existing Rust file → verify: only that line changes.
- apply a markdown-wrapped ```diff patch to an existing text file → verify: it parses and applies correctly.
- apply a valid patch with optional `diff --git` header lines → verify: the tool accepts it when the required `---` / `+++` headers and hunks are valid.
- apply a small multi-hunk patch to one text file → verify: all requested hunks apply correctly in exact mode.
- apply a multi-file patch to existing text files → verify: all files change only when every file and hunk validates and applies successfully.
- apply a patch with stale context → verify: the tool returns `ok: false` and leaves all files unchanged.
- apply a patch to a missing file → verify: the tool returns `ok: false` and reports the target in `rejected_paths`.
- apply a patch that tries to create a new file → verify: the tool rejects it in this first slice.
- apply a patch that tries to delete a file → verify: the tool rejects it in this first slice.
- apply a patch that makes a file empty → verify: the tool succeeds without treating that result as deletion.
- apply a patch that targets a binary file or invalid UTF-8 content → verify: the tool rejects it.
- apply a patch that targets a symlink or other non-regular file → verify: the tool rejects it.
- apply a patch with duplicate normalized target paths → verify: the tool rejects it before writing any file.
- apply a patch that exceeds configured size, file-count, hunk-count, or output-size limits → verify: the tool rejects it clearly.
- apply a conflict-marker-only patch → verify: the tool rejects it.
- compare a small edit done through `fs_patch` versus `fs_write_file` flow → verify: the patch path avoids whole-file rewrite output and reduces changed surface.
- inspect updated docs and prompt guidance → verify: they teach `fs_patch` for small edits to existing text files and `fs_write_file` for whole-file replacement or file creation.
- run `cargo check -p themion-core` → verify: core tool changes compile.
- run `cargo check -p themion-core --all-features` → verify: all-feature core build compiles.
- run `cargo check -p themion-cli` → verify: default CLI build compiles.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build compiles.

## Implementation checklist

- [x] add `mpatch` to `crates/themion-core/Cargo.toml`
- [x] add the `fs_patch` tool schema with the exact `patch` + optional `reason` shape
- [x] parse raw unified diff and markdown-wrapped ```diff input
- [x] accept optional `diff --git` header lines while requiring canonical unified-diff file headers and hunks
- [x] normalize and validate patch target paths by stripping `a/` / `b/`, rejecting absolute or traversal paths, rejecting empty paths, rejecting rename-style mismatches, and rejecting duplicate normalized targets
- [x] reject unsupported patch text such as conflict-marker-only input
- [x] implement strict exact patch application for existing regular UTF-8 text files only
- [x] reject missing-file, create-file, delete-file, rename-file, binary-file, invalid-UTF-8, and special-file patch targets clearly in the first slice
- [x] make the whole `fs_patch` call atomic so any failure leaves all files unchanged
- [x] enforce conservative pre-apply limits for patch size, file count, hunk count, and output size
- [x] allow empty final file content without treating it as deletion
- [x] preserve current line-ending style when practical and reject line-ending mismatches that prevent exact apply
- [x] return the compact `file_patch` result shape with `changed_paths` and `rejected_paths`
- [x] keep `fs_write_file` as the whole-file write and file-creation path
- [x] update tool guidance so small edits to existing text files prefer `fs_patch`
- [x] update `docs/engine-runtime.md` and any related durable guidance that teaches file-edit tool choice
- [x] add focused tests for success, stale-context failure, missing-file rejection, create/delete rejection, duplicate-target rejection, binary rejection, invalid-UTF-8 rejection, special-file rejection, markdown diff input, conflict-marker rejection, and atomic multi-file behavior
- [x] update PRD/docs status notes after implementation lands
