# PRD-080: Rename the Primary Agent Identity from `main` to `master`

- **Status:** Implemented
- **Version:** v0.52.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-30

## Summary

- Themion currently defines the first user-facing agent in two coupled ways: the built-in default `agent_id` is `main`, and the built-in primary role on the initial interactive agent is also `main`.
- At code level, that identity is not isolated to one constant. It is currently spread across workflow defaults, local agent bootstrapping, Stylos request defaults, role validation, status snapshots, transcript formatting, tests, and docs.
- This PRD proposes a coordinated rename from `main` to `master` for both the built-in default agent id and the built-in primary role, while keeping the existing one-primary-agent interactive model otherwise unchanged.
- The document is intentionally implementation-shaped: it inventories the current definitions and concrete call sites first, then maps the rename to specific files, behaviors, tests, and compatibility decisions.
- The release target remains minor, but implementation must still treat this as a user-visible default rename and complete the migration/docs/test surface in one pass.

## Goals

- Rename the default built-in agent identity from `main` to `master`.
- Rename the built-in primary role from `main` to `master` for the initial interactive agent.
- Keep the first shipped agent model simple: one primary interactive agent should continue to exist, but it should now be identified as `master` rather than `main`.
- Make local runtime state, workflow defaults, Stylos defaults, tool docs, and user-visible transcript/status behavior agree on the same primary identity.
- Document the current code-level blast radius clearly enough that implementation can proceed without relying on ad hoc search-and-replace.

## Non-goals

- No redesign of multi-agent architecture, role semantics, or agent scheduling beyond the `main` → `master` rename.
- No change to unrelated role names such as `interactive`, `background`, `reviewer`, or `planner`.
- No broader refactor of workflow naming, phase naming, or provider/model behavior.
- No database-schema change.
- No requirement in this PRD to preserve indefinite wire-level backward compatibility with `main`; any compatibility alias, if added, should be explicit and transitional.

## Background & Motivation

### Current state

The current repository defines the first built-in agent identity in two coupled layers:

- **default agent id / workflow identity**
  - `crates/themion-core/src/workflow.rs` defines `DEFAULT_AGENT` as `"main"`
  - workflow state initialization, resets, and DB fallback behavior inherit that value
- **initial primary role**
  - `docs/architecture.md` documents that the initial runtime agent descriptor uses `roles = ["main", "interactive"]`
  - `themion-cli` validates that exactly one agent currently carries the `main` role for the primary interactive path

That identity also appears in code and docs as a default routing and targeting assumption:

- `docs/engine-runtime.md` documents `to_agent_id` defaulting to `main` for Stylos talk targeting
- `crates/themion-core/src/tools.rs` exposes Stylos tool docs that say `to_agent_id` defaults to `main`
- `crates/themion-cli/src/tui.rs`, `crates/themion-cli/src/stylos.rs`, and related tests construct default local agent descriptors with `agent_id = "main"`, `label = "main"`, and `roles` including `"main"`
- role validation and primary-agent selection logic currently rely on exactly one `main` role in several places
- historical PRDs such as archived PRD-021, PRD-022, and PRD-027 also describe the built-in identity or default target as `main`

So this is not a one-line constant rename. `main` is currently part of:

- default local agent construction
- workflow-state defaults
- role-based primary-agent validation
- Stylos request defaults and examples
- exported status snapshots and transcript strings
- code-level test fixtures and docs across both crates

### Why this should be a dedicated PRD

Because `main` currently acts as both a default agent id and a role-level selector, renaming it affects behavior, defaults, user-visible output, and network-facing expectations together.

A dedicated PRD keeps the rename explicit and reviewable instead of scattering ad hoc string edits through core, CLI, tests, and docs.

## Design

### 1. Rename the built-in default agent id from `main` to `master`

Themion should treat `master` as the built-in identity of the first shipped agent anywhere the current system assumes `main` as the default agent id.

