# PRD-014: Codex CLI Web-Search Instruction Injection

- **Status:** Implemented
- **Version:** v0.8.0
- **Scope:** `themion-core` (prompt assembly / predefined instruction injection), docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Add a built-in instruction layer that explicitly tells Themion to use Codex CLI for web-search-style research when the task needs current external information that built-in tools cannot provide.
- Keep that instruction separate from the base system prompt, existing predefined coding guardrails, repository-local instructions such as `AGENTS.md`, and workflow context.
- Make the instruction narrow and intentional: it should cover web-search-like external research needs, not arbitrary external tool delegation.
- Improve the default agent behavior for tasks that depend on current upstream documentation, online references, or other non-local information.
- Preserve compatibility with both chat-completions-style backends and the Codex Responses backend by expressing the behavior as an injected prompt input rather than provider-specific runtime branching.

## Non-goals

- No implementation of a new built-in external-tool runtime in this PRD.
- No commitment to arbitrary external program delegation.
- No change to the existing local shell tool contract beyond guiding the model toward a specific intended use.
- No merging of this instruction into the configured base system prompt text.
- No replacement of repository-local instruction files such as `AGENTS.md`.
- No promise that Codex CLI is always installed, authenticated, or available.

## Background & Motivation

### Current state

Themion already has:

- a base system prompt from configuration
- a predefined coding-guardrail layer
- separate injected repository-local instructions such as `AGENTS.md`
- local workspace tools including `shell_run_command`

This gives the agent a good local coding workflow, but it does not currently provide a dedicated built-in web-search tool.

When a task depends on current external information, the agent can end up in an awkward middle state:

- it knows the information may exist online
- it has a generic shell tool
- but it is not explicitly instructed that using Codex CLI for web-search-style research is the preferred path

As a result, the model may either stop too early, ask the user to do research manually, or fail to recognize that Codex CLI is the intended external helper.

### Why prompt injection is the right first step

This repository already treats prompt inputs as layered and separate:

1. base system prompt
2. predefined built-in instruction layers
3. repository-local instructions such as `AGENTS.md`
4. workflow context and phase instructions
5. recent conversation and recall hints

That architecture is well suited to introducing a narrow behavioral instruction without prematurely building a new runtime feature.

The desired product behavior is primarily instructional:

- when local tools are insufficient for a current-information task
- the agent should know that Codex CLI is the preferred external helper for web-search-style research
- and should use it through the existing shell capability in a constrained, explicit way

This means the first implementation can be a prompt-assembly change rather than a larger external-tool framework.

## Design

### New predefined instruction layer for Codex CLI web search

Themion should inject a short built-in instruction layer that tells the agent:

- use Codex CLI for web-search-style research when the task requires current or external information unavailable in the local repository
- prefer that path over pretending certainty or immediately asking the user to research manually
- keep the use narrow and task-directed
- report clearly when Codex CLI is unavailable or fails

The exact wording may vary, but the instruction should normatively communicate all four expectations.

An example acceptable shape is:

> When a task needs current external information or web search beyond the local repository, use Codex CLI via the shell as the preferred research path when available. Keep the query focused on the task, summarize the result for the user, and if Codex CLI is unavailable or fails, say so clearly instead of pretending to know.

This instruction should be built into Themion rather than relying on each repository to restate it in `AGENTS.md`.

**Alternative considered:** add only documentation and expect users or repositories to teach this behavior through custom system prompts or `AGENTS.md`. Rejected: this behavior is intended as a product-level default, so it should be present even in repositories without local prompt customization.

### Prompt ordering

This new Codex CLI instruction should remain a separate injected prompt input and should be ordered consistently with the repository's prompt-layer model.

Preferred conceptual order:

1. base system prompt
2. predefined coding guardrails
3. predefined Codex CLI web-search instruction
4. repository-local instructions such as `AGENTS.md`
5. workflow context and phase instructions
6. recall hint when older turns are omitted
7. recent conversation window

This keeps the Codex CLI behavior as a built-in default while still allowing repository-local instructions to refine or further constrain it.

**Alternative considered:** merge the Codex CLI instruction into the existing predefined coding guardrails block. Rejected: using Codex CLI for web search is a distinct behavioral policy, not a general coding guardrail, and keeping it separate preserves maintainability and prompt-layer clarity.

### Scope of the instruction

The instruction must stay narrow.

It should steer the model toward Codex CLI specifically for cases such as:

- checking current upstream documentation
- confirming recent API behavior
- finding examples or references not present in the local repository
- retrieving external information that is time-sensitive or version-sensitive

It should not imply:

- general permission to run arbitrary external tools for unrelated purposes
- a requirement to use Codex CLI for purely local coding tasks
- a promise that every external-information task can be solved automatically

