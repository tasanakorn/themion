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
  - PRDs live in `docs/prd/`; when creating or updating a PRD, follow `docs/prd/PRD_AUTHORING_GUIDE.md`
- `scripts/`
  - repository maintenance helpers only
  - keep durable/real repo helpers here; do not leave one-off research utilities here
  - use `scripts/bump_version.py <semver>` to update crate package versions consistently
- `experiments/`
  - scratch analysis, temporary measurement code, and PRD-specific research artifacts
  - prefer `experiments/prdNNN/` for work tied to one PRD or investigation
  - move one-off scripts and exploratory code here instead of leaving them under crate `examples/` or `scripts/`
- crate-local tests and inline module tests
  - prefer keeping tests close to the code they cover unless the crate already uses a different pattern

When adding code:
- provider/backend integrations belong in `themion-core`
- reusable runtime behavior belongs in `themion-core`
- terminal UI, local config, and filesystem-driven user flows belong in `themion-cli`

## Architecture expectations

> [!IMPORTANT]
> These architecture rules are important repository requirements, not optional preferences.
> Changes that violate this layering or source-of-truth guidance are not acceptable unless the user explicitly asks for an exception and the docs/instructions are updated accordingly.

- Keep `themion-core` provider/backend logic separate from CLI concerns.
- Prefer putting reusable agent/runtime logic in `themion-core`.
- Keep file IO, config loading, TUI event handling, and user-facing flows in `themion-cli`.
- Treat the TUI as a strict human input/output surface only. `tui.rs` and `tui_runner.rs` should only collect human input, translate that input into runtime/app-state intents, observe runtime/app-state outputs, and render those outputs back to the human. They must not own or interpret runtime orchestration, agent-management policy, watchdog behavior, Stylos coordination, board routing, incoming-prompt admission, or other non-visual system decisions.
- Stylos transport, watchdog behavior, agent roster publication, board routing, agent discovery, incoming-prompt handling, and other multi-agent runtime logic are not TUI responsibilities. Investigate and implement those behaviors in CLI runtime/orchestrator modules first, and touch `tui.rs` only for display/input wiring that is strictly required.
- Treat Stylos as a lower-layer facility/foundation/transport adapter that the runtime can opt into or out of, not as the owner of core local runtime features.
- Core local features such as watchdog scheduling, board notes, durable note workflow, and local incoming-prompt handling should not disappear just because the `stylos` feature is disabled. Feature gating may change transport-specific behavior, identity format, publication/discovery paths, or remote coordination capabilities, but it should not remove the whole local feature unless the product requirement explicitly says so.
- When debugging local-agent visibility or board-targeting issues, do not assume `tui.rs` is the right fix location just because the app has a terminal UI; verify the runtime ownership path and prefer runtime-owned state/publication code.
- Preserve the `ChatBackend` abstraction when adding or changing model providers.
- Do not collapse provider-specific behavior into ad hoc conditionals when a backend-specific module already exists.

### Runtime layering and source-of-truth rules

- Preserve this layering by default:

  ```text
  | TUI | HEADLESS |
  -----------------
  | HUB / APP_STATE |
  -----------------
  | AGENT CORE | STYLOS |
  ```

- `TUI` and `HEADLESS` are surfaces. They should send human or external intents/commands and render or report observed runtime state. They must not become the canonical owner of agent roster, workflow, watchdog policy, board-routing policy, incoming-prompt admission, or Stylos-published status.
- `HUB / APP_STATE` is the runtime owner. It should own agent registry state, session/runtime metadata, workflow state, watchdog/board/stylos coordination state, admission/scheduling decisions, and the status snapshot that other layers consume.
- `AGENT CORE` and `STYLOS` are lower-layer services/adapters used by the hub. Stylos should query or publish hub-owned state rather than asking TUI-owned state for the truth.
- Do not make hub-owned runtime behavior depend wholesale on Stylos compile-time availability unless the behavior is inherently transport-specific. Prefer keeping one runtime-owned feature path and varying only the Stylos-backed portions such as remote transport, discovery, or external publication.
- Avoid cross-layer ownership leaks such as TUI-owned agent rosters, TUI-assembled Stylos status snapshots, direct Stylos-to-TUI dependencies, or TUI-side runtime decision trees. Treat these as architecture violations to fix, not acceptable end states.
- When introducing a new runtime behavior, decide first which layer owns the state and which layers only observe or project it.
- If a behavior can be described as “the system decided”, that decision belongs outside TUI. The TUI may display the decision or forward a human request that influences it, but should not be the layer that makes it.
- TUI-owned state should be limited to presentation concerns such as focus, scrolling, cursor/composer buffers, view-local formatting, and other ephemeral render/input details.
- If a new event path would require `tui.rs` to branch on Stylos/watchdog-specific policy or reconstruct runtime truth, stop and move that logic into the runtime/app-state layer instead.

