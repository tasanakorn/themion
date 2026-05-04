# PRD-099: Build Metadata Injection and Stylos Version Reporting

- **Status:** Implemented
- **Version:** v0.61.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-04

## Summary

- Themion can currently report a running instance's profile, provider, model, and workflow state, but it does not publish the actual app build identity that is running.
- Add build-time metadata injection for normal `cargo build` and `cargo run` flows so the binary knows its semantic version plus additional `app_version_hash` and `app_version_dirty` metadata captured at build time.
- Surface that metadata in runtime-owned instruction/context paths so the model can refer to the current running build identity without guessing from repository files.
- Publish the same metadata through Stylos status and health/query surfaces so one instance can truthfully report what another instance is running.
- Keep the source of truth build-time and runtime-owned rather than reconstructing version details separately in TUI or remote query clients.

## Goals

- Make every normal locally built Themion binary carry a compact build identity that preserves the existing `app_version` meaning while adding `app_version_hash` and `app_version_dirty` when git metadata is available.
- Ensure `cargo build` and `cargo run` produce the same injected runtime metadata path without requiring manual flags.
- Make the active running build identity available to runtime-owned instruction/context assembly.
- Publish the running build identity through Stylos status or health reporting so remote instance listing can include truthful version/build information.
- Include the same build identity in the local startup banner so operators can immediately see what build launched.
- Keep metadata ownership in runtime/app-state rather than TUI-local formatting or ad hoc shell probing.
- Degrade safely when git is unavailable or the source tree is not a git checkout.

## Non-goals

- No requirement to make release packaging depend on a specific external tool beyond ordinary `cargo` and an optional available `git` executable.
- No requirement to embed full diff content, branch names, tags, or commit messages in the runtime metadata.
- No requirement to make build-time dirty detection enumerate untracked files unless implementation explicitly chooses that policy and documents it.
- No requirement in this PRD to expose build metadata through every user-facing command surface immediately beyond startup/banner, runtime context, and Stylos status or health reporting.
- No requirement to make remote instances shell out on demand to answer version questions after startup.
- No change to core prompt precedence; injected build metadata must remain runtime/context data, not a replacement for system, developer, user, or `AGENTS.md` authority.

## Background & Motivation

### Current state

Themion already records some runtime attribution such as `app_version`, `profile`, `provider`, and `model` in turn metadata, and Stylos status currently publishes process-local agent/runtime state such as active profile, provider, model, workflow, and project directory. The existing `app_version` field already has consumers and meaning, so the safest extension path is to preserve it and add separate build-identity fields rather than reinterpret `app_version` itself.

However, the current live status path does not expose the actual running build identity in a way that lets another instance answer a basic question like "what Themion version are you running?" truthfully and directly. Today, a human or peer may be forced to infer from the repository checkout, ask the remote instance in chat, or inspect local manifests manually.

That is weaker than it should be because the product already has:

- runtime-owned app/session state
- distinct prompt/context injection paths
- Stylos-published process and agent status
- local turn metadata that already acknowledges version attribution as useful runtime truth

The missing piece is not a new product surface so much as a consistent source of build identity that is captured at compile time and then reused everywhere runtime status or instructions need it.

### Why this matters now

This matters for both debugging and coordination:

- when multiple local or remote instances are visible, the operator should be able to tell which build each one is actually running
- when a repository checkout is dirty, support/debugging should be able to distinguish a clean released build from an in-progress local build
- when the model reasons about runtime behavior, it is better if runtime-owned context can state the current build identity directly instead of leaving the model to infer from files that may not match the running binary
- startup output should make the launched build obvious before the operator even asks for status
- remote health/status queries should answer build-identity questions from published runtime truth rather than ad hoc chat questions or shell access

**Alternative considered:** infer version and git state dynamically at query time by reading manifests or shelling out from the querying side. Rejected: that can describe the checkout visible to the querier rather than the binary actually running, and it duplicates logic outside the runtime-owned source of truth.