The model should still prefer local repository evidence first when the answer already exists locally.

**Alternative considered:** introduce a broad instruction to use any available shell command for external research. Rejected: that is too open-ended for the product need described here and weakens the intent to make Codex CLI the specific, narrow preferred path.

### Interaction with existing tools

This PRD does not add a new runtime tool. Instead, it changes how the model is instructed to use the existing shell capability.

Normative expectations:

- the model may use `shell_run_command` to invoke Codex CLI when external research is needed
- the model should keep commands focused on the user task rather than launching open-ended external workflows
- command failure, missing installation, or missing authentication should be surfaced clearly to the user
- if local repository information is sufficient, the model should not use Codex CLI unnecessarily

This preserves the current runtime architecture while making the preferred research path explicit.

**Alternative considered:** wait for a dedicated `web_search` or `codex_search` tool before teaching the behavior. Rejected: the current architecture already has a viable path through the shell tool, and the product need can be addressed earlier through prompt guidance.

### Interaction with `AGENTS.md`

`AGENTS.md` should remain a separate injected instruction source.

Repository-local instructions may:

- further constrain how Codex CLI is used
- discourage it for specific repositories or environments
- require additional reporting or verification after external research

They should not need to restate the product-default behavior just to make the agent aware of Codex CLI as the intended web-search path.

**Alternative considered:** put this instruction in the root `AGENTS.md` instead of built-in prompt assembly. Rejected: root `AGENTS.md` is repository-local guidance, while this behavior is intended to be a built-in cross-project default.

### Documentation expectations

The docs should describe this as a built-in instruction-layer change, not as a new runtime tool.

They should explain:

- when the agent is expected to use Codex CLI
- that the behavior is expressed through prompt injection
- that Codex CLI is the preferred path for web-search-like external research when available
- that failure or unavailability should be reported plainly

The docs should also preserve the repo's established distinction between:

- system prompt
- predefined built-in instruction layers
- repository-local instructions
- workflow context

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/` prompt assembly file(s) | Add a new predefined injected instruction for Codex CLI web-search usage and place it after the coding guardrails but before `AGENTS.md`. |
| `crates/themion-core/src/` predefined instruction source file(s) | Store the Codex CLI web-search instruction as its own built-in prompt constant rather than merging it into the base system prompt or coding guardrails text. |
| `docs/engine-runtime.md` | Document the new predefined Codex CLI instruction layer and its role in prompt assembly. |
| `docs/architecture.md` | Update the prompt-assembly description so the predefined instruction order includes the Codex CLI web-search instruction layer. |
| `docs/README.md` | Keep the PRD title, status, and scope aligned with the rewritten document. |

## Edge Cases

- Codex CLI is not installed on the user's machine → the agent should report that clearly rather than pretending web research succeeded.
- Codex CLI is installed but not authenticated or otherwise unusable → the agent should surface the failure plainly.
- the answer already exists in local docs or source → the model should prefer local evidence and avoid unnecessary external lookup.
- a repository's `AGENTS.md` explicitly forbids networked or external research → repository-local instructions should still be able to override the built-in default behavior within their scope.
- the model needs external information but the shell tool itself is unavailable or blocked → the agent should state that limitation clearly.
- the task asks for speculative design advice rather than current factual research → the agent should not force Codex CLI usage when external lookup is unnecessary.
- different providers receive prompt inputs through different wire formats internally → the instruction must remain backend-agnostic and preserve the existing prompt-layer separation semantics.

## Migration

This change is additive and backward-compatible.

Existing `system_prompt` overrides continue to work because the Codex CLI guidance is introduced as a separate predefined instruction layer rather than a silent rewrite of configured prompt text.

Repositories with their own `AGENTS.md` files do not need migration. They may optionally tighten or refine this behavior, but the built-in default should become available automatically after implementation.

## Testing

- inspect prompt assembly after the change → verify: the Codex CLI web-search instruction appears as a separate injected layer after predefined coding guardrails and before `AGENTS.md`.
- run a session in a repository without local Codex CLI guidance → verify: the built-in prompt still includes explicit direction to use Codex CLI for web-search-style research when needed.
- run a task that can be answered from local repository files alone → verify: the prompt change does not imply unnecessary Codex CLI usage for purely local questions.
- run a task that depends on current external information → verify: the prompt assembly gives the model an explicit built-in path pointing to Codex CLI rather than only generic shell access.
- run with a repository-local `AGENTS.md` that further constrains external research → verify: repository-local instructions still appear separately after the built-in Codex CLI instruction layer.
- run chat-completions-style and Codex-style backend paths using shared prompt assembly → verify: both continue to receive compatible prompt inputs with unchanged layer-separation behavior.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: prompt-assembly and documentation changes compile cleanly.
