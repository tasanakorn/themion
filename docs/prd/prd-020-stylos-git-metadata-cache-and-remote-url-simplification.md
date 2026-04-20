# PRD-020: Stylos Git Metadata Cache and Remote URL Simplification

- **Status:** Implemented
- **Version:** v0.10.1
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-20

## Goals

- Record the already-landed Stylos status-payload change that simplifies exported git remote metadata from per-remote fetch/push objects to a unique list of git remote URLs.
- Reduce repeated synchronous git command execution during TUI-driven Stylos snapshot refreshes by moving git inspection to a startup-initialized cache with a 30-second refresh TTL.
- Preserve the existing Stylos startup behavior of detecting whether the current project directory is a git repository and including git metadata in status payloads when available.
- Keep the implementation scoped to `themion-cli` because Stylos status publishing and local project inspection are CLI-local runtime concerns.

## Non-goals

- No change to Themion's provider, workflow, history, or shell-command behavior.
- No introduction of a continuous polling loop dedicated solely to git inspection.
- No expansion of Stylos git metadata to include branch state, dirty state, commit hashes, or other repository details.
- No attempt to normalize or rewrite remote URL formats; the exported values remain the git-reported URLs.

## Background & Motivation

### Current state

PRD-019 added basic optional Stylos support in `themion-cli`, including a periodic status payload that reports local session details such as project directory, workflow state, activity status, and git metadata.

In the first shipped form, git metadata was gathered by running synchronous git commands during status-snapshot refresh setup in the TUI. The payload exported remotes as structured objects containing:

- remote name
- fetch URL
- push URL

That shape was more detailed than current consumers needed, and it caused `refresh_stylos_status()` to repeat git inspection on activity and workflow-driven refresh paths even though the published status already emits on a fixed interval.

### Why simplify and cache

The landed change reflects two practical observations:

- for current Stylos presence/reporting use, the unique git URLs matter more than the git remote names or separate fetch/push slots
- repeatedly shelling out to `git` on every TUI-side status refresh trigger is unnecessary churn when repository metadata changes relatively rarely during a session

A small cache initialized at startup and refreshed on a modest TTL keeps the payload useful while reducing overhead and simplifying downstream consumption.

## Design

### Export only unique git remote URLs

The Stylos status payload should expose `git_remotes` as `Vec<String>` containing unique remote URLs for the current repository.

Normative behavior:

- only repositories detected as git work trees export remote URLs
- duplicate URLs are collapsed so identical fetch/push entries do not produce repeated payload values
- ordering should be stable enough for deterministic snapshots by collecting through an ordered set
- remote names are not exported
- separate fetch/push fields are not exported

Example payload shape:

```json
"git_remotes": [
  "git@github.com:tasanakorn/themion.git"
]
```

**Alternative considered:** keep the existing `{ name, fetch, push }` structure and let consumers simplify it themselves. Rejected: it exported more structure than current use required and made the status payload noisier than necessary.

### Detect git state at startup and refresh through a 30-second TTL cache

Git repository inspection should run once during Stylos startup to seed the initial status snapshot, then be reused through a small cache object owned by the CLI runtime.

Normative behavior:

- cache initialization performs the initial git repository and remote inspection
- cache reads return the most recent snapshot immediately
- if the cached value is older than 30 seconds, the next read refreshes it synchronously before returning
- no dedicated background git polling task is added
- if the project directory is not a git repository, the cached snapshot reports `project_dir_is_git_repo = false` and an empty remote list

This preserves startup detection while ensuring later repository or remote changes can still appear during a long-running session without paying the git-command cost on every UI-driven refresh event.

**Alternative considered:** refresh git metadata on every Stylos status publish tick. Rejected: simpler than ad hoc TUI refreshes, but still more eager than needed for metadata that changes infrequently.

### Keep status publication behavior otherwise unchanged

The Stylos status publisher should continue emitting on the existing periodic interval and continue carrying the same surrounding status fields such as workflow, activity, provider, model, and profile.

Only the git-metadata acquisition path and payload shape change.

This keeps the behavioral delta narrow and avoids conflating a metadata simplification with broader Stylos protocol redesign.

**Alternative considered:** move all status snapshot assembly into the periodic publisher task. Rejected: unnecessary for this targeted change and broader than the performance/shape problem being addressed.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/stylos.rs` | Replace structured remote export with unique remote URL strings, add a startup-seeded git-status cache with a 30-second TTL, and preserve initial fallback snapshot behavior. |
| `crates/themion-cli/src/tui.rs` | Stop running direct git inspection on every `refresh_stylos_status()` call and instead read git state through the shared cache when building Stylos snapshots. |
| `docs/README.md` | Add this PRD to the PRD index as the historical record for the landed change. |

## Edge Cases

- repository has identical fetch and push URLs for one or more remotes → verifyable behavior should export each URL only once.
- repository has multiple remotes with distinct URLs → all unique URLs should appear in the payload.
- repository remotes change while Themion is running → the exported URLs should update on the next cache read after the 30-second TTL expires.
- repository is removed or the working directory stops being a valid git work tree during the session → a later cache refresh should degrade to `project_dir_is_git_repo = false` with an empty remote list.
- `git` is unavailable or git commands fail during refresh → the cache should degrade safely to non-repo/empty-remote output rather than breaking Stylos status publication.
- an external consumer still expects the old object-based `git_remotes` shape → that consumer must be updated because this is a wire-shape simplification in the exported Stylos status payload.

## Migration

This is a small implemented patch-level change to the Stylos status payload.

Runtime migration behavior:

- feature-disabled builds are unaffected
- feature-enabled builds continue to publish git metadata, but in the simplified `Vec<String>` form
- consumers of Stylos status payloads that previously parsed `{ name, fetch, push }` entries must migrate to string URLs

No database, profile, or provider migration is required.

## Testing

Implementation status: landed in code. Stylos git inspection now seeds from startup, refreshes through a 30-second TTL cache, and exports unique remote URL strings instead of structured remote objects.

- run feature-enabled Themion in a git repository with one remote whose fetch and push URLs are identical → verify: the Stylos status payload exports a single URL string, not duplicate entries.
- run feature-enabled Themion in a repository with multiple distinct remotes → verify: the payload exports the unique set of remote URLs.
- trigger repeated TUI status refresh paths within 30 seconds, such as activity changes and workflow updates → verify: git commands are not re-run on each refresh path and the cached snapshot is reused.
- change repository remotes while Themion remains running, then wait more than 30 seconds and trigger another status snapshot read → verify: the updated remote URL set appears in the payload.
- run feature-enabled Themion outside a git repository → verify: `project_dir_is_git_repo` is `false` and `git_remotes` is empty.
- make git inspection fail during refresh, such as by removing git from `PATH` in a controlled test environment → verify: Stylos status publication continues with safe fallback git metadata.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the CLI and Stylos integration compile cleanly with the new cache and payload shape.
