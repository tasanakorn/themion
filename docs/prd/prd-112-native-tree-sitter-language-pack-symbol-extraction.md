# PRD-112: Auto-Detect, Auto-Download, and Extract Symbols with Native Tree-sitter Language Pack

- **Status:** Partially implemented
- **Version:** v0.70.0
- **Scope:** `themion-core`, `themion-cli`, docs, experiments
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-09

## Summary

- Themion should use Rust-native `tree-sitter-language-pack` instead of building its own wasm grammar runtime.
- The first slice should auto-detect language from file path, auto-download the needed grammar, parse one file, and return one simple symbol result.
- Keep parser, cache, and download ownership in runtime code, not in `tui.rs` or web UI code.
- Start with one model-facing tool for extracting symbols from one file.
- Defer project-wide indexing, graph extraction, and custom grammar-runtime work.

## Goals

- Add one runtime-owned Tree-sitter facility that uses `tree-sitter-language-pack` from Rust.
- Support automatic language detection from file path or extension.
- Support automatic grammar download through the language pack when the detected language is not cached yet.
- Provide one simple model-facing tool that extracts symbols from one file.
- Keep the result small, structured, and useful for common code-navigation tasks.
- Preserve repository layering: runtime behavior in `themion-cli`, tool contract in `themion-core`, no parser ownership in TUI.

## Non-goals

- Do not implement a custom Themion wasm parser runtime.
- Do not manage raw `WasmStore` or standalone grammar `.wasm` files in Themion-owned runtime code.
- Do not add project-wide indexing, reference search, call graphs, or graph database extraction in this PRD.
- Do not add many overlapping Tree-sitter tools in the first slice.
- Do not move parser, cache, or download decisions into `tui.rs` or web UI code.
- Do not require every language to be preloaded at startup.

## Background & Motivation

### Current state

Themion still lacks a simple structure-aware code tool for common coding tasks such as "show me the symbols in this file". Today the model often falls back to linear file reading even when a small symbol outline would be enough.

The earlier PRD-112 draft aimed at a wasm-grammar loading facility. The experiment work showed a simpler path. `tree-sitter-language-pack` already provides Rust-native language detection, parser creation, and on-demand grammar download. In the local experiment it successfully downloaded Rust, cached it, and parsed the benchmark files without Themion building its own wasm runtime layer.

That makes the product direction smaller and clearer. Themion does not need to own grammar wasm lifecycle directly for the first useful slice. It only needs a runtime-owned wrapper that auto-detects the language, downloads the grammar when needed, and parses the file.

**Alternative considered:** keep the custom wasm-runtime direction so Themion controls grammar loading directly. Rejected because it adds runtime complexity that the experiment did not justify for the first product slice.

## Design

### 1. Use `tree-sitter-language-pack` as the parser backend

Themion should use the Rust-native language pack as the first parser backend.

Required behavior:

- runtime code uses `tree-sitter-language-pack` to detect language from file path or extension
- runtime code uses `get_parser(language)` to obtain a parser for the detected language
- if the language is not yet available locally, the language pack may auto-download it through its normal supported path
- Themion should treat the language pack cache as the grammar storage mechanism for this feature
- Themion should not add a second custom grammar download/cache layer in the same slice

### 2. Keep runtime ownership in `themion-cli`

The parser facility belongs to runtime code, not UI code.

Required behavior:

- `themion-cli` owns parser access, language detection, file reading, cache/download decisions, and result shaping
- `themion-core` owns only the model-facing tool contract and any shared result types that must cross the boundary
- `tui.rs` and web UI code may invoke the runtime path indirectly, but must not own parser setup, language-pack state, or grammar-download policy
- if the system decides whether to download, reject, or reuse a grammar, that decision belongs in runtime code outside the TUI

### 3. Provide one simple tool: auto-detect, auto-download, and extract symbols from one file

The first user-visible capability should be one bounded tool for one file.

Required behavior:

- input identifies one source file path
- runtime detects the language from the file path or extension
- runtime checks whether the detected language is already available
- runtime obtains a parser through the language pack, including automatic download when needed
- runtime parses the file and extracts a small symbol list
- output is structured and bounded, not a raw syntax-tree dump

The first symbol set should focus on common top-level and nested declarations when the language grammar exposes them clearly, such as:

- modules
- classes / structs / enums / traits / interfaces when relevant
- functions
- methods
- constants or similar named declarations when practical

The first slice may support only a small normalized symbol shape even if language-specific detail exists.

### 4. Normalize the result shape for model use

The tool output should be simple and stable.

Required behavior:

- return the detected language name
- return file path or a normalized file identifier
- return a list of symbols with bounded fields such as name, kind, optional parent/container, and source span
- preserve nesting when practical, either through parent ids or simple child grouping
- avoid presentation-heavy prose in the core result
- avoid language-specific payload explosion in the first slice

A small stable symbol format is preferred over a rich but inconsistent per-language shape.

### 5. Make the first tool contract implementation-ready

The first tool should be named `source_extract_symbols`.

Required input shape:

- `path`: source file path to analyze

Required output shape:

- `language`: detected language name
- `path`: analyzed file path
- `symbols`: list of symbol objects
- optional `parse_error`: bounded error text when parsing fails or only partial recovery is possible

Each symbol object should use one small normalized shape:

- `name`: symbol name
- `kind`: normalized kind such as `module`, `class`, `struct`, `enum`, `trait`, `interface`, `function`, `method`, or `constant`
- optional `parent_name`: nearest named container when useful
- `span`: object with `start_line`, `start_byte`, `end_line`, and `end_byte`

The tool should not require the caller to pass a language name in the first slice. The runtime should detect language from the file path, then decide whether to reuse or download the grammar.

**Alternative considered:** require both `path` and `language` in the first tool contract. Rejected because the experiment showed the language pack can detect from file path, and one canonical input shape is simpler for the model.

### 6. Keep language support additive

The first implementation should be able to start narrow without blocking later expansion.

Required behavior:

- the runtime path should be language-pack based from the start, even if only a few languages are tested first
- unsupported or undetected files must return a clear bounded error
- adding more languages later should not require a new parser architecture
- future follow-up work may add richer extraction rules or more languages without changing the basic tool contract

### 7. Keep the feature intentionally small

This PRD is for symbol extraction only.

Not part of this slice:

- cross-file symbol search
- project-wide indexes
- references
- call graphs
- import graphs
- knowledge-graph relation extraction
- custom Tree-sitter query-pack management beyond what the language pack already provides

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-112-native-tree-sitter-language-pack-symbol-extraction.md` | Define the final PRD direction around auto-detect, auto-download, and one simple symbol-extraction tool backed by Rust-native `tree-sitter-language-pack`. |
| `docs/README.md` | Track PRD-112 as partially implemented in v0.70.0 and keep the scope summary aligned with the landed first slice. |
| `crates/themion-core/src/tools.rs` | Future home for the `source_extract_symbols` tool definition and its model-facing schema. |
| `crates/themion-cli/src/` | Future home for runtime-owned language detection, parser acquisition, file parsing, and symbol extraction. |
| `experiments/prd112-tree-sitter-wasm-grammar/` | Research artifact showing native binding, standalone wasm, and language-pack behavior that informed this PRD rewrite. |

## Edge Cases

- file extension is unknown or missing → verify: the tool returns a clear unsupported-or-undetected-language result.
- language is detected but grammar is not cached locally → verify: the runtime uses the language-pack download path or returns a clear download failure.
- source file has syntax errors → verify: the tool still returns a bounded partial symbol result when parsing recovers, or a clear parse failure when it cannot.
- source file is very large → verify: the tool returns bounded symbol output rather than a full-tree dump.
- language pack supports a language but Themion has not added extraction rules for it yet → verify: the result clearly reports limited or unsupported symbol extraction rather than pretending to be complete.
- UI surfaces request the feature later → verify: they consume the same runtime-owned path rather than creating parser ownership locally.

## Migration

This feature is additive. No user data migration is required.

The earlier draft framing around a Themion-owned wasm runtime is replaced by this PRD direction. Follow-up implementation should not introduce a parallel custom grammar-runtime path unless a later PRD re-justifies it.

When the feature lands, the version should be promoted to a concrete minor release because it adds a new user-visible tool capability.

## Testing

- call language detection on a known source path such as `src/main.rs` → verify: the runtime resolves the language to `rust` or another correct language name.
- parse a file whose language is not cached yet → verify: the runtime can obtain the parser through language-pack download and then parse successfully.
- parse a supported file through the runtime-owned path → verify: `source_extract_symbols` returns a bounded structured symbol result with detected language, path, and normalized symbols.
- parse a file with syntax errors → verify: the result is bounded and clearly indicates partial recovery or parse failure.
- request symbol extraction for an unsupported or undetected file → verify: the failure is clear and model-usable.
- run `cargo check -p themion-core` after implementation starts → verify: the tool contract compiles.
- run `cargo check -p themion-core --all-features` after implementation starts → verify: all-feature core build compiles.
- run `cargo check -p themion-cli` after implementation starts → verify: the runtime integration compiles.
- run `cargo check -p themion-cli --all-features` after implementation starts → verify: all-feature CLI build compiles.

## Implementation Notes

Partially implemented in v0.70.0. The landed first slice adds the `source_extract_symbols` tool contract in `themion-core`, adds runtime-owned execution in `themion-cli`, uses `tree-sitter-language-pack` for path-based language detection and on-demand grammar download, and returns a normalized symbol list for one file. The current implementation uses the language pack's structure extraction as the first symbol source and reports parse recovery through a bounded `parse_error` field when parse errors are present.

Not yet implemented in this slice: richer language-specific symbol normalization, explicit grammar cache/status reporting, broader docs beyond the PRD/docs index status updates, or any cross-file/project-wide analysis features.

PRD-113 supersedes this PRD as the preferred source-analysis tool direction in v0.71.0. The PRD-112 parser foundation remains in use, while `source_outline` is now preferred for one-file app analysis because it adds file metadata, imports, and graph-ready edges. `source_extract_symbols` remains available as a symbol-only compatibility view.

## Implementation checklist

- [x] add `tree-sitter-language-pack` as the supported runtime parser backend
- [x] define the `source_extract_symbols` tool in `themion-core` with the canonical input/output shape
- [x] add runtime-owned language detection from file path in `themion-cli`
- [x] add runtime-owned parser acquisition and on-demand download through the language pack
- [x] extract a bounded normalized symbol list from one parsed file
- [x] define clear unsupported-language, download-failure, and parse-failure behavior
- [ ] document the landed runtime/tool path after implementation
