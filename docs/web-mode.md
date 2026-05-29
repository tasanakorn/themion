# Web Mode

Themion web mode starts from `themion-cli --web`. It is a browser I/O surface over the CLI-owned runtime and active project directory.

## Project workspace

The Workspace tab provides an IDE-like local project view:

- directory tree rooted at the active project directory
- CodeMirror 6 editor for text/code files
- JetBrains Mono Nerd Font as the default editor font
- safe previews for image, video, Markdown, HTML, and PDF files
- stale-save conflict detection before writing edited files

The editor and preview UI live in `crates/themion-cli-web-ui`. Filesystem policy and file access live in the `themion-cli --web` server.

## Workspace APIs

The browser uses project-relative paths only:

- `GET /api/workspace/tree?path=<path>`
- `GET /api/workspace/file?path=<path>`
- `PUT /api/workspace/file`
- `GET /api/workspace/meta?path=<path>`
- `GET /api/workspace/preview?path=<path>`

The server rejects absolute paths, `..` traversal, and symlink escapes outside the active project root.

## Limits and safety

- Editable text files are limited to 1 MiB.
- Preview files are limited to 50 MiB.
- Saves require the conflict token returned by the file read endpoint.
- Markdown previews are rendered as escaped/sanitized HTML.
- HTML previews are displayed in a sandboxed iframe in the browser UI.
- Unsupported files are not opened in the editor.