## Design

### 1. Inject build metadata during ordinary Cargo builds

Themion should attach compact build identity metadata during normal compilation.

Required behavior:

- ordinary `cargo build` and `cargo run` flows should inject build metadata automatically without requiring special user flags
- the injected metadata should include at least:
  - semantic app version, preserved as the existing `app_version` meaning
  - `app_version_hash` derived from the current git commit when available
  - `app_version_dirty` derived from build-time dirty/clean checkout state
- implementation should use Cargo-supported build-time environment injection such as `build.rs` and `cargo:rustc-env`
- build-time git probing should emit correct Cargo rerun directives for relevant git state files so rebuilds refresh metadata when the checked-out commit or dirty state changes
- the implementation should prefer one canonical hash field for runtime/status publication; if a shorter display form is also useful, it should be derived for presentation rather than becoming the primary contract unless the implementation deliberately standardizes both
- if git metadata cannot be determined, the binary should still build successfully and should expose explicit unknown/unavailable values rather than fabricated ones

This makes the running binary the source of truth for its own build identity.

**Alternative considered:** compute git metadata lazily at runtime on first use. Rejected: that makes runtime behavior depend on later filesystem state and on whether the original checkout still exists, which can diverge from the binary that was actually built.

### 2. Define one runtime-owned build identity snapshot

Themion should materialize the injected metadata into one runtime-owned structure that other surfaces consume.

Required behavior:

- `themion-cli` startup should resolve the build metadata into a compact runtime-owned snapshot or equivalent shared state value
- the startup path should use that same snapshot to render a startup banner that includes `app_version` plus the additional build-identity fields when available
- that runtime-owned value should be the source consumed by local status rendering, turn attribution, instruction/context injection, and Stylos publication
- TUI, headless mode, and Stylos should not each reconstruct git/version information separately
- the snapshot should preserve unknown/unavailable states explicitly

This follows the repository architecture rule that shared runtime truth belongs in hub/app-state rather than in surfaces.

### 3. Inject build identity into runtime instruction/context assembly

The running build identity should be available to the model as runtime context when appropriate.

Required behavior:

- prompt/context assembly should include a compact runtime-owned build identity note when runtime metadata is already being injected
- the note should be informational rather than policy-bearing, for example identifying current `app_version`, `app_version_hash`, and `app_version_dirty` values
- the new metadata must remain separate from system/developer/user instruction authority and from repository-local `AGENTS.md` content
- the instruction/context wording should avoid encouraging the model to assume that build metadata is equivalent to repository cleanliness at the current moment after startup; it reflects build-time capture

This makes the running binary's identity inspectable inside the same runtime context system that already carries workflow and local role state.

**Alternative considered:** expose the metadata only through debug commands and not through model-visible runtime context. Rejected: model-visible runtime state is already part of Themion's architecture, and build identity is sometimes directly relevant to truthful self-description and debugging.

### 4. Publish build identity through Stylos status and health surfaces

Remote peers should be able to retrieve build identity through ordinary Stylos status/query paths.

Required behavior:

- Stylos-published instance or agent status should include the running build identity fields from the runtime-owned snapshot
- the published structured fields should include at least `app_version`, `app_version_hash`, and `app_version_dirty`, with explicit unknown/unavailable representation when needed
- remote status and health-oriented query surfaces should report the same build identity consistently
- when build metadata is unknown or unavailable, the published payload should still include explicit fields or values that preserve that fact rather than omitting the concept silently
- remote consumers should not need to ask the target instance in chat just to learn its version/build identity

This makes remote version reporting a first-class runtime status capability instead of a manual convention.

### 5. Keep version/build metadata compact and durable

The metadata should stay small, stable, and useful across local and remote reporting.

Required behavior:

- prefer a compact schema such as `app_version` + `app_version_hash` + `app_version_dirty` rather than a large build manifest blob
- if a human-readable combined string is useful, it should be derived from canonical structured fields rather than replace them
- existing uses of `app_version` should keep their current meaning and behavior; build identity expansion should happen through additional fields rather than by changing what `app_version` means
- if turn-level runtime metadata is extended, it should add new fields alongside `app_version` rather than replacing or reinterpreting it
- documentation should state clearly which fields are build-time captured and which are live runtime state

### 6. Prefer additive metadata and avoid database-shape pressure

The safest rollout is additive and should not require database schema churn.

Required behavior:

- preserve the existing `app_version` field semantics everywhere they already exist
- prefer adding `app_version_hash` and `app_version_dirty` as additional runtime/status fields instead of mutating existing version field behavior
- prefer not to require any database schema change for this feature
- if turn persistence needs the new metadata, prefer extending existing flexible metadata storage such as JSON or optional maps rather than introducing a new mandatory schema migration
- the implementation should avoid making local or remote version reporting depend on a database backfill

**Alternative considered:** replace `app_version` with a decorated combined version string such as `0.60.2+<hash>-dirty`. Rejected: that changes the meaning and compatibility expectations of an existing field when additive fields can carry the extra build identity more safely.

### 7. Documentation and guidance must reflect the new source of truth

The documented runtime and Stylos behavior should be updated alongside the new capability.

Required behavior:

- `docs/architecture.md` should describe the injected build metadata as part of runtime-owned status truth, startup banner output, and Stylos publication
- `docs/engine-runtime.md` should describe build identity as one of the runtime context inputs and note that it is build-time captured metadata
- if status/health examples or field descriptions exist, they should be updated so version/build identity reporting is visible there too
- the PRD table in `docs/README.md` should gain the new entry in the correct sorted position

### 8. Exact field contract

This PRD standardizes the additive build-identity field names so implementation, docs, and remote consumers use one contract.

Required behavior:

- `app_version` remains the existing semantic version string and keeps its current meaning
- `app_version_hash` is an additional string field representing the current build's git commit hash when available
- `app_version_dirty` is an additional boolean field representing whether the checkout was dirty at build time
- when git metadata is unavailable, `app_version_hash` should use a stable explicit placeholder such as `"unknown"` rather than disappearing or changing type
- when git metadata is unavailable, `app_version_dirty` should default to `false` and the accompanying `app_version_hash="unknown"` value is what signals unavailable git identity
- the implementation should prefer publishing one canonical hash value in structured status fields; if local display wants a shortened form, it should derive that from `app_version_hash` for presentation only

### 9. Exact dirty semantics

This PRD defines a narrow dirty-state contract so the field is predictable.

Required behavior:

- `app_version_dirty=true` means tracked repository content differed from `HEAD` at build time
- tracked dirty detection should include modified, deleted, added-to-index, and renamed tracked files that make the checkout non-clean for `git diff --quiet --ignore-submodules HEAD --` style checks
- untracked files should not by themselves set `app_version_dirty=true` in this implementation slice
- documentation should state that `app_version_dirty` reflects tracked dirty state at build time rather than a full current filesystem audit

**Alternative considered:** treat any untracked file as dirty. Rejected: that is more volatile for everyday local work and less stable as a build-identity signal unless the product explicitly wants that stronger definition later.

### 10. Exact startup banner expectation

The launched process should show build identity immediately in startup output.

Required behavior:

- the standard startup banner should include `app_version` and append build metadata when available
- the preferred banner form is `themion v<app_version> (<app_version_hash>[ dirty])`, for example `themion v0.61.0 (abc1234)` or `themion v0.61.0 (abc1234 dirty)`
- when `app_version_hash="unknown"`, the banner should still render predictably, for example `themion v0.61.0 (unknown)`
- startup banner formatting should consume the shared runtime-owned build identity snapshot rather than recomputing git state locally in the presentation layer

### 11. Exact Stylos status shape expectation

