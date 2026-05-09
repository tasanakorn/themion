# PRD-113: Migrate Source Symbols to Source Outline

- **Status:** Implemented
- **Version:** v0.71.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-09

## Summary

- Replace the narrow `source_extract_symbols` direction with a more useful `source_outline` tool.
- Keep the same one-file, auto-detect, Tree-sitter-backed workflow from PRD-112.
- Add graph-ready IDs, file metadata, imports, and simple edges so agents can trace app structure faster.
- Keep the first outline result bounded and syntax-based; do not claim full semantic resolution.
- Preserve compatibility by making `source_extract_symbols` a legacy symbol-only view during migration.

## Goals

- Give agents a basic code-analysis tool that is faster than reading whole files.
- Support common app-analysis and trace tasks: “what is in this file?”, “what does it import?”, and “what contains this symbol?”.
- Promote the product concept from symbol extraction to file outline extraction.
- Keep runtime ownership in `themion-cli` and the model-facing contract in `themion-core`.
- Reuse `tree-sitter-language-pack` and the current language detection/download behavior.
- Return graph-ready nodes and edges that can later feed a project source graph or Project Memory KG.

## Non-goals

- Do not build a whole-project index in this PRD.
- Do not add cross-file call graphs, reference search, or precise type resolution.
- Do not require language-specific semantic analyzers.
- Do not store outlines in Project Memory or a dedicated source graph yet.
- Do not move parser, import extraction, or outline policy into `tui.rs` or web UI code.
- Do not remove `source_extract_symbols` abruptly while agents may still call it.

## Background & Motivation

### Current state

PRD-112 added the first Tree-sitter-backed source tool: `source_extract_symbols`. It detects a language from one path, obtains a parser through `tree-sitter-language-pack`, parses one file, and returns a normalized symbol list.

The current runtime path lives in `crates/themion-cli/src/source_analysis.rs`. It reads a project-relative file, detects language from path or extension, calls `tree_sitter_language_pack::process`, flattens `StructureItem` values into `SourceExtractedSymbol`, and reports parse recovery through `parse_error` when `processed.metrics.error_count > 0`. The language-pack process result already includes `imports: Vec<ImportInfo>` when `ProcessConfig::new(language)` is used, so the outline should normalize that existing import data before adding custom language-specific import parsers.

That is useful, but app analysis often needs one more level of structure. Agents need to see the file node, imports, containment, and stable IDs before they can trace how code connects. A pure symbol list still pushes the agent back to raw file reads for import and relationship clues.

The next step should stay small. A one-file outline gives enough structure for faster navigation without committing to a full Graphify-style project graph.

## Design

### 1. Add `source_outline` as the primary one-file source tool

`source_outline` should be the new canonical one-file analysis tool.

Tool schema:

```json
{
  "name": "source_outline",
  "description": "Detect language and return a bounded one-file outline with symbols, imports, and simple edges.",
  "parameters": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "Source file path to analyze."
      }
    },
    "required": ["path"]
  }
}
```

The tool intentionally has one required input. Do not add `language`, `include_calls`, `project_dir`, or mode switches in this slice. The current project directory remains the tool execution scope, matching `source_extract_symbols`.

### 2. Define the outline result shape

The result should be small, stable, and graph-ready.

