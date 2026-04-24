# PRD-045: Project-Scoped History Recall and Search Across Sessions

- **Status:** Implemented
- **Version:** v0.28.0
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-23

## Summary

> **Implementation note:** Landed in `themion-core` with current-project-only history scope, omitted `session_id` meaning the active session, explicit `session_id="*"` for all sessions in the current project, `history_recall` rows including `session_id`, updated tool schemas, and updated recall-hint/docs wording.


- `history_recall` and `history_search` already support explicit `session_id` filtering, but cross-session project memory is still awkward.
- Tighten the safety model so history tools are always scoped to the caller's current project directory, with no caller-supplied `project_dir` override.
- Make omitted `session_id` target only the current session by default.
- Add an explicit magic value `session_id="*"` for project-scoped cross-session recall and search across all stored sessions in the current project.
- Keep explicit `session_id` support for cases that need one exact session in the current project.
- Make ordering and returned metadata clear enough that cross-session results are not ambiguous.
- Update prompt-visible tool descriptions and runtime docs so the model understands the current-session default and the explicit project-wide opt-in path.

## Goals

- Make `history_recall` use the current session when the caller does not specify `session_id`.
- Make `history_search` search the current session when `session_id` is omitted.
- Add an explicit `session_id="*"` mode for project-wide recall and search across all stored sessions for the current project directory.
- Preserve the existing ability to target one exact session via `session_id`, while keeping all access bounded to the current project directory.
- Return enough session-identifying metadata that cross-session results remain interpretable.
- Keep the tool contracts simple and consistent with the current-project safety boundary.
- Update docs so the new behavior is described accurately in architecture and runtime references.

## Non-goals

- No cross-project history search or recall in this PRD.
- No caller-selectable `project_dir` scope override for history tools.
- No automatic summarization, ranking model, or embedding-based retrieval layer.
- No new history tables or a major persistence redesign.
- No removal of explicit `session_id` targeting within the current project.
- No change to the current SQLite storage model beyond what the query behavior needs.
- No TUI-specific history browser work in this PRD.

## Background & Motivation

### Current state

Themion already persists conversation history in SQLite and exposes `history_recall` and `history_search` as model-callable tools. The current runtime docs describe these tools as the mechanism for reaching older stored history outside the prompt window.

Today the implementation in `crates/themion-core/src/db.rs` already accepts both `session_id` and `project_dir` filters for recall and search. However, the current contract is not ideal:

- cross-session project memory is still awkward to request intentionally
- a caller-visible `project_dir` parameter makes it easier than necessary to aim history lookup at another project
- `history_recall` returns rows without `session_id`, so cross-session recall would be ambiguous
- the prompt-visible recall hint still frames old history mainly as earlier turns of the current session
- the intended safe default behavior is not documented clearly enough

The durable history system exists, but the tool contract should make the safety boundary clearer: history access should stay inside the active project, use the current session by default, and require an explicit opt-in for cross-session project recall/search.

### Why current-session defaulting is safer

For safety, omitted `session_id` should not automatically widen access to every stored session in the current project. The most conservative and predictable default is the active session the model is already working in.

This reduces accidental cross-session retrieval while keeping older same-session context easy to reach.

**Alternative considered:** make omitted `session_id` search or recall all sessions in the current project. Rejected: convenient, but too broad as a silent default for a memory tool.

### Why project-wide cross-session retrieval should be explicit

Cross-session retrieval within the current project is still useful, but it should be deliberate.

Using a single explicit magic value such as `session_id="*"` makes the broadening of scope obvious while keeping the tool surface small.

**Alternative considered:** keep a caller-provided `project_dir` parameter and let omission of `session_id` imply project-wide lookup. Rejected: it exposes unnecessary scope control and makes the broader behavior too implicit.

### Why cross-project access should not be exposed

If the intended safety boundary is the active project, the caller should not be able to redirect history tools to another `project_dir` at all.

The runtime already knows the caller's current project directory. Treating that project scope as implicit avoids ambiguity, removes an unnecessary parameter, and prevents accidental or opportunistic cross-project retrieval.

**Alternative considered:** allow explicit `project_dir` but ignore values outside the current project. Rejected: this preserves a misleading parameter and complicates the contract without adding useful capability.

### Why cross-session recall must include session identity

Search results already return `session_id`, which makes cross-session hits interpretable. Recall results do not. If `history_recall` is allowed to surface messages from multiple sessions under one project when `session_id="*"`, the returned payload must identify which session each message came from.

Without that, `turn_seq` values from different sessions can collide and become misleading.

**Alternative considered:** keep project-wide recall but sort by recency and omit `session_id` to save tokens. Rejected: cross-session ambiguity would make the result harder to trust and use correctly.

## Design

### History tools are always scoped to the caller's current project

Both history tools should be bounded to the caller's current project directory. The caller should not be able to provide a `project_dir` argument.

Normative direction:

