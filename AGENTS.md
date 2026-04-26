# AGENTS.md

Instructions for coding agents working in this repository.

## Project overview

- This is `themion`, a Rust workspace for a terminal AI agent.
- Main crates:
  - `crates/themion-core`: agent loop, provider clients, tools, SQLite history.
  - `crates/themion-cli`: TUI, config, login flows, app wiring.
- Docs live in `docs/`.

## Folder structure hints

- `crates/themion-core/src/`
  - reusable agent/runtime logic, provider backends, tool handling, and history behavior
  - look here first for prompt assembly, streaming, and backend-specific API translation
- `crates/themion-cli/src/`
  - terminal UI, config loading, login flows, startup wiring, and other user-facing local behavior
  - look here first for file IO, TUI event handling, and app/session orchestration
- `docs/`
  - project docs and behavior notes; consult relevant docs before changing documented flows
  - PRDs live in `docs/prd/`; when creating or updating a PRD, follow `docs/prd/README.md`
- `scripts/`
  - repository maintenance helpers
  - use `scripts/bump_version.py <semver>` to update crate package versions consistently
- crate-local tests and inline module tests
  - prefer keeping tests close to the code they cover unless the crate already uses a different pattern

When adding code:
- provider/backend integrations belong in `themion-core`
- reusable runtime behavior belongs in `themion-core`
- terminal UI, local config, and filesystem-driven user flows belong in `themion-cli`

## Architecture expectations

- Keep `themion-core` provider/backend logic separate from CLI concerns.
- Prefer putting reusable agent/runtime logic in `themion-core`.
- Keep file IO, config loading, TUI event handling, and user-facing flows in `themion-cli`.
- Preserve the `ChatBackend` abstraction when adding or changing model providers.
- Do not collapse provider-specific behavior into ad hoc conditionals when a backend-specific module already exists.

## Prompt / instruction handling

- Follow Codex-style prompt construction used by this repo.
- Keep the base system prompt separate from contextual instruction files.
- Treat root `AGENTS.md` as a separate injected message, not as text merged into the system prompt.
- If changing prompt assembly, preserve compatibility with both chat-completions-style backends and the Codex responses backend.

## Coding guidelines

- Make the smallest change that cleanly solves the requested problem.
- Avoid unrelated refactors.
- Match existing style and structure in surrounding files.
- Prefer explicit, readable code over clever abstractions.
- Do not introduce new dependencies unless they are clearly justified.
- Avoid breaking public shapes unless required by the task.
- Keep comments concise and useful.
- When a timestamp is serialized for cross-language consumers, state and preserve the unit explicitly.
- Prefer milliseconds for machine-consumed status timestamps unless a documented consumer requires another precision.
- If a field keeps the same name but its unit changes, update the relevant docs and consumer expectations in the same task when practical.
- Ensure implemented code does not introduce new build warnings; fix warning sources in the touched scope when practical.

## Rust-specific guidance

- Follow current repository conventions rather than imposing new style rules.
- Use `anyhow::Result` in application-layer code where the surrounding code already does.
- Keep serde structs and API translation code close to the backend that uses them.
- Preserve streaming behavior and tool-call handling when editing client code.
- Be careful with async trait/object boundaries; do not introduce unnecessary lifetime complexity.
- This workspace uses feature flags; when editing feature-gated code, ensure both default builds and relevant opt-in feature builds still compile.
- Do not reference feature-gated modules, types, or helpers from always-on code paths unless the reference is guarded consistently.

## Tools and file edits

- Prefer focused edits to existing files.
- Create new modules only when they meaningfully isolate behavior.
- Do not rewrite large files unnecessarily.
- Do not touch generated/build output such as `target/`.
- Do not edit lockfiles unless a dependency change is required.
- When changing crate versions in `Cargo.toml`, always check whether `Cargo.lock` changed as a result.
- If a version bump changes `Cargo.lock`, stage and commit the lockfile with the related `Cargo.toml` changes in the same commit unless the user explicitly asks for separate commits.
- Before committing a version bump, inspect `git status` so you can see whether `Cargo.lock` or other generated dependency metadata also changed.
- Read the relevant file before editing it.
- Verify tool availability before depending on non-standard local commands.

## Validation

After code changes, run the narrowest useful validation first.

Typical checks:

- `cargo check -p themion-core -p themion-cli`
- `cargo test -p themion-core`
- `cargo test -p themion-cli`

If you changed only one crate, prefer checking that crate first.
If you changed feature-gated code or code that references feature-gated modules, also run the relevant feature-on and feature-off build checks for the affected crate.
Typical feature checks for `themion-cli`:

- `cargo check -p themion-cli`
- `cargo check -p themion-cli --features stylos`

## When writing PRDs

