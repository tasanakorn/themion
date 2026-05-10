# AGENTS.md

Instructions for coding agents working in this repository.

## Project overview

- `themion` is a Rust workspace for a terminal AI agent.
- Main crates:
  - `crates/themion-core`: agent loop, provider clients, tools, SQLite history.
  - `crates/themion-cli`: TUI, config, login, startup wiring.
- Docs live in `docs/`.
- Default persistent database: `system.db` in the Themion data directory.
  - Linux default: `$XDG_DATA_HOME/themion/system.db`
  - If `XDG_DATA_HOME` is unset: `~/.local/share/themion/system.db`
  - Legacy `history.db` in the same directory is migrated when the default database opens.

## Project map

- `crates/themion-core/src/`
  - reusable runtime logic, provider backends, tools, history behavior
  - check here first for prompt assembly, streaming, backend API translation
- `crates/themion-cli/src/`
  - TUI, config loading, login, startup wiring, local user-facing behavior
  - check here first for file I/O, TUI events, app/session orchestration
- `crates/themion-web/`
  - deprecated migration residue, not a target for new product work
  - use only as source material when moving behavior into `themion-cli --web`
- `docs/`
  - product and behavior docs
  - PRDs live in `docs/prd/`; follow `docs/prd/PRD_AUTHORING_GUIDE.md`
- `scripts/`
  - durable repository maintenance helpers only
  - use `scripts/bump_version.py <semver>` for version bumps
- `experiments/`
  - temporary analysis, research, measurement code, PRD-specific scratch work
  - prefer `experiments/prdNNN/` for PRD-specific work
- Tests
  - keep tests close to the touched code unless the crate already uses another pattern

When adding code:
- provider and backend integrations go in `themion-core`
- reusable runtime behavior goes in `themion-core`
- terminal UI, local config, and filesystem-driven user flows go in `themion-cli`
- new browser product work for PRD-106 goes under `themion-cli --web`, not `crates/themion-web`, unless the user explicitly asks for an exception

## Core architecture rules

These rules are required, not optional.

- Keep provider/backend logic separate from CLI concerns.
- Put reusable runtime behavior in `themion-core`.
- Keep file I/O, config loading, TUI event handling, and local user flows in `themion-cli`.
- TUI is only for input and display. Do not put runtime policy or orchestration in `tui.rs` or `tui_runner.rs`.
- Stylos, watchdogs, board routing, agent discovery, incoming prompt handling, and multi-agent runtime logic are runtime responsibilities, not TUI responsibilities.
- Stylos is a lower-layer transport/foundation. It does not own core local runtime behavior.
- Core local features such as watchdog scheduling, board notes, durable note workflow, and local incoming-prompt handling must still exist when the `stylos` feature is disabled unless the product requirement says otherwise.
- When debugging local-agent visibility or board targeting, check runtime ownership first. Do not assume the fix belongs in `tui.rs`.
- Preserve the `ChatBackend` abstraction when changing model providers.
- Do not replace backend-specific modules with ad hoc provider conditionals.

### Runtime ownership rules

Use this default layering:

```text
| TUI | HEADLESS |
-----------------
| HUB / APP_STATE |
-----------------
| AGENT CORE | STYLOS |
```

Ownership rules:
- `TUI` and `HEADLESS` send commands and render results. They are not the source of truth for roster, workflow, watchdog policy, board routing, incoming prompt admission, or Stylos status.
- `HUB / APP_STATE` owns runtime state: agent registry, session metadata, workflow state, scheduling/admission decisions, watchdog/board/Stylos coordination, and shared status snapshots.
- `AGENT CORE` and `STYLOS` are lower-layer services used by the hub.
- If a behavior is “the system decided”, that decision belongs outside TUI.
- Keep TUI-owned state limited to presentation details such as focus, scroll, cursor state, composer buffers, and local formatting.
- If a change would make TUI rebuild runtime truth or branch on Stylos/watchdog policy, move that logic into the runtime/app-state layer instead.
- Avoid cross-layer ownership leaks such as TUI-owned agent rosters, TUI-built Stylos status snapshots, direct Stylos-to-TUI dependencies, or TUI-side runtime decision trees.
- Do not make hub-owned runtime behavior disappear only because Stylos is compiled out unless the behavior is truly transport-specific.

### Async/eventing rules

