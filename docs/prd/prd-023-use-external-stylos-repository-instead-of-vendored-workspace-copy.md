# PRD-023: Use External Stylos Repository Instead of Vendored Workspace Copy

- **Status:** Implemented
- **Version:** v0.13.0
- **Scope:** workspace, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-20

## Goals

- Replace the current vendored Stylos crates under `vendor/stylos/` with dependencies sourced from the dedicated Stylos repository at `https://github.com/tasanakorn/stylos.git`.
- Remove the local Stylos workspace copy from Themion so Stylos evolves in its own repository instead of being maintained as embedded source.
- Keep Themion's existing Stylos feature shape intact from the perspective of Themion code, limiting the change to dependency sourcing and workspace structure where practical.
- Document the migration clearly so future Stylos-related work in Themion assumes the external repository as the source of truth.

## Non-goals

- No redesign of Themion's Stylos runtime behavior, queryables, status payloads, or tool contracts in this PRD.
- No attempt to rename Stylos crates or collapse them behind a new Themion-local wrapper crate.
- No requirement to publish Stylos crates to crates.io as part of this change.
- No unrelated refactor of Themion's workspace layout beyond what is needed to stop vendoring Stylos.
- No change to non-Stylos builds except the dependency graph adjustments required by removing the vendored copy.

## Background & Motivation

### Current state

Themion previously carried a local vendored Stylos workspace copy under `vendor/stylos/`, and the root workspace included those crates directly:

- `vendor/stylos/stylos-common`
- `vendor/stylos/stylos-identity`
- `vendor/stylos/stylos-config`
- `vendor/stylos/stylos-transport`
- `vendor/stylos/stylos-session`

This made early Stylos integration easy while the design was still moving quickly, but Stylos has now been redesigned and moved into its own repository: `https://github.com/tasanakorn/stylos.git`.

That means the vendored copy in Themion is no longer the right source of truth. Continuing to keep a local duplicate creates several risks:

- Themion may silently drift from the real Stylos implementation.
- Stylos fixes may need to be applied in two places.
- Workspace membership in Themion incorrectly suggests Stylos is still developed as part of this repository.
- Future contributors may edit the vendored copy here and assume those edits are canonical.

### Why switch to a git dependency first

The new source of truth is a separate git repository, not a published crates.io release line called out in the request.

Using git-sourced Cargo dependencies is therefore the narrowest change that matches the user's intent:

- Themion can consume the real Stylos repository directly.
- the exact Stylos revision can still be pinned for reproducible builds.
- the vendored source tree can be deleted from Themion.
- no Stylos publishing workflow is required to complete this migration.

Because this change removes the embedded workspace copy and changes the dependency source for an existing feature area, it is treated as a minor release target.

**Alternative considered:** wait to switch until Stylos crates are published on crates.io. Rejected: the request explicitly names the git repository as the new source, and delaying would keep the stale vendored copy alive longer.

### Why remove vendored crates from the workspace entirely

Leaving the vendored Stylos crates in the workspace while also introducing external git dependencies would create an ambiguous and fragile setup.

A clean migration should make one source canonical. In this case that should be the external Stylos repository. Themion's workspace should therefore stop listing `vendor/stylos/*` members once the consuming crates have been moved to git dependencies.

**Alternative considered:** keep the vendored tree as an unused fallback. Rejected: it invites accidental edits, duplicate dependency resolution confusion, and stale documentation.

## Design

### Source Stylos crates from the external git repository

Themion now stops depending on Stylos crates via local `path` dependencies and instead consumes them from `https://github.com/tasanakorn/stylos.git`.

Implemented behavior:

- Themion's shared workspace dependency for `stylos` now points at the external Stylos git repository.
- the dependency declaration is pinned to a specific git revision for reproducible builds.
- Stylos-enabled code resolves from a single external repository revision.
- dependency declarations remain feature-gated where they were already feature-gated.

This keeps Themion consuming Stylos as an external product while minimizing code churn.

**Alternative considered:** create a Themion-local facade crate that re-exports Stylos dependencies. Rejected: unnecessary indirection for a straightforward source-of-truth migration.

### Remove vendored Stylos crates from workspace membership

The root `Cargo.toml` workspace membership no longer includes vendored Stylos crates.

Implemented behavior:

- the root workspace now includes only `crates/themion-core` and `crates/themion-cli`
- the vendored Stylos crates are no longer workspace members
- the rest of the Themion workspace layout remains unchanged

This ensures `cargo metadata`, workspace-wide checks, and contributor expectations reflect reality after the migration.

**Alternative considered:** keep the crates as excluded or dormant members. Rejected: even dormant copies encourage confusion about which repo should receive Stylos changes.

### Delete the local vendored Stylos source tree after dependency migration

