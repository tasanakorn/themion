# PRD Authoring Guide

This project keeps Product Requirements Documents in `docs/prd/`.

Use this guide whenever you create or update a PRD.

## Workflow

1. Start docs-first.
   - Read relevant files in `docs/` before reading source code.
   - Use source only to fill gaps or confirm undocumented behavior.
2. Read the most recent 2–3 PRDs in `docs/prd/`.
   - Match their structure, heading depth, table style, and prose voice.
3. Keep scope focused.
   - One concern per PRD.
   - Do not turn the document into an implementation task list for unrelated changes.
4. Update `docs/README.md`.
   - Add the new PRD to the PRD table with link, status, and short description.

## File naming and numbering

- Filename format: `prd-NNN-<slug>.md`
- Use the next sequential number in `docs/prd/`.
- Zero-pad the number to 3 digits.
- Keep the slug kebab-case and concise.
- Detect the highest existing `NNN` in `docs/prd/` and increment it by 1 for the new PRD.

Current examples:
- `prd-001-config-and-repl-feedback.md`
- `prd-002-persistent-history-multi-agent.md`
- `prd-003-openai-codex-provider.md`

## Document header

Start each PRD with a title and metadata block like the existing PRDs:

- `# PRD-NNN: <Title>`
- `- **Status:** <status>`
- `- **Version:** vX.Y.Z`
- `- **Scope:** <affected crates / areas>`
- `- **Author:** <name> (design) + <tool/agent name> (PRD authoring)`
- `- **Date:** YYYY-MM-DD`

Default status should be `Draft` unless there is a clear reason to use another status. Use `Draft` for PRDs that are not yet ready to implement and still need refinement or correctness verification before they should be treated as implementation-ready.

### Status guidance

Use PRD status labels intentionally:

- `Draft` — not ready to implement yet; needs refinement, correctness verification, or open design review before implementation should begin
- `Proposed` — implementation-shaped and ready for review/approval as a candidate plan
- `Implemented` / `Partially implemented` — reflect landed work in the repository

When a PRD still has unresolved design questions, uncertain correctness, or needs more validation of the proposed behavior, prefer `Draft` over `Proposed`.

### Summary / TL;DR

After the metadata block, add a short plain-language summary section before the main body:

- Use the heading `## Summary`.
- Keep it short, usually 3–7 flat bullets.
- Explain the proposal in simple terms so a reader can quickly understand the approach without reading the full PRD.
- Lead with the product problem, intended outcome, or user-visible behavior before implementation tactics.
- Prefer direct language such as "keep X, add Y, avoid Z" over abstract product or architecture phrasing.
- When useful, include one bullet for the main problem, one for the proposed approach, and one for what explicitly stays unchanged.
- If the work is phased, summarize the overall product outcome first and then note which phase is being proposed or implemented now.

This section is for fast comprehension, not for replacing the full PRD.

### Short semver guideline

Use the PRD `Version` field to record the intended release target using semantic versioning.

For implementation-ready or implemented PRDs, use an exact target version:

- Patch (`vX.Y.Z` → `vX.Y.(Z+1)`): bug fixes, wording-only doc corrections, or small behavior adjustments that do not materially expand user-facing capability.
- Minor (`vX.Y.Z` → `vX.(Y+1).0`): new user-visible features, meaningful workflow changes, or additive capabilities that remain backward compatible.
- Major (`vX.Y.Z` → `v(X+1).0.0`): breaking changes, removals, or incompatible behavior/config changes.

For `Draft` PRDs, prefer a semantic target when the exact landing release is not yet decided:

- `>vX.Y.Z +patch`
- `>vX.Y.Z +minor`
- `>vX.Y.Z +major`

This means the PRD is expected to land after the referenced version and indicates the intended semver impact without pretending the exact implementation release is already fixed.

Examples:

- `- **Version:** >v0.36.0 +minor` for a draft that is not yet implementation-ready but is expected to be a minor release when it lands
- `- **Version:** v0.37.0` once the PRD becomes implementation-ready or the target release is known

When a draft becomes implementation-ready, replace the semantic target with a concrete version. When updating a PRD for newly implemented work, detect whether the change is patch, minor, or major scope and set or update the concrete version accordingly instead of always bumping the minor version.

## Canonical section order

