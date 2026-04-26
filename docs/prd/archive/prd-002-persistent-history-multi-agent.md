# PRD-002: Persistent Chat History, Multi-Agent Session Management, and Context Window Strategy

- **Status:** Implemented
- **Version:** v0.2.0
- **Scope:** `themion-core` (agent, tools, new `db` module); `themion-cli` (TUI status bar, `build_agent`, session bootstrap); workspace `Cargo.toml`
- **Author:** Tasanakorn (design) + Claude Code (PRD authoring)
- **Date:** 2026-04-18

## 1. Goals

- Persist every message of every turn to a local SQLite database keyed by a session UUID and the project working directory, so that history survives process restarts.
- Send only the last N complete turns (default N = 5) to the model on each API call, with a system-injected hint that older turns exist and are reachable via tools.
- Expose two new tools, `recall_history` and `search_history`, that let the model pull older context on demand.
- Support multiple `Agent` instances in one process — one designated as interactive, all sharing the same database handle.
- Extend the TUI status bar to show the project directory, cumulative session token usage (input / output / cached), and the estimated context length from the last API call.

## 2. Non-goals

- No cross-device sync, cloud backup, or export/import tooling.
- No conversation branching, editing, or deletion of past messages.
- No automatic summarization, compression, or embedding of older turns.
- No migration of existing in-memory history on upgrade — sessions before v0.2.0 have no database row and are discarded at exit as they are today.
- No schema migrations framework beyond the initial `CREATE TABLE IF NOT EXISTS` statements; the v0.2.0 schema is additive-only for the life of this PRD.

## 3. Background & Motivation

Today `Agent::messages` is an in-memory `Vec<Message>` that grows for the lifetime of the process and vanishes at exit. Every turn, the full history plus the system prompt is sent to OpenRouter, which means token cost per turn grows linearly and the context window fills quickly on long sessions. Restarting the CLI loses everything — even when the user reopens themion in the same project directory minutes later.

At the same time, the TUI displays only profile / provider / model; there is no feedback on how many tokens have been spent this session or how close the next call is to hitting the context ceiling.

Persisting history to SQLite, capping the context at a rolling window, and surfacing the cost in the status bar together unlock longer sessions, faster iterations, and a clear path toward multi-agent workflows where a background agent can run in the same process as the interactive one.

### 3.1 Current state

- `Agent` in `crates/themion-core/src/agent.rs` owns `messages: Vec<Message>`. The context sent to the API is built inline each LLM round as `[system_prompt] + all_messages` (the `msgs_with_system` construction inside the `for _ in 0..10` loop, roughly lines 112–118).
- `TurnStats` tracks per-turn `llm_rounds`, `tool_calls`, `tokens_in`, `tokens_out`, `tokens_cached`, `elapsed_ms` and is emitted via `AgentEvent::TurnDone(stats)`. No cumulative aggregation across turns exists; no turn sequence number is stored on the struct.
- Tools live in `crates/themion-core/src/tools.rs`: `tool_definitions()` returns a `serde_json::Value` array; `call_tool(name, args_json) -> String` dispatches by name. No tool today has access to shared mutable state beyond the process filesystem.
- The TUI status bar (`crates/themion-cli/src/tui.rs` lines 433–441) renders a single line with background `DarkGray` and foreground `White`, showing `profile · provider · model`.
- `App` in the CLI holds `agent: Option<Agent>`; `build_agent` at `tui.rs:328` is the free function that constructs an `Agent` from a profile.
- Workspace deps already present: tokio, reqwest, serde, serde_json, anyhow, toml, ratatui, crossterm, tui-textarea, tokio-stream, dirs (cli-only). SQLite and UUID libraries are absent.
- `dirs::config_dir()` is used by the existing config loader; no code currently reads `dirs::data_dir()`.

## 4. Design

### 4.1 Database schema and location

