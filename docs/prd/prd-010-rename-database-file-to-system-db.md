# PRD-010: Rename Persistent Database File from `history.db` to `system.db`

- **Status:** Implemented
- **Version:** v0.5.2
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Rename Themion's persistent SQLite database file from `history.db` to `system.db`.
- Align the filename with the database's broader role as system state storage, not only conversation history.
- Preserve existing persisted user data by defining a clear migration path from the old filename to the new one.
- Keep the change narrow: update path resolution, migration behavior, and documentation without redesigning database contents.

## Non-goals

- No schema redesign or table renaming in this PRD.
- No change to the logical contents of the SQLite database beyond its on-disk filename.
- No change to XDG directory placement; the database should remain under `$XDG_DATA_HOME/themion/`.
- No change to auth/config file locations.
- No removal of existing history, workflow, or session features.

## Background & Motivation

### Current state

Themion currently stores its SQLite database at:

- `$XDG_DATA_HOME/themion/history.db`
- typically `~/.local/share/themion/history.db`

This path is documented in runtime and architecture docs and is also hardcoded in CLI startup paths that open the shared database.

The database now stores more than conversation history. In addition to messages and turns, it also persists session metadata and workflow runtime state. As the system grows, `history.db` becomes an increasingly narrow and misleading name for a file that acts as Themion's broader local system database.

### Why `system.db` is a better fit

`system.db` better reflects the database's role as the local persistence layer for:

- conversation history
- session records
- workflow state
- future internal runtime metadata that may not be strictly “history”

This is primarily a naming and migration change, but it affects a user-visible filesystem contract and should therefore be specified explicitly.

### Why migration needs to be designed carefully

Changing only the path constant without a migration plan would make existing users appear to lose all prior session history after upgrade, because the new binary would create a fresh `system.db` and ignore the existing `history.db`.

Because this repository already documents persistent history as a user-facing feature, the rename should preserve existing data automatically when practical.

## Design

### Canonical database path

Themion should change its canonical SQLite database path from:

- `$XDG_DATA_HOME/themion/history.db`

to:

- `$XDG_DATA_HOME/themion/system.db`

This new filename should be used consistently in runtime code, startup wiring, and documentation.

**Alternative considered:** keep `history.db` and only update docs to describe the broader purpose. Rejected: that preserves a misleading user-visible contract and makes future system-state expansion harder to explain cleanly.

### Startup migration behavior

On database open, Themion should support a one-way filename migration with the following behavior:

1. If `system.db` already exists, use it as the source of truth.
2. Else if `history.db` exists and `system.db` does not, migrate the old database to `system.db` automatically.
3. Else if neither file exists, create a new `system.db`.

The migration should preserve the SQLite file contents rather than attempting row-by-row logical export/import.

Preferred implementation behavior:

- move or rename `history.db` to `system.db` when possible
- also handle SQLite sidecar files when present, especially WAL/SHM companions
- if direct rename is not practical on the host platform or current file state, fall back to a safe copy-based migration strategy
- after a successful migration, subsequent opens should use only `system.db`

`system.db` should become the only canonical live filename after the change lands.

**Alternative considered:** create `system.db` fresh and leave manual migration to the user. Rejected: that creates avoidable apparent data loss and breaks the expectation that persistent history survives upgrades.

### Conflict resolution when both files exist

If both `history.db` and `system.db` exist, Themion should treat `system.db` as canonical and should not attempt to merge the two files automatically.

Rationale:

- automatic merge semantics are ambiguous and risky
- `system.db` represents the post-migration canonical location
- silently combining two SQLite files could duplicate or corrupt user-visible history

In this case, Themion may log a concise warning indicating that `history.db` was ignored because `system.db` already exists.

**Alternative considered:** always prefer the newest file by modification time. Rejected: modification time is a weak signal for SQLite correctness and could cause surprising source-of-truth changes.

### Sidecar-file handling

SQLite deployments using WAL mode may involve related files such as:

- `history.db-wal`
- `history.db-shm`