Required behavior:

- `themion-core` should change the default built-in agent identifier from `main` to `master`
- workflow-state initialization and reset paths should default to `master`
- local agent descriptors created by `themion-cli` should default to `agent_id = "master"` and `label = "master"` where they currently hardcode `main`
- network-targeting defaults that currently omit `to_agent_id` should default to `master`
- user-facing docs and examples should show `master` as the default built-in target identity

This makes the primary agent id consistent across core and CLI behavior.

**Alternative considered:** rename only the displayed label while keeping the internal id as `main`. Rejected: that would preserve hidden inconsistency and make targeting/debug output harder to reason about.

### 2. Rename the built-in primary role from `main` to `master`

The initial interactive agent should continue to carry an explicit primary role, but that role should now be `master`.

Required behavior:

- the initial primary agent role set should become `roles = ["master", "interactive"]`
- TUI/runtime validation that currently expects exactly one `main` role should instead expect exactly one `master` role
- helper selection logic that looks up the primary agent by role should use `master`
- exported snapshots, transcript examples, and tests should reflect the renamed primary role consistently

This keeps the role layer aligned with the renamed built-in agent id.

**Alternative considered:** keep role `main` while only renaming the agent id to `master`. Rejected: the system currently uses `main` in both places, so splitting them would increase confusion rather than reduce it.

### 3. Make the implementation code-level and file-complete

This rename should not be treated as a broad conceptual goal only. Implementation should update the concrete call sites that currently encode `main` as a default id, default role, or default target.

Required behavior:

- replace shared default constants before replacing leaf call sites where practical
- update hardcoded local bootstrap fixtures and test fixtures that still spell out `main`
- update user-facing strings that describe the built-in default target or role expectation
- keep code and docs aligned in the same implementation slice so shipped docs do not lag the runtime identity

This reduces the risk of mixed `main`/`master` behavior surviving in one crate after the rename lands in another.

**Alternative considered:** rely on a simple repository-wide string replacement. Rejected: several usages represent different semantics such as default agent id, role validation, transcript text, and historical docs, so the PRD should separate those categories explicitly.

### 4. Compatibility policy: normalize explicit `main` inputs for one transition window

This PRD should recommend a narrow transitional compatibility policy rather than leaving compatibility fully undecided.

Required behavior:

- the built-in shipped identity should become `master` everywhere user-visible after implementation
- explicit incoming or user-specified `main` values should remain accepted for one transition window in the narrowest useful places where users or older peers are likely to send them
- accepted `main` values should normalize to `master` at explicit input boundaries rather than surviving as a second canonical identity inside runtime state
- normalized behavior should produce snapshots, workflow state, prompt metadata, and transcript output that use `master`, not mixed `main`/`master` strings
- transitional acceptance, if implemented, should be documented as compatibility behavior rather than as a permanent synonym guarantee

Recommended normalization boundaries:

- local command/request parsing that accepts `to_agent_id`
- Stylos inbound request handling where target `agent_id` or `to_agent_id` is matched against the local primary agent
- any primary-role selection helper that interprets external role input, if such a path exists during implementation review

Recommended non-goal for compatibility:

- do not preserve `main` as a stored canonical role or canonical agent id in snapshots, workflow state, or default local descriptors once the rename lands

This keeps migration practical without diluting the rename into indefinite dual naming.

**Alternative considered:** reject `main` immediately everywhere. Rejected: the runtime already exposes these strings through tools, requests, and peer interactions, so a short transition window lowers avoidable friction for a minor-scoped release.

### 5. Define canonical internal state after normalization

Implementation should have one canonical post-parse identity so internal state does not drift into mixed naming.

Required behavior:

- after any optional compatibility normalization, internal runtime state should use `master` as the canonical built-in agent id
- the canonical primary role in in-memory agent descriptors should be `master`
- workflow state `agent_name`, exported status snapshots, transcript-visible built-in agent labels, and test fixtures should all align on `master`
- helper functions introduced for compatibility should return canonical `master` values rather than preserving the original input spelling