```json
{
  "language": "rust",
  "path": "crates/themion-cli/src/source_analysis.rs",
  "file": {
    "id": "file:crates/themion-cli/src/source_analysis.rs",
    "kind": "file",
    "path": "crates/themion-cli/src/source_analysis.rs"
  },
  "symbols": [
    {
      "id": "symbol:crates/themion-cli/src/source_analysis.rs:function:source_extract_symbols:20:0",
      "name": "source_extract_symbols",
      "kind": "function",
      "parent_id": null,
      "parent_name": null,
      "span": {
        "start_line": 20,
        "start_byte": 600,
        "end_line": 55,
        "end_byte": 1800
      }
    }
  ],
  "imports": [
    {
      "id": "import:crates/themion-cli/src/source_analysis.rs:0:0",
      "module": "std::fs",
      "items": [],
      "alias": null,
      "is_wildcard": false,
      "span": {
        "start_line": 2,
        "start_byte": 35,
        "end_line": 2,
        "end_byte": 47
      },
      "resolved": false,
      "resolved_path": null
    }
  ],
  "edges": [
    {
      "from": "file:crates/themion-cli/src/source_analysis.rs",
      "to": "symbol:crates/themion-cli/src/source_analysis.rs:function:source_extract_symbols:20:0",
      "relation": "contains",
      "confidence": "extracted"
    },
    {
      "from": "file:crates/themion-cli/src/source_analysis.rs",
      "to": "import:crates/themion-cli/src/source_analysis.rs:0:0",
      "relation": "imports",
      "confidence": "extracted"
    }
  ],
  "parse_error": null,
  "warnings": []
}
```

Required top-level fields:

- `language`: detected language name from the language pack result
- `path`: analyzed project-relative path
- `file`: graph-ready file node
- `symbols`: normalized declaration list
- `imports`: normalized import/include/use/require list
- `edges`: simple graph-ready relationships
- `warnings`: bounded list of extractor warnings; empty when none

Optional top-level fields:

- `parse_error`: omitted when no parse error exists; bounded text when parsing reports errors

Optional object fields such as `alias`, `resolved_path`, `parent_id`, and `parent_name` should be omitted when absent. Examples may show `null` only to make optional fields visible during review.

The tool must not return raw syntax-tree dumps or unbounded source snippets.

### 3. Define shared result structs

`themion-core` should own serializable result types. Names may vary, but the shape should be equivalent to:

```rust
pub struct SourceOutlineResult {
    pub language: String,
    pub path: String,
    pub file: SourceOutlineFile,
    pub symbols: Vec<SourceOutlineSymbol>,
    pub imports: Vec<SourceOutlineImport>,
    pub edges: Vec<SourceOutlineEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub struct SourceOutlineFile {
    pub id: String,
    pub kind: String,
    pub path: String,
}

pub struct SourceOutlineSymbol {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_name: Option<String>,
    pub span: SourceSymbolSpan,
}

pub struct SourceOutlineImport {
    pub id: String,
    pub module: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub is_wildcard: bool,
    pub span: SourceSymbolSpan,
    pub resolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,
}

pub struct SourceOutlineEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
    pub confidence: String,
}
```

Use the existing `SourceSymbolSpan` shape for spans. Keep line and byte units unchanged from PRD-112/current code.

### 4. Keep `source_extract_symbols` as a compatibility view

The product should migrate agents toward `source_outline`, but existing callers should not break immediately.

Required behavior:

- `source_extract_symbols` remains available during the migration slice.
- It should call the same runtime outline extractor where practical.
- It should project `SourceOutlineSymbol` back into the legacy `SourceExtractedSymbol` shape by dropping `id` and `parent_id`.
- It should return only the legacy shape: `language`, `path`, `symbols`, and optional `parse_error`.
- Its tool description should say it is a symbol-only view and `source_outline` is preferred for app analysis and tracing.

Compatibility projection logic:

```text
source_extract_symbols(path):
  outline = source_outline_internal(path)
  return {
    language: outline.language,
    path: outline.path,
    symbols: outline.symbols.map(drop id, parent_id),
    parse_error: outline.parse_error
  }
```

**Alternative considered:** remove `source_extract_symbols` immediately. Rejected because the tool is already shipped in v0.70.0 and a compatibility view costs little while agents learn the better tool.

### 5. Use graph-ready IDs without promising full semantic resolution

Outline IDs should be stable enough for one-file graph edges and future indexing.

Required ID rules:

- Use project-relative paths, not absolute machine-local paths.
- Include enough disambiguation for duplicate names in one file.
- Prefer stable syntax facts: path, kind, normalized name, start line, and start byte.
- Do not depend on allocation order alone for symbol IDs.
- Edges must reference IDs that appear in `file`, `symbols`, or `imports`.
- Hard failures such as unreadable files or unsupported language detection should keep the current normal tool-error behavior rather than inventing a second JSON error result shape.

