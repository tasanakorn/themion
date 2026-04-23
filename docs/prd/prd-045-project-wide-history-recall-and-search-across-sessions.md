# PRD-045: Project-Wide History Recall and Search Across Sessions

- **Status:** Proposed
- **Version:** v0.28.0
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-23

## Summary

- `history_recall` and `history_search` already support explicit `session_id` filtering, but cross-session project memory is still awkward.
- Add first-class project-wide behavior so omitting `session_id` searches or recalls across all sessions for the same `project_dir` by default.
- Keep explicit `session_id` support for cases that need one exact session.
- Make ordering and returned metadata clear enough that results from multiple sessions are not ambiguous.
- Update prompt-visible tool descriptions and runtime docs so the model understands that project memory is wider than the active session.
- Do not add cross-project search in this PRD; the new default remains bounded to the current project directory.

## Goals

- Make `history_recall` useful as project-wide memory when the caller does not specify `session_id`.
- Make `history_search` search all stored sessions for the current `project_dir` when `session_id` is omitted.
- Preserve the existing ability to target one exact session via `session_id`.
- Return enough session-identifying metadata that multi-session results remain interpretable.
- Keep the tool contracts simple and consistent with current defaulting rules.
- Update docs so the new behavior is described accurately in architecture and runtime references.

## Non-goals

- No cross-project history search or recall in this PRD.
- No automatic summarization, ranking model, or embedding-based retrieval layer.
- No new history tables or a major persistence redesign.
- No removal of explicit `session_id` targeting.
- No change to the current SQLite storage model beyond what the query behavior needs.
- No TUI-specific history browser work in this PRD.

## Background & Motivation

### Current state

Themion already persists conversation history in SQLite and exposes `history_recall` and `history_search` as model-callable tools. The current runtime docs describe these tools as the mechanism for reaching older stored history outside the prompt window.

Today the implementation in `crates/themion-core/src/db.rs` already accepts both `session_id` and `project_dir` filters for recall and search. However, the current contract is still too session-centric in practice:

- tool descriptions emphasize explicit `session_id`
- `history_recall` returns rows without `session_id`, so project-wide multi-session recall would be ambiguous
- the prompt-visible recall hint still frames old history mainly as earlier turns of the current session
- the intended default behavior for omitted `session_id` is not documented clearly enough as project-wide memory

The result is that the durable history system exists, but the agent is not guided to treat it confidently as shared memory across sessions in the same project directory.

### Why project-wide defaulting is the right memory model

For coding work, the most useful boundary is usually the repository or project directory rather than one process-lifetime session UUID. A user may restart Themion, open a fresh session, or have multiple sessions over time in the same repo while still expecting prior work to remain discoverable.

Using the active `project_dir` as the default scope keeps retrieval relevant and bounded while matching how the rest of the app already keys persistent session storage.

**Alternative considered:** keep the current session as the default and require the model to opt into `project_dir` explicitly for broader recall. Rejected: this makes durable project memory harder to use and hides the most useful retrieval scope behind extra tool arguments.

### Why multi-session recall must include session identity

Search results already return `session_id`, which makes cross-session hits interpretable. Recall results do not. If `history_recall` is allowed to surface messages from multiple sessions under one project, the returned payload must identify which session each message came from.

Without that, `turn_seq` values from different sessions can collide and become misleading.

**Alternative considered:** keep project-wide recall but sort by recency and omit `session_id` to save tokens. Rejected: cross-session ambiguity would make the result harder to trust and use correctly.

## Design

### Default omitted `session_id` to the caller's current `project_dir`

Both history tools should treat omitted `session_id` as a project-wide query scoped to the caller's current `project_dir`.

Normative direction:

- when `session_id` is provided, it remains the strongest filter and the query targets that exact session
- when `session_id` is omitted and `project_dir` is omitted, the tools must default to the caller's current `project_dir`
- when `session_id` is omitted and `project_dir` is provided, the tools should use that explicit project directory
- docs and tool descriptions should say this plainly instead of implying that omission means only the active session

This makes project memory the normal path while keeping single-session precision available.

**Alternative considered:** reject calls that omit both `session_id` and `project_dir` as too ambiguous. Rejected: the runtime already has the caller project context, so forcing extra arguments would add friction without improving correctness.

### Make `history_recall` return session identity for every recalled message

`history_recall` should return `session_id` with each recalled message row.

Normative direction:

- add `session_id` to the `RecalledMessage` shape in `crates/themion-core/src/db.rs`
- include `session_id` in the SQL select list for recall queries
- include `session_id` in the JSON returned by the tool layer
- update docs and examples so cross-session recall is visibly supported rather than implicit

This makes project-wide recall safe to use because callers can distinguish messages from different sessions.

**Alternative considered:** add only a session-level wrapper around grouped messages rather than per-row `session_id`. Rejected: the current flat result shape is simpler, and per-row identity is enough for correct interpretation.

### Define stable ordering for project-wide recall

When recall spans multiple sessions, ordering should remain predictable.

Normative direction:

- for `direction="newest"`, order by session recency first and then message position within that session from newest to oldest
- for `direction="oldest"`, order by session age first and then message position within that session from oldest to newest
- if the implementation relies on existing persisted timestamps or monotonic message identifiers to express session recency, the docs should state the chosen basis clearly enough for maintainers
- the order must be stable enough that repeated calls with the same data return the same sequence

The exact SQL expression may follow current schema realities, but the resulting behavior should be deterministic and understandable.

**Alternative considered:** sort only by `turn_seq` across all sessions. Rejected: `turn_seq` is only monotonic within one session and does not define a meaningful global order.