This gives the codebase a clean stable target after external input is accepted or normalized.

**Alternative considered:** preserve the original input spelling and echo back `main` when the caller used `main`. Rejected: that would preserve ambiguity in snapshots, prompts, and tests and make the rename incomplete.

### 6. Use small explicit helpers rather than spreading ad hoc normalization logic

If compatibility handling is implemented, the rename should not introduce repeated string-comparison logic across many call sites.

Required behavior:

- prefer a small helper for built-in-primary-agent-id normalization such as `normalize_primary_agent_id(...)`
- prefer a small helper for built-in-primary-role matching such as `is_primary_role(...)` or an equivalent local utility where that matches current code style
- use those helpers at external input boundaries and primary-agent selection points rather than repeating `== "main" || == "master"` checks inline
- keep helper ownership aligned with current layering: reusable runtime semantics in `themion-core` when shared across crates, CLI-local helpers in `themion-cli` when the behavior is only local to CLI request handling

This keeps the implementation narrow while reducing future cleanup risk.

**Alternative considered:** duplicate compatibility checks inline at each touched call site. Rejected: that would make later alias removal and review harder.

### 7. Start from a concrete current-state inventory and map it to implementation buckets

Before implementation, the PRD should preserve a concrete inventory of the current `agent_id` and role definitions so reviewers can see exactly what is being renamed.

Required behavior:

- the PRD should summarize the current built-in definitions near the top of the document
- the implementation plan should distinguish constant/default sites, bootstrap/selection logic, Stylos targeting defaults, test fixtures, and docs
- changes should be tracked by file/area rather than only by general subsystem names

This keeps the rename scoped by observable current behavior rather than by guesswork.

**Alternative considered:** skip the current-state inventory and treat the rename as a simple search-and-replace. Rejected: current usage spans defaults, validation, networking, fixtures, and docs, so an explicit inventory reduces the risk of partial rollout.

### 8. Implement in a bounded sequence that minimizes mixed-state regressions

Because the rename touches defaults, parsing, selection logic, and test fixtures together, implementation should follow a stable order rather than landing as arbitrary edits.

Required behavior:

- first update shared canonical defaults and any helper functions that define the built-in identity
- next update external input normalization boundaries so explicit `main` inputs, if still supported, map cleanly to `master`
- then update CLI bootstrap, selection logic, and snapshot/transcript emitters so runtime state is consistently canonical
- after runtime behavior is canonical, update tests and docs to match the shipped identity and compatibility story
- avoid intermediate commits or implementation states where one crate emits `master` while another still validates only `main`

This reduces the chance of passing local edits while leaving cross-crate behavior inconsistent.

