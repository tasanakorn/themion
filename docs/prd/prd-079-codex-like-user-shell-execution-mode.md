# PRD-079: Use the User's Shell for `shell_run_command`

- **Status:** Implemented
- **Version:** v0.51.0
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-30

## Summary

- Themion currently runs `shell_run_command` through fixed `sh -c <command>` execution.
- That often diverges from the user's real terminal environment, especially for `zsh`-based setups with startup-managed environment behavior.
- This PRD makes `shell_run_command` use the user's configured shell instead of always forcing `sh`.
- The change should stay minimal: resolve the user shell, build the right command argv for that shell, and keep the existing bounded shell-tool behavior.
- Broader process-model upgrades such as streaming output, sandboxing, or approval flows are intentionally out of scope.

## Goals

- Make `shell_run_command` execute through the user's configured shell instead of hardcoded `sh`.
- Improve compatibility with shell-managed user environments such as `zsh` startup behavior and tools like `direnv`.
- Keep the implementation small and focused on shell selection plus argv construction.
- Preserve existing timeout, cwd, and bounded-output behavior.

## Non-goals

- No broader redesign of shell subprocess lifecycle behavior in this PRD.
- No approval system, sandbox wrapper, or shell snapshot behavior.
- No requirement to add interactive shell support or TTY attachment.
- No redesign of unrelated tools, CLI flows, or config systems.
- No requirement to add a separate execution mode matrix if a direct replacement is sufficient.

## Background & Motivation

### Current state

Themion's current shell tool path uses `sh -c <command>` from `crates/themion-core/src/tools.rs`. That is simple, but it ignores the user's real shell choice.

For users with richer shell-managed environments, especially `zsh`, this creates a common mismatch:

- the command works in the user's terminal
- the same command behaves differently inside Themion

The difference is often not the inherited process environment alone. It is the shell itself and the startup behavior associated with that shell.

### Why this should change

Users increasingly expect agent shell commands to behave like commands run in their own terminal. For this request, the desired change is narrow: use the user's shell rather than generic `sh`.

This PRD intentionally avoids turning that request into a larger shell-runtime redesign.

## Design

### 1. Replace fixed `sh -c` with user-shell execution

`shell_run_command` should resolve the user's configured shell and execute through that shell instead of always forcing `sh`.

Required behavior:

- on supported platforms, Themion should detect the user's configured shell
- `shell_run_command` should execute the command through that shell rather than through `/bin/sh`
- on Unix-like systems, Themion should prefer `['<user-shell>', '-lc', '<command>']`
- if login-shell execution is explicitly disabled in later work, Themion may use `['<user-shell>', '-c', '<command>']` instead
- if the user shell cannot be resolved, Themion should fall back to `sh -c <command>`
- the user-facing tool name and arguments should remain unchanged

This is the core product requirement.

**Alternative considered:** add a new separate shell tool for user-shell behavior. Rejected: the user asked for the normal shell tool to use the user's shell, not for a parallel feature surface.

### 2. Build shell-aware argv instead of assuming POSIX `sh`

Themion should choose command arguments appropriate for the resolved shell rather than assuming one fixed `sh -c` form everywhere.

Required behavior:

- `themion-core` should resolve the shell executable and construct the argv used to run the command
- on Unix-like systems, the current implementation resolves the user shell path and uses login-shell execution with `-lc`, which cleanly covers common shells such as `zsh`, `bash`, and `sh` in this release
- shell-specific argv logic should live in a small helper rather than being inlined awkwardly inside tool-dispatch code
- Windows-specific command forms should be handled in the helper where applicable, while this release keeps Unix behavior intentionally minimal around `-lc`

This keeps the implementation small while still avoiding a hardcoded `sh` assumption.

**Alternative considered:** replace only the executable path and keep all logic inline in the tool handler. Rejected: even this minimal change is cleaner if shell resolution and argv construction are separated from the tool handler.

### 3. Preserve existing bounded tool behavior

This PRD should change shell choice, not the overall shell tool contract.

Required behavior:

- existing timeout behavior should remain in place
- existing working-directory handling should remain in place
- existing bounded combined stdout/stderr handling should remain in place
- docs should describe that `shell_run_command` now uses the user's shell when available, prefers login-shell execution on Unix, and falls back to `sh` otherwise

This keeps the change focused and lowers regression risk.

**Alternative considered:** expand the PRD to also redesign streaming, cancellation, or subprocess policy. Rejected: those are separate improvements and would make this request larger than needed.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Replace direct fixed `sh -c` spawning with user-shell resolution plus shell-aware argv construction, while keeping existing timeout/cwd/output behavior. |
| `crates/themion-core/src/` small helper module or helper functions | Add minimal user-shell resolution and argv construction support. |
| `docs/engine-runtime.md` | Update shell tool documentation to state that `shell_run_command` uses the user's shell when available, prefers login-shell execution on Unix, and falls back to `sh` otherwise. |
| `docs/architecture.md` | Update architecture/tooling notes so shell execution no longer claims a fixed `sh -c` path. |
| `docs/README.md` | Track the PRD entry and status. |

## Edge Cases

- the user shell is `zsh` and startup-managed environment affects command behavior → verify: Themion executes through `zsh -lc` rather than `sh`.
- the user shell is `bash` → verify: Themion executes through `bash -lc` rather than `sh`.
- the resolved shell is PowerShell or `cmd` on Windows → verify: helper-selected `-Command` or `/c` execution is applied appropriately.
- the user shell cannot be resolved → verify: Themion falls back to `sh -c` cleanly.
- the command times out → verify: existing timeout behavior still applies.
- the command produces large output → verify: existing truncation and bounded-result behavior still applies.

## Migration

This feature requires no database migration.

Rollout guidance:

- keep the same `shell_run_command` tool surface
- change only the shell used to execute the command when a user shell can be resolved
- prefer login-shell execution on Unix-like systems
- preserve current fallback and bounded-tool behavior otherwise

## Testing

- run `shell_run_command` in an environment with a user shell of `zsh` → verify: the command executes through `zsh -lc` rather than `sh`.
- run `shell_run_command` in an environment with a user shell of `bash` → verify: the command executes through `bash -lc` rather than `sh`.
- run `shell_run_command` with PowerShell or `cmd` on Windows → verify: Themion uses the helper-selected command form for that shell.
- make the user shell unavailable and run `shell_run_command` → verify: Themion falls back to `sh -c`.
- run a command that exceeds the configured timeout → verify: timeout behavior remains unchanged.
- run a command that emits output beyond `result_limit` → verify: truncation behavior remains unchanged.
- run `cargo check -p themion-core` after implementation → verify: the touched crate builds cleanly.
- run `cargo check -p themion-core --all-features` after implementation → verify: the touched crate still builds cleanly with all features enabled.

## Implementation checklist

- [x] resolve the user's shell for `shell_run_command`
- [x] replace fixed `sh -c` execution with shell-aware user-shell argv construction
- [x] prefer `-lc` on Unix-like systems and allow shell-specific argv handling where needed
- [x] keep fallback to `sh -c` when the user shell is unavailable
- [x] preserve current timeout, cwd, and bounded-output behavior
- [x] update runtime and architecture docs to describe the new shell behavior
