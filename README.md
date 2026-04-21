# themion

> A terminal AI agent in Rust with a core local runtime, tool use, persistent history, and optional Stylos-powered mesh visibility and delegation.

```
████████╗██╗  ██╗███████╗███╗   ███╗██╗ ██████╗ ███╗   ██╗
╚══██╔══╝██║  ██║██╔════╝████╗ ████║██║██╔═══██╗████╗  ██║
   ██║   ███████║█████╗  ██╔████╔██║██║██║   ██║██╔██╗ ██║
   ██║   ██╔══██║██╔══╝  ██║╚██╔╝██║██║██║   ██║██║╚██╗██║
   ██║   ██║  ██║███████╗██║ ╚═╝ ██║██║╚██████╔╝██║ ╚████║
   ╚═╝   ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝╚═╝ ╚═════╝ ╚═╝  ╚═══╝
```

themion is a Rust-powered AI agent with a full-featured TUI. Give it a task in plain English and it can reason, call tools, inspect your workspace, and produce results directly from your terminal.

Themion keeps its core runtime local: workflows, tool execution, model calls, and persistent history all live in the current process. When built with the optional `stylos` feature, it can also participate in a mesh for presence, discovery, direct queries, talk, and lightweight task delegation across other Themion processes.

> After 0.2.0, themion will use themion to help develop itself.

## What Themion is

- **A local agent runtime** — the main loop, tools, prompt assembly, workflow state, and session history run in-process
- **A terminal-first UI** — Ratatui-based interaction with streaming output, keyboard navigation, and direct shell shortcuts
- **A multi-backend coding agent** — Codex is the recommended default, with support for other OpenAI-compatible backends
- **An optional Stylos consumer** — Stylos extends Themion with mesh visibility and controlled cross-process coordination; it does not replace Themion's execution engine

## Features

- **First-class Codex login** — Sign in with `/login codex` and use your ChatGPT / Codex subscription directly, without managing an API key
- **Full TUI** — Ratatui-powered interface with streaming output, scroll, mouse support, and a braille spinner while thinking
- **Agentic tool use** — Reads files, writes files, lists directories, runs shell commands; loops until done
- **Direct shell shortcut** — Run local commands instantly from the TUI with `!<command>` and see the output in the conversation pane
- **Persistent session history** — SQLite-backed conversation history with windowed context and keyword search
- **Multi-profile support** — Switch between providers and models on the fly with `/config profile use`
- **Flexible backends** — Codex is the recommended default, with OpenRouter and local OpenAI-compatible servers like llama.cpp, Ollama, or LM Studio as alternatives
- **Optional Stylos integration** — When compiled with `--features stylos`, discover other Themion processes, inspect status, send direct talk requests, and delegate lightweight tasks
- **Print mode** — Pipe a single prompt and get a result; perfect for scripting
- **Single binary** — Ships as one statically-linked executable with no runtime dependencies

## Why Stylos in Themion?

Stylos gives Themion a network-facing coordination layer without moving core agent execution out of process.

In Themion, Stylos is used for:

- **presence and discovery** — find live or free agents across the mesh
- **status inspection** — query a specific Themion process for its current agent snapshot
- **direct talk** — send sender-aware peer messages into another Themion agent's normal runtime
- **lightweight task delegation** — enqueue work to another process and poll for status or results

Stylos is **not** Themion's runtime. Themion still owns:

- prompt construction and instruction injection
- workflow and phase handling
- tool execution
- model/provider calls
- persistent session history
- the local TUI and operator experience

What Stylos does **not** do in Themion:

- it does not move core execution out of process
- it does not provide a distributed scheduler or durable remote job system
- it does not replace the local workflow/runtime loop

A useful way to think about it:

- **Themion** = local agent runtime and UX
- **Stylos** = mesh visibility, addressing, and lightweight delegation between Themion processes

## Stylos integration model

When Stylos is enabled, Themion exposes one Stylos session per process and the status/query model supports multiple in-process agents through one process snapshot. In the first shipped step, Themion still usually boots one main interactive agent, but the reporting and query model already supports multiple agents per process.

Key ideas:

- **one process, one Stylos session**
- **the status/query model can report multiple in-process agents from that process**
- **discovery queries are mesh-wide** (`agents/alive`, `agents/free`, `agents/git`)
- **direct queries target a specific instance** using `<hostname>:<pid>`
- **Stylos session identity is hostname-based; direct Themion instance query paths use `<hostname>:<pid>`** as an application-level Themion path so multiple local processes can be addressed distinctly