**Alternative considered:** update tests and docs first, then runtime later. Rejected: that would make repository docs temporarily describe behavior the code does not yet implement.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/workflow.rs` | Rename `DEFAULT_AGENT` from `main` to `master` and update default workflow-state initialization/reset behavior. |
| `crates/themion-core/src/agent.rs` | Update local default-agent fallbacks such as `self.local_agent_id.as_deref().unwrap_or("main")`; if compatibility helpers are shared from core, use them here so internal runtime identity remains canonical `master`. |
| `crates/themion-core/src/db.rs` | Ensure DB-loaded workflow-state fallback behavior continues to inherit the renamed `DEFAULT_AGENT`. |
| `crates/themion-core/src/tools.rs` | Update Stylos tool docs and schema text that currently describe `to_agent_id` as defaulting to `main`; keep any emitted workflow metadata aligned with `DEFAULT_AGENT`. |
| `crates/themion-core/tests/memory_tools.rs` | Update tool-call test fixtures that currently pass `to_agent_id = "main"`; add compatibility coverage only if the final implementation accepts `main` as transitional input. |
| `crates/themion-cli/src/app_runtime.rs` | Rename built-in local agent bootstrap identity that currently hardcodes `"main"`. |
| `crates/themion-cli/src/app_state.rs` | Rename initial/default local agent identity that currently hardcodes `"main"`. |
| `crates/themion-cli/src/tui.rs` | Rename fallback `to_agent_id` resolution, primary-role validation, primary-agent selection logic, initial agent bootstrap fixtures, transcript/status expectations, and tests that currently assume `main`; centralize compatibility normalization or role matching helpers rather than duplicating inline checks. |
| `crates/themion-cli/src/stylos.rs` | Update default targeting, peer-message prompt examples, status/test fixtures, and fallback sender/receiver agent identity handling that currently use `main`; apply compatibility normalization at inbound/outbound request boundaries if the transition-window policy is implemented. |
| `docs/engine-runtime.md` | Update runtime docs that currently describe default `to_agent_id` behavior with `main`; document any transitional `main` acceptance if shipped. |
| `docs/architecture.md` | Update architecture docs that currently describe the initial agent as `roles = ["main", "interactive"]`. |
| archived / active PRDs referenced for current behavior | Update only the implementation-status or current-behavior notes that should stop claiming `main` as the shipped identity; preserve historical intent where the document is describing past design. |
| `docs/README.md` | Track the new PRD entry and later status/version updates when implementation lands. |

## Current implementation inventory

Note: this inventory is a historical snapshot of pre-implementation `main` call sites captured to scope the rename. It is not a list of current remaining issues after the shipped `master` migration.

### Core defaults and workflow identity

- `crates/themion-core/src/workflow.rs:168` — `pub const DEFAULT_AGENT: &str = "main";`
- `crates/themion-core/src/workflow.rs:315` — workflow state initializes `agent_name` from `DEFAULT_AGENT`
- `crates/themion-core/src/db.rs:662` — DB fallback uses `crate::workflow::DEFAULT_AGENT.to_string()`
- `crates/themion-core/src/agent.rs:1071` — workflow state reset assigns `DEFAULT_AGENT.to_string()`
- `crates/themion-core/src/agent.rs:1475` — workflow state reset assigns `DEFAULT_AGENT.to_string()` again in another path
- `crates/themion-core/src/agent.rs:1314` — defaulting path uses `unwrap_or(DEFAULT_AGENT)`
- `crates/themion-core/src/agent.rs:614` — local agent fallback currently uses `self.local_agent_id.as_deref().unwrap_or("main")`
- `crates/themion-core/src/tools.rs:1534` — workflow/tool metadata emits `agent: DEFAULT_AGENT`

### Stylos tool docs and default-target wording

- `crates/themion-core/src/tools.rs:913` — Stylos talk tool description says `to_agent_id defaults to main`
- `crates/themion-core/src/tools.rs:916` — Stylos talk tool schema describes `to_agent_id` with `Default: main.`
- `docs/engine-runtime.md:211` — runtime docs say `to_agent_id` defaults to `main`
- `docs/prd/archive/prd-027-stylos-talk-from-and-to-identifiers.md:142` — historical shipped PRD says `to_agent_id` defaults to `main` when omitted

### CLI bootstrap, role validation, and primary-agent selection

- `crates/themion-cli/src/app_runtime.rs:142` — built-in local agent bootstrap currently hardcodes `"main"`
- `crates/themion-cli/src/app_state.rs:201` — built-in local agent bootstrap currently hardcodes `"main"`
- `crates/themion-cli/src/tui.rs:511` and `:514` — role validation counts agents with role `main` and errors unless exactly one exists
- `crates/themion-cli/src/tui.rs:535` and `:539` — primary-agent filtering and duplicate/zero checks use role `main`
- `crates/themion-cli/src/tui.rs:1083` — mutable primary-agent lookup uses `has_role(h, "main")`
- `crates/themion-cli/src/tui.rs:2534` and `:2536` — fallback selection prefers an agent with role `main`
- `docs/architecture.md:456` — architecture docs define the initial interactive agent as `roles = ["main", "interactive"]`
- `docs/prd/archive/prd-021-single-process-multi-agent-runtime-and-stylos-reporting.md` — archived shipped PRD documents the original `main` + `interactive` status shape

### Explicit local agent bootstrap fixtures and test fixtures

- `crates/themion-cli/src/tui.rs:946-948` — local default agent fixture uses `agent_id = "main"`, `label = "main"`, `roles = ["main", "interactive"]`
- `crates/themion-cli/src/tui.rs:2040-2042` — fixture repeats `main` / `main` / `main + interactive`
- `crates/themion-cli/src/tui.rs:2086-2088` — fixture repeats `main` / `main` / `main + interactive`
- `crates/themion-cli/src/tui.rs:2116-2118` — fixture repeats `main` / `main` / `main + interactive`
- `crates/themion-cli/src/tui.rs:2210-2212` — fixture repeats `main` / `main` / `main + interactive`
- `crates/themion-cli/src/tui.rs:2757-2759` — fixture repeats `main` / `main` / `main + interactive`
- `crates/themion-cli/src/stylos.rs:1035-1037` — status/test fixture uses `agent_id = "main"`, `label = "main"`, `roles = ["main", "interactive"]`
- `crates/themion-cli/src/stylos.rs:2145-2147` — another fixture uses `agent_id = "main"`, `label = "main"`, `roles = ["main"]`
- `crates/themion-core/tests/memory_tools.rs:241` — memory tool test uses `to_agent_id = "main"`

### Transcript strings, prompts, and request-detail formatting

- `crates/themion-cli/src/tui.rs:394` — request parsing fallback currently uses `.unwrap_or("main")`
- `crates/themion-cli/src/tui.rs:3606` — test fixture includes `from_agent_id: Some("main".to_string())`
- `crates/themion-cli/src/tui.rs:3620` — detail string expects `stylos_request_talk ... to_agent_id=main`
- `crates/themion-cli/src/tui.rs:3630` — expected formatted detail includes `to_agent_id=main`
- `crates/themion-cli/src/stylos.rs:2241-2243` — peer-message prompt test expects `to_agent_id=main`
- `crates/themion-cli/src/stylos.rs:2255` — prompt builder test passes `Some("main")`
- `crates/themion-cli/src/stylos.rs:2266` — prompt builder test expects `from_agent_id=main`

### Validation and snapshot tests that encode `main`

- `crates/themion-cli/src/tui.rs:3530-3559` — role-validation tests explicitly name one-main, zero-main, and two-main cases
- `crates/themion-cli/src/tui.rs:3589` — snapshot assertion expects `roles == ["main", "interactive"]`
- `crates/themion-cli/src/tui.rs:3597` — test fixture handle uses `handle("main", &["main", "interactive"])`
- `docs/prd/archive/prd-022-stylos-queryables-for-agent-presence-availability-and-task-requests.md:331-333` — archived shipped PRD snapshot examples use `agent_id = "main"`, `label = "main"`, `roles = ["main"]`

## File-by-file implementation checklist

### `crates/themion-core/src/workflow.rs`

- [x] rename `DEFAULT_AGENT` from `"main"` to `"master"`
- [x] keep workflow state initialization paths canonical on `DEFAULT_AGENT`
- [x] confirm no nearby doc strings or tests still refer to `main` as the built-in default

### `crates/themion-core/src/agent.rs`

- [x] replace remaining hardcoded fallback `unwrap_or("main")` paths with canonical `master` behavior
- [x] if compatibility helpers live in core, route fallback/default-agent matching through them
- [x] confirm workflow resets and any self-agent-id resolution emit `master` canonically

### `crates/themion-core/src/db.rs`

- [x] confirm DB fallback paths automatically inherit the renamed `DEFAULT_AGENT`
- [x] add or update tests only if there is direct coverage for fallback agent-name behavior

### `crates/themion-core/src/tools.rs`

- [x] update Stylos tool descriptions that say `to_agent_id` defaults to `main`
- [x] update JSON schema description text from `Default: main.` to `Default: master.`
- [x] verify any emitted workflow metadata that references `DEFAULT_AGENT` stays canonical after the rename

### `crates/themion-core/tests/memory_tools.rs`

- [x] update explicit `to_agent_id = "main"` fixtures to `"master"`
- [x] if transitional alias acceptance is shipped through the tested path, add one compatibility test that proves `main` normalizes to `master`

### `crates/themion-cli/src/app_runtime.rs`

- [x] rename built-in bootstrap identity from `"main"` to `"master"`
- [x] verify nearby bootstrap code does not separately hardcode the old role or label

### `crates/themion-cli/src/app_state.rs`

- [x] rename built-in bootstrap identity from `"main"` to `"master"`
- [x] verify state initialization still matches TUI/bootstrap expectations exactly

### `crates/themion-cli/src/tui.rs`

- [x] replace fallback `to_agent_id` parsing default from `main` to `master`
- [x] rename primary-role validation from one-`main` to one-`master`
- [x] rename primary-agent lookup helpers from role `main` to role `master`
- [x] update all built-in local agent fixtures to `agent_id = "master"`, `label = "master"`, `roles = ["master", "interactive"]`
- [x] update transcript/request-detail test expectations that currently spell `to_agent_id=main` or `from_agent_id=main`
- [x] if compatibility handling is implemented here, centralize it through helper functions rather than repeating inline dual-name checks

### `crates/themion-cli/src/stylos.rs`

- [x] replace outbound and inbound default-target assumptions that currently use `main`
- [x] update peer-message prompt fixtures and expected prompt headers from `main` to `master`
- [x] update snapshot/test fixtures that currently build local agent descriptors with `main`
- [x] if compatibility handling is implemented here, normalize inbound target identity before selection and emit canonical `master` afterward

### `docs/engine-runtime.md`

- [x] update runtime docs so default `to_agent_id` is `master`
- [x] if a transition-window alias exists, document that `main` is temporarily accepted and normalized to `master`

### `docs/architecture.md`

- [x] update the initial interactive-agent description from `roles = ["main", "interactive"]` to `roles = ["master", "interactive"]`
- [x] verify surrounding prose still reads correctly once “main agent” wording becomes “master agent” or “primary interactive agent”

### archived / active PRDs

- [x] update only the current-behavior or implementation-status notes that would otherwise incorrectly describe shipped runtime behavior after the rename lands
- [x] preserve historical descriptions where the PRD is intentionally describing the old design at the time it was written

### `docs/README.md`

- [x] list PRD-080 with implemented status in the PRD index
- [x] keep PRD-080 status/version metadata in the index aligned with the shipped release

## Edge Cases

- a local code path omits `to_agent_id` for Stylos talk or task routing → verify: the default target becomes `master`.
- a runtime validation path expects exactly one primary role → verify: it now requires exactly one `master` role rather than one `main` role.
- a status snapshot contains one initial interactive agent → verify: its `agent_id`, `label`, and primary role use `master` consistently.
- a test fixture or transcript string still hardcodes `main` after the rename → verify: the remaining mismatch is found and updated before release.
- an older peer or local caller still sends `main` explicitly → verify: implementation-defined compatibility behavior is documented clearly rather than left ambiguous.
- a path already using `DEFAULT_AGENT` changes automatically while a nearby hardcoded `"main"` path does not → verify: mixed defaults are not left behind in the same subsystem.
- a compatibility path accepts `main` for targeting input → verify: the resulting runtime state and emitted snapshots still surface only `master` as canonical built-in identity.

## Migration

This change requires no database schema migration, but it does require an explicit compatibility decision for runtime-visible defaults and targeting strings.

Rollout guidance:

- update all built-in defaults, tests, and docs in one coordinated change
- prefer replacing shared constants/default helpers first, then leaf fixtures and wording
- accept explicit `main` inputs only as a narrow transitional alias if implementation follows the recommended compatibility policy
- if aliases are supported, normalize them to canonical `master` before state is stored or emitted
- if aliases are not supported, user-facing docs should call out that explicit `to_agent_id=main` references and primary-role assumptions must be updated to `master`
- if archived PRDs are updated at all, limit the changes to implementation-status or current-behavior notes rather than rewriting historical design intent

## Testing

- start Themion with the default single-agent configuration → verify: the initial agent uses `agent_id = "master"`, `label = "master"`, and `roles = ["master", "interactive"]`.
- inspect workflow state initialization and reset paths → verify: default `agent_name` is `master` in all reset/default paths that previously used `DEFAULT_AGENT`.
- call Stylos talk or task paths without an explicit `to_agent_id` → verify: default targeting uses `master`.
- run local validation paths that require one primary agent → verify: they accept exactly one `master` role and reject zero or multiple `master` roles.
- submit explicit `to_agent_id=main` through any compatibility-supported request path → verify: the request is accepted, normalized to the built-in primary agent, and any resulting emitted state or transcript wording uses canonical `master`.
- if role-input compatibility is implemented, submit explicit external role `main` through that supported path → verify: it resolves to the primary built-in agent without storing `main` as canonical state.
- inspect exported status snapshots, prompt headers, and transcript detail strings after the rename → verify: built-in primary-agent identity is consistently `master` everywhere user-visible.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check -p themion-core --all-features` after implementation → verify: `themion-core` still builds cleanly across features.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: `themion-cli` still builds with Stylos enabled.
- run `cargo check -p themion-cli --all-features` after implementation → verify: `themion-cli` still builds cleanly across feature combinations.