The database lives at `dirs::data_dir().join("themion/history.db")` — on Linux this resolves to `$XDG_DATA_HOME/themion/history.db` (default `~/.local/share/themion/history.db`). The parent directory is created on first run with `std::fs::create_dir_all`. A single SQLite file is shared by every themion process on the host. `DbHandle::open` enables WAL mode immediately after opening the connection (`PRAGMA journal_mode=WAL`) so concurrent readers are never blocked by a writer and `SQLITE_BUSY` contention is eliminated for the common multi-process case.

Three persistent tables (`agent_sessions`, `agent_turns`, `agent_messages`) plus one FTS5 virtual table (`agent_messages_fts`). The `agent_` prefix reserves namespace for future non-agent modules that may share the same database file:

```sql
CREATE TABLE IF NOT EXISTS agent_sessions (
    session_id     TEXT PRIMARY KEY,          -- UUID v4, text form
    project_dir    TEXT NOT NULL,             -- absolute path, canonicalized
    created_at     INTEGER NOT NULL,          -- unix epoch seconds
    is_interactive INTEGER NOT NULL DEFAULT 0 -- 0 or 1; informational, not enforced
);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_project ON agent_sessions(project_dir, created_at);

CREATE TABLE IF NOT EXISTS agent_turns (
    turn_id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL REFERENCES agent_sessions(session_id),
    turn_seq         INTEGER NOT NULL,        -- 1-based, monotonic within a session
    tokens_in        INTEGER NOT NULL DEFAULT 0,
    tokens_out       INTEGER NOT NULL DEFAULT 0,
    tokens_cached    INTEGER NOT NULL DEFAULT 0,
    llm_rounds       INTEGER NOT NULL DEFAULT 0,
    tool_calls_count INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL,
    UNIQUE(session_id, turn_seq)
);

CREATE TABLE IF NOT EXISTS agent_messages (
    message_id      INTEGER PRIMARY KEY AUTOINCREMENT,
    turn_id         INTEGER NOT NULL REFERENCES agent_turns(turn_id),
    session_id      TEXT NOT NULL REFERENCES agent_sessions(session_id),
    seq             INTEGER NOT NULL,         -- monotonic within a turn
    role            TEXT NOT NULL,            -- "user" | "assistant" | "tool"
    content         TEXT,                     -- nullable; assistant tool-only messages have NULL content
    tool_calls_json TEXT,                     -- serialized Vec<ToolCall> when role="assistant"
    tool_call_id    TEXT                      -- set when role="tool"
);
CREATE INDEX IF NOT EXISTS idx_agent_messages_session_seq ON agent_messages(session_id, message_id);

CREATE VIRTUAL TABLE IF NOT EXISTS agent_messages_fts USING fts5(
    content,
    content='agent_messages',
    content_rowid='message_id',
    tokenize='porter unicode61'
);

-- Keep FTS in sync via triggers
CREATE TRIGGER IF NOT EXISTS agent_messages_ai AFTER INSERT ON agent_messages BEGIN
    INSERT INTO agent_messages_fts(rowid, content) VALUES (new.message_id, new.content);
END;
```

System-role messages are never stored — the system prompt is ephemeral per request today and remains so.

**Alternative considered:** one database file per project (keyed by project_dir hash). Rejected: the single-file model is simpler, makes cross-project search trivial for future use, and SQLite handles concurrent readers/writers across processes via file locks without user-visible complexity.

**Alternative considered:** separate `tool_calls` and `tool_results` tables. Rejected: `Message` already carries `tool_calls` and `tool_call_id` as optional fields; mirroring the wire shape in one row keeps the writer straightforward and round-trips cleanly back into `Message`.

### 4.2 Context windowing

`Agent` gains two fields: `session_id: Uuid` and `window_turns: usize` (default 5). `Message` does not gain a turn-sequence field — turn membership is tracked in the `messages` DB table and in a new per-agent vector `turn_boundaries: Vec<usize>` that records the index in `self.messages` at which each new turn started.

Before each LLM round, the context is built as:

1. `[system_prompt]`
2. (if older turns exist) one synthetic `role="system"` hint message (constructed in-place, never stored in `self.messages`):
   `"Earlier turns 1..M are stored and searchable. Use recall_history to load a range or search_history to find a keyword."`
3. The slice of `self.messages` starting at `turn_boundaries[turn_boundaries.len() - window_turns]` through the end.