Remote consumers should receive the additive build-identity fields directly in published status.

Required behavior:

- each published agent status entry should include `app_version`, `app_version_hash`, and `app_version_dirty`
- those fields should sit alongside existing agent runtime fields such as provider, model, profile, workflow, and project metadata rather than replacing any current field
- the same field names and meanings should be used consistently anywhere Stylos status or health-style reporting exposes build identity
- remote listing tools should be able to display version/build identity without requiring chat follow-up or repository probing

Example agent status shape excerpt:

```json
{
  "agent_id": "master",
  "provider": "openai-codex",
  "model": "gpt-5.4",
  "app_version": "0.61.0",
  "app_version_hash": "abc1234",
  "app_version_dirty": true
}
```

### 12. No database migration requirement

This feature should remain runtime/build-time additive and should not require a database migration to be considered complete.

Required behavior:

- implementation should not add a new mandatory DB column or table just to support startup banner or Stylos status reporting
- existing `app_version` storage behavior should remain valid without backfill
- if turn metadata is extended, it should use an already-flexible metadata container rather than introducing a schema migration solely for these new fields

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-099-build-metadata-and-stylos-version-reporting.md` | Define the product requirement for build-time metadata injection, runtime instruction/context exposure, and Stylos status publication. |
| `docs/README.md` | Add the new PRD entry in sorted order. |
| `docs/architecture.md` | Document runtime-owned build identity metadata, startup banner exposure, and publication through shared status surfaces. |
| `docs/engine-runtime.md` | Document build metadata injection into runtime context/prompt assembly, exact field names, and build-time semantics. |
| `crates/themion-cli/build.rs` and/or workspace build wiring | Inject `app_version`, `app_version_hash`, and `app_version_dirty` into the compiled binary during ordinary Cargo builds. |
| `crates/themion-cli/src/app_state.rs` | Materialize one runtime-owned build identity snapshot that other CLI/runtime surfaces can consume, including startup banner rendering inputs. |
| `crates/themion-core/src/agent.rs` or nearby prompt-assembly code | Include compact build identity information in runtime context assembly where runtime-owned context is already injected. |
| `crates/themion-cli/src/stylos.rs` | Publish `app_version`, `app_version_hash`, and `app_version_dirty` through Stylos status and related health/query payloads. |
| startup banner and status/debug output surfaces | Show the same build identity locally using the shared runtime-owned snapshot rather than recomputation. |

## Edge Cases

- the user runs `cargo build` in a normal git checkout with no uncommitted changes → verify: the binary reports `app_version` plus `app_version_hash` and `app_version_dirty=false`.
- the user runs `cargo run` with local uncommitted changes → verify: the running app reports the same `app_version` plus build-time `app_version_hash` and `app_version_dirty=true`.
- the project is built from a source tree without a usable `.git` directory → verify: the build succeeds, reports `app_version_hash="unknown"`, and keeps `app_version_dirty=false`.
- `git` is not available on `PATH` during build → verify: the build succeeds, reports `app_version_hash="unknown"`, and keeps `app_version_dirty=false` instead of failing the whole build.
- a remote peer queries Stylos status from two instances built from different commits of the same semantic version → verify: the status payload distinguishes them by `app_version_hash` and `app_version_dirty` while preserving the same `app_version`.
- a new commit is checked out after a previous build and Cargo rebuilds the binary → verify: the injected SHA values update to the new commit rather than staying stale.
- a build is created from a dirty tracked checkout and the checkout later becomes clean or changes again → verify: runtime-reported dirty status still reflects the build-time captured state rather than the later filesystem state.
- the checkout has only untracked files and no tracked-file changes → verify: `app_version_dirty` remains `false` in this implementation slice.
- prompt/context assembly includes build metadata → verify: it appears as informational runtime context and does not merge with or override `AGENTS.md` or other higher-priority instruction sources.
- feature-disabled builds without Stylos still expose local build identity through the shared runtime-owned metadata path → verify: local runtime surfaces, including startup banner output, continue to work without transport-specific dependencies.

## Migration

This feature should not require a database migration.

Rollout guidance:

- add build-time metadata injection first so local binaries become the source of truth for build identity
- route startup banner, local runtime, and prompt/context consumers to the shared runtime-owned build identity snapshot
- extend Stylos status/health payloads to publish the same metadata rather than inventing a second remote-only representation
- avoid database-shape churn; if any persistence touches are needed, prefer additive use of existing flexible metadata storage
- update docs so local and remote version/build reporting expectations match the shipped runtime behavior

## Testing

- run `cargo build -p themion-cli` in a clean checkout → verify: the built binary carries `app_version`, `app_version_hash`, and `app_version_dirty=false` when git is available.
- run `cargo build -p themion-cli` in a dirty checkout → verify: the built binary carries `app_version`, `app_version_hash`, and `app_version_dirty=true`.
- run `cargo run -p themion-cli -- --headless` or equivalent local startup path → verify: the startup banner renders `themion v<app_version> (<app_version_hash>[ dirty])` from the shared snapshot and runtime-owned status/context can report the same build identity without shelling out again.
- inspect the model-visible runtime context after startup → verify: `app_version`, `app_version_hash`, and `app_version_dirty` appear as separate informational runtime context data rather than merged into `AGENTS.md` or policy instructions.
- inspect turn-level runtime metadata after a new turn if implementation extends that path → verify: stored `app_version` keeps its current meaning and any new build-identity fields are additive rather than behavior-changing.
- inspect the startup banner and query local debug/runtime inspection output after startup → verify: local surfaces show the shared build identity snapshot rather than recomputed git state and preserve the same `app_version`, `app_version_hash`, and `app_version_dirty` values.
- query Stylos status from another instance when the `stylos` feature is enabled → verify: the payload includes `app_version`, `app_version_hash`, and `app_version_dirty` from the running target instance.
- compare two simultaneously running instances from different commits → verify: remote listing can distinguish their build identities by `app_version_hash` even when `app_version` matches.
- run `cargo check -p themion-cli` after implementation → verify: the touched crate builds cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the touched crate builds cleanly with Stylos enabled.
- run `cargo check -p themion-cli --all-features` after implementation → verify: the touched crate builds cleanly across feature combinations.
- run `cargo check -p themion-core` and `cargo check -p themion-core --all-features` if prompt/runtime-core code changes are required there → verify: the touched core crate still builds cleanly in default and all-features configurations.

## Implementation checklist

- [x] add build-time version/git metadata injection for ordinary Cargo build and run flows, including `app_version_hash`, `app_version_dirty`, and correct git-sensitive rerun behavior
- [x] define one runtime-owned build identity snapshot for startup banner, local status, prompt/context, and Stylos publication
- [x] inject compact build identity metadata into runtime instruction/context assembly
- [x] publish `app_version`, `app_version_hash`, and `app_version_dirty` through Stylos status and health/query payloads
- [x] keep startup banner and local debug/status surfaces aligned with the shared build identity snapshot where practical
- [ ] update architecture/runtime docs and the PRD index to reflect the new behavior

## Implementation notes

- Landed in `v0.61.0`.
- Implemented build-time injection in `crates/themion-cli/build.rs` with additive `app_version_hash` and `app_version_dirty` fields while preserving existing `app_version` behavior.
- Startup now prints `themion v<app_version> (<app_version_hash>[ dirty])` on launch paths including `--help`.
- Runtime prompt context now includes build identity as a separate runtime-context message.
- Stylos status snapshots now publish `app_version`, `app_version_hash`, and `app_version_dirty` per agent.
- Turn metadata was extended additively inside existing JSON `meta` storage; no database migration was added.
- `docs/architecture.md` and `docs/engine-runtime.md` still need follow-through updates in a subsequent doc pass.