## Implementation checklist

- [x] replace `DEFAULT_AGENT` and any other shared built-in default-agent helpers from `main` to `master`
- [x] add or update small helper functions for built-in-primary-agent normalization and primary-role matching if compatibility handling is implemented
- [x] update core fallback paths that still hardcode `"main"` outside `DEFAULT_AGENT`
- [x] update CLI bootstrap/default agent construction in `app_runtime.rs`, `app_state.rs`, `tui.rs`, and `stylos.rs`
- [x] rename primary-role validation and primary-agent selection logic from `main` to `master`
- [x] apply transitional input normalization at the chosen external request boundaries, if the compatibility policy is shipped
- [x] update prompt, transcript, request-detail, and snapshot strings that still encode `main`
- [x] update tests and fixtures that currently assert `main` as built-in agent id, label, role, or default target
- [x] update runtime/docs wording for default `to_agent_id`, primary-agent role behavior, and any transitional compatibility support

## Technical note: preferred implementation decision record

Preferred implementation decision for this PRD:

- canonical shipped built-in agent id: `master`
- canonical shipped built-in primary role: `master`
- default omitted `to_agent_id`: `master`
- compatibility policy: accept explicit external `main` only for one transition window where it enters through supported request boundaries, normalize immediately to `master`, and never emit `main` as canonical runtime state after normalization

Implications of that decision:

- snapshot payloads should show `agent_id = "master"`, `label = "master"`, and primary role `"master"`
- workflow state and default-agent metadata should show `master`
- prompt and transcript strings should show `master` after normalization
- the implementation should avoid storing or re-emitting raw external `main` values except where a test intentionally exercises the compatibility boundary itself

Decision rationale:

- this keeps the user-visible rename real rather than cosmetic
- it reduces breakage for recent users or peers still sending `main`
- it gives implementation a clean canonical internal target
- it keeps later alias removal tractable because normalization is explicit and centralized
