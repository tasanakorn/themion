# PRD-018: Stronger Short Commit-Message Guardrail

- **Status:** Implemented
- **Version:** v0.9.1
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Tighten the predefined commit-message guardrail so short instructions still produce practical real-world commit subjects.
- Keep the guardrail token-efficient while reducing vague subjects such as `update pending changes` or `commit changes`.
- Preserve the existing rule that commits and branches require an explicit user request.
- Improve default commit-subject quality without introducing a heavy formatting policy.

## Non-goals

- No requirement to adopt Conventional Commits or any other strict repository-wide commit format.
- No requirement to generate commit bodies by default.
- No change to the rule that the user may explicitly provide their own commit message.
- No runtime commit-message linting or rejection logic.
- No broader git workflow policy for branching, rebasing, squashing, or PR titles.

## Background & Motivation

### Current state

PRD-016 added this predefined guardrail shape:

> Do not create commits or branches unless explicitly asked. If the user asks you to create a commit, use a useful brief summary of the change as the commit message.

That improved behavior compared with having no guidance at all, but it still leaves too much room for low-information commit subjects in practice.

Recent repository history shows examples such as:

- `Update pending changes`
- `Commit pending changes`
- `Update Cargo.lock`

These are short, but they are not very useful in real-world history navigation because they often fail to name the actual feature, fix, component, or reason for the change.

The problem is not that the instruction is too short. The problem is that the current wording does not define enough of what makes a short subject useful.

### Why this should remain a short predefined guardrail

This behavior still fits the predefined guardrail layer:

- it is a cross-project default
- it is narrow and behavioral rather than provider-specific
- it should help even in repositories with no local commit guidance
- it should remain cheap in prompt tokens

The improvement should therefore strengthen the wording, not replace it with a long policy block.

## Design

### Replace vague "useful brief summary" wording with short specificity guidance

Themion should revise the predefined commit rule so it stays short but adds one practical quality bar: the commit subject should name the actual change, not a placeholder summary.

The new wording should communicate all of the following:

- do not create commits or branches unless explicitly asked
- if the user asks for a commit and does not provide the message, write a brief subject naming the actual feature, fix, docs change, refactor, or component affected
- avoid vague placeholders such as `update changes`, `pending changes`, `misc fixes`, or `commit changes`

A target wording shape is:

> Do not create commits or branches unless explicitly asked. If asked to commit, write a brief specific message naming the actual change. Avoid vague messages like `update changes` or `misc fixes`.

Example instruction text that keeps the rule short is:

> Do not create commits or branches unless explicitly asked. If asked to commit, write a brief specific message naming the actual change, not a vague placeholder.

The final text may differ, but it should stay compact and preserve these semantics.

**Alternative considered:** keep PRD-016 wording unchanged and rely on model quality alone. Rejected: recent commit history shows that the current wording is too weak to reliably produce practical commit subjects.

### Keep the rule short and token-efficient

The revised guardrail should remain a single short instruction line in `PREDEFINED_GUARDRAILS`.

It should not expand into a long style guide. The main requirement is a better default heuristic:

- brief
- specific
- grounded in the actual change
- not placeholder text

This keeps the prompt cheap while materially improving outcome quality.

**Alternative considered:** add a multi-bullet built-in commit-style guide with examples and exceptions. Rejected: the request is specifically to keep the instruction short and token-optimized.

### Prefer actual change names over generic file-action phrasing

When the change is narrow enough to name directly, the subject should prefer the thing changed over a generic action word.

Good default patterns include subjects that name:

- the feature or fix
- the subsystem or component
- the docs or PRD change when the commit is docs-only
- the reason for a lockfile-only or metadata-only update when that is the real scope

This means the guardrail should nudge the agent away from messages like:

- `Update pending changes`
- `Update Cargo.lock`

and toward messages like:

- `Add Esc turn interruption in TUI`
- `Improve interrupt handling for streaming turns`
- `docs: mark PRD-017 implemented`
- `Refresh lockfile after v0.9.0 version bump`

The predefined rule does not need to include all of these examples verbatim, but docs should make the expectation clear.

**Alternative considered:** require every subject to use a type prefix such as `core:` or `docs:`. Rejected: repository-local instructions should remain free to impose stricter formatting, but the built-in default should stay format-light.

### Repository-local and user-provided instructions still take precedence

The stronger default guardrail should remain subordinate to more specific instructions.

Precedence should remain:

- explicit user-provided commit message text wins
- repository-local instructions may require stricter formatting
- predefined guardrails provide the fallback default

This preserves the current prompt-layer architecture and avoids making the built-in rule overly rigid.

**Alternative considered:** make the predefined rule override repository-local commit conventions. Rejected: repository-local workflow policy must remain authoritative within its scope.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/predefined_guardrails.rs` | Replace the current commit-when-asked sentence with a similarly short but stronger instruction requiring a brief specific message that names the actual change and avoids vague placeholders. |
| `docs/engine-runtime.md` | Update the predefined coding-guardrails description so it reflects the stronger short commit-subject guidance. |
| `docs/architecture.md` | Update the built-in guardrail summary to note that user-requested commit subjects should be brief and specific rather than vague placeholders. |
| `docs/prd/prd-016-commit-when-asked-brief-summary-guardrail.md` | Add a status/implementation note that PRD-016 landed a first-pass guardrail and PRD-018 refines the wording because the original phrasing proved too weak in practice. |
| `docs/README.md` | Add this PRD to the index with proposed status and scope. |

## Edge Cases

- the user explicitly provides a commit message → the user text should still be used as-is unless it conflicts with higher-priority repository policy.
- the repository requires a stricter commit format such as Conventional Commits → repository-local instructions should still take precedence.
- the commit contains multiple related changes because the user asked for one combined commit → the subject should still briefly name the actual combined scope rather than falling back to placeholder wording.
- the change is docs-only → the subject should say so explicitly rather than pretending it is code work.
- the change is lockfile-only or metadata-only because of another change → the subject should mention the reason, not only the filename, when that reason is known.
- the actual scope is unclear because the working tree contains unrelated edits → the agent should avoid inventing a misleading specific summary and should instead clarify or stage only the relevant changes.

## Migration

This is a backward-compatible prompt-quality refinement.

No config, storage, or workflow migration is required. Repositories with stricter local commit conventions may continue to define them separately.

## Testing

- inspect `PREDEFINED_GUARDRAILS` after the change → verify: the commit rule remains short and explicitly asks for a brief specific message naming the actual change while avoiding vague placeholders.
- run a coding session where the user asks for a commit without providing a message → verify: the default instruction favors subjects that name the feature, fix, docs change, or component rather than generic placeholders.
- test a docs-only change → verify: the default commit subject can explicitly describe the docs scope instead of generic `update` wording.
- test a lockfile-only change caused by another known change → verify: the default subject prefers the reason for the lockfile refresh when that reason is available.
- test a repository with stricter commit-message rules in `AGENTS.md` → verify: repository-local instructions still take precedence over the built-in default.
- run `cargo check -p themion-core` after implementation → verify: the prompt-guardrail wording change compiles cleanly and touched docs remain aligned.
