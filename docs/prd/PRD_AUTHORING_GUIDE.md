# PRD Authoring Guide

This project keeps Product Requirements Documents in `docs/prd/`.

Use this guide whenever you create or update a PRD.

## Workflow

1. Start docs-first.
   - Read relevant files in `docs/` before reading source code.
   - Use source only to fill gaps or confirm undocumented behavior.
2. Read the most recent 2–3 PRDs in `docs/prd/`.
   - Match their structure, heading depth, table style, and prose voice.
   - Match their strengths, not their verbosity.
3. Keep scope focused.
   - One concern per PRD.
   - Do not turn the document into an implementation task list for unrelated changes.
4. Optimize for review efficiency.
   - Make the document easy to skim and easy to decide on.
   - Prefer concise requirements over long narrative explanation.
   - Include detail only when it changes a product or implementation decision.
   - Remove repeated explanation across sections.
   - If one sentence says the same thing as three, keep the one sentence.
   - If technical research or evidence is useful but too detailed for the main PRD body, move it into an optional technical note or appendix rather than bloating the core review path.
5. Write for non-native English readers.
   - Prefer short sentences.
   - Prefer common words over formal or abstract wording.
   - Prefer direct statements such as "Add X" or "Do not add Y".
   - Avoid idioms, clever phrasing, and long nested clauses.
   - If a requirement is important, say it once clearly instead of repeating it in different words.
6. Update `docs/README.md`.
   - Add the new PRD to the PRD table with link, status, and short description.
7. Treat guideline-document updates as mandatory when the PRD changes documented behavior or repository guidance.
   - If the proposed change affects architecture expectations, authoring guidance, workflow conventions, prompt/instruction handling, validation expectations, or any other durable guidance document, update that guidance in the same task.
   - Do not leave guidance drift for later when the needed update is already known.
   - Treat this as important follow-through, not an optional polish pass.

## File naming and numbering

- Filename format: `prd-NNN-<slug>.md`
- Use the next sequential number in `docs/prd/`.
- Zero-pad the number to 3 digits.
- Keep the slug kebab-case and concise.
- Detect the highest existing `NNN` in `docs/prd/` and increment it by 1 for the new PRD.

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
- Keep it short, usually 3–5 flat bullets.
- Go beyond 5 bullets only when the extra bullets change a review decision.
- Explain the proposal in simple terms so a reader can quickly understand it without reading the full PRD.
- Lead with the product problem, intended outcome, or user-visible behavior before implementation tactics.
- Prefer direct language such as "keep X, add Y, avoid Z".
- Avoid repeating lower-level design detail that appears later.

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
- Omit any section that does not help the reader make a decision.
- Do not add filler text such as `None` just to preserve numbering.
- Keep top-level headings as `##`.
- Use `###` for subsections.
- Use `####` only rarely.
- Optional `## Appendix` or `## Technical note` sections may appear after the main required sections when they materially help preserve evidence without slowing the main review path.

## Default size target

Treat PRDs as short decision documents by default.

- Prefer about 1–3 short paragraphs per section, or a short bullet list.
- Prefer one clear design subsection over many small subsections when possible.
- Add detail only when it changes behavior, scope, or implementation direction.
- Avoid repeating the same requirement in Summary, Goals, Background, and Design.
- If a PRD becomes hard to skim in a few minutes, shorten it.
- If detailed evidence is needed, move it to an appendix.

## Section expectations

### Goals
- Describe what the PRD intends to achieve.
- Prefer concrete, user-visible or architecture-visible outcomes.
- Prefer short bullets.
- Keep the product requirement or user/problem outcome primary.

### Non-goals
- State what is explicitly out of scope.
- Keep the list tight and decision-relevant.
- Prefer 3–6 bullets unless more are clearly needed.

### Background & Motivation
- Explain current behavior and why the change is needed.
- Include a `### Current state` subsection when useful.
- Ground this section in existing docs first.
- Keep this section short by default.
- Explain the problem, why it matters, and why now.
- Avoid long history retellings.

### Design
- Describe the proposed behavior and structure.
- Break into focused subsections only when each subsection adds a distinct decision.
- Prefer 2–5 design subsections for most PRDs.
- Do not add a standalone `Alternatives` section by default.
- Include alternative discussion only when it helps a reviewer understand a meaningful design decision, tradeoff, or rejected direction.
- If an alternative note is useful, keep it to one short sentence when possible.
- Make each subsection easy to review: state what changes, what stays the same, and why.
- Prefer requirements phrased as what the product must do.
- Avoid speculative implementation detail unless it is needed to make the requirement clear.

### Changes by Component
- Use a table.
- Map files or modules to the changes they require.
- Keep each row short.
- Mention only components that matter to implementation.

### Edge Cases
- List realistic failure modes, constraints, or unusual scenarios.
- Focus on behavior, not hypothetical trivia.
- Prefer 3–6 bullets unless more are clearly needed.

### Migration
- Explain rollout, upgrade, downgrade, compatibility, or user transition behavior.
- Omit if there is no meaningful migration story.
- Keep the discussion concrete and brief.

### Testing
- Write each test outcome as `step → verify:`.
- Make verification observable and concrete.
- Prefer concise bullet lists.
- Include only checks that materially prove the requirement.

### Implementation checklist
- If the PRD describes intended implementation work in enough detail to guide coding, add an `## Implementation checklist` section near the end of the PRD.
- Use markdown task list items such as `- [ ]` while the work is proposed or in progress.
- Keep the checklist scoped to concrete implementation slices.
- Do not restate the whole PRD as checklist items.
- Omit the section when it would add noise instead of clarity.

### Appendix / technical note
- Use this only when supporting evidence materially helps the decision but would slow down the main review path.
- Keep the main PRD body self-sufficient.
- Treat appendices as supporting evidence, not as a place to hide core requirements.

## Style

Match the style of neighboring PRDs, but prefer clarity over imitation when older PRDs are too verbose.

- Declarative, direct prose.
- Consistent heading hierarchy.
- Similar metadata formatting.
- Similar table formatting.
- Keep the `## Summary` section plain-language and easy to skim.
- Keep the document reading like a Product Requirements Document rather than only an engineering implementation plan.
- Prefer bullets, compact paragraphs, and tables over long narrative blocks.
- Do not repeat the same point across sections unless the later section adds new decision-relevant detail.
- Prefer plain English that a non-native reader can understand quickly.
- Prefer short subject-verb-object sentences.
- Avoid filler phrases such as "it is important to note that", "in order to", or "should be able to" when a shorter statement works.
- If an alternatives note does not help the reviewer make or understand a decision, omit it.

The finished PRD should be easy to understand in a few minutes.

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
- Summary is short and leads with the product problem or outcome.
- Top-level sections follow the required order.
- Empty or low-value sections were omitted.
- Goals describe the product requirement or user/problem outcome, not only the implementation phase.
- Background explains the actual product problem, not transcript history.
- Design states what the product must do.
- Repeated explanation was removed.
- Most sentences are short and plain enough for non-native English readers.
- Testing uses `step → verify:` lines.
- Add an `Implementation checklist` section only when it adds real implementation value.
- If detailed evidence is needed, move it into an optional appendix or technical note.
- `docs/README.md` PRD table was updated.
- If the PRD changes documented behavior or durable repository guidance, all affected guideline documents were updated in the same task.
- A reviewer should be able to understand the problem and proposed change in a few minutes.
