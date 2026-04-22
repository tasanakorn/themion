# System Prompt Research for Themion

## Goal

Identify what prompt architecture Themion should adopt by comparing Themion's current layering with patterns observed in `openai/codex` and `badlogic/pi-mono`, then recommend a direction that keeps Themion's current strengths while improving precedence clarity, prompt safety, debuggability, and extensibility.

## Scope and constraints

- This document is research only; it does not change implementation.
- The comparison focuses on prompt architecture, instruction layering, repo-local guidance, tool-use guidance, workflow/task-mode behavior, and multi-agent implications.
- The user requested balanced treatment of advantages and disadvantages from each repo.
- This work is limited to documentation changes under `docs/` in this repository.

## Executive summary

Themion already has the right high-level instinct: it does **not** merge everything into one giant system prompt. It already separates the configured base prompt, built-in guardrails, built-in Codex CLI research guidance, repository-local `AGENTS.md`, and workflow/runtime context.

The strongest conclusion from fresh Codex and pi-mono comparison is not that Themion needs a radically different model. It is that Themion should make its existing model more explicit and more principled.

The main recommendations are:

- keep the base system prompt small and durable
- treat prompt inputs as a structured stack, not just concatenated text
- document authority and precedence explicitly
- treat repo files, tool output, logs, and web content as untrusted data unless promoted by a higher-priority layer
- keep workflow/task overlays separate from the immutable core
- make multi-agent prompting an explicitly separate prompt domain
- prefer a few narrowly-scoped high-value overlays over a proliferation of prompt variants
- improve provenance and observability so the system can explain where an instruction came from

In short: Themion should move **closer to Codex in explicit hierarchy and operational boundaries**, while staying **closer to pi-mono in core-prompt restraint and composability**.

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
- `openai/codex/codex-rs/protocol/src/prompts/base_instructions/default.md`
- `openai/codex/codex-rs/core/src/agents_md.rs`
- `openai/codex/codex-rs/core/hierarchical_agents_message.md`
- peer-agent findings from direct repo review of Codex prompt architecture and recent external prompt/agent guidance

### pi-mono repo

- `badlogic/pi-mono` root `AGENTS.md`
- `badlogic/pi-mono/.pi/prompts/is.md`
- `badlogic/pi-mono/.pi/prompts/pr.md`
- `badlogic/pi-mono/.pi/prompts/cl.md`
- `badlogic/pi-mono/packages/coding-agent/README.md`
- `badlogic/pi-mono/packages/coding-agent/src/core/system-prompt.ts`
- `badlogic/pi-mono/packages/coding-agent/docs/extensions.md`
- `badlogic/pi-mono/packages/coding-agent/examples/extensions/prompt-customizer.ts`
- `badlogic/pi-mono/packages/coding-agent/examples/extensions/subagent/README.md`
- peer-agent repo-specific findings on prompt architecture and extension-time prompt mutation

## Themion current state

Themion already uses a layered prompt model rather than a single merged prompt blob.

Observed order in `crates/themion-core/src/agent.rs` and the current docs:

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

Themion's open question is therefore not whether to layer prompt inputs, but how much structure, specialization, safety framing, and provenance each layer should carry.

## What has become newly important in 2024-2025

Older prompt-engineering advice often treated success as a matter of writing a better single prompt. The newer pattern across current coding-agent systems and recent safety guidance is different.

### 1. Instruction hierarchy is a first-class design concern

Modern agent behavior is increasingly framed as a chain of authority, not a flat bag of instructions. The durable pattern is:

1. platform/system policy
2. developer/operator instructions
3. repo/project-local instructions
4. user task instructions
5. untrusted retrieved data, files, tool output, logs, and web content

Themion already approximates this structurally, but the hierarchy should be documented more explicitly.

### 2. Prompt injection is a containment problem, not just a wording problem

Recent guidance across vendors and security sources converges on the same lesson: the correct defense is not merely "write a stronger prompt." Practical resilience comes from authority separation, least privilege, approval gates, explicit treatment of untrusted content, and observability.

This matters for Themion because coding agents routinely inspect:

- repository files
- `README`s and docs
- issue text and PR descriptions
- terminal output and logs
- web-research results
- peer-agent or delegated results

Those artifacts may contain instructions, but they should generally be treated as data unless adopted by a higher-priority layer.

### 3. Capability-sensitive prompting is more robust than static capability claims

pi-mono now assembles prompt guidance from the actually enabled tools and structured prompt inputs. This is stronger than a static prompt that describes tools or workflows that may not exist in the current run.

Themion already injects some runtime-specific guidance, but this principle could be applied more explicitly.

### 4. Verification contracts matter more than generic "be careful" advice

High-performing coding-agent systems now emphasize concrete validation loops: narrow tests first, then broader checks; explicit reporting of blockers; and clear expectations around what counts as sufficient verification.

Themion already includes this spirit in built-in guardrails and repo-local instructions, but it should preserve this as an explicit architectural layer rather than allow it to diffuse into scattered wording.

### 5. Memory and context management are reliability concerns, not just convenience features

Long-lived prompt stacks, repeated overlays, and persistent instructions can erode adherence. Compact stable policy plus scoped transient overlays is increasingly more reliable than putting every rule into one permanent message.

### 6. Provenance and observability are now part of prompt architecture

If a system cannot explain where an instruction came from, conflicts become difficult to debug. pi-mono's recent structured resource/source work and Codex's explicit `AGENTS.md` handling both point toward the same architectural lesson: instruction sources should be inspectable and attributable.

## Findings from Codex

### What Codex appears to optimize for

Codex appears to optimize for predictable coding-agent behavior across many repositories and many tasks. Its prompt design emphasizes explicit structure, clear precedence, concrete operational guidance, and model-visible handling of scoped repo instructions.

### Repo-grounded evidence

Codex's current base prompt artifacts explicitly describe an `AGENTS.md` authority model in `codex-rs/protocol/src/prompts/base_instructions/default.md` and related prompt copies. They specify:

- `AGENTS.md` files may appear anywhere in a repo tree
- each file governs the directory subtree rooted at its location
- deeper files override higher-level ones when they conflict
- direct system, developer, and user instructions outrank `AGENTS.md`

Codex also has explicit supporting machinery around this model in `codex-rs/core/src/agents_md.rs` and `codex-rs/core/hierarchical_agents_message.md`, which shows that prompt architecture is treated as runtime policy, not just documentation prose.

### Architectural strengths worth learning from

#### 1. Strong instruction layering

A major Codex strength is that prompt inputs are layered rather than merged into one monolithic prompt. Durable base behavior, runtime overlays, repo-local guidance, and user/context data are treated as distinct inputs.

Why this matters for Themion:

- it keeps stable policy easier to maintain
- it makes conflicts easier to reason about
- it reduces pressure to keep inflating one giant base prompt

#### 2. Explicit precedence and scope

Codex is strong on making precedence visible, especially around repo-local guidance such as `AGENTS.md` and nested instruction scope.

Why this matters for Themion:

- Themion already separates `AGENTS.md`, but could document scope and precedence more explicitly
- this becomes more important as Themion adds workflows, peer-message behavior, and more built-in prompt fragments

#### 3. Concrete operational tool guidance

Codex gives highly actionable tool guidance rather than generic encouragement to use tools. It specifies behaviors around search, editing, plans, validation, and final reporting.

Why this matters for Themion:

- Themion already nudges tool grounding, but some behaviors are still split across system prompt text, repo docs, and developer instructions
- high-value operational guidance can improve consistency without requiring a giant base prompt

#### 4. Task-specific behavior framing

Codex uses specialized task logic and workflow-specific instructions in ways that suggest not every task should inherit the exact same response contract.

Why this matters for Themion:

- Themion already has workflows and phase instructions
- a task-mode layer could complement workflows for common intents like code review, research, implementation, or peer coordination

#### 5. Strong distinction between instructions and evidence

Codex's guardian/review-related prompting explicitly treats transcripts, tool outputs, and related artifacts as untrusted evidence rather than instructions to obey.

Why this matters for Themion:

- this is a strong modern pattern for prompt-injection resistance
- Themion can borrow the underlying principle even outside a dedicated guardian subsystem

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

Codex contains similar prompt guidance in multiple prompt artifacts and generated/bundled representations. That helps compatibility, but it also creates drift risk.

Risk for Themion:

- if the same guidance is encoded in system prompt text, built-in guardrails, workflow instructions, docs, and `AGENTS.md`, behavior may drift in hard-to-debug ways

#### 4. Blurring policy vs UX preference

Codex includes both execution policy and product-style output rules. That can be useful, but it makes the system harder to reason about.

Risk for Themion:

- stable safety/truthfulness/tool-grounding rules should not be tangled with optional answer-style preferences

## Findings from pi-mono

### What pi-mono appears to optimize for

pi-mono appears to optimize for extensibility and user customization rather than one centrally maximized built-in prompt. It presents the coding agent as a minimal harness that users adapt with prompt templates, skills, extensions, themes, and repo-local rules.

### Repo-grounded evidence

Fresh pi-mono review shows that its current prompt system is intentionally assembled from structured inputs in `packages/coding-agent/src/core/system-prompt.ts`, including:

- base instructions
- tool-sensitive guidance derived from available tools
- appended guidelines
- discovered context files
- skills
- execution facts such as date and cwd

A particularly relevant recent change is that `before_agent_start` now exposes `systemPromptOptions` (`BuildSystemPromptOptions`) so extensions can inspect structured prompt ingredients rather than rediscover them from text. This is documented in `packages/coding-agent/CHANGELOG.md`, `packages/coding-agent/docs/extensions.md`, and the `prompt-customizer.ts` example.

pi also explicitly documents:

- automatic loading of `AGENTS.md` and `CLAUDE.md`
- `--no-context-files`
- `.pi/SYSTEM.md` for replacement semantics
- `APPEND_SYSTEM.md` for append semantics
- prompt templates and extensions as distinct mechanisms
- subagent examples that run in isolated contexts with separate prompt domains

### Architectural strengths worth learning from

#### 1. Minimal core philosophy

pi's README explicitly frames the agent as a minimal harness that users adapt to their workflow.

Why this matters for Themion:

- Themion should resist turning the base system prompt into an ever-growing policy document
- compact durable defaults are easier to preserve across repositories and providers

#### 2. Structured prompt composition rather than raw concatenation

pi's newer architecture makes prompt construction inspectable as structured inputs, not just a final string.

Why this matters for Themion:

- it enables safer customization
- it makes provenance clearer
- it creates a better foundation for extensions or future task overlays than ad hoc string splicing

#### 3. Prompt specialization through separate artifacts

pi-mono uses discrete prompt files such as `.pi/prompts/is.md` and `.pi/prompts/pr.md` for issue analysis and PR review.

Why this matters for Themion:

- this maps well to optional task-mode prompt fragments
- it supports specialization without polluting all turns with every rule

#### 4. Repo-local operating rules are explicit and scoped

The root `AGENTS.md` in pi-mono is very operational: command restrictions, test policy, git discipline, changelog rules, and multi-agent guidance.

Why this matters for Themion:

- Themion already supports this model and should keep leaning into it
- repo-local rules are best handled as scoped contextual instructions, not universal behavior

#### 5. Replacement, append, and discovered-context semantics are distinct

pi distinguishes between replacing the base system prompt, appending to it, and discovering project/user context files.

Why this matters for Themion:

- these are different kinds of authority and should not be conflated
- this separation improves debuggability and makes precedence easier to reason about

#### 6. Multi-agent behavior is treated as a separate prompt domain

pi's subagent example isolates context windows and delegated system prompts per agent/process rather than treating collaboration as a few extra paragraphs in one shared prompt.

Why this matters for Themion:

- multi-agent prompting should not be modeled as one overloaded monologue
- delegated-agent prompts are authority surfaces and deserve explicit trust boundaries

### pi-mono disadvantages and risks

#### 1. Less centralized consistency

A highly customizable architecture can reduce consistency between sessions, repos, or users.