Expected ID style:

```text
file:<path>
symbol:<path>:<kind>:<name>:<start_line>:<start_byte>
import:<path>:<start_line>:<start_byte>
```

Edges should represent syntax facts only unless deterministically resolved:

- `contains`: file or container symbol contains a symbol
- `imports`: file imports a module/path/name

The first slice may include unresolved imports as import records. It must mark unresolved or ambiguous details instead of pretending they are exact cross-file links.

### 6. Normalize language-pack imports as a first-class field

Imports are important for tracing app flow. The outline should use import data already produced by `tree-sitter-language-pack` before adding any custom syntax extraction.

`ProcessConfig::new(language)` enables import extraction by default. The runtime should map `processed.imports: Vec<ImportInfo>` into `SourceOutlineImport` records.

Language-pack import shape:

```rust
pub struct ImportInfo {
    pub source: String,
    pub items: Vec<String>,
    pub alias: Option<String>,
    pub is_wildcard: bool,
    pub span: Span,
}
```

Required outline import fields:

- `id`
- `module`: normalized from `ImportInfo.source`
- `items`: copied from `ImportInfo.items`; empty when the language pack does not split imported names
- optional `alias`: copied from `ImportInfo.alias`
- `is_wildcard`: copied from `ImportInfo.is_wildcard`
- `span`: copied from `ImportInfo.span` using the same line/byte units as symbols
- `resolved`: boolean, default false for this one-file slice
- optional `resolved_path`: present only when the runtime can resolve it confidently without project indexing

Do not write custom Rust/JS/Python import parsers in the first implementation unless the language-pack data is unusable for a required test case. If `processed.imports` is empty, return an empty import list without warning by default. Add a warning only for actual extractor limits or truncation.

### 7. Define runtime extraction logic

The shared internal function should produce `SourceOutlineResult` once. Both public tools should use it.

Required logic:

```text
source_outline_internal(project_dir, path_arg):
  1. resolve path_arg under project_dir using the existing source-analysis path behavior
  2. read the file as UTF-8 text
  3. detect language from path, then extension fallback, matching current behavior
  4. process source through tree_sitter_language_pack::process
  5. flatten processed.structure into outline symbols
  6. assign symbol IDs and parent IDs while preserving parent_name
  7. add file -> symbol contains edges for top-level symbols
  8. add parent symbol -> child symbol contains edges for nested symbols
  9. map processed.imports into outline imports
  10. add file -> import imports edges
  11. set parse_error when processed.metrics.error_count > 0
  12. apply output bounds and warnings
```

The symbol flattening logic should preserve the current behavior where a child without a named direct parent inherits the nearest named container. When a named child is emitted, nested emitted children should use that child's ID as `parent_id`.

Output bounds should be deterministic:

- max symbols: 500
- max imports: 200
- max edges: 1000

If truncation is needed, keep earlier source-order items and add warning text in the form `symbols truncated at 500`, `imports truncated at 200`, or `edges truncated at 1000`.

### 8. Preserve runtime layering

The parser and outline extraction remain runtime-owned.

Required placement:

- `themion-core` defines the model-facing tool schema and shared result structs.
- `themion-cli/src/source_analysis.rs` owns file reading, language detection, parser acquisition, symbol/import extraction, and result shaping.
- TUI and web surfaces may display or call the tool indirectly, but they must not own parser setup or source-analysis policy.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Add `source_outline` schema, result structs, and dispatch. Mark `source_extract_symbols` as the legacy symbol-only view in descriptions. |
| `crates/themion-cli/src/source_analysis.rs` | Refactor current one-file extraction into a shared `source_outline_internal` path. Add import extraction and graph-ready node/edge shaping. |
| `crates/themion-core/tests/` / `crates/themion-cli` tests | Add coverage for `source_outline`, legacy symbol compatibility, parse errors, ID/edge consistency, and bounded unsupported import behavior. |
| `docs/prd/prd-112-native-tree-sitter-language-pack-symbol-extraction.md` | After implementation, add a short note that PRD-113 supersedes the preferred tool name while preserving the PRD-112 parser foundation. |
| `docs/README.md` | List this PRD and update status/version/scope when implementation lands. |