### Keep `history_search` project-wide by default and document it explicitly

`history_search` already has the right high-level shape for project-wide retrieval, but its contract should be documented more clearly as the default memory search path across sessions in one project.

Normative direction:

- preserve explicit `session_id` filtering when requested
- when `session_id` is omitted, search all sessions for the selected project directory
- keep returning `session_id`, `turn_seq`, `role`, and `snippet`
- update prompt-visible descriptions and runtime docs so the model understands that this is not limited to the active session

This clarifies intended behavior without redesigning the search tool.

**Alternative considered:** add a separate `history_search_project` tool. Rejected: unnecessary tool-surface expansion for behavior that should be the default of the existing tool.

### Update recall hint wording to describe project memory, not only current-session overflow

The synthetic recall hint injected into the prompt window should reflect the widened memory model.

Normative direction:

- when older turns exist in the current session, keep the current-session hint useful
- expand the wording so it also reminds the model that persistent history tools can search or recall across prior sessions in the same project directory when `session_id` is omitted
- avoid merging this behavior into the base system prompt; it should remain part of the existing contextual runtime hinting path

This makes the model more likely to use the tools as intended for durable project memory.

**Alternative considered:** leave the hint unchanged and rely only on updated tool schemas. Rejected: the recall hint is exactly where the model is told older context exists, so it should describe the broader retrieval surface truthfully.

### Keep the schema additive and backward compatible

The history tool change should be additive rather than a breaking rename or tool split.

Normative direction:

- keep the existing `history_recall` and `history_search` tool names
- keep `session_id` optional
- add `session_id` to recall results rather than changing existing field meanings
- preserve current result-limit bounds unless a separate PRD changes them

This keeps the memory improvement small and compatible with current runtime architecture.

**Alternative considered:** replace `history_recall` with a more complex grouped-history API. Rejected: too much surface change for a targeted memory improvement.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/db.rs` | Update recall query/result shape so project-wide recall includes `session_id` and uses deterministic cross-session ordering when `session_id` is omitted. |
| `crates/themion-core/src/tools.rs` | Update `history_recall` and `history_search` tool descriptions so omitted `session_id` clearly means project-wide lookup for the selected `project_dir`. |
| `crates/themion-core/src/tools.rs` | Return `session_id` in `history_recall` JSON payloads. |
| `crates/themion-core/src/agent.rs` | Update the synthetic recall hint so it mentions project-wide history recall/search across sessions in the same project directory when `session_id` is omitted. |
| `docs/architecture.md` | Update persistent-history/tooling documentation to describe project-wide cross-session memory as the default scope when `session_id` is omitted. |
| `docs/engine-runtime.md` | Clarify history tool semantics, default scoping, and cross-session result interpretation. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Edge Cases

- `history_recall` is called with explicit `session_id` from another project directory → verify: the exact session filter wins and the result is scoped to that session only.
- `history_recall` is called without `session_id` in a project that has many past sessions → verify: messages can come from multiple sessions and each row includes `session_id` so they are distinguishable.
- `history_search` is called without `session_id` in a project with multiple sessions containing the same keyword → verify: hits can include multiple sessions and each hit reports its `session_id`.
- multiple sessions share the same `turn_seq` values → verify: project-wide recall remains interpretable because ordering does not rely only on `turn_seq` and the payload includes `session_id`.
- the caller passes an explicit `project_dir` with no matching stored sessions → verify: both tools return an empty array rather than failing ambiguously.
- the current session is the only stored session for the project → verify: default project-wide behavior still returns the expected current-session results.
- `history_search` runs when FTS5 is unavailable → verify: existing empty-result fallback behavior remains unchanged.
- `history_recall` requests `direction="newest"` across many sessions → verify: ordering is deterministic and not dependent on undefined SQLite row order.

## Migration

This is an additive behavior and contract clarification for existing history tools.

Expected rollout shape:

- keep existing tool names and optional arguments
- make project-wide behavior the explicit documented default when `session_id` is omitted
- add `session_id` to recall results so multi-session responses are self-describing
- avoid schema churn unless implementation needs a small additive index or query-support column

No user-facing config migration is required.

## Testing

- create two or more sessions under the same `project_dir`, then call `history_recall` without `session_id` → verify: results can include messages from multiple sessions and every row includes `session_id`.
- create two or more sessions under the same `project_dir`, then call `history_search` without `session_id` → verify: hits are returned across those sessions rather than only from the active session.
- call `history_recall` with explicit `session_id` for one known session → verify: only that session's messages are returned.
- call `history_search` with explicit `session_id` for one known session → verify: only that session's hits are returned.
- call either tool without `session_id` in a project that has no prior stored sessions beyond the active one → verify: behavior remains correct and bounded.
- inspect the synthetic recall hint during a windowed session → verify: it mentions that omitted `session_id` can search or recall across sessions in the same project directory.
- run `cargo check -p themion-core` after implementation → verify: the history-tool changes compile cleanly.
- run targeted history/db tests if present, or add them near the touched code → verify: project-wide multi-session recall/search semantics are covered automatically.

## Implementation checklist

- [ ] update `history_recall` result shape to include `session_id`
- [ ] make project-wide omitted-`session_id` behavior explicit in history tool descriptions and docs
- [ ] ensure recall ordering across multiple sessions is deterministic and documented
- [ ] update the synthetic recall hint in `agent.rs` to mention project-wide cross-session memory
- [ ] add or update tests for multi-session same-project recall and search behavior
- [ ] update `docs/architecture.md` and `docs/engine-runtime.md`
- [ ] update `docs/README.md` with the new PRD entry
