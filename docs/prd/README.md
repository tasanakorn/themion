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
- `- **Author:** <name> (design) + Claude Code (PRD authoring)`
- `- **Date:** YYYY-MM-DD`

Default status should be `Proposed` unless there is a clear reason to use another status.

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
- Omit sections that would only contain placeholders.
- Do not add filler text such as `None` just to preserve numbering.
- Keep top-level headings as `##`.
- Use `###` for subsections.
- Use `####` only rarely.

## Section expectations

### Goals
- Describe what the PRD intends to achieve.
- Prefer concrete, user-visible or architecture-visible outcomes.

### Non-goals
- State what is explicitly out of scope.
- This should prevent scope creep and future ambiguity.

### Background & Motivation
- Explain current behavior and why the change is needed.
- Include a `### Current state` subsection when useful.
- Ground this section in existing docs first.

### Design
- Describe the proposed behavior and structure.
- Break major topics into focused subsections.
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

## Style

Match the style of neighboring PRDs:

- Declarative, direct prose.
- Consistent heading hierarchy.
- Similar metadata formatting.
- Similar table formatting.
- Use blockquote admonitions only when actually needed, such as supersession notes.

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
- Top-level sections follow the required order.
- Empty sections were omitted.
- Major design choices include inline `Alternative considered` notes.
- Testing uses `step → verify:` lines.
- `docs/README.md` PRD table was updated.
- Structure and voice match recent PRDs.