## Edge Cases

- unsupported file extension → verify: the tool returns a clear undetected-language error like the existing symbol tool.
- language-pack returns no imports → verify: symbols still return and imports are empty without failing the whole outline.
- unreadable file or undetected language → verify: the tool uses the normal tool-error path rather than returning a second JSON error shape.
- source file has parse errors → verify: partial symbols/imports may return with bounded `parse_error`.
- nested declarations share names → verify: IDs remain distinct through parent/span data.
- emitted edge references a missing node → verify: tests fail; every edge target/source must be present in file, symbols, or imports.
- import target cannot be resolved → verify: `resolved` is false and no fake `resolved_path` is emitted.
- very large file → verify: output is bounded and reports truncation or limits through `warnings` when needed.
- caller still uses `source_extract_symbols` → verify: the tool works and returns the old shape from the shared extractor.

## Migration

This is an additive migration first.

- New agents should prefer `source_outline` for one-file app analysis and trace setup.
- `source_extract_symbols` remains available as a compatibility view.
- Later PRDs may deprecate or remove `source_extract_symbols` after `source_outline` is widely used.
- No database migration is required because the outline is not stored in this PRD.

The release target is `v0.71.0`. This is a minor release because it adds a new user-visible source-analysis tool and changes the preferred model-facing workflow.

## Testing

- inspect tool schemas → verify: `source_outline` exists with only `path`, and `source_extract_symbols` still exists as a compatibility view.
- call `source_outline` on a known Rust file → verify: output includes language, file node, symbols, contains edges, and parse status.
- call `source_outline` on a file where `tree-sitter-language-pack` returns imports → verify: imports are represented with module/source, items, alias/wildcard data, spans, and unresolved status.
- call `source_extract_symbols` on the same file → verify: it still returns the legacy symbol-only shape from the shared extractor.
- inspect serialized optional fields → verify: absent optional values are omitted, not serialized as null.
- parse a file with nested declarations → verify: symbol parent data, parent IDs, and contains edges distinguish nested symbols.
- check every edge in an outline result → verify: `from` and `to` IDs refer to returned file, symbol, or import IDs.
- parse a file with syntax errors → verify: partial outline behavior is bounded and `parse_error` is present.
- request an unsupported file → verify: the error is clear and model-usable.
- run `cargo check -p themion-core` → verify: core tool contract compiles.
- run `cargo check -p themion-core --all-features` → verify: all-feature core build compiles.
- run `cargo check -p themion-cli` → verify: runtime integration compiles.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build compiles.

## Implementation Notes

Implemented in v0.71.0. The landed slice adds the `source_outline` tool contract and shared result structs in `themion-core`, refactors `themion-cli` source analysis around a shared one-file outline extractor, maps `tree-sitter-language-pack` `ImportInfo` into outline imports, and keeps `source_extract_symbols` as a symbol-only compatibility view over the same extractor.

The implemented outline returns graph-ready file, symbol, import, and edge records. It applies the PRD bounds of 500 symbols, 200 imports, and 1000 edges, omits absent optional fields, and keeps existing tool-error behavior for hard failures such as unreadable files or undetected languages.

Validation run for this slice:

- `cargo test -p themion-cli source_analysis`
- `cargo check -p themion-cli`
- `cargo check -p themion-core --all-features`
- `cargo check -p themion-cli --all-features`

## Implementation checklist

- [x] add `source_outline` tool schema and shared result structs
- [x] refactor one-file source analysis into a shared outline extractor
- [x] add graph-ready file/symbol/import/edge output
- [x] keep `source_extract_symbols` as a compatibility view
- [x] map `tree-sitter-language-pack` `ImportInfo` into outline import records
- [x] add deterministic ID, bounds, optional-field omission, and edge consistency tests
- [x] add tests for outline output and legacy compatibility
- [x] update PRD/docs status notes after implementation lands