- remove caller-facing `project_dir` parameters from `history_recall` and `history_search`
- derive the project scope from the caller's current runtime context
- keep all history queries bounded to that current project directory
- when an explicit `session_id` refers to a session outside the current project, return no results rather than crossing the project boundary

This keeps the safety boundary simple and consistent.

**Alternative considered:** keep `project_dir` in the tool schema but document that callers should not use it. Rejected: if it should not be used, it should not be exposed.

### Default omitted `session_id` to the current session

Both history tools should treat omitted `session_id` as a current-session query.

Normative direction:

- when `session_id` is omitted, query only the current active session
- docs and tool descriptions should say this plainly
- the prompt-visible hinting should reinforce that broader retrieval requires explicit opt-in

This makes the safest path the default path.

**Alternative considered:** reject omitted `session_id` and require callers to pass either a UUID or `"*"`. Rejected: unnecessary friction for the most common same-session recall/search behavior.

### Use `session_id="*"` as the explicit project-wide cross-session mode

Both history tools should accept exactly one special value, `session_id="*"`, to search or recall across all stored sessions in the current project.

Normative direction:

- `session_id="*"` means all sessions in the caller's current project directory
- do not accept loose aliases such as `"all"`
- keep explicit concrete `session_id` handling for exact single-session targeting inside the current project
- update docs and tool descriptions so the distinction between omitted `session_id`, `"*"`, and a concrete UUID is obvious

This keeps the contract explicit without expanding the tool surface.

**Alternative considered:** add a separate `history_search_project` or `history_recall_project` tool. Rejected: unnecessary tool-surface expansion for behavior that can be expressed clearly with one explicit magic value.

### Make `history_recall` return session identity for every recalled message

`history_recall` should return `session_id` with each recalled message row.

Normative direction:

- add `session_id` to the `RecalledMessage` shape in `crates/themion-core/src/db.rs`
- include `session_id` in the SQL select list for recall queries
- include `session_id` in the JSON returned by the tool layer
- update docs and examples so cross-session recall via `session_id="*"` is visibly supported rather than implicit

This makes cross-session recall safe to use because callers can distinguish messages from different sessions.

**Alternative considered:** add only a session-level wrapper around grouped messages rather than per-row `session_id`. Rejected: the current flat result shape is simpler, and per-row identity is enough for correct interpretation.

### Define stable ordering for project-wide recall

When recall spans multiple sessions via `session_id="*"`, ordering should remain predictable.

Normative direction:

- for `direction="newest"`, order by session recency first and then message position within that session from newest to oldest
- for `direction="oldest"`, order by session age first and then message position within that session from oldest to newest
- if the implementation relies on existing persisted timestamps or monotonic message identifiers to express session recency, the docs should state the chosen basis clearly enough for maintainers
- the order must be stable enough that repeated calls with the same data return the same sequence

The exact SQL expression may follow current schema realities, but the resulting behavior should be deterministic and understandable.

**Alternative considered:** sort only by `turn_seq` across all sessions. Rejected: `turn_seq` is only monotonic within one session and does not define a meaningful global order.

### Keep `history_search` current-session by default and explicit for project-wide mode

`history_search` should stay simple: current session by default, all sessions in the current project only when `session_id="*"`.

Normative direction:

- preserve explicit concrete `session_id` filtering when requested
- when `session_id` is omitted, search only the current session
- when `session_id="*"`, search all sessions for the current project
- keep returning `session_id`, `turn_seq`, `role`, and `snippet`
- update prompt-visible descriptions and runtime docs so the model understands that project-wide search is available but not implicit

This keeps search aligned with the same safety model as recall.

**Alternative considered:** make search broader than recall by default. Rejected: recall and search should share the same scoping rules to avoid confusion.

### Update recall hint wording to describe the safe default and explicit broadening path

The synthetic recall hint injected into the prompt window should reflect the safer memory model.

Normative direction:

- when older turns exist in the current session, keep the current-session hint useful
- expand the wording so it also reminds the model that persistent history tools can search or recall across prior sessions in the same project only when it explicitly passes `session_id="*"`
- avoid merging this behavior into the base system prompt; it should remain part of the existing contextual runtime hinting path

This makes the model more likely to use the tools as intended without silently broadening scope.

**Alternative considered:** leave the hint unchanged and rely only on updated tool schemas. Rejected: the recall hint is exactly where the model is told older context exists, so it should describe the broader retrieval surface truthfully.

### Keep the schema additive and backward compatible where practical

The history tool change should remain targeted.

Normative direction:

- keep the existing `history_recall` and `history_search` tool names
- keep `session_id` optional
- add `session_id` to recall results rather than changing existing field meanings
- remove caller-facing `project_dir` parameters from these tool schemas as part of the safety tightening
- preserve current result-limit bounds unless a separate PRD changes them

This keeps the memory improvement small while tightening scope control.

