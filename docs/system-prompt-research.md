# System Prompt Research for Themion

## Goal

Identify what system-prompt architecture Themion should adopt by comparing Themion's current prompt layering with patterns observed in the `openai/codex` and `badlogic/pi-mono` repositories, then recommend a balanced direction that preserves advantages while avoiding the major downsides of each approach.

## Scope and constraints

- This document is research only; it does not change implementation.
- The comparison focuses on prompt architecture, instruction layering, repo-local guidance, tool-use guidance, and task-mode behavior.
- The user requested balanced treatment of advantages and disadvantages from each repo.
- This work is limited to documentation changes under `docs/` in this repository.

## Sources reviewed

### Themion

- `docs/architecture.md`
- `docs/engine-runtime.md`
- `docs/codex-integration-guide.md`
- `docs/prd/prd-013-minimal-karpathy-inspired-system-prompt-guardrails.md`
- `docs/prd/prd-014-codex-cli-web-search-instruction-injection.md`
- `crates/themion-core/src/agent.rs`
- `crates/themion-cli/src/config.rs`
- root `AGENTS.md`

### Codex repo

- `openai/codex` root `AGENTS.md`
- `openai/codex/docs/agents_md.md`
- peer-agent findings from direct repo review of Codex prompt architecture

### pi-mono repo

- `badlogic/pi-mono` root `AGENTS.md`
- `badlogic/pi-mono/.pi/prompts/is.md`
- `badlogic/pi-mono/.pi/prompts/pr.md`
- `badlogic/pi-mono/.pi/prompts/cl.md`
- `badlogic/pi-mono/packages/coding-agent/README.md`
- peer-agent request for repo-specific findings

## Themion current state

Themion already uses a layered prompt model rather than a single merged prompt blob.

Observed order in `crates/themion-core/src/agent.rs`:

1. base system prompt from config
2. predefined coding guardrails
3. predefined Codex CLI web-search instruction
4. injected repository-local instructions from `AGENTS.md`
5. workflow context and phase instructions
6. optional recall hint for omitted turns
7. recent conversation window

This is a strong starting point. The repo already documents and implements two important principles that both comparison repos reinforce:

- stable built-in policy should remain separate from repo-local instructions
- dynamic runtime context should be injected separately instead of being folded into the main system prompt

Themion's open question is therefore not whether to layer prompt inputs, but how much structure, specialization, and operational detail each layer should contain.

## Findings from Codex

### What Codex appears to optimize for

Codex appears to optimize for predictable coding-agent behavior across many repositories and many tasks. Its prompt design emphasizes explicit structure, clear precedence, and concrete operational guidance.

### Architectural strengths worth learning from

#### 1. Strong instruction layering

A major Codex strength is that prompt inputs are layered rather than merged into one monolithic prompt. Durable base behavior, runtime overlays, user input, repo-local guidance, and tool/runtime context are treated as distinct inputs.

Why this matters for Themion:

- it keeps stable policy easier to maintain
- it makes conflicts easier to reason about
- it reduces pressure to keep inflating one giant base prompt

#### 2. Explicit precedence and scope

Codex reportedly makes precedence clearer, especially around repo-local guidance such as `AGENTS.md` and nested instruction scope. That is valuable because prompt conflicts become resolvable instead of accidental.

Why this matters for Themion:

- Themion already separates `AGENTS.md`, but could document scope and precedence more explicitly
- this becomes more important as Themion adds workflows, peer-message behavior, and more built-in prompt fragments

#### 3. Concrete operational tool guidance

Codex gives highly actionable tool guidance rather than generic encouragement to use tools. That reduces ambiguity around search, editing, validation, and fallback behavior.

Why this matters for Themion:

- Themion already nudges tool grounding, but some behaviors are still spread across system prompt text, developer instructions, and repo docs
- high-value operational guidance can improve consistency without needing a huge system prompt

#### 4. Task-specific guidance branches

Codex appears to vary guidance by task mode such as review, implementation, validation, or other specific work patterns. This is a strong idea because not every task should inherit the same response contract.

Why this matters for Themion:

- Themion already has workflows and phase instructions
- a task-mode layer could complement workflows for common intents like code review, research, implementation, or peer-to-peer coordination

#### 5. Prompt composition from fragments/templates

Codex's architecture suggests prompt fragments and templated assembly rather than repeated copy-pasted whole prompts.

Why this matters for Themion:

- it lowers drift risk
- it makes built-in instruction layers easier to revise independently
- it fits Themion's existing layered approach well

### Codex disadvantages and risks

#### 1. Prompt sprawl

The biggest downside is prompt bloat. A highly specified prompt can become long, repetitive, and costly to maintain.

Risk for Themion:

- context usage increases
- maintenance gets harder
- small inconsistencies between prompt fragments become more likely

#### 2. Rigid behavior from over-specification

Detailed formatting and behavior rules can improve determinism, but they can also make the agent less adaptive or overly mechanical.

Risk for Themion:

- the assistant could optimize for satisfying the prompt instead of the user's actual need
- a smaller TUI-oriented agent may not need the same level of answer-format contract as Codex

#### 3. Drift between variants

When similar prompt variants exist in multiple places, duplication can create subtle divergence.

Risk for Themion:

- if the same guidance is encoded in system prompt text, built-in guardrails, workflow instructions, docs, and AGENTS.md, behavior may drift in hard-to-debug ways

#### 4. Blurring policy vs UX preference

Codex seems to include both execution policy and product-style output rules. That can be useful, but it makes the system harder to reason about.

Risk for Themion:

- stable safety/truthfulness/tool-grounding rules should not be tangled with optional answer-style preferences

## Findings from pi-mono

### What pi-mono appears to optimize for

pi-mono appears to optimize for extensibility and user customization rather than one centrally maximized built-in prompt. It presents the coding agent as a minimal harness that users adapt with prompt templates, skills, extensions, themes, and repo-local rules.

### Architectural strengths worth learning from

#### 1. Minimal core philosophy

pi's README explicitly frames the agent as a minimal harness that users adapt to their workflow. That keeps the built-in product philosophy clear and avoids prematurely hardcoding every behavior into the core prompt.

Why this matters for Themion:

- Themion should resist turning the base system prompt into an ever-growing policy document
- compact durable defaults are easier to preserve across repositories and providers

#### 2. Prompt specialization through separate artifacts

pi-mono uses discrete prompt files such as `.pi/prompts/is.md` and `.pi/prompts/pr.md` for issue analysis and PR review. This is a practical pattern: task-specific instructions live outside the universal base prompt.

Why this matters for Themion:

- this maps well to optional task-mode prompt fragments
- it supports specialization without polluting all turns with every rule

#### 3. Repo-local operating rules are explicit

The root `AGENTS.md` in pi-mono is very operational: command restrictions, test policy, git discipline, changelog rules, and parallel-agent safety. This shows the value of keeping local repo operating rules outside the generic product prompt.

Why this matters for Themion:

- Themion already supports this model and should keep leaning into it
- repo-local rules are best handled as scoped contextual instructions, not universal behavior

#### 4. Customization over monolithic defaults

pi's package design pushes advanced behavior into prompt templates, skills, and extensions. This can keep the core assistant more adaptable.

Why this matters for Themion:

- not every advanced behavior has to live in the system prompt
- some behaviors belong in workflow/task selection or dedicated tools rather than in the base assistant identity

### pi-mono disadvantages and risks

#### 1. Less centralized consistency

A highly customizable architecture can reduce consistency between sessions, repos, or users.

Risk for Themion:

- if too much behavior is pushed outward, default quality may depend too heavily on repo-local instructions
- Themion benefits from stronger built-in defaults than a highly customizable framework may require

#### 2. Operational knowledge can become fragmented

When behavior lives across prompt templates, repo rules, commands, and extensions, it can be harder to know what instruction source is driving a given behavior.

Risk for Themion:

- Themion should avoid making prompt behavior so modular that users cannot reason about precedence

#### 3. Minimalism can under-specify important defaults

A minimal harness philosophy is appealing, but some critical coding-agent behaviors need a product-level default rather than optional add-ons.

Risk for Themion:

- truthfulness, narrow validation, targeted edits, repo-instruction precedence, and external research behavior should remain built-in defaults

## Comparative summary

### Codex-style strengths

- clear layered architecture
- explicit precedence
- strong operational tool guidance
- good support for task-specific behavior
- fragment/template-based assembly

### Codex-style weaknesses

- prompt length and maintenance burden
- risk of rigidity
- duplication/drift across variants
- policy and UX preferences can become mixed together

### pi-mono-style strengths

- compact core philosophy
- strong separation between core product behavior and repo-local operating rules
- task specialization through separate prompt files
- extensibility without forcing every rule into one prompt

### pi-mono-style weaknesses