Example:

- use discovery to find candidate agents across the mesh
- use a direct instance query like `.../instances/<hostname>:<pid>/query/status` to inspect one specific Themion process

Remote talk and task requests are accepted into the target process's normal local runtime; they are not synchronous remote execution. Task lifecycle tracking is lightweight and currently process-local and in-memory.

This keeps remote coordination pragmatic: a remote task request is accepted into the target process and then runs through that agent's normal local runtime rather than becoming a separate distributed execution system.

## Installation

### Install to `~/.local/bin`

For normal use, install the CLI crate with Cargo in release mode:

```bash
cargo install --path crates/themion-cli --root ~/.local
```

That installs the binary to:

```text
~/.local/bin/themion
```

Make sure `~/.local/bin` is on your `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

To make that permanent, add it to your shell config (for example `~/.bashrc` or `~/.zshrc`).

### Build without installing

If you only want a local build from the repo, use a release build:

```bash
cargo build --release -p themion-cli
./target/release/themion
```

Release builds are recommended for the best runtime performance. Use debug builds only if you're actively developing themion itself.

## Quick Start

After installation:

```bash
themion
```

If you built from source without installing:

```bash
./target/release/themion
```

Recommended first run inside the TUI:

```text
/login codex
```

That starts the built-in Codex login flow and switches you to the Codex-backed profile after authentication.

Or use a one-shot prompt in print mode:

```bash
themion "summarise the files in this directory"
```

If you're running from the build directory instead of an installed binary:

```bash
./target/release/themion "summarise the files in this directory"
```

Inside the TUI, prefix input with `!` to run a local shell command immediately:

```text
!pwd
!ls -la
!cargo check -p themion-cli
```

## Configuration

No environment variables are required. All settings are managed with `/config` inside the TUI and saved to `~/.config/themion/config.toml`.

### Codex (recommended)

```text
/login codex
```

This is the easiest setup path. It uses your existing ChatGPT / Codex subscription, stores auth in `~/.config/themion/auth.json`, and avoids API-key setup entirely.

### OpenRouter (alternative)

```text
/config profile create openrouter
/config profile set provider=openrouter
/config profile set api_key=sk-or-v1-...
/config profile set model=anthropic/claude-3.5-sonnet
/config profile use openrouter
```

Get an API key at [openrouter.ai](https://openrouter.ai) if you want access to Claude, GPT-4o, Gemini, Mistral, and many other hosted models.

### Local OpenAI-compatible server (alternative: llama.cpp / Ollama / LM Studio)

```text
/config profile create local
/config profile set provider=llamacpp
/config profile set endpoint=http://localhost:8080/v1
/config profile use local
```

No API key needed — just point `endpoint` at any running OpenAI-compatible server.

### Profile management

```text
/config profile list              # show all profiles
/config profile create <name>     # create from current settings
/config profile use <name>        # switch profiles
/config profile set key=value     # update a setting
/config                           # show active settings
```

### Environment variables (optional overrides)

| Variable              | Overrides          | Default                        |
| --------------------- | ------------------ | ------------------------------ |
| `OPENROUTER_API_KEY`  | profile `api_key`  | —                              |
| `OPENROUTER_MODEL`    | profile `model`    | `minimax/minimax-m2.7`         |
| `LLAMACPP_BASE_URL`   | profile `base_url` | `http://localhost:8080/v1`     |
| `SYSTEM_PROMPT`       | system prompt      | generic assistant              |

## TUI Key Bindings

| Key           | Action                  |
| ------------- | ----------------------- |
| `Enter`       | Send message            |
| `↑ / ↓`       | Navigate input history  |
| `Alt+↑ / ↓`   | Scroll conversation     |
| `Page Up/Down`| Scroll conversation     |
| `Ctrl-C`      | Quit                    |

## Architecture

For architecture and runtime details, see:

- [`docs/architecture.md`](docs/architecture.md)
- [`docs/engine-runtime.md`](docs/engine-runtime.md)
- [`docs/codex-integration-guide.md`](docs/codex-integration-guide.md)

## Adding a Tool

1. Add the OpenAI function schema to the `json!([...])` array in `tool_definitions()` (`crates/themion-core/src/tools.rs`)
2. Add a match arm in `call_tool()` in the same file
3. Nothing else — the agent loop passes `tool_definitions()` to the LLM on every request automatically

## License

MIT — see [LICENSE](LICENSE)
