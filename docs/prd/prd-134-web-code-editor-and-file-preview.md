# PRD-134: Web Code Editor and File Preview

- **Status:** Implemented
- **Version:** v0.81.0
- **Scope:** `themion-cli --web`, `themion-cli-web-ui`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-29

## Summary

- Add an IDE-like code workspace to the `themion-cli --web` browser UI.
- Let the browser explore the active project directory, open files, edit text/code, and save changes through the CLI-owned web server.
- Add syntax highlighting for common source files and safe previews for images, video, Markdown, HTML, and PDF files.
- Use CodeMirror 6 as the target browser editor package, packaged with the `themion-cli-web-ui` assets.
- Keep runtime and filesystem policy in `themion-cli`; the browser UI sends intents and renders results.

## Goals

- Provide a browser file explorer rooted at the active Themion project directory.
- Support opening, editing, and saving text/code files from the web UI.
- Provide an IDE-like layout with a directory tree, editor tabs or file switcher, and a preview panel.
- Add syntax highlighting for common languages without requiring the model/tool layer to own UI behavior.
- Preview common non-code files: images, video, Markdown, HTML, and PDF.
- Protect user files with path confinement, binary/text detection, file-size limits, and conflict detection.
- Keep the feature under `themion-cli --web`, not the deprecated `crates/themion-web` product path.

## Non-goals

- Do not add cloud-hosted editing or multi-user collaboration.
- Do not make the browser UI the runtime owner for agents, sessions, tools, or filesystem policy.
- Do not replace the existing model-facing file tools.
- Do not implement a full VS Code extension host, language server protocol, debugger, or terminal in this PRD.
- Do not allow arbitrary filesystem browsing outside the active project unless a later explicit setting allows it.
- Do not execute previewed HTML or Markdown scripts with project privileges.

## Background & Motivation

Themion web currently acts as another local I/O surface over the CLI-owned runtime. It can show runtime status and agent interaction, but it does not yet provide a first-class project file workspace.

Users often need to inspect files, edit code, and preview assets while talking to agents. A web code editor keeps that loop in one browser surface. It should behave like a lightweight IDE while respecting Themion's architecture: `themion-cli` owns local config, project resolution, filesystem policy, and web serving; the browser renders state and sends user intents.

This PRD extends PRD-106's web direction. New browser product work belongs under `themion-cli --web`, not `crates/themion-web`.

## Readiness Decisions

These decisions make this PRD ready to implement:

- target CodeMirror 6 for the first editor implementation
- use the existing embedded JetBrains Mono Nerd Font as the default coding font
- keep all filesystem APIs server-side in `themion-cli --web`
- keep browser editor state in `themion-cli-web-ui`, not in Themion runtime or history storage
- use project-relative paths in API contracts and validate every path against the active project root
- require stale-save conflict detection in the first implementation slice
- defer file watching, Monaco, LSP, debugger, and multi-user editing to later PRDs

## Design

### 1. Project-scoped file explorer

The web UI must show a navigable directory tree rooted at the active project directory.

Required behavior:

- list directories and files under the active project root
- sort directories before files, with stable name ordering
- show useful file metadata such as size, modified time, and file kind when available
- hide or collapse large/generated directories by default when practical, such as `target/`, `.git/`, and dependency caches
- never follow paths outside the project root through `..`, symlinks, encoded path traversal, or absolute-path input
- return clear errors for missing files, permission errors, and unsupported file types

### 2. Text/code editing

The editor must support safe text-file editing from the browser.

Required behavior:

- open UTF-8 text files and common source files in an editor pane
- preserve line endings where practical
- save file changes through a server-side write endpoint that validates path scope again
- use a file revision marker, content hash, mtime, or equivalent value to detect stale saves
- reject a save when the file changed on disk after the browser opened it, unless the user explicitly reloads or confirms overwrite
- provide clear dirty-state, save-success, and save-error UI feedback
- apply maximum file-size limits for editor open and save operations to avoid loading very large files into the browser by accident

The editor component must remain a UI dependency under the web surface. It must not move filesystem policy into browser-only code.

### 3. Target editor package and packaging

Use CodeMirror 6 as the target editor component for the first implementation.

Required behavior:

- integrate CodeMirror 6 into the existing `themion-cli-web-ui` browser surface
- keep CodeMirror and its language/theme packages as browser UI dependencies, not `themion-core` dependencies
- package the editor assets through the existing `themion-cli` web asset build and embedding flow so the installed `themion` binary can serve the editor without requiring a CDN at runtime
- include only the language packages needed for the first slice: Rust, TOML, Markdown, JSON, YAML, HTML, CSS, JavaScript/TypeScript, shell-like scripts, and plain text fallback
- keep editor state, selections, scroll positions, and unsaved buffers in browser UI state
- expose file open/save through Themion file APIs rather than direct browser filesystem access
- use the already packaged JetBrains Mono Nerd Font as the default editor font, because it is a strong coding font and is already embedded in the `themion-cli --web` asset pipeline

Monaco is not the first target because it brings larger worker and asset packaging needs. A later PRD may switch to Monaco or add LSP support if Themion needs richer IDE behavior.

### 4. Syntax highlighting

The editor should highlight common source formats.

Required behavior:

- choose syntax highlighting from file extension, filename, or detected language
- support common Themion project file types first: Rust, TOML, Markdown, JSON, YAML, HTML, CSS, JavaScript/TypeScript, shell scripts, and plain text
- fall back to plain text when a language is unknown
- keep highlighting client-side unless server-side highlighting is clearly simpler and does not leak runtime ownership into the UI layer

### 5. File preview panel

The web UI should preview supported non-code files without requiring a download first.