### Async/eventing guidance

- Prefer layered command flow: use `mpsc` for intents/commands within the runtime ownership boundary.
- Prefer `tokio::sync::watch` for cross-layer state observation when multiple surfaces or adapters need the same current runtime truth.
- Use `broadcast` for lossy notifications when appropriate, but do not treat it as the canonical state store.
- The watched value should be a useful hub-owned snapshot that proves single source of truth, not scattered booleans or UI-local fragments.
- Keep live executors/resources such as `Agent` objects outside the watched snapshot; publish cheap-to-clone runtime/view snapshots instead.
- TUI, headless flows, and Stylos status/query paths should all derive their visible state from the same hub-owned snapshot or equivalent runtime-owned state provider.
- If a design would require reconstructing the same runtime truth separately in TUI and Stylos, stop and move that ownership into the hub/app-state layer instead.

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
- When validation is needed after code changes, check the narrowest useful target first, then also run the relevant non-`--all-features` build(s) for the touched crate so default or specifically targeted feature combinations are verified, and finally run an `--all-features` build for each touched crate before considering the task done.
- Do not reference feature-gated modules, types, or helpers from always-on code paths unless the reference is guarded consistently.

## Tool design and implementation guidance

- Follow `docs/tool-design-and-implementation-guide.md` when adding, removing, or changing tools.
- Treat tool schemas as AI-model-facing contracts first, not human-friendly documentation surfaces. Prefer short, exact, systematic wording over conversational prose.
- Prefer the smallest stable tool surface that cleanly solves the product need. Reduce the number of tools and the number of parameters unless extra surface clearly improves correctness, safety, or round-trip reduction.
- Prefer one canonical parameter shape per concept. Avoid permanent dual-parameter compatibility shapes such as old+new aliases in the long-term public contract unless the product requirement explicitly needs both.
- Design read/query tools to reduce common fan-out patterns. If callers routinely need several near-identical calls and client-side merging, prefer one model-friendly query shape that answers the combined question directly.
- Keep tool descriptions compact: usually one short purpose sentence, plus one short constraint sentence only when needed. Keep parameter descriptions literal, bounded, and compact.
- Keep exact limits, special tokens, and contract-critical semantics, but move broad policy or tutorial-style explanation out of per-tool schema text when possible.
- When reviewing a proposed new tool, ask first whether an existing tool can absorb the capability more cleanly. Do not mirror internal helper boundaries into separate tools without a strong product reason.

## Tools and file edits

- Prefer focused edits to existing files.
- Create new modules only when they meaningfully isolate behavior.
- Put temporary experiments, token-analysis helpers, replay probes, and other exploratory utilities under `experiments/` rather than shipping them as crate examples or permanent maintenance scripts unless the user asks for that promotion.
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
- `cargo check --all-features -p themion-core -p themion-cli`
- `cargo test -p themion-core`
- `cargo test -p themion-cli`

If you changed only one crate, prefer checking that crate first.
Before considering the task complete, also run the relevant non-`--all-features` build(s) for each touched crate, including the default feature set when applicable, and then run `cargo check --all-features` for each touched crate, even if the primary change was not feature-gated.
If you changed feature-gated code or code that references feature-gated modules, also run the relevant feature-on and feature-off build checks for the affected crate.
Typical feature checks for `themion-cli`:

- `cargo check -p themion-cli`
- `cargo check -p themion-cli --features stylos`
- `cargo check -p themion-cli --all-features`

## When writing PRDs