The synthetic hint is built only when `turn_boundaries.len() > window_turns`, so short sessions send exactly the same payload as today. The in-memory `self.messages` retains the full history for the process lifetime; windowing only affects what leaves the process.

**Alternative considered:** trim `self.messages` in place after each turn. Rejected: throwing away in-memory state forces every windowed turn to hit SQLite for recall_history on follow-ups; keeping the Vec intact costs memory but avoids a round-trip for the common "immediately reference the previous turn" case.

**Alternative considered:** windowing by token count rather than turn count. Rejected: token estimates vary per model tokenizer and the existing code has no tokenizer; a turn-count cap is predictable, cheap to reason about, and easy for the user to tune later via config.

### 4.3 New tools

Two tools are added to `tool_definitions()` in `crates/themion-core/src/tools.rs`. Both read from the shared DB; neither writes.

`recall_history`:

| Field       | Type   | Required | Notes                                                   |
| ----------- | ------ | -------- | ------------------------------------------------------- |
| session_id  | string | no       | UUID; defaults to the calling agent's own session.      |
| project_dir | string | no       | Absolute path filter; ignored when `session_id` is set. |
| limit       | int    | no       | Default 20, max 200.                                    |
| direction   | string | no       | `"newest"` (default) or `"oldest"`.                     |

Returns a JSON array of `{turn_seq, role, content, tool_calls, tool_call_id}` objects ordered by `(turn_seq, seq)`.

`search_history`:

| Field       | Type   | Required | Notes                                                                  |
| ----------- | ------ | -------- | ---------------------------------------------------------------------- |
| query       | string | yes      | Passed directly to FTS5 `MATCH`.                                       |
| session_id  | string | no       | Default: any session for the caller's `project_dir`.                   |
| project_dir | string | no       | Default: the caller's `project_dir`. Ignored when `session_id` is set. |
| limit       | int    | no       | Default 10, max 100.                                                   |

Returns a JSON array of `{session_id, turn_seq, role, snippet}` where `snippet` comes from FTS5 `snippet()` with a 16-token window.

Both tools need DB access. `call_tool` is extended from `call_tool(name, args) -> String` to `call_tool(name, args, ctx: &ToolCtx) -> String`, where:

```rust
pub struct ToolCtx {
    pub db: Arc<DbHandle>,
    pub session_id: Uuid,
    pub project_dir: PathBuf,
}
```

Existing tools (`read_file`, `write_file`, `list_directory`, `bash`) ignore `ctx`. `tool_call_detail()` in `agent.rs` gets explicit arms for `recall_history` and `search_history` so the TUI shows a useful one-liner.

**Alternative considered:** encode the DB handle as a global `OnceCell<Arc<DbHandle>>` inside `tools.rs`. Rejected: thread-through via `ToolCtx` keeps tools testable without global state and makes the multi-agent story honest — each agent's tools see that agent's identity.

### 4.4 Multi-agent session management

`App` is refactored:

```rust
pub struct AgentHandle {
    pub agent: Option<Agent>,
    pub session_id: Uuid,
    pub is_interactive: bool,
    pub label: String,      // human-readable for future UI
}

pub struct App {
    // ...existing fields minus `agent`...
    pub agents: Vec<AgentHandle>,
    pub db: Arc<DbHandle>,
}
```

Today the CLI constructs exactly one `AgentHandle` with `is_interactive = true`. All TUI input-routing code that previously read `self.agent` now reads the unique handle where `is_interactive == true` (a helper `App::interactive_mut()` wraps the lookup). The `Vec` shape is chosen now so that future background agents plug in without another refactor.

Each `AgentHandle` owns its own `session_id`. Every agent writes to the same `Arc<DbHandle>`. Writes from a `run_loop` happen via `tokio::task::spawn_blocking` because `rusqlite::Connection` is synchronous; the `DbHandle` wraps `Arc<Mutex<Connection>>` internally so concurrent agents serialize through the mutex without deadlocking the runtime.