Required behavior:

- image preview for common browser-supported image formats
- video preview for browser-supported video formats
- Markdown preview rendered from sanitized Markdown
- HTML preview in a sandboxed frame or sanitized preview mode
- PDF preview using the browser's built-in PDF rendering when available
- provide a download/open-raw option for files that cannot be edited or previewed safely
- enforce size and MIME/type checks before serving previews

Preview safety rules:

- do not execute project HTML or Markdown scripts in the main Themion page context
- use sandboxing, content security policy headers, or sanitized rendering for active content
- serve binary preview data only after project-root path validation

### 6. Web API and realtime updates

The CLI-owned web server should expose small file APIs for the browser workspace.

Required API shape:

- `GET /api/workspace/tree?path=<project-relative-path>` lists one directory
- `GET /api/workspace/file?path=<project-relative-path>` returns editable text content plus a conflict token
- `PUT /api/workspace/file` saves text content with `path`, `content`, and the conflict token from the read response
- `GET /api/workspace/meta?path=<project-relative-path>` returns metadata and detected kind
- `GET /api/workspace/preview?path=<project-relative-path>` streams or serves preview bytes for supported binary/media files
- optionally watch project file changes and notify the browser over the existing shared websocket

Required behavior:

- all file API handlers live in `themion-cli --web` or a small CLI-owned helper module
- API responses use project-relative paths, not absolute paths, unless a debug-only field is needed
- errors are structured enough for the UI to show a useful message
- file watching is optional for the first slice; stale-save detection is required
- all endpoints reject absolute paths, parent traversal, and symlink escape outside the active project root

Recommended response fields:

- `path`: project-relative path
- `kind`: `directory`, `text`, `image`, `video`, `markdown`, `html`, `pdf`, `binary`, or `unsupported`
- `size_bytes`
- `modified_at_ms`: UTC epoch milliseconds when available
- `conflict_token`: opaque string for editable file reads
- `content`: UTF-8 text content only for editable text reads
- `error`: structured code and short message for failures

### 7. UI integration

The editor should fit into the existing web UI instead of becoming a separate product.

Required behavior:

- add a clear route or panel for the workspace/editor
- preserve existing agent/runtime views
- let users switch between chat/runtime views and file workspace views without losing unsaved edits unexpectedly
- make editor state local to the browser surface; do not store unsaved buffers in the agent runtime database
- if browser refresh loses unsaved changes, warn before unload when practical

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-134-web-code-editor-and-file-preview.md` | Add this PRD. |
| `crates/themion-cli/src/web.rs` and web helpers | Add project-scoped file APIs for directory listing, text read/save, metadata, and previews. |
| `crates/themion-cli` browser UI assets | Add workspace route/panel, file tree, editor, syntax highlighting, dirty-state handling, and preview panel. |
| `crates/themion-cli-web-ui` | Integrate CodeMirror 6 and the selected language packages as browser UI dependencies; use the embedded JetBrains Mono Nerd Font as the default editor font. |
| web asset build pipeline | Bundle and embed CodeMirror/editor assets in the existing `themion-cli --web` asset build output; do not depend on CDN loading. |
| docs for web mode | Document file workspace behavior, path scope, size limits, and preview safety rules after implementation. |
| `docs/README.md` | Add this PRD entry and later update implementation status. |

## Edge Cases

- browser requests `../Cargo.toml` or an absolute path → verify: server rejects the path.
- a symlink inside the project points outside the project → verify: server rejects access outside the root.
- file changes on disk after the browser opens it → verify: save is rejected with a conflict message.
- user opens a large binary file → verify: editor refuses it and offers preview/download only when safe.
- Markdown contains scripts or unsafe HTML → verify: preview sanitizes or blocks active content.
- HTML preview contains scripts → verify: it runs only in a sandboxed context or is blocked.
- unsupported media type → verify: UI shows metadata and raw/download options, not a broken editor.
- project file is deleted while open → verify: reload/save reports a clear missing-file error.

## Migration

No database migration is expected.

This is an additive web UI feature. Existing TUI, headless, agent runtime, model-facing file tools, and persistent history behavior should continue unchanged.

## Testing

- open `themion-cli --web` and navigate to the workspace view → verify: the active project directory tree is shown.
- open a Rust file → verify: text loads, syntax highlighting appears, and line endings are preserved on save.
- edit and save a text file → verify: file content changes on disk and UI reports success.
- modify the same file outside the browser before saving → verify: browser save is rejected as stale.
- request a path outside the project root → verify: API rejects it.
- preview image, video, Markdown, HTML, and PDF samples → verify: each supported type renders in the preview panel or fails safely with a clear message.
- preview unsafe Markdown or HTML → verify: scripts do not execute in the main Themion page context.
- open an unsupported binary file → verify: editor does not corrupt or display binary as text.
- run `cargo check -p themion-cli` → verify: default CLI build passes.
- run `cargo check -p themion-cli --all-features` → verify: web and feature combinations compile.

## Implementation checklist

- [ ] Add project-scoped file API types and path validation helpers.
- [ ] Add directory listing, text read, text save, metadata, and preview endpoints.
- [ ] Add stale-save conflict detection.
- [ ] Add CodeMirror 6 packaging under `themion-cli-web-ui` and use the embedded JetBrains Mono Nerd Font in editor styling.
- [ ] Add workspace route/panel, file tree, CodeMirror editor, syntax highlighting, and dirty-state UI.
- [ ] Add image, video, Markdown, HTML, and PDF preview support with safety controls.
- [ ] Add file-size and binary/text handling limits.
- [ ] Update web-mode docs and run default/all-features CLI validation.