Risk for Themion:

- if too much behavior is pushed outward, default quality may depend too heavily on repo-local instructions or extensions
- Themion benefits from stronger built-in defaults than a highly customizable framework may require

#### 2. Operational knowledge can become fragmented

When behavior lives across prompt templates, repo rules, skills, and extensions, it can be harder to know what instruction source is driving a given behavior.

Risk for Themion:

- Themion should avoid making prompt behavior so modular that users cannot reason about precedence

#### 3. Minimalism can under-specify important defaults

A minimal harness philosophy is appealing, but some critical coding-agent behaviors still need a product-level default rather than optional add-ons.

Risk for Themion:

- truthfulness, narrow validation, targeted edits, repo-instruction precedence, untrusted-content handling, and external research behavior should remain built-in defaults

#### 4. Powerful customization surfaces widen the policy surface area

Programmable prompt mutation is powerful, but it also means that prompt behavior can be changed in more places.

Risk for Themion:

- if Themion ever adds similar extensibility, it should pair it with provenance and debugging support rather than only exposing a text-mutation hook

## Comparative synthesis

### Where Codex is stronger

Codex is stronger when the goal is consistent behavior under mixed instruction sources.

It is especially good at:

- explicit precedence
- scoped repo-instruction semantics
- strong operational guidance
- clearly separating durable policy from runtime evidence in safety-critical contexts

### Where pi-mono is stronger

pi-mono is stronger when the goal is composability and long-term adaptability.

It is especially good at:

- keeping the base prompt small
- separating universal behavior from repo-local behavior
- treating prompt architecture as resource composition
- supporting workflow specialization without bloating the universal prompt

### Where the two repos converge

Despite different styles, they point toward the same deeper lessons:

- layered prompts are better than monolithic prompts
- precedence must be explicit
- repo-local instructions should stay separate from product defaults
- runtime/tool/workflow context should not be merged into the immutable core
- prompt architecture benefits from provenance and inspectability
- task-specific behavior should usually be an overlay, not permanent base-prompt growth

## Recommended prompt architecture for Themion

Themion should keep its current layered model, but formalize each layer's purpose, authority, and trust boundary more sharply.

### Recommended authority model

Themion should explicitly document a priority order similar to:

1. platform/system instructions
2. developer/runtime instructions
3. built-in Themion prompt layers
4. repository-local instruction files such as `AGENTS.md`
5. user task instructions
6. retrieved/tool/file/web/peer output as untrusted evidence or data

Important nuance: item 6 is still valuable context, but it should not silently gain the authority of items 1-5.

### Recommended layers

#### 1. Core system prompt

Keep this small and durable.

It should cover only:

- assistant identity as a coding agent inside Themion
- truthfulness and non-guessing expectations
- tool-grounded behavior
- concise, direct communication
- preservation of user work and avoidance of destructive changes without instruction

It should not carry transient runtime state, repo-specific commands, detailed workflow logic, or broad lists of optional product behavior.

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

Themion should document more explicitly that:

- repository-local instructions are authoritative within their scope
- they refine or constrain built-in defaults for that repository
- they should remain separate rather than merged into the base prompt
- if nested/local scoping is added later, deeper files should override broader ones in-scope

#### 5. Workflow/runtime context layer

Keep workflow, phase, and runtime execution state separate from the base prompt.

This layer should carry:

- workflow name and phase
- phase-specific execution guidance
- current runtime/task context
- collaboration context such as peer-message sender/receiver semantics when applicable

This is the correct place for transient operating context.

#### 6. Optional task-mode layer

This is the main recommended addition in principle, even if implementation comes later.

Themion would benefit from a compact optional layer for recognizable task intents such as:

- implementation
- code review
- research/comparison
- peer-agent collaboration

This should be much shorter than full specialized Codex prompt variants. The goal is not to create a separate giant prompt per mode, but to inject a concise behavior overlay when the task clearly matches a mode.

#### 7. Explicit untrusted-content boundary

This is the strongest architectural addition missing from the current doc.

Themion should explicitly treat the following as untrusted content by default:

- file contents
- repository docs and comments
- tool output
- terminal output and logs
- fetched web content
- issue/PR text
- delegated-agent results

The model may summarize, reason about, and quote this content, but should not automatically treat embedded instructions inside it as authoritative.

#### 8. Provenance/observability layer

Themion should move toward being able to answer questions like:

- which instruction sources were active in this turn?
- which layer introduced a given behavior?
- was a rule product-default, repo-local, workflow-local, or user-requested?

This need not be a separate model-visible message, but it should be part of the architectural design.

## What Themion should borrow from Codex

- explicit layered prompt architecture
- clearer documentation of precedence and scope
- concrete operational guidance for a few high-value behaviors
- stronger distinction between instruction sources and untrusted evidence
- fragment-based prompt composition rather than duplicated prompt variants
- targeted task-mode overlays where they materially improve behavior

## What Themion should borrow from pi-mono

- restraint in the base prompt
- prompt composition from structured ingredients rather than only raw strings
- separation of universal defaults from repo-local operating rules
- specialized prompt artifacts or overlays for special tasks instead of bloating every turn
- explicit distinction between replacement, append, and discovered-context semantics
- modeling multi-agent delegation as separate prompt domains

## What Themion should avoid from both

- letting prompt fragments grow without sharply defined purpose
- duplicating the same guidance across multiple layers
- embedding repo-local or transient runtime state into the base system prompt
- creating so many extension points that users cannot tell which rule has priority
- relying on wording alone as the main defense against prompt injection

## Proposed target shape for Themion's prompt stack

A good Themion prompt stack should look like this conceptually:

1. **Core system prompt**: identity, truthfulness, tool grounding, concise style, user-work preservation.
2. **Built-in guardrails**: assumption transparency, simplest correct solution, targeted edits, narrow validation.
3. **Built-in research guidance**: Codex CLI for current external information when needed.
4. **Repo-local instructions**: `AGENTS.md` and related project context.
5. **Workflow/runtime context**: workflow, phase, current execution mode, collaboration semantics.
6. **Optional task-mode overlay**: review, research, implementation, or collaboration overlays when clearly applicable.
7. **Conversation window and recall hints**.

Surrounding that stack, the runtime should maintain two non-textual architectural commitments:

- **authority/precedence rules** for resolving conflicts
- **untrusted-content boundaries** for files, tools, logs, web content, and delegated output

This is closer to Codex in architecture, but closer to pi-mono in restraint.

## Practical recommendation

If Themion evolves its prompt system further, the safest direction is:

- keep the core system prompt short
- keep built-in product defaults separate from repo-local rules
- add only a small number of high-value specialized overlays
- document precedence more explicitly than today
- define untrusted-content handling explicitly rather than leaving it implicit
- prefer reusable prompt fragments over new monolithic prompt variants
- preserve provenance so prompt behavior stays explainable as the system grows

In short: Themion should adopt Codex's discipline about layers, authority, and safety boundaries, while adopting pi-mono's discipline about keeping the core prompt small and composable.

## Risks and uncertainties in this research

- This document compares architecture and documented behavior, not benchmarked outcome quality.
- Some Codex and pi-mono findings come from prompt artifacts, docs, tests, and examples rather than one single canonical architecture spec.
- Some of the newer lessons cited here are durable cross-vendor themes, but the exact mechanisms used by any one product may change quickly.
- Themion's current docs describe its own layer order clearly, but some future recommendations here, such as stronger untrusted-content wording or first-class task-mode overlays, are still design recommendations rather than implemented behavior.

## Suggested follow-up docs work

If the project wants to turn this research into implementation or a PRD later, likely follow-ups would be:

- document prompt-layer precedence more explicitly in `docs/engine-runtime.md`
- document untrusted-content boundaries for repo files, tool output, logs, web research, and delegated-agent results
- define whether Themion wants a first-class task-mode overlay concept in addition to workflows
- document how peer-message instructions relate to workflow instructions and repo-local instructions
- define whether instruction-source provenance should become visible in diagnostics or status output
- ensure any future prompt fragment has a narrow purpose and a single canonical source
