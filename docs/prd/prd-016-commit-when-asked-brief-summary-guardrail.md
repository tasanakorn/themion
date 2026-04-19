# PRD-016: Commit-When-Asked Guardrail for Useful Brief Commit Messages

- **Status:** Implemented
- **Version:** v0.8.1
- **Scope:** `themion-core` (predefined prompt guardrails), docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Update `PREDEFINED_GUARDRAILS` so that when the user explicitly asks the agent to create a git commit, the agent is instructed to use a useful brief summary of the change as the commit message.
- Keep commit behavior aligned with the existing guardrail that forbids creating commits unless the user explicitly asks.
- Improve default commit quality without turning commit-message generation into a large formatting policy.
- Preserve the existing prompt-layer architecture by changing only the predefined coding-guardrail instruction layer.

## Non-goals

- No change to the existing default rule that the agent must not create commits or branches unless explicitly asked.
- No requirement to enforce a specific commit-message convention such as Conventional Commits.
- No automatic commit creation as part of ordinary code changes.
- No broader git workflow policy covering branching, squashing, rebasing, or PR titles.
- No runtime validation that rejects a commit if the message is weak; this PRD is prompt guidance, not a new git-policy subsystem.

## Background & Motivation

### Current state

Themion already injects predefined coding guardrails through `PREDEFINED_GUARDRAILS` in `crates/themion-core/src/predefined_guardrails.rs`.

That built-in guardrail block currently tells the agent to:

- avoid important hidden assumptions
- prefer the simplest solution
- make targeted changes
- run the narrowest useful validation

Repository and CLI guidance also already says:

- do not create commits or branches unless explicitly asked

That means commit creation is already intentionally opt-in. However, once the user does explicitly ask for a commit, the current predefined guardrails do not give any product-level guidance about the quality of the commit message.

As a result, commit messages can become inconsistent, overly vague, or unnecessarily verbose depending on the model's default behavior and the surrounding repository instructions.

### Why this belongs in predefined guardrails

This repository already uses predefined guardrails for small cross-project coding behaviors that should apply by default. Commit-message quality for explicit user-requested commits fits that pattern:

- it is a narrow coding-agent behavior
- it does not require provider-specific logic
- it should work even in repositories that do not restate commit-message expectations in `AGENTS.md`

The desired behavior is small and default-oriented: if the user asks the agent to commit, the agent should choose a concise message that briefly and usefully summarizes the actual change.

## Design

### Add a commit-message rule to `PREDEFINED_GUARDRAILS`

Themion should extend the predefined guardrails with a short additional rule covering explicit user-requested commits.

The rule should communicate all of the following:

- only create a commit when the user explicitly asks
- when creating that user-requested commit, use a useful brief summary of the change as the commit message
- keep the message grounded in the actual change that was made

An acceptable wording shape would be:

> Do not create commits or branches unless explicitly asked. If the user asks you to create a commit, use a useful brief summary of the change as the commit message.

The final wording may differ, but it should preserve both the existing permission boundary and the new quality expectation for the commit message.

**Alternative considered:** leave commit-message quality entirely to repository-local `AGENTS.md` files. Rejected: this is a small product-default behavior that should apply even in repositories without local git guidance.

### Keep the rule minimal and non-prescriptive

The new guardrail should stay intentionally lightweight.

It should encourage commit messages that are:

- brief
- useful
- descriptive of the actual change

It should not require:

- a strict prefix format
- issue references
- body text on every commit
- multi-line templating

This keeps the rule compatible with many repositories while still nudging the agent away from low-value messages such as `update`, `fix stuff`, or overly long prose paragraphs.

**Alternative considered:** require Conventional Commits for all agent-generated commits. Rejected: that would impose a repository policy that does not currently exist as a product-wide default.

### Prompt ordering and layer separation remain unchanged

This change should remain inside the existing predefined coding-guardrail layer.

The prompt assembly order should stay:

1. base system prompt
2. predefined coding guardrails
3. predefined Codex CLI web-search instruction
4. repository-local instructions such as `AGENTS.md`
5. workflow context and phase instructions
6. recall hint when older turns are omitted
7. recent conversation window

No prompt layers should be merged or reordered.

**Alternative considered:** add a separate predefined git-policy instruction layer. Rejected: the requested behavior is small enough to fit the existing coding-guardrail block without adding another prompt layer.

### Interaction with repository-local instructions

Repository-local instructions such as `AGENTS.md` should remain able to refine or override commit-message expectations within their scope.

Examples:

- a repository may require Conventional Commits
- a repository may forbid agent-created commits entirely
- a repository may require issue IDs or subsystem prefixes

The predefined guardrail should serve only as the default baseline when no stricter repository-local rule applies.

**Alternative considered:** make the predefined commit-message wording override repository-local commit conventions. Rejected: repository-local instructions should remain authoritative for repository-specific workflow policy.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/predefined_guardrails.rs` | Extend `PREDEFINED_GUARDRAILS` with a short rule that user-requested commits should use a useful brief summary of the change as the commit message, while preserving the existing explicit-request boundary for commits. |
| `docs/engine-runtime.md` | Update the predefined coding-guardrails description to mention the new commit-when-asked guidance. |
| `docs/architecture.md` | Update the prompt-input/guardrail description so the built-in coding guardrails include the explicit commit-message expectation. |
| `docs/README.md` | Add this PRD to the index with proposed status and scope. |

## Edge Cases

- the user asks for a commit before any actual code or doc change exists → the agent should still avoid inventing a misleading summary and should either clarify or describe the real staged change accurately.
- the user explicitly supplies their own commit message → the user instruction should take precedence over the default brief-summary guidance.
- repository-local instructions require a stricter commit format → those local instructions should continue to take precedence within that repository.
- the requested change spans multiple unrelated edits because the user explicitly asked for that broader work → the commit message should still summarize the actual combined change briefly rather than pretending the commit is narrower than it is.
- the user asks for a branch but not a commit → the existing explicit-request rule still governs branch creation; this PRD only adds message guidance for commits.
- the user asks for a commit in a dirty working tree containing unrelated changes → the agent should avoid a misleading summary and should either stage only the relevant changes or explain the blocker, depending on the user's request and repository state.

## Migration

This change is additive and backward-compatible.

It updates only the built-in prompt guidance used during coding sessions. Repositories do not need to change configuration or database state.

Repositories with stricter local git conventions may continue to express them in `AGENTS.md` or other local instructions.

## Testing

- inspect prompt assembly after the change → verify: the predefined coding-guardrail layer includes guidance that explicit user-requested commits should use a useful brief summary of the change as the commit message.
- run a coding session where the user asks for edits but does not ask for a commit → verify: the built-in guardrails still say not to create commits unless explicitly asked.
- run a coding session where the user explicitly asks for a git commit without providing a message → verify: the built-in guardrails give the model a default instruction to produce a brief useful summary-based commit message.
- run a coding session where the user explicitly provides the commit message text → verify: user instruction can still take precedence over the default brief-summary guidance.
- run a session with repository-local commit conventions in `AGENTS.md` → verify: repository-local instructions remain separate and can further constrain commit-message formatting.
- run `cargo check -p themion-core` after implementation → verify: the prompt-guardrail update and any touched docs compile and remain consistent with the workspace.