- Follow `docs/prd/README.md` for PRD authoring conventions in this repository.
- Before writing, read the most recent 2–3 PRDs in `docs/prd/` and match their structure, heading style, and prose voice.
- Keep PRDs docs-first: ground the document in existing behavior described in `docs/`, then read source only where documentation leaves gaps.
- Use sequential filenames `prd-NNN-<slug>.md` and update the PRD table in `docs/README.md` with the new entry.
- Keep canonical top-level sections in this order when they are relevant: Goals, Non-goals, Background & Motivation, Design, Changes by Component, Edge Cases, Migration, Testing.
- Omit sections that would contain only placeholders.
- In Testing, write each outcome as `step → verify:`.
- For major design choices, include a brief inline `Alternative considered` note in the relevant design subsection instead of adding a standalone alternatives section.
- Keep PRDs centered on product requirements and intended behavior, not only engineering tactics.
- When a PRD is phased, preserve the overall product outcome and make phases describe delivery slices beneath it rather than replacing it with "Phase 1" as the effective goal.
- Treat implemented PRDs as historical specs/contracts, not living design docs.
- Do not modify an implemented PRD unless the user explicitly instructs it.
- The only routine exception is updating status/implementation notes in the PRD and `docs/README.md` so they reflect what has actually landed.
- When updating a partially implemented PRD, preserve the broader product intent and clearly mark what phase has landed versus what remains deferred.
- When implementing a feature from an existing PRD, update the relevant PRD and `docs/README.md` status/notes so the docs reflect what has actually landed.
- When starting implementation of an existing PRD, automatically consider a repository version bump as part of the work rather than assuming no bump is needed.
- If the PRD specifies a target software version, bump the repository version to match it unless the user explicitly asks not to.
- If the PRD does not specify a target version, still decide whether the change is release-worthy and bump the repository version when appropriate.
- Do not stop at pre-bump validation only: when a task includes a version bump, also run the relevant post-bump validation so version-sensitive issues are checked fairly in both directions.

### Instruction precedence and follow-through

- Treat repository instructions, accepted PRDs, and explicit user requirements as authoritative defaults, not optional suggestions.
- Do not ask the user to reconfirm an action that the repository guide or the active PRD already requires, unless the user previously gave conflicting instructions.
- If a PRD or repo guide already resolves a decision such as the target version, required docs updates, or required validation, perform that work and report it instead of asking for permission again.
- Ask a clarifying question only when there is real ambiguity that blocks correct execution, not when the repository guidance already answers the question.
- Before declaring a PRD implementation done, explicitly check that all PRD-required follow-through work is also done, including version bumps, docs/status updates, lockfile checks, and post-bump validation where applicable.
- If the user asks why a required follow-through step was missed, treat that as a process failure to correct immediately, not as a new optional request.

### Required PRD completion checklist

When implementing an existing PRD, do not consider the task complete until you have checked all of the following:

- behavior described by the PRD is implemented or any gap is clearly reported
- the PRD still reads as a product requirement or historical product contract, not only as an engineering task list
- if the PRD is phased, the currently landed phase is clear without erasing the broader product outcome or deferred phases
- `docs/README.md` and the PRD status/implementation notes reflect what actually landed
- version bump expectation was checked against the PRD and repository guidance
- `Cargo.lock` was checked after any manifest or version change
- relevant pre-bump and post-bump validation were run when version-sensitive work is involved
- touched crates still build cleanly in the relevant feature configurations
- newly introduced warnings in the touched scope were fixed or clearly called out if blocked

## Git discipline

- Do not create commits automatically; only commit when the user explicitly asks you to commit.
- Stage and commit only files relevant to the requested change.
- Use clear commit messages.
- Do not include unrelated modifications in a commit.
- Before `git add -A` or committing all pending changes, inspect `git status` and confirm there are no unrelated edits.
- When a task changes dependency manifests or version metadata, confirm whether related generated files such as `Cargo.lock` also need to be staged before committing.

## Lessons learned

- Do not assume common local tools such as `rg` are available; fall back to standard shell tools or verified alternatives.
- When adding a new exported/status field, trace where it is produced and consumed so paired changes land together.
- For activity/status transitions, track both the state value and the time the state changed so downstream consumers can interpret snapshots correctly.
- Keep debug and protocol text formats consistent across producers, consumers, and tests; if you choose a structured format, reuse it everywhere.
- For low-level or debug-oriented message headers, prefer explicit `key=value` fields with a stable type tag such as `type=peer_message` rather than positional text.
- If asked to commit, keep commits scoped unless the user explicitly requests committing all pending changes.
- Feature-flag regressions are easy to miss; when touching gated code, verify the crate still builds with the feature enabled and disabled as relevant.
- When editing code, avoid leaving newly introduced warnings behind; either fix them in the touched area or call them out clearly if blocked.
- When bumping crate versions, do not stop at editing `Cargo.toml`; explicitly check `git status` for `Cargo.lock` and include it in the same commit when it changed.
- When implementing a PRD, automatically consider whether the work should include a version bump, and if the PRD already names a target version, treat bumping to that version as the default expectation.
- When a task includes a version bump, validate after the bump too; do not assume pre-bump checks are sufficient for version-sensitive behavior.
- When revising phased PRDs, keep the overall product outcome visible so the document does not collapse into a phase-only implementation plan.

## When updating docs

- Keep docs aligned with real behavior.
- If you change provider behavior, prompt construction, login flow, or config semantics, update the relevant docs or PRD notes when appropriate.

## Avoid

- Unrequested renames or mass formatting changes.
- Mixing TUI/UI work with core backend refactors unless necessary.
- Merging system prompt text and contextual instruction-file text into one message.
- Silent behavior changes to profile/config resolution without updating docs.