**Alternative considered:** replace the `Vec` with a `HashMap<Uuid, AgentHandle>`. Rejected: ordered iteration matters for the future status-bar view that lists agents left-to-right; a `Vec` with a helper lookup is clearer at today's scale (1–4 handles expected).

**Alternative considered:** use `tokio-rusqlite` for async SQLite. Rejected: the extra dependency adds a background thread per connection; `spawn_blocking` with a `Mutex<Connection>` is sufficient for the write volume expected here (a few rows per turn).

### 4.5 Status bar

The status bar format becomes:

```
  <project_leaf>  ·  <profile>  ·  <model>  ·  in:<N> out:<N> cached:<N>  ·  ctx:~<N>tok
```

`project_leaf` is the final path component of `App.project_dir` (e.g. `themion` for `/home/tas/Documents/Projects/workspace-stele/themion`). Provider is dropped from the line to keep it within typical terminal widths; the profile name carries that signal indirectly.

`App` gains:

```rust
pub session_tokens: TurnStats, // zero-initialized; cumulative across the session
pub last_ctx_tokens: u64,      // mirrors the most recent TurnStats.tokens_in
```

`session_tokens` reuses the existing `TurnStats` struct as the brief specifies — `tokens_in`, `tokens_out`, and `tokens_cached` accumulate across turns; `llm_rounds`, `tool_calls`, and `elapsed_ms` also accumulate (not displayed in the bar today but available for future views). On every `AgentEvent::TurnDone(stats)` the TUI event loop adds each numeric field of `stats` into `session_tokens` and overwrites `last_ctx_tokens` with `stats.tokens_in`. The existing `Entry::Stats` push remains unchanged.

Background and foreground colors stay `DarkGray` / `White`; layout height stays `Constraint::Length(1)`.

**Alternative considered:** two-line status bar (first line identity, second line counters). Rejected: doubles the persistent chrome in the TUI and crowds small terminals; the single line fits at 100 columns with realistic numbers.

### 4.6 Session bootstrap

On `App::new`:

1. Resolve `project_dir = std::env::current_dir()?.canonicalize()?`. Canonicalization is required so that `cd` via symlinks still yields one logical project identity.
2. Open the DB: `DbHandle::open(dirs::data_dir().ok_or_else(...)?.join("themion/history.db"))`. `DbHandle::open` creates parent dirs, opens the connection, and runs the schema-init statements inside a single transaction.
3. Generate `session_id = Uuid::new_v4()`.
4. Insert a row into `agent_sessions` with `is_interactive = 1`.
5. Pass `(session_id, project_dir.clone(), db.clone())` into `build_agent`, which threads them into `Agent::new_with_db` (new constructor that supersedes `Agent::new_verbose` call sites in the TUI path).

`build_agent` signature changes from taking a `Profile` only to taking `(&Profile, Uuid, PathBuf, Arc<DbHandle>)`. Print mode (`main.rs`, non-TUI branch) is updated symmetrically: it opens the DB, creates a session row, and calls `Agent::new_with_db`, so print-mode turns also persist.

### 4.7 DB module layout

A new module `crates/themion-core/src/db.rs` exposes:

```rust
pub struct DbHandle { /* Arc<Mutex<Connection>> inside */ }

impl DbHandle {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Arc<Self>>; // enables WAL, runs schema-init
    pub fn insert_session(&self, id: Uuid, project_dir: &Path, interactive: bool) -> anyhow::Result<()>;
    pub fn begin_turn(&self, session: Uuid, turn_seq: u32) -> anyhow::Result<i64>; // returns turn_id
    pub fn append_message(&self, turn_id: i64, session: Uuid, seq: u32, msg: &Message) -> anyhow::Result<()>;
    pub fn finalize_turn(&self, turn_id: i64, stats: &TurnStats) -> anyhow::Result<()>;
    pub fn recall(&self, args: RecallArgs) -> anyhow::Result<Vec<RecalledMessage>>;
    pub fn search(&self, args: SearchArgs) -> anyhow::Result<Vec<SearchHit>>;
}
```

All write methods are sync; agents call them through `tokio::task::spawn_blocking({ let db = self.db.clone(); move || db.append_message(...) }).await??`. The mutex is held only for the duration of one statement or transaction, so agents contend briefly on write but never cross-block the runtime.