- defaults may be less uniformly strong without extra local instructions
- behavior can become fragmented across many extension points
- precedence can become less obvious if not documented sharply

## Recommended prompt architecture for Themion

Themion should keep its current layered model, but formalize each layer's purpose more sharply.

### Recommended layers

#### 1. Core system prompt

Keep this small and durable.

It should cover only:

- assistant identity as a coding agent inside Themion
- truthfulness and non-guessing expectations
- tool-grounded behavior
- concise, direct communication
- preservation of user work and avoidance of destructive changes without instruction

It should not carry transient runtime state, repo-specific commands, or detailed task-mode logic.

#### 2. Built-in coding guardrails layer

Keep a compact built-in guardrail layer, similar to the current one.

It should cover cross-project defaults such as:

- avoid hidden assumptions
- prefer the simplest correct solution
- keep changes targeted
- validate with the narrowest useful check

This remains product-level policy, not repo policy.

#### 3. Built-in external research layer

Keep the Codex CLI external-research instruction separate, as Themion already does.

This is a good example of high-value operational guidance that does not belong in the core identity prompt.

#### 4. Repository-local instruction layer

Continue injecting `AGENTS.md` separately.

Improve documentation so Themion states more explicitly:

- repository-local instructions are authoritative within their scope
- they refine or constrain built-in defaults for that repository
- they should remain separate rather than merged into the base prompt

If Themion later adds nested instruction-file support, it should also document precedence clearly.

#### 5. Workflow/runtime context layer

Keep workflow, phase, and runtime execution state separate from the base prompt.

This layer should carry:

- workflow name and phase
- phase-specific execution guidance
- current runtime/task context
- peer-message context or sender/receiver semantics when applicable

This is the right place for transient operating context.

#### 6. Optional task-mode layer

This is the main recommended addition in principle, even if implementation comes later.

Themion would benefit from a compact optional layer for recognizable task intents such as:

- implementation
- code review
- research/comparison
- peer-agent collaboration

This should be shorter than Codex's full specialized prompt variants. The goal is not to create a separate giant prompt per mode, but to inject a concise behavior overlay when the task clearly matches a mode.

### What Themion should borrow from Codex

- explicit layered prompt architecture
- clearer documentation of precedence and scope
- concrete operational guidance for a few high-value behaviors
- fragment-based prompt composition rather than duplicated prompt variants
- targeted task-mode overlays where they materially improve behavior

### What Themion should borrow from pi-mono

- restraint in the base prompt
- separation of universal defaults from repo-local operating rules
- specialized prompt artifacts or overlays for special tasks instead of bloating every turn
- recognition that not every behavior belongs in the universal system prompt

### What Themion should avoid from both

- letting prompt fragments grow without sharply defined purpose
- duplicating the same guidance across multiple layers
- embedding repo-local or transient runtime state into the base system prompt
- creating so many extension points that users cannot tell which rule has priority

## Proposed target shape for Themion's system prompt

A good Themion system-prompt stack should look like this conceptually:

1. **Core system prompt**: identity, truthfulness, tool grounding, concise style, user-work preservation.
2. **Built-in guardrails**: assumption transparency, simplest correct solution, targeted edits, narrow validation.
3. **Built-in research guidance**: Codex CLI for current external information when needed.
4. **Repo-local instructions**: `AGENTS.md` and related project context.
5. **Workflow/runtime context**: workflow, phase, current execution mode, peer-message semantics.
6. **Optional task-mode overlay**: review, research, implementation, or collaboration overlays when clearly applicable.
7. **Conversation window and recall hints**.

This is closer to Codex in architecture, but closer to pi-mono in restraint.

## Practical recommendation

If Themion evolves its prompt system further, the safest direction is:

- keep the core system prompt short
- keep built-in product defaults separate from repo-local rules
- add only a small number of high-value specialized overlays
- document precedence more explicitly than today
- prefer reusable prompt fragments over new monolithic prompt variants

In short: Themion should adopt Codex's discipline about layers and precedence, while adopting pi-mono's discipline about keeping the core prompt small.

## Suggested follow-up docs work

If the project wants to turn this research into implementation or a PRD later, likely follow-ups would be:

- document prompt-layer precedence more explicitly in `docs/engine-runtime.md`
- define whether Themion wants a first-class task-mode overlay concept in addition to workflows
- document how peer-message instructions relate to workflow instructions and repo-local instructions
- ensure any future prompt fragment has a narrow purpose and a single canonical source
