# PRD-116: Compact Detail Levels for Source Outline

- **Status:** Implemented
- **Version:** v0.72.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-09

## Summary

- `source_outline` is useful, but its current full JSON shape can be large for big files.
- Add compact detail levels so agents can request a smaller outline for normal navigation.
- Keep the current full object shape available for compatibility and debugging.
- Make the compact shape preserve the facts agents need most: language, path, symbols, spans, parent names, imports, warnings, and parse status.
- Do not move source-analysis policy into TUI or Web UI code.

## Goals

- Reduce `source_outline` result size for large files without making agents read whole files more often.
- Preserve one-file navigation usefulness in the compact output.
- Keep the existing full outline available for callers that need graph-ready IDs, edges, import resolution fields, or the current object schema.
- Make the compact schema explicit and documented so model callers can rely on it.
- Keep `source_extract_symbols` as the legacy symbol-only view unless a later PRD removes it.
- Keep source-analysis ownership in `themion-core` tool contracts and `themion-cli/src/source_analysis.rs` runtime extraction.

## Non-goals

- Do not build a whole-project source graph or index.
- Do not add semantic reference search, call graphs, or type resolution.
- Do not remove `source_extract_symbols` in this PRD.
- Do not make compact output browser-owned, TUI-owned, or prompt-postprocessed outside the source-analysis tool path.
- Do not add permanent duplicate parameter names for the same concept.
- Do not change file reading, language detection, parser download, import extraction, or output bounds except where needed for detail projection.

## Background & Motivation

### Current state

PRD-113 made `source_outline` the preferred one-file source-analysis tool. The current result is graph-ready. It includes a file node, symbol objects with IDs and span objects, import objects with IDs and resolution fields, and simple edges.

That shape is useful for graph work, but it is verbose for common navigation. A test on `crates/themion-cli/src/tui.rs` showed a source file around 94 KB producing a current full JSON outline around 71 KB. Much of that size comes from repeated `id` and `parent_id` strings, span object keys, edge objects, and unresolved import metadata.

Most model calls only need a quick answer to “what is in this file and where is it?” For those calls, a compact outline can preserve the useful facts while cutting result size sharply.

## Design

### 1. Add one detail parameter to `source_outline`

Add an optional `detail` parameter to `source_outline`.

Recommended tool schema:

```json
{
  "name": "source_outline",
  "description": "Detect language and return a bounded one-file outline. Use detail=normal for compact navigation or full for graph-ready IDs and edges.",
  "parameters": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "Source file path to analyze."
      },
      "detail": {
        "type": "string",
        "enum": ["normal", "full"],
        "description": "Output detail. Default: full for compatibility. Use normal for compact navigation."
      }
    },
    "required": ["path"]
  }
}
```

Required behavior:

- accept `detail: "normal"` for compact model-facing output
- accept `detail: "full"` for the current graph-ready object output
- reject unknown detail values with a clear tool error
- avoid a second permanent alias such as both `level` and `detail`
- keep the internal extractor producing one complete outline, then project it into the requested detail shape

The preferred parameter name is `detail` because it describes result shape without implying parser precision.

### 2. Make `normal` the compact model-facing shape

`normal` should keep the fields most useful for navigation while dropping high-repeat graph metadata.

Required `normal` shape:

```json
{
  "language": "rust",
  "path": "crates/themion-cli/src/tui.rs",
  "detail": "normal",
  "symbols": [
    ["function", "build_lines", [1900, 61000, 2050, 67000], null],
    ["method", "push", [1080, 34000, 1090, 34500], "App"]
  ],
  "imports": [
    ["std::collections::VecDeque", 21]
  ],
  "parse_error": "parse reported 1 error(s)",
  "warnings": []
}
```

Required field rules:

- `language`: same detected language string as full output
- `path`: same project-relative path as full output
- `detail`: the string `normal`
- `symbols`: array rows shaped as `[kind, name, span, parent_name]`
- symbol `span`: array shaped as `[start_line, start_byte, end_line, end_byte]`
- `parent_name`: string when known; `null` when absent
- `imports`: array rows shaped as `[text, start_line]`
- import `text`: a concise import display string based on the current `module` plus imported items/alias when available
- `parse_error`: omitted when absent
- `warnings`: omitted when empty, or included with existing warning text

`normal` must omit `file`, long IDs, `parent_id`, `edges`, import `resolved`, `resolved_path`, and span object keys.

**Alternative considered:** use named compact objects instead of arrays. Rejected for the normal detail because repeated keys are a major part of the current size problem. The array row contract must be documented and tested to keep it understandable.

### 3. Keep `full` as the compatibility and graph shape

`full` should preserve the current PRD-113 output shape.

Required behavior:

- return the current `SourceOutlineResult` object fields: `language`, `path`, `file`, `symbols`, `imports`, `edges`, optional `parse_error`, and optional `warnings`
- keep ID, parent ID, edge, and import resolution fields unchanged unless a later PRD changes them
- keep the same bounds and truncation warnings from PRD-113
- include `detail: "full"` only if doing so does not break existing full-shape callers; otherwise omit it from full output for compatibility

`full` remains the right mode for graph experiments, edge validation, future Project Memory/source-graph work, and tests that need exact IDs.

### 4. Default behavior and compatibility

This PRD should ship without changing the behavior of existing `source_outline(path)` calls. Omitted `detail` must mean `full` in this implementation slice.

Required behavior:

- omitted `detail` returns the current full PRD-113 object shape
- `detail: "full"` returns the same full shape explicitly
- `detail: "normal"` returns the compact model-facing shape
- tool guidance should tell agents to pass `detail: "normal"` for ordinary navigation
- tests must prove omitted `detail` and explicit `full` remain compatible with the current full output
- if a later release wants compact output as the default, that default flip needs a separate compatibility decision or PRD update

This keeps PRD-116 implementation-ready now. It delivers compact output for callers that ask for it and avoids a silent schema change for existing callers.

### 5. Keep extraction shared and projection-only

The compact output should not create a second parser path.

Required behavior:

- keep one internal extraction path that reads the file, detects language, parses with `tree-sitter-language-pack`, builds symbols/imports/edges, and applies bounds
- project the extracted outline into `normal` or `full` at serialization time
- do not duplicate symbol flattening or import extraction logic for each detail mode
- keep `source_extract_symbols` projecting from the same internal outline path
- keep hard errors such as unreadable files and undetected languages on the normal tool-error path

### 6. Preserve output bounds and warnings

Compact output should reduce bytes, not hide important truncation or parse status.

Required behavior:

- keep PRD-113 bounds: max symbols 500, max imports 200, max edges 1000 for full output
- apply the same symbol and import limits to normal output
- keep warnings such as `symbols truncated at 500` and `imports truncated at 200`
- omit empty `warnings` in both detail modes when practical
- preserve line and byte units exactly: lines and bytes use the same units as current `SourceSymbolSpan`

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Add the optional `detail` parameter to the `source_outline` schema and define serializable compact result/row shapes if kept in core. |
| `crates/themion-cli/src/source_analysis.rs` | Parse `detail`, keep shared full extraction, and project full outline into compact `normal` rows when requested. |
| `crates/themion-cli/src/source_analysis.rs` tests | Add coverage for `normal` shape, `full` compatibility shape, default behavior, invalid detail errors, warning preservation, and compact size on a representative file. |
| `docs/prd/prd-113-source-outline-tool.md` | After implementation, add a short note that PRD-116 adds compact detail levels while preserving PRD-113 full output. |
| `docs/README.md` | List this PRD and update status/version/scope when implementation lands. |

## Edge Cases

- caller omits `detail` → verify: output remains the current full PRD-113 shape.
- caller passes an unknown detail value → verify: the tool returns a clear invalid-detail error.
- source file has no symbols or imports → verify: `normal` returns empty arrays without failing.
- symbol has no parent → verify: parent slot is `null`, not an invented empty parent.
- import has items or alias → verify: compact import text is concise but still identifies what the file imports.
- outline truncates symbols/imports → verify: `normal` keeps the same truncation warnings as `full`.
- parse errors occur → verify: compact output still includes bounded `parse_error`.
- graph user needs IDs or edges → verify: `detail: "full"` returns the current graph-ready fields.

## Migration

This is an additive tool-contract change. Omitted `detail` remains compatible with the PRD-113 full shape. Existing callers do not need to change.

New model calls should pass `detail: "normal"` for ordinary navigation once implemented. Callers that need graph-ready IDs and edges should pass `detail: "full"` or omit `detail`.

No database migration is required.

## Testing

- inspect tool schemas → verify: `source_outline` exposes one optional `detail` parameter with `normal` and `full` values.
- call `source_outline` with `detail: "normal"` on a Rust file → verify: output uses compact symbol and import arrays with language, path, and detail.
- call `source_outline` with `detail: "full"` on the same file → verify: output keeps file, IDs, parent IDs, imports, and edges from the current full shape.
- call `source_outline` without `detail` → verify: output matches the current full PRD-113 shape.
- call `source_outline` with an invalid detail value → verify: the tool fails with a clear error.
- parse a file with nested symbols → verify: compact rows preserve parent names.
- parse a file with imports → verify: compact import rows preserve useful import text and start line.
- force or simulate truncation → verify: compact output includes the same warnings as full output.
- compare serialized sizes on `crates/themion-cli/src/tui.rs` or another large file → verify: `normal` is materially smaller than `full` while preserving navigation facts.
- run `cargo check -p themion-core` → verify: core tool contract compiles.
- run `cargo check -p themion-core --all-features` → verify: all-feature core build compiles.
- run `cargo test -p themion-cli source_analysis` → verify: source-outline projection tests pass.
- run `cargo check -p themion-cli` → verify: default CLI build compiles.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build compiles.

## Implementation checklist

- [x] add the optional `detail` parameter to the `source_outline` tool schema
- [x] define compact `normal` result and row serialization
- [x] parse and validate `detail` in the source-analysis request handler
- [x] keep full extraction shared and add projection from full outline to normal rows
- [x] preserve full output compatibility through `detail: "full"`
- [x] keep omitted `detail` compatible with the current full PRD-113 shape
- [x] add focused tests for normal, full, default, invalid detail, nested parent names, imports, warnings, and size reduction
- [x] update PRD-113 implementation notes and docs index after implementation lands

## Implementation Notes

Implemented in v0.72.0. `source_outline` now accepts optional `detail` with `normal` and `full` values. Omitted `detail` and explicit `detail: "full"` keep the PRD-113 full shape for compatibility. `detail: "normal"` returns compact array rows for symbols and imports, includes `detail: "normal"`, preserves parse errors and warnings, and omits file nodes, IDs, parent IDs, edges, and import resolution fields.

Validation run for this slice:

- `cargo test -p themion-cli source_analysis`
- `cargo check -p themion-core -p themion-cli` before the version bump
- `cargo check -p themion-core -p themion-cli` after the version bump
