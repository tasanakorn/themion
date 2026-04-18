# themion

> A terminal AI agent that thinks, uses tools, and gets things done — built in Rust.

```
█████  █   █  █████  █   █  ███   ███   █   █
  █    █   █  █      ██ ██   █   █   █  ██  █
  █    █████  ████   █ █ █   █   █   █  █ █ █
  █    █   █  █      █   █   █   █   █  █  ██
  █    █   █  █████  █   █  ███   ███   █   █
```

themion is a Rust-powered AI agent with a full-featured TUI. Give it a task in plain English and watch it reason, call tools, and produce results — all from your terminal.

## Features

- **Full TUI** — Ratatui-powered interface with streaming output, scroll, mouse support, and a braille spinner while thinking
- **Agentic tool use** — Reads files, writes files, lists directories, runs shell commands; loops until done
- **Persistent session history** — SQLite-backed conversation history with windowed context and keyword search
- **Multi-profile support** — Switch between providers and models on the fly with `/config profile use`
- **Multi-model** — Works with any OpenRouter model: Claude, GPT-4o, Gemini, Mistral, and more
- **Print mode** — Pipe a single prompt and get a result; perfect for scripting
- **Single binary** — Ships as one statically-linked executable with no runtime dependencies

## Quick Start

```bash
# Build a release binary
cargo build --release

# Set your API key (uses OpenRouter)
export OPENROUTER_API_KEY=sk-or-...

# Launch the TUI
./target/release/themion

# Or fire a one-shot prompt (print mode)
./target/release/themion "summarise the files in this directory"
```

## Configuration

| Variable             | Required | Default                  |
| -------------------- | -------- | ------------------------ |
| `OPENROUTER_API_KEY` | yes      | —                        |
| `OPENROUTER_MODEL`   | no       | `minimax/minimax-m2.7`   |
| `SYSTEM_PROMPT`      | no       | generic assistant prompt |

You can also manage profiles interactively inside the TUI:

```
/config profile list
/config profile create work
/config profile set model=anthropic/claude-3.5-sonnet
/config profile use work
```

## TUI Key Bindings

| Key           | Action                  |
| ------------- | ----------------------- |
| `Enter`       | Send message            |
| `↑ / ↓`       | Navigate input history  |
| `Alt+↑ / ↓`   | Scroll conversation     |
| `Page Up/Down`| Scroll conversation     |
| `Ctrl-C`      | Quit                    |

## Architecture

```
crates/
├── themion-core/
│   ├── agent.rs    # Agent loop: LLM → tools → repeat (windowed context, SQLite history)
│   ├── client.rs   # OpenRouter API client (streaming + non-streaming)
│   └── tools.rs    # Tool registry: bash, read_file, write_file, list_directory, recall/search history
└── themion-cli/
    ├── main.rs     # Entry point — TUI mode or print mode
    ├── tui.rs      # Ratatui TUI: layout, events, spinner animation
    └── config.rs   # XDG config file, profile management
```

The agent loop runs up to 10 iterations per turn: push user message → call LLM → execute any tool calls → feed results back → repeat until no more tool calls → return final response.

Context is managed via a sliding window of the last 5 turns. Earlier turns are persisted to SQLite and retrievable via `recall_history` and `search_history` tools that the model can call itself.

## Adding a Tool

1. Add the OpenAI function schema to the `json!([...])` array in `tool_definitions()` (`crates/themion-core/src/tools.rs`)
2. Add a match arm in `call_tool()` in the same file
3. Nothing else — the agent loop passes `tool_definitions()` to the LLM on every request automatically

## License

MIT — see [LICENSE](LICENSE)
