# themion

> Just another AI agent. Started as a weekend experiment — works on purpose. Built in Rust, runs in your terminal.

```
████████╗██╗  ██╗███████╗███╗   ███╗██╗ ██████╗ ███╗   ██╗
╚══██╔══╝██║  ██║██╔════╝████╗ ████║██║██╔═══██╗████╗  ██║
   ██║   ███████║█████╗  ██╔████╔██║██║██║   ██║██╔██╗ ██║
   ██║   ██╔══██║██╔══╝  ██║╚██╔╝██║██║██║   ██║██║╚██╗██║
   ██║   ██║  ██║███████╗██║ ╚═╝ ██║██║╚██████╔╝██║ ╚████║
   ╚═╝   ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝╚═╝ ╚═════╝ ╚═╝  ╚═══╝
```

themion is a Rust-powered AI agent with a full-featured TUI. Give it a task in plain English and watch it reason, call tools, and produce results — all from your terminal.

> After 0.2.0, themion will use themion to help develop itself.

## Features

- **First-class Codex login** — Sign in with `/login codex` and use your ChatGPT / Codex subscription directly, without managing an API key
- **Full TUI** — Ratatui-powered interface with streaming output, scroll, mouse support, and a braille spinner while thinking
- **Agentic tool use** — Reads files, writes files, lists directories, runs shell commands; loops until done
- **Direct shell shortcut** — Run local commands instantly from the TUI with `!<command>` and see the output in the conversation pane
- **Persistent session history** — SQLite-backed conversation history with windowed context and keyword search
- **Multi-profile support** — Switch between providers and models on the fly with `/config profile use`
- **Flexible backends** — Codex is the recommended default, with OpenRouter and local OpenAI-compatible servers like llama.cpp, Ollama, or LM Studio as alternatives
- **Print mode** — Pipe a single prompt and get a result; perfect for scripting
- **Single binary** — Ships as one statically-linked executable with no runtime dependencies

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
