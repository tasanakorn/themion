# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

Runtime is **Bun** (TypeScript + JSX, no build step). There are no tests or linters configured.

- `bun run start` — run the agent once (REPL if no args, print-mode if args provided)
- `bun run dev` — run with `--watch` for hot reload on source changes
- `bun run src/index.tsx "your prompt"` — print-mode: send a single message and exit
- `LLM_URL=http://host:port bun run start` — override local LLM endpoint (defaults to `http://localhost:30434`)

Type-check only (no runner script): `bunx tsc --noEmit`.

## Architecture

Themion is a thin custom agent built on top of **`@mariozechner/pi-coding-agent`** (pi-ai / pi-agent-core). The pi-coding-agent library provides the entire agent loop, tool execution, session management, and Ink-based TUI — this repo just wires it up for a local LLM and adds project-specific tools.

### Entry flow (`src/index.tsx`)

1. Builds in-memory `AuthStorage`, `ModelRegistry`, `SettingsManager` (compaction disabled).
2. Defines a single `Model` targeting an OpenAI-compatible local endpoint (`${LLM_BASE_URL}/v1`, api `openai-responses`, provider `openai`, dummy key). Context window 128k, `maxTokens` and `reasoning` come from `config.ts`.
3. `createAgentSessionRuntime` is called with a `CreateAgentSessionRuntimeFactory` that:
   - Constructs a `DefaultResourceLoader` with a `systemPromptOverride` returning `SYSTEM_PROMPT`.
   - Calls `createAgentSessionServices`, then `createAgentSessionFromServices` with the custom resource loader explicitly injected (the override must be passed in **both** places or the default loader wins).
   - Registers `codingTools` (from pi-coding-agent) plus `customTools: [escalateTool, ...tmuxTools]`.
4. If CLI args are present → `runPrintMode`; otherwise → `InteractiveMode.run()` (Ink TUI).

When modifying the agent wiring, remember that `pi-coding-agent` owns the UI and loop — don't reinvent those. Changes typically mean adjusting model config, settings, the resource loader, or registered tools.

### Configuration (`src/config.ts`)

Central config module. Notable knobs:

- `ENABLE_THINKING = false` — intentional: Gemma 3n routes reasoning into `reasoning_content`, which exhausts the token budget before any visible answer. Keep off unless targeting a model that doesn't have this issue.
- `ALLOWED_PATH_PREFIXES` — hardcoded to this repo and `/tmp`. Consumed by `guardPath` in `src/guard.ts`, which **must** be called on any path a new filesystem-touching tool accepts.
- `ALLOWED_COMMANDS` — declarative allow-list for shell tools (historical; current custom tool set does not shell out arbitrarily — the tmux tool uses its own stricter char-blocklist in `tmux.ts`).
- `SYSTEM_PROMPT` — injected via the resource loader override; tells the model to use tools and to call `escalate` when out of its depth.

### Custom tools (`src/tools/`)

All tools use `defineTool` from pi-coding-agent with TypeBox schemas.

- **`escalate.ts`** — no-op tool that returns an `ESCALATION_REQUESTED: <reason>` message. Signals that the local small model is over its head; an outer orchestrator is expected to notice this and rerun on a stronger model. Do not make it "do" anything — the signal is the point.
- **`tmux.ts`** — seven tmux tools (`tmux_list`, `tmux_capture`, `tmux_send_keys`, `tmux_send_text`, `tmux_split_pane`, `tmux_kill_pane`, `tmux_select_layout`). All targets run through `validateTarget`, which rejects shell metacharacters (`DANGEROUS_CHARS`) to prevent injection into `tmux -t`. `runTmux` spawns via `Bun.spawn` with a 10s timeout. `tmux_list` renders a `Session > Window > Pane` hierarchy (do not flatten this — the hierarchy is the UX contract with the model).

When adding a new tool: define it with TypeBox, validate all user-supplied strings that reach a shell or filesystem boundary, and register it in the `customTools` array in `src/index.tsx`.

## Stele Shared Memory Protocol — themion

**Scope:** `stele/themion` | **Type:** general
**Server:** [Stele](https://github.com/tasanakorn/stele) — shared memory for multi-agent Claude Code

### Storage

- **Flat Memory** (`store_memory`/`recall_memories`) — facts, decisions, conventions, notes.
- **Knowledge Graph** (`create_entities`/`create_relations`/`search_nodes`/`open_nodes`) — things with relationships.

### Scope & Retrieval

Scopes use **prefix matching** — querying `stele/themion` also matches `stele/themion/backend`, `stele/themion/frontend`, etc.

| Scope           | Covers                      |
| --------------- | --------------------------- |
| `stele`         | Workspace-wide standards    |
| `stele/themion` | This project (+ sub-scopes) |

**Multi-scope reads:** `scope: ["stele/themion", "global"]` to include shared cross-project knowledge. Write tools remain single-scope.

### Workflow

- **Task start:** Run `/stele:sync` — pulls latest shared state. Do not assume you know the current state.
- **Before architectural changes:** Run `open_nodes` or `read_graph` to check dependencies.
- **End of session:** Run `/stele:checkpoint` — persists decisions, discoveries, and fixes back to Stele.

### Autonomous Updates (no permission needed)

You MUST update Stele immediately when any of these occur — do not defer:

- **Contract change** (API, env var, shared interface) → store + tag `#contract #breaking`
- **Lesson learned** (non-obvious bug fix) → store + tag `#wisdom`
- **Relationship discovered** (A depends on B) → `create_relations`
- **Convention established** (new agreed rule) → store + tag `#active`

Standard tags: `#active`, `#todo`, `#contract`, `#breaking`, `#wisdom`, `#conflict`. Run `/stele:checkpoint` for full tagging convention and project-specific tags.