## 5. Changes by Component

| File                                            | Change                                                                                                                                                     |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Cargo.toml` (workspace)                        | Add `rusqlite = { version = "0.31", features = ["bundled"] }`, `uuid = { version = "1", features = ["v4", "serde"] }` to `[workspace.dependencies]`.       |
| `crates/themion-core/Cargo.toml`                | Pull in `rusqlite`, `uuid`, existing `anyhow`, `serde_json`.                                                                                               |
| `crates/themion-core/src/db.rs` (new)           | `DbHandle` with `open`, session/turn/message writers, `recall`, `search`; schema-init inside `open`.                                                        |
| `crates/themion-core/src/lib.rs`                | Add `pub mod db;` and re-export `DbHandle`.                                                                                                                |
| `crates/themion-core/src/agent.rs`              | Add `session_id`, `project_dir`, `db`, `window_turns`, `turn_boundaries` fields; new `Agent::new_with_db` constructor; window-aware context assembly in `run_loop`; per-message DB writes via `spawn_blocking`; bump `turn_boundaries` at the start of `run_loop`; explicit `tool_call_detail` arms for `recall_history` and `search_history`. |
| `crates/themion-core/src/tools.rs`              | Add `recall_history` and `search_history` schemas; add `ToolCtx` struct; change `call_tool` signature to `call_tool(name, args, ctx)`; dispatch the two new tools to `DbHandle::recall` / `::search`; existing four tools ignore `ctx`.                                                                                            |
| `crates/themion-cli/Cargo.toml`                 | Pull in `uuid` for session IDs in `build_agent`; no rusqlite dep (CLI uses `DbHandle` through `themion-core`).                                             |
| `crates/themion-cli/src/tui.rs`                 | Status-bar format change; `App.agents: Vec<AgentHandle>` replaces `agent: Option<Agent>`; `App.db`, `App.session_tokens`, `App.last_ctx_tokens` fields; `build_agent` signature extended; `App::interactive_mut` helper; on `TurnDone` update cumulative counters. |
| `crates/themion-cli/src/main.rs`                | Print mode opens the DB, generates a session UUID, inserts a session row, and uses `Agent::new_with_db`.                                                   |
| `docs/architecture.md`                          | Add a "Persistent history" subsection describing the DB path, schema summary, and windowing rule; note the multi-agent handle shape.                       |
| `docs/README.md`                                | Add the PRD-002 row to the PRD table.                                                                                                                      |

## 6. Edge Cases

- **Data dir unavailable** (`dirs::data_dir()` returns `None`, e.g. locked-down CI): fall back to an in-memory `DbHandle::open_in_memory()` and log one stderr line noting that history persistence is disabled for this run.
- **DB file locked by another process**: WAL mode allows concurrent readers alongside a single writer; `SQLITE_BUSY` is only possible when two writers overlap. `DbHandle::open` sets `busy_timeout` to 5 seconds as a backstop for that rare case.
- **Concurrent writers across processes**: two themion instances in the same CWD each create their own `agent_sessions` row (different UUIDs). They write to disjoint `turn_id` ranges because `turn_id` is `AUTOINCREMENT`; no primary-key collision is possible.
- **Canonicalize fails on `project_dir`** (e.g. CWD was deleted mid-session): fall back to the raw `current_dir()` string. Recall still works within the process; cross-session recall may not match by path.
- **Schema already exists from a newer themion version**: `CREATE TABLE IF NOT EXISTS` is a no-op. Unknown columns added by a future version are left untouched; this PRD promises additive-only changes within v0.2.x.
- **FTS5 unavailable in the bundled SQLite**: the `rusqlite` `bundled` feature ships FTS5 by default. Guard with a `PRAGMA compile_options` check on first open; if FTS5 is missing, `search_history` is registered but returns an empty array plus an error string rather than a panic. The `agent_messages_fts` virtual table creation is skipped in that branch.
- **Tool invocation before first turn is written**: the model can in principle call `recall_history` on turn 1. The tool simply returns an empty array; no error.
- **Windowing with tool-heavy turns**: a single turn can contain many assistant and tool messages. Windowing is by turn, so all messages within the last N turns are included regardless of count — there is no per-message trim.
- **`tool_calls_json` round-trip**: stored as the serialized `Vec<ToolCall>`; on recall, the tool returns the deserialized array in its JSON response to the model. The model sees structured tool-call info, not a raw string blob.
- **Empty `content` on assistant messages that only issue tool calls**: `content` is stored as SQL `NULL`; the FTS trigger inserts `NULL` into `agent_messages_fts`, which FTS5 treats as no-content-to-index. Search skips these rows naturally.
- **Status-bar overflow** on narrow terminals: the bar is truncated by Ratatui's `Paragraph` widget; no wrapping is introduced. The counters are last, so the project name and profile remain visible under truncation.

## 7. Migration

On first launch after upgrade, the `history.db` file does not exist. `DbHandle::open` creates it and runs the schema statements. Existing in-flight shell sessions are unaffected — there is no in-memory history to migrate because prior versions never persisted any.

Downgrade from v0.2.x back to v0.1.x is safe: the older binary ignores `history.db` entirely. Re-upgrading preserves the database untouched.

Public API additions in `themion-core`:

- `Agent::new_with_db(client, model, system_prompt, session_id, project_dir, db) -> Agent`.
- `pub mod db;` exposing `DbHandle`, `RecallArgs`, `SearchArgs`, `RecalledMessage`, `SearchHit`.
- `ToolCtx` struct and the widened `call_tool` signature. Existing `call_tool(name, args)` callers must add a `ToolCtx` — this is a breaking change for any external consumer of `themion-core`, which currently has none beyond `themion-cli`.

## 8. Testing

| Step                                                                                                             | Verify                                                                                                               |
| ---------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Fresh install, run `cargo run -p themion-cli`, ask one question, exit                                            | `~/.local/share/themion/history.db` exists; `agent_sessions` has one row; `agent_turns` has one row; `agent_messages` has user and assistant rows. |
| Run two concurrent `cargo run -p themion-cli` processes in the same CWD, each asks one question                  | Two distinct `session_id` rows in `agent_sessions`, each with its own turns and messages; no primary-key errors logged. |
| Ask six questions in one session, then inspect the seventh request payload                                        | Only messages from turns 2–6 are in the body; a `role="system"` hint mentioning turn 1 is present before the user message. |
| In a windowed session, ask the model to recall a fact from turn 1                                                  | The model invokes `recall_history`; the tool returns the turn-1 messages; the next assistant reply cites them.       |
| Ask the model to `search_history` for a keyword present only in an earlier turn                                    | Tool returns at least one hit with the expected `session_id` and `turn_seq`; FTS snippet contains the keyword.       |
| Restart the CLI in the same CWD, call `search_history` for a keyword from the prior session                        | Tool returns hits across both sessions (filtered by project_dir default).                                            |
| Check the TUI status bar after the third turn                                                                     | Line shows the CWD leaf, profile, model, cumulative `in:/out:/cached:` summing the first three turns, and `ctx:~<N>tok` equal to turn 3's `tokens_in`. |
| Delete `~/.local/share/themion/history.db`, restart                                                               | File is recreated on first DB write; no user-visible error.                                                          |
| Unit test `DbHandle::open` on a temp path                                                                          | All `agent_*` tables and the FTS trigger exist after `open`; re-`open` is idempotent.                               |
| Unit test the windowing rule with a synthetic `Agent` holding six turn boundaries                                 | For `window_turns = 5`, the slice returned by the builder starts at the boundary of turn 2 and a system hint is prepended; for `window_turns = 10`, no hint is added. |
| Integration test: start two `AgentHandle`s in one `App`, mark one interactive, send input                         | The message reaches only the interactive agent; both agents' turns land in SQLite with distinct session UUIDs.       |
| Corrupt `history.db` (truncate to 0 bytes), restart                                                                | `DbHandle::open` logs a clear stderr line naming the DB path; process does not panic; empty schema is re-created.   |