Migration logic should account for these files so the renamed database remains consistent.

Preferred behavior:

- if a rename-based migration is used, migrate matching sidecar files alongside the main database when they exist
- if a copy-based migration is used, ensure the copied database is opened in a consistent state before treating migration as successful
- do not leave behind mismatched `history.db`/`system.db` sidecar files that could confuse later debugging

**Alternative considered:** ignore sidecar files and rename only the main DB file. Rejected: WAL-mode SQLite state may be incomplete or misleading if related files are not handled carefully.

### Documentation updates

All docs that mention the database path should be updated from `history.db` to `system.db` once implementation lands.

This includes at minimum:

- `docs/engine-runtime.md`
- `docs/architecture.md`
- any future PRD implementation notes that describe the canonical database path

Historical PRDs that describe older shipped behavior should generally remain unchanged, except where the repository's PRD policy allows status or implementation-note updates to reflect what landed.

**Alternative considered:** rewrite older implemented PRDs to pretend `system.db` was always the filename. Rejected: implemented PRDs are historical records and should not be rewritten to erase prior behavior.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/db.rs` | Added shared migration-aware open logic that treats `system.db` as canonical, migrates `history.db` automatically when needed, handles `-wal`/`-shm` sidecars, and warns when both old and new files exist. |
| `crates/themion-cli/src/main.rs` | Switched non-interactive startup to the shared default database opener so command-line runs use `themion/system.db` with migration support. |
| `crates/themion-cli/src/tui.rs` | Switched interactive startup to the shared default database opener so TUI sessions use `themion/system.db` with the same migration behavior. |
| `docs/engine-runtime.md` | Updated the documented canonical database path and related runtime wording to `system.db`. |
| `docs/architecture.md` | Updated startup and persistence path references to `system.db`. |
| `docs/README.md` | Marked this PRD implemented and kept the docs index aligned. |

## Edge Cases

- Existing user has only `history.db` → migrate automatically and preserve prior data.
- Existing user has neither file → create `system.db` on first use with no user-visible migration prompt.
- Existing user has both `history.db` and `system.db` → use `system.db` and avoid implicit merge behavior.
- `history.db` exists but sidecar files are also present → migrate them consistently so WAL-mode data is not stranded.
- Migration rename fails because of platform or filesystem constraints → fall back to a safe copy-based migration or return a clear error instead of silently starting with an empty DB.
- Migration is interrupted mid-upgrade → the next startup should avoid destroying whichever source file still contains the valid data.
- User downgrades to an older binary after migration → old binaries may not see `system.db`; this downgrade behavior should be documented as a compatibility caveat.

## Migration

This change is a filesystem-level persistence migration.

Upgrade expectations:

- first launch after upgrade automatically adopts `system.db` as the canonical filename
- if only `history.db` exists, Themion now migrates it automatically
- if migration succeeds, future launches use `system.db`
- if both files exist, Themion prefers `system.db` and leaves `history.db` untouched while warning that the legacy file was ignored

Downgrade expectations:

- older binaries that still look only for `history.db` may not see data after the rename unless the user manually renames `system.db` back
- this is acceptable if documented, because the main forward path preserves data for upgraded users

No schema migration is required solely for the filename rename.

## Testing

- start with no database file present → verify: Themion creates `$XDG_DATA_HOME/themion/system.db` and does not create `history.db`.
- start with only `history.db` present → verify: startup migrates data to `system.db` and existing sessions/history remain visible.
- start with `history.db`, `history.db-wal`, and `history.db-shm` present → verify: migration preserves a consistent usable database under the `system.db` filename set.
- start with both `history.db` and `system.db` present → verify: Themion opens `system.db` and does not silently merge databases.
- simulate migration failure during rename or copy → verify: Themion surfaces a clear error or safe fallback behavior rather than silently creating an empty replacement database.
- inspect runtime docs after implementation → verify: canonical path references use `system.db` consistently.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: path-resolution changes compile cleanly across CLI and core crates.