**Alternative considered:** replace `history_recall` with a more complex grouped-history API. Rejected: too much surface change for a targeted memory improvement.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/db.rs` | Update recall and search query handling so all history access is bounded to the current project, omitted `session_id` targets the current session, `session_id="*"` enables project-wide cross-session behavior, and recall includes `session_id`. |
| `crates/themion-core/src/tools.rs` | Remove caller-facing `project_dir` parameters from `history_recall` and `history_search` and update tool descriptions to explain omitted `session_id`, `session_id="*"`, and explicit UUID behavior. |
| `crates/themion-core/src/tools.rs` | Return `session_id` in `history_recall` JSON payloads. |
| `crates/themion-core/src/agent.rs` | Update the synthetic recall hint so it mentions current-session default behavior and explicit cross-session recall/search via `session_id="*"` within the same project. |
| `docs/architecture.md` | Update persistent-history/tooling documentation to describe current-project-only scoping and explicit `session_id="*"` cross-session behavior. |
| `docs/engine-runtime.md` | Clarify history tool semantics, current-session defaulting, project-only scope, and cross-session result interpretation. |
| `docs/README.md` | Update the PRD title if needed and keep the PRD table entry aligned with the revised scope. |

## Edge Cases

- `history_recall` is called without `session_id` → verify: only messages from the current session are returned.
- `history_search` is called without `session_id` → verify: only hits from the current session are returned.
- `history_recall` is called with `session_id="*"` in a project that has many past sessions → verify: messages can come from multiple sessions in the current project and each row includes `session_id` so they are distinguishable.
- `history_search` is called with `session_id="*"` in a project with multiple sessions containing the same keyword → verify: hits can include multiple sessions in the current project and each hit reports its `session_id`.
- `history_recall` is called with explicit `session_id` for one known session in the current project → verify: only that session's messages are returned.
- `history_search` is called with explicit `session_id` for one known session in the current project → verify: only that session's hits are returned.
- `history_recall` or `history_search` is called with explicit `session_id` for a session stored under another project → verify: no results are returned rather than crossing the project boundary.
- multiple sessions share the same `turn_seq` values → verify: project-wide recall via `session_id="*"` remains interpretable because ordering does not rely only on `turn_seq` and the payload includes `session_id`.
- `history_search` runs when FTS5 is unavailable → verify: existing empty-result fallback behavior remains unchanged.
- `history_recall` requests `direction="newest"` across many sessions with `session_id="*"` → verify: ordering is deterministic and not dependent on undefined SQLite row order.
- a caller attempts to pass `session_id="all"` or another alias → verify: it is rejected or treated as a non-matching concrete value rather than accepted as project-wide mode.

## Migration

This is an additive behavior change plus a safety tightening for existing history tools.

Expected rollout shape:

- keep existing tool names
- remove caller-visible `project_dir` parameters
- make current-session behavior the explicit default when `session_id` is omitted
- use `session_id="*"` as the explicit project-wide cross-session mode
- add `session_id` to recall results so cross-session responses are self-describing
- avoid schema churn unless implementation needs a small additive index or query-support column

No user-facing config migration is required.

## Testing

- create two or more sessions under the same current project, then call `history_recall` without `session_id` → verify: only current-session messages are returned.
- create two or more sessions under the same current project, then call `history_search` without `session_id` → verify: only current-session hits are returned.
- create two or more sessions under the same current project, then call `history_recall` with `session_id="*"` → verify: results can include messages from multiple sessions and every row includes `session_id`.
- create two or more sessions under the same current project, then call `history_search` with `session_id="*"` → verify: hits are returned across those sessions rather than only from the active session.
- call `history_recall` with explicit `session_id` for one known current-project session → verify: only that session's messages are returned.
- call `history_search` with explicit `session_id` for one known current-project session → verify: only that session's hits are returned.
- call either tool with explicit `session_id` for a session from a different project → verify: no results are returned.
- inspect the synthetic recall hint during a windowed session → verify: it mentions current-session default behavior and explicit project-wide recall/search via `session_id="*"`.
- run `cargo check -p themion-core` after implementation → verify: the history-tool changes compile cleanly.
- run targeted history/db tests if present, or add them near the touched code → verify: current-session defaulting, project-only scoping, and explicit `session_id="*"` behavior are covered automatically.

## Implementation checklist

- [ ] remove caller-facing `project_dir` parameters from `history_recall` and `history_search`
- [ ] default omitted `session_id` to the current session
- [ ] support `session_id="*"` as explicit project-wide cross-session mode
- [ ] keep all history queries bounded to the caller's current project
- [ ] update `history_recall` result shape to include `session_id`
- [ ] ensure recall ordering across multiple sessions is deterministic and documented
- [ ] update the synthetic recall hint in `agent.rs` to mention current-session defaulting and explicit `session_id="*"` behavior
- [ ] add or update tests for same-session default and project-wide `session_id="*"` recall/search behavior
- [ ] update `docs/architecture.md` and `docs/engine-runtime.md`
- [ ] update `docs/README.md` with the revised PRD title/status entry if needed
