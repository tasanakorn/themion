# themion

A CLI AI agent built in Rust. Give it tasks and watch it use tools to get things done.

## Features

- **Function Calling** — The agent can read files, write files, list directories, and execute shell commands
- **REPL Mode** — Interactive conversation with persistent history across turns
- **Multi-Model Support** — Works with any OpenRouter-compatible model ( Anthropic, OpenAI, Meta, Google, etc.)
- **Print Mode** — Pipe a prompt and get a result; great for scripting
- **Rust-Powered** — Fast, memory-safe, and ships as a single static binary

## Quick Start

```bash
# Build
cargo build --release

# Run with a single prompt (print mode)
OPENROUTER_API_KEY=sk-... cargo run -- "list the files in the current directory"

# Run interactive REPL
OPENROUTER_API_KEY=sk-... cargo run
```

## Configuration

| Environment Variable | Required | Default |
|---------------------|----------|---------|
| `OPENROUTER_API_KEY` | Yes | — |
| `OPENROUTER_MODEL` | No | `minimax/minimax-m2.7` |
| `SYSTEM_PROMPT` | No | Generic assistant |

## Architecture

```
src/
├── client.rs   # OpenRouter API client (reqwest + serde)
├── agent.rs    # Core loop: call LLM → execute tools → repeat
├── tools.rs    # Tool definitions and implementations
└── main.rs     # Entry point (REPL vs print mode)
```

The agent loops up to 10 iterations: call LLM → if tool calls present, execute them and feed results back → repeat until done.

## Adding Tools

1. Add the OpenAI function schema to `tool_definitions()` in `src/tools.rs`
2. Add a match arm in `call_tool()` in the same file
3. That's it — the agent loop picks it up automatically

## License

MIT