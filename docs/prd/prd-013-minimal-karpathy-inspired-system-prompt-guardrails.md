# PRD-013: Minimal Karpathy-Inspired Predefined Coding Guardrails

- **Status:** Implemented
- **Version:** v0.7.0
- **Scope:** `themion-core` (prompt assembly / predefined instruction injection), docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Add a minimal predefined coding-agent instruction layer inspired by the key ideas commonly packaged as “Karpathy's `CLAUDE.md`”.
- Inject that instruction layer as a separate prompt input before repository-local instructions such as `AGENTS.md`, rather than mixing it into the base system prompt.
- Keep the added guidance short and general so it improves default coding-agent behavior without turning prompt assembly into a large policy stack.
- Reduce three common coding-agent failure modes in default runs: hidden assumptions, overcomplicated solutions, and unrelated edits.
- Preserve Themion's existing prompt-input separation across the base system prompt, predefined guardrails, repository instructions, workflow context, and conversation history.
- Document the relevant source material and summarize the Karpathy-inspired topics being applied so future prompt changes remain grounded in the intended behavior.

## Non-goals

- No adoption of Anthropic's `CLAUDE.md` file format or memory mechanism inside Themion.
- No merging of the predefined guardrails into the configured base system prompt text.
- No change to the conceptual role of `AGENTS.md` and other repository-local instruction files beyond shifting them one position later in the injected instruction order.
- No broad rewrite of tool instructions, workflow instructions, or the overall system prompt tone beyond this minimal guardrail layer.
- No attempt to reproduce every community variant of “Karpathy's `CLAUDE.md`” or treat it as a formal standard.
- No provider-specific prompt forks unless a backend compatibility issue makes a wording adjustment necessary.

## Background & Motivation

### Current state

Themion already separates prompt inputs into distinct layers:

- a base system prompt from configuration
- injected contextual instruction files such as `AGENTS.md`
- workflow context and phase instructions
- recent conversation and optional recall hints

This is already the right architecture for this repository. The docs explicitly describe the base system prompt and `AGENTS.md` as separate prompt inputs rather than one merged block.

However, the current default behavior still relies on a fairly generic base system prompt plus whatever repository-local instructions happen to exist. That means useful coding-agent guardrails are not consistently present across sessions, especially in print mode or repositories without local instruction files.

### Karpathy-inspired topic overview to apply

The community phrase “Karpathy's `CLAUDE.md`” generally refers to a short set of behavioral rules derived from Andrej Karpathy's observations about coding-agent failure modes and then packaged into Claude Code's `CLAUDE.md` convention. It is best understood as a popular community template rather than a formal Karpathy-authored specification.

The key topics consistently associated with that material are:

1. **Avoid hidden assumptions**
   - Do not silently guess when important facts are missing.
   - State assumptions and ask clarifying questions when ambiguity is genuinely blocking.

2. **Prefer simple solutions**
   - Solve the requested problem directly.
   - Avoid unnecessary abstractions, speculative generalization, and overengineering.

3. **Make surgical changes**
   - Touch only the code relevant to the request.
   - Avoid unrelated refactors, rewrites, or comment churn.

4. **Verify outcomes**
   - Check whether the result actually works.
   - Use the narrowest useful validation rather than stopping at code generation alone.

These topics fit Themion's design goals well because they complement the existing workflow/runtime structure without requiring new tools or a new prompt architecture.

### Why this should be a predefined injected layer

These guardrails describe cross-project default behavior, but they are still conceptually different from the base system prompt's core assistant identity.

They are better modeled as a predefined instruction layer that sits between:

- the base system prompt
- repository-local instructions such as `AGENTS.md`

This preserves the repository's architectural preference for separate instruction inputs while making the guardrails consistently present across sessions.

Injecting them before `AGENTS.md` gives them stable precedence among contextual instructions without requiring Themion to flatten distinct instruction sources into one message.

## Design

### Prompt ordering

Themion should keep prompt inputs separate and assemble them in this conceptual order:

1. base system prompt
2. predefined Karpathy-inspired coding guardrails
3. repository-local instructions such as `AGENTS.md`
4. workflow context and phase instructions
5. recall hint when older turns are omitted
6. recent conversation window

The key requirement is that the new guardrail block is prepended before `AGENTS.md` and remains separate from the base system prompt.

**Alternative considered:** merge the guardrails directly into the base system prompt. Rejected: the repository already treats the system prompt and contextual instruction files as separate layers, and these guardrails are better represented as a reusable injected instruction than as core system identity text.

### Minimal predefined guardrail block

Themion should add a short guardrail block covering four ideas:

- do not make important hidden assumptions
- prefer the simplest solution that cleanly solves the task
- avoid unrelated code changes
- verify the result with the narrowest useful check

The wording should be concise and operational, for example in a shape like:

> When working on code, avoid making important assumptions silently. If requirements are ambiguous and the ambiguity blocks a correct solution, ask a brief clarifying question. Prefer the simplest solution that cleanly solves the user's request. Make targeted changes and avoid unrelated refactors. After changes, run the narrowest useful validation and report the result.

The exact final wording may differ, but the implementation should preserve all four behavioral topics.

**Alternative considered:** copy a longer community `CLAUDE.md` template nearly verbatim into the injected instructions. Rejected: the community template is useful as inspiration, but Themion's built-in instruction layer should stay short, backend-friendly, and easy to compose with repository-local guidance.

### Interaction with `AGENTS.md`

`AGENTS.md` should remain a separate injected message/input and should continue to represent repository-local instructions.

The new predefined guardrails should not:

- replace `AGENTS.md`
- merge with `AGENTS.md`
- prevent `AGENTS.md` from adding stricter or more specific local constraints

Instead, they should act as a built-in coding baseline that repository instructions can refine.

**Alternative considered:** inject the guardrails after `AGENTS.md`. Rejected: placing them after repo-local instructions weakens their role as a stable built-in behavioral baseline and makes them feel more like an afterthought than a first-class instruction layer.

### Compatibility with prompt assembly

The implementation must preserve the repository's current prompt assembly model:

- the base system prompt remains its own prompt input
- the new Karpathy-inspired guardrails are injected as their own prompt input
- `AGENTS.md` remains a separate injected prompt input
- workflow context remains separate
- compatibility with both chat-completions-style backends and the Codex Responses backend is preserved

This is an instruction-layer refinement, not a prompt-assembly simplification.

**Alternative considered:** flatten the base system prompt, predefined guardrails, and `AGENTS.md` into one combined system message. Rejected: the repository explicitly avoids merging these layers, and flattening them would make precedence and maintenance less clear.

### Source framing in docs

The docs should briefly explain that this instruction layer is inspired by the commonly shared “Karpathy's `CLAUDE.md`” idea set, while avoiding language that implies an official upstream spec.

The documentation should summarize the applied topics in plain terms:

- assumption transparency
- simplicity
- minimal/surgical edits
- verification

When useful, docs may reference the Anthropic `CLAUDE.md` memory convention and the community `andrej-karpathy-skills` repository as background, but the repo docs should make clear that Themion is adopting only a minimal behavioral subset as a predefined instruction layer, not the full Claude Code mechanism.

**Alternative considered:** avoid any mention of the source inspiration and document only the final wording. Rejected: a short attribution/explanation helps future maintainers understand why these particular guardrails were selected and prevents cargo-cult prompt growth.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/` prompt assembly file(s) | Add a built-in predefined coding-guardrail prompt input and inject it before `AGENTS.md` while preserving the existing layer separation. |
| `crates/themion-core/src/` config/prompt source file(s) | Keep the configured base system prompt behavior unchanged; do not fold the new guardrails into the system prompt text. |
| `docs/engine-runtime.md` | Document the additional predefined instruction layer and show that it is injected after the base system prompt and before `AGENTS.md`. |
| `docs/architecture.md` | Update the prompt-assembly description so it reflects the new built-in guardrail layer and its order relative to the system prompt and repository instructions. |
| `docs/README.md` | Add this PRD to the PRD index and keep its status aligned with implementation progress. |

## Edge Cases

- a task is ambiguous but still safely solvable with an explicit assumption → the guardrails should encourage stating the assumption rather than forcing unnecessary clarification questions.
- a task is clearly specified and straightforward → the guardrails should not make the agent verbose or hesitant; it should still act directly.
- a repository `AGENTS.md` asks for stricter rules than the predefined layer → repository-local instructions should continue to layer on top without losing their local authority.
- a user explicitly requests a large refactor → the simplicity and surgical-change guidance should not forbid it, but should still discourage unrelated edits outside the requested scope.
- a task cannot be validated locally because required tools, network access, or credentials are unavailable → the agent should report that limitation rather than pretending verification occurred.
- provider backends serialize prompt inputs differently internally → the applied wording must remain backend-agnostic and not rely on provider-specific prompt features.
- a user provides a custom system prompt override → the predefined guardrail layer should still be injected separately unless implementation explicitly documents a different override contract.

## Migration

This change is additive and backward-compatible.

Existing configured `system_prompt` values should continue to work because the new behavior is introduced as a separate predefined instruction layer rather than as an in-place rewrite of the configured base system prompt.

After implementation, default sessions should exhibit slightly more disciplined coding-agent behavior without any workflow, database, or profile migration.

## Testing

- run a default session with no repository-local instruction file controlling coding style → verify: prompt assembly still includes the base system prompt plus the new predefined coding guardrails.
- inspect prompt assembly after the change → verify: the new guardrail layer appears before injected `AGENTS.md` content and remains separate from both the base system prompt and workflow context.
- exercise an ambiguous coding request in a controlled test or prompt snapshot → verify: the predefined guidance supports asking a brief clarifying question only when ambiguity is genuinely blocking.
- exercise a straightforward implementation request → verify: the predefined guidance still supports direct action rather than adding unnecessary delay or verbosity.
- run with a repository-local `AGENTS.md` file → verify: repository instructions are still injected separately after the predefined guardrail layer.
- run with a custom configured `system_prompt` override → verify: the override still applies as the base system prompt and the predefined guardrail layer is not silently merged into it.
- run chat-completions-style and Codex-style backend paths that rely on shared prompt assembly → verify: both continue to receive compatible prompt inputs with unchanged layer separation semantics.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: any prompt-assembly or documentation updates compile cleanly and do not break the workspace.