- Follow `docs/prd/PRD_AUTHORING_GUIDE.md` for PRD authoring conventions in this repository.
- Before writing, read the most recent 2–3 PRDs in `docs/prd/` and match their structure, heading style, and prose voice.
- Keep PRDs docs-first: ground the document in existing behavior described in `docs/`, then read source only where documentation leaves gaps.
- Use sequential filenames `prd-NNN-<slug>.md` and update the PRD table in `docs/README.md` with the new entry.
- Keep canonical top-level sections in this order when they are relevant: Goals, Non-goals, Background & Motivation, Design, Changes by Component, Edge Cases, Migration, Testing.
- Omit sections that would contain only placeholders.
- In Testing, write each outcome as `step → verify:`.
- For major design choices, include a brief inline `Alternative considered` note in the relevant design subsection instead of adding a standalone alternatives section.
- Keep PRDs centered on product requirements and intended behavior, not only engineering tactics.
- Write PRDs in terms of what the product must do, not around placeholder example tokens or temporary shorthand unless the literal token is itself the requirement.
- If discussion includes an example marker or sketch such as `[xxxx]`, verify whether it is the real requirement or only a shorthand for a broader product distinction before baking it into the PRD.
- When a user correction changes the intent of a PRD, rewrite the PRD around the corrected product requirement instead of merely patching the old framing.
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

### PRD execution behavior

- When starting work on an existing PRD, create a durable todo board note for yourself that captures the PRD-specific definition-of-done intent for the implementation slice you are beginning.
- When executing a PRD, try to drive the work to an actual done state rather than stopping early to re-ask the human for routine follow-through that the PRD, repo instructions, or current code/docs can resolve.
- Do not silently bind important assumptions just to keep moving; if a missing decision would materially change the implementation, docs, validation, or release outcome, identify it explicitly.
- If one part of a PRD becomes blocked on a human answer or decision, look for other concrete work within that same PRD that can still be completed correctly without that answer, and continue progressing those unblocked parts first.
- Ask the human only after you have exhausted the meaningful unblocked work that can be done safely without guessing.
- When you do need human input to continue a PRD, end your turn with a clear summary of what remains blocked and exactly which answers or decisions the human must provide.

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
- Feature-flag regressions are easy to miss; when touching gated code, verify the crate still builds with the feature enabled and disabled as relevant, and finish with an `--all-features` check for each touched crate.
- When editing code, avoid leaving newly introduced warnings behind; either fix them in the touched area or call them out clearly if blocked.
- When bumping crate versions, do not stop at editing `Cargo.toml`; explicitly check `git status` for `Cargo.lock` and include it in the same commit when it changed.
- When implementing a PRD, automatically consider whether the work should include a version bump, and if the PRD already names a target version, treat bumping to that version as the default expectation.
- When a task includes a version bump, validate after the bump too; do not assume pre-bump checks are sufficient for version-sensitive behavior.
- When revising phased PRDs, keep the overall product outcome visible so the document does not collapse into a phase-only implementation plan.
- When writing or revising a PRD, state the product requirement directly and do not silently promote placeholder discussion tokens into the requirement itself.
- If the user corrects the framing of a PRD, rewrite the document around the corrected product intent rather than preserving misleading earlier wording.
- Stylos logic is runtime/orchestrator work, not TUI work; when local agent creation, status publication, discovery, or board routing misbehaves, inspect runtime ownership first and only change TUI code for narrow presentation/input plumbing.
- For cross-layer runtime state, prefer hub/app-state ownership with `watch`-observable snapshots so TUI, headless, and Stylos consume the same source of truth instead of rebuilding state separately.
- When delegating work to another agent via a board note, state explicitly in the note body how the result must be returned. If you need a durable response, say clearly that the agent should place a done note back or update the delegated note result through the board workflow rather than replying only in chat.

## When updating docs

- Keep docs aligned with real behavior.
- If you change provider behavior, prompt construction, login flow, or config semantics, update the relevant docs or PRD notes when appropriate.

## Avoid

- Unrequested renames or mass formatting changes.
- Mixing TUI/UI work with core backend refactors unless necessary.
- Merging system prompt text and contextual instruction-file text into one message.
- Silent behavior changes to profile/config resolution without updating docs.