Use these top-level sections in this order when they are relevant:

1. Goals
2. Non-goals
3. Background & Motivation
4. Design
5. Changes by Component
6. Edge Cases
7. Migration
8. Testing

Rules:
- The `## Summary` section appears before these canonical sections.
- Omit sections that would only contain placeholders.
- Do not add filler text such as `None` just to preserve numbering.
- Keep top-level headings as `##`.
- Use `###` for subsections.
- Use `####` only rarely.

## Section expectations

### Goals
- Describe what the PRD intends to achieve.
- Prefer concrete, user-visible or architecture-visible outcomes.
- Keep the product requirement or user/problem outcome primary; do not let a delivery phase become the effective goal of the PRD.

### Non-goals
- State what is explicitly out of scope.
- This should prevent scope creep and future ambiguity.
- Prefer product-scope boundaries first; implementation exclusions may follow when they clarify the proposed delivery slice.

### Background & Motivation
- Explain current behavior and why the change is needed.
- Include a `### Current state` subsection when useful.
- Ground this section in existing docs first.
- If the work is phased, explain the overall product motivation before narrowing to the current implementation slice.

### Design
- Describe the proposed behavior and structure.
- Break major topics into focused subsections.
- If the work is phased, preserve the distinction between the overall product behavior and the current delivery phase.
- For each major design choice, include an inline note in this format:
  - `**Alternative considered:** <option>. Rejected: <reason>.`
- Do not add a standalone `Alternatives` section.

### Changes by Component
- Use a table.
- Map files or modules to the changes they require.
- Keep it specific enough to guide implementation without becoming noisy.

### Edge Cases
- List realistic failure modes, constraints, or unusual scenarios.
- Focus on behavior, not hypothetical trivia.

### Migration
- Explain rollout, upgrade, downgrade, compatibility, or user transition behavior.
- Omit if there is no meaningful migration story.

### Testing
- Write each test outcome as `step → verify:`.
- Make verification observable and concrete.
- Prefer behavior-focused validation over vague statements.

### Implementation checklist
- If the PRD describes intended implementation work in enough detail to guide coding, add an `## Implementation checklist` section near the end of the PRD.
- Use markdown task list items such as `- [ ]` while the work is proposed or in progress.
- Update items to `- [x]` as implementation lands when the PRD is later updated to reflect shipped work.
- Keep the checklist scoped to concrete implementation slices implied by the PRD; do not turn it into a generic restatement of every paragraph.
- The checklist should track engineering work, not replace the product outcome stated elsewhere in the PRD.
- If the work is phased, make it clear which checklist items belong to the currently proposed or implemented phase and avoid erasing later phases from the document.
- Omit the section only when the PRD is purely exploratory, historical, or otherwise not pretending to define an implementation path.

## Style

Match the style of neighboring PRDs:

- Declarative, direct prose.
- Consistent heading hierarchy.
- Similar metadata formatting.
- Similar table formatting.
- Use blockquote admonitions only when actually needed, such as supersession notes.
- Keep the `## Summary` section plain-language and easy to skim.
- Keep the document reading like a Product Requirements Document rather than only an engineering implementation plan.

The finished PRD should look visually consistent with nearby PRDs in `docs/prd/`.

## Research guidance

When authoring a PRD:

- Start with `docs/README.md`, `docs/architecture.md`, and other relevant docs.
- Check for existing PRDs that overlap the same area.
- If behavior is already documented, prefer that as the source of truth.
- Read code only where documentation is missing, stale, or ambiguous.

## Checklist

Before finishing:

- PRD number is the next sequential number.
- Filename matches `prd-NNN-<slug>.md`.
- Header metadata is complete.
- Add a `## Summary` section after the metadata block.
- Summary leads with the product problem/outcome before implementation tactics.
- Top-level sections follow the required order.
- Goals still describe the product requirement or user/problem outcome, not only the current implementation phase.
- If the PRD is phased, the overall product intent remains visible and the current phase is clearly identified.
- Empty sections were omitted.
- Major design choices include inline `Alternative considered` notes.
- Testing uses `step → verify:` lines.
- Add an `Implementation checklist` section when the PRD defines an implementation path.
- `docs/README.md` PRD table was updated.
- Structure and voice match recent PRDs.