Once Cargo manifests no longer pointed into `vendor/stylos/`, the vendored source tree was removed from the repository.

Implemented behavior:

- the live `vendor/stylos/` crate sources were removed after the dependency migration
- no active build path depends on the removed vendored copy
- docs now treat the external repository as the source of truth

This completes the migration cleanly instead of leaving dead source behind.

**Alternative considered:** keep the directory for historical reference. Rejected: git history already preserves that reference without burdening the live tree.

### Keep Themion's Stylos feature behavior stable

This PRD changes where Stylos crates come from, not how Themion exposes Stylos behavior.

Implemented behavior:

- the `stylos` cargo feature in `themion-cli` continues to gate Stylos support
- existing Stylos queryables, status publication, and injected tool behavior remain functionally unchanged for Themion callers
- no broader Stylos protocol redesign was folded into this migration

This keeps the migration understandable and reviewable as a dependency-source change rather than a protocol redesign.

**Alternative considered:** combine the repository migration with a broader Stylos API rewrite inside Themion. Rejected: too much scope for one change and harder to validate safely.

### Update documentation to treat the external Stylos repo as authoritative

Themion docs should no longer imply that Stylos source lives inside this repository.

Implemented behavior:

- this PRD now records the external Stylos repository as authoritative
- the PRD index marks this migration as implemented
- contributor expectations now align with Stylos living in its own repository and being consumed by Themion as an external dependency

This reduces future confusion for contributors working on Stylos-related code.

**Alternative considered:** leave docs implicit and let Cargo manifests speak for themselves. Rejected: contributor-facing docs should make repo boundaries obvious.

## Changes by Component

| File | Change |
| ---- | ------ |
| `Cargo.toml` | Removed vendored Stylos crates from workspace `members` and pinned the shared `stylos` dependency to the external GitHub repository over HTTPS. |
| `crates/themion-cli/Cargo.toml` | Continues to consume optional Stylos support through the workspace dependency, preserving existing feature gating. |
| `vendor/stylos/` | Removed the vendored Stylos source tree. |
| `docs/README.md` | Added this PRD to the PRD index and marked it implemented. |
| `docs/prd/prd-023-use-external-stylos-repository-instead-of-vendored-workspace-copy.md` | Updated status and implementation notes to reflect the landed migration. |

## Edge Cases

- the external Stylos repository layout does not exactly match the old vendored crate paths → Themion should update dependency declarations to the new crate/package layout without reintroducing a local copy.
- the external Stylos redesign changed crate names or features → Themion should make the smallest compatible manifest and code updates needed, and document the compatibility delta in implementation notes.
- SSH-based git dependency access is unavailable in some build environments → Themion now uses an equivalent HTTPS repository URL supported by Cargo for the same repo.
- one Themion crate resolves Stylos from git while another still references `vendor/stylos/` → this mixed-source state should be treated as incomplete migration and not left behind.
- removing vendored workspace members reveals lockfile or feature-resolution changes → those changes are expected only insofar as they follow from consuming Stylos externally and should remain scoped to the migration.
- non-Stylos builds run in environments without access to the Stylos repository → they should continue to compile as long as the `stylos` feature remains disabled and manifests keep Stylos optional where they are optional today.

## Migration

This is a source-of-truth and dependency-management migration.

Migration expectations:

- Themion stops treating Stylos crates as in-repo workspace members.
- Stylos-enabled builds fetch Stylos from `https://github.com/tasanakorn/stylos.git` instead of reading `vendor/stylos/`.
- the repository no longer carries a live vendored Stylos source tree.
- future Stylos implementation work should happen in the Stylos repository, while Themion keeps only integration code and dependency references.
- if a pinned git revision is used, updating Stylos in Themion becomes an explicit dependency bump rather than an in-place vendor edit.

This is expected to be a backward-compatible integration change for Themion users, though contributor workflow changes because Stylos development moves to the external repository.

## Testing

- update Cargo manifests to consume Stylos from the external git repository and remove vendored workspace members → verify: `cargo metadata` resolves without references to `vendor/stylos` workspace packages.
- run `cargo check -p themion-cli` after the migration → verify: non-Stylos builds still compile cleanly.
- run `cargo check -p themion-cli --features stylos` after the migration → verify: Stylos-enabled CLI builds resolve and compile against the external Stylos repository.
- run `cargo build -p themion-cli --features stylos` after the migration → verify: Stylos-enabled CLI links successfully against the external Stylos repository.
- search the repository for `vendor/stylos` after manifest and doc updates → verify: only intentional historical references remain, if any.
- inspect the repository tree after cleanup → verify: the live `vendor/stylos/` source tree has been removed.
- review relevant docs after updates → verify: they no longer imply Stylos source is maintained inside Themion.