- Prefer layered command flow with `mpsc` inside the runtime ownership boundary.
- Prefer `tokio::sync::watch` when multiple surfaces/adapters need the same current runtime state.
- Use `broadcast` for lossy notifications when appropriate, but not as the main state store.
- Publish useful hub-owned snapshots, not scattered booleans or UI-local fragments.
- Keep live executors such as `Agent` objects out of watched snapshots; publish cheap-to-clone runtime/view data instead.
- TUI, headless flows, and Stylos status/query paths should read from the same hub-owned runtime state.

## Prompt and instruction handling

- Follow the repo’s Codex-style prompt construction.
- Keep the base system prompt separate from contextual instruction files.
- Treat root `AGENTS.md` as its own injected message, not text merged into the system prompt.
- If you change prompt assembly, keep compatibility with both chat-completions-style backends and the Codex responses backend.

## Coding rules

- Make the smallest change that cleanly solves the task.
- Avoid unrelated refactors.
- Match the surrounding style and structure.
- Prefer explicit, readable code.
- Do not add new dependencies unless clearly justified.
- Avoid breaking public interfaces unless required.
- Keep comments short and useful.
- When serializing timestamps for cross-language consumers, preserve and document the unit.
- Prefer milliseconds for machine-consumed status timestamps unless a documented consumer requires another unit.
- If a field keeps the same name but changes unit, update docs and consumers in the same task when practical.
- Do not leave new warnings behind in touched code when practical.

## Rust rules

- Follow current repository conventions.
- Use `anyhow::Result` in application-layer code when surrounding code already does.
- Keep serde structs and API translation close to the backend that uses them.
- Preserve streaming behavior and tool-call handling when editing client code.
- Be careful around async trait/object boundaries; avoid unnecessary lifetime complexity.
- Run `cargo fmt` on touched Rust files or crates before finishing, unless the user asks not to.
- Do not make formatting-only changes outside the requested scope.
- This workspace uses feature flags. When touching feature-gated code, make sure default and relevant opt-in builds still compile.
- Do not reference feature-gated modules, types, or helpers from always-on code unless the reference is guarded correctly.

## Tool design rules

- Follow `docs/tool-design-and-implementation-guide.md` when adding, removing, or changing tools.
- Treat tool schemas as model-facing contracts first.
- Keep tool surfaces as small as possible while still solving the product need.
- Prefer one canonical parameter shape per concept.
- If callers often need many similar read/query calls, prefer one tool shape that answers the combined question directly.
- Keep tool descriptions short, exact, and systematic.
- Keep parameter descriptions literal, bounded, and compact.
- Keep exact limits and contract-critical semantics, but move tutorial-style explanation out of schema text when possible.
- Before adding a new tool, check whether an existing tool can absorb the capability cleanly.

## Files and edits

- Prefer focused edits to existing files.
- Create new modules only when they clearly isolate behavior.
- Put temporary experiments, token-analysis helpers, replay probes, and similar exploratory utilities under `experiments/`.
- Do not rewrite large files without need.
- Do not touch generated output such as `target/`.
- Do not edit lockfiles unless a dependency or version change requires it.
- Read the relevant file before editing it.
- Verify tool availability before depending on non-standard local commands.

## Validation

Run the narrowest useful validation first.

Typical checks:
- `cargo check -p themion-core -p themion-cli`
- `cargo check --all-features -p themion-core -p themion-cli`
- `cargo test -p themion-core`
- `cargo test -p themion-cli`

Rules:
- If you changed one crate, check that crate first.
- Before finishing, run the relevant non-`--all-features` builds for each touched crate, including the default feature set when applicable.
- Then run `cargo check --all-features` for each touched crate.
- If you touched feature-gated code or code that references feature-gated modules, run the relevant feature-on and feature-off checks.

Typical `themion-cli` feature checks:
- `cargo check -p themion-cli`
- `cargo check -p themion-cli --features stylos`
- `cargo check -p themion-cli --all-features`

## PRD rules

### Writing PRDs

- Follow `docs/prd/PRD_AUTHORING_GUIDE.md`.
- Before writing, read the most recent 2–3 PRDs in `docs/prd/` and match their structure, headings, and voice.
- Keep PRDs short decision documents.
- Write in plain English for non-native readers: short sentences, common words, direct statements, small bullet lists.
- Start from docs first; read source only where docs leave gaps.
- Use filenames `prd-NNN-<slug>.md`.
- Update the PRD table in `docs/README.md` when adding a PRD.
- Use these top-level sections when relevant, in this order: Goals, Non-goals, Background & Motivation, Design, Changes by Component, Edge Cases, Migration, Testing.
- Omit empty placeholder sections.
- In Testing, write each outcome as `step → verify:`.
- Put major alternatives as short inline notes in the relevant section.
- Keep PRDs about product requirements and intended behavior, not only engineering tactics.
- If a PRD is hard to skim in a few minutes, shorten it.
- Do not turn placeholder examples or sketch markers into requirements without confirming they are real requirements.
- If the user corrects the framing, rewrite the PRD around the corrected product intent.
- If a PRD is phased, keep the overall product outcome visible. Phases should describe delivery slices, not replace the main goal.
- Treat implemented PRDs as historical specs/contracts.
- Do not modify an implemented PRD unless the user explicitly asks.
- Normal exception: update status/implementation notes in the PRD and `docs/README.md` so docs match reality.

