## Stele Shared Memory Protocol — themion

**Scope:** `stele/themion` | **Type:** general
**Server:** [Stele](https://github.com/tasanakorn/stele) — shared memory for multi-agent Claude Code

### Storage

- **Flat Memory** (`store_memory`/`recall_memories`) — facts, decisions, conventions, notes.
- **Knowledge Graph** (`create_entities`/`create_relations`/`search_nodes`/`open_nodes`) — things with relationships.

### Scope & Retrieval

Scopes use **prefix matching** — querying `stele/themion` also matches `stele/themion/backend`, `stele/themion/frontend`, etc.

| Scope            | Covers                      |
| ---------------- | --------------------------- |
| `stele`          | Workspace-wide standards    |
| `stele/themion`  | This project (+ sub-scopes) |

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