### Executing PRDs

- When starting work on an existing PRD, create a durable todo board note for yourself.
- Try to drive PRD work to an actual done state.
- Do not stop early to ask the user for follow-through already resolved by the PRD, repo instructions, docs, or code.
- Do not make important assumptions silently.
- If one part is blocked by a human answer, continue other safe, unblocked PRD work first.
- Ask the user only after meaningful safe progress is exhausted.
- When blocked, clearly state what remains blocked and exactly what human answer is needed.

### PRD follow-through

- Treat repository instructions, accepted PRDs, and explicit user requirements as authoritative defaults.
- Do not ask the user to reconfirm actions already required by the repo guide or active PRD unless the user gave conflicting instructions.
- If the PRD or repo already answers a decision such as target version, docs update, or required validation, do the work and report it.
- Ask clarifying questions only when real ambiguity blocks correct execution.
- Before declaring PRD work done, check all PRD-required follow-through: version bump review, docs/status updates, lockfile check, and post-bump validation when needed.
- If the user asks why required follow-through was missed, treat that as a mistake to fix immediately.

### PRD completion checklist

Before finishing PRD implementation, check all of these:
- PRD behavior is implemented, or any gap is clearly reported.
- The PRD still reads like a product requirement or historical product contract, not just an engineering task list.
- If phased, the landed phase is clear without hiding the broader product goal or deferred phases.
- `docs/README.md` and PRD status/implementation notes match what actually landed.
- Version bump expectation was checked against the PRD and repository guidance.
- `Cargo.lock` was checked after any manifest or version change.
- Relevant pre-bump and post-bump validation ran when version-sensitive work was involved.
- Touched crates build cleanly in relevant feature configurations.
- New warnings in touched scope were fixed or clearly called out if blocked.

## Git rules

- Do not create commits automatically.
- Commit only when the user explicitly asks.
- Stage and commit only files relevant to the requested change.
- Use clear commit messages.
- Do not include unrelated changes.
- Before `git add -A` or committing all pending changes, inspect `git status` and confirm there are no unrelated edits.
- When manifests or version metadata change, check whether `Cargo.lock` or other generated dependency files also changed.
- If a version bump changes `Cargo.lock`, stage and commit it with the related `Cargo.toml` changes unless the user explicitly asks for separate commits.

## Documentation updates

- Keep docs aligned with real behavior.
- If you change provider behavior, prompt construction, login flow, config semantics, or implemented PRD status, update the relevant docs or PRD notes.

## Quick reminders

- Do not assume `rg` or other common local tools exist.
- When adding an exported/status field, trace both producer and consumer.
- For activity/status transitions, track both the value and when it changed.
- Keep debug/protocol text formats consistent across producers, consumers, and tests.
- For low-level/debug headers, prefer explicit `key=value` fields with a stable type tag.
- Stylos logic belongs in runtime/orchestrator layers, not TUI.
- When delegating by board note, say exactly how the result should be returned.
- If one web UI view updates and another stays stale, compare data sources before changing rendering.
- After rebuilding `themion-cli --web` assets, hard refresh the browser and restart the web process before concluding the fix failed.

## Response ending marker

- The last line of every user-facing assistant response must be exactly one of:
  - `TASK_FINISH`
  - `STOP_BUT_HAVE_TO_CONTINUE`
  - `NEED_HUMAN_ACTION`
- Use `TASK_FINISH` only when the task is actually complete.
- Use `STOP_BUT_HAVE_TO_CONTINUE` when work remains.
- Use `NEED_HUMAN_ACTION` when progress is blocked on a human decision, approval, clarification, or other explicit action.

## Avoid

- Unrequested renames or mass formatting changes.
- Mixing TUI/UI work with core backend refactors unless necessary.
- Merging system prompt text and contextual instruction-file text into one message.
- Silent behavior changes to profile/config resolution without updating docs.
