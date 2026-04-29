# PRD-066: Guide Model Instructions for Human Response Length and Chunking

- **Status:** Implemented
- **Version:** v0.42.0
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-29

## Implementation status

Landed in `v0.42.0` as a focused prompt-guidance update plus docs follow-through. The implemented guardrail now covers both response sizing and format escalation: plain prose first for simple answers, bullets/headings/tables only when their extra structure materially helps, about 4±1 meaningful chunks by default when more structure is needed, expansion toward about 7±2 chunks only for user-requested fuller explanation, chunk counting that treats major sections/comparison units as structure, and answer-first ordering for recommendation or next-action style replies. Prompt assembly remains unchanged; the behavior continues to live in the predefined guardrail layer in `themion-core`.

## Summary

- Themion's built-in instruction layer already tells the model to be concise and to preserve important tool-learned findings in chat, but it does not yet give one explicit default policy for how to size ordinary human-facing answers.
- Add a small built-in response-shaping rule: prefer a complete 1–2 sentence answer when that is enough, otherwise organize the answer into about 4±1 meaningful chunks, and allow expansion toward about 7±2 chunks only when the user explicitly asks for a fuller explanation that does not fit the smaller structure. Also make format escalation explicit: prefer plain prose first, add bullets only when they improve scanning, add headings only for genuinely distinct parts, and reserve tables for cases where comparison is materially clearer that way.
- Keep this as a heuristic for human readability rather than a rigid counting rule, so the model can still answer naturally, correctly, and in the shape the task requires.
- Preserve all existing prompt-layer separation and current tool-summary guidance; this PRD only adds a clearer default answer-organization rule to the predefined guardrail layer.
- The implementation target is intentionally small: adjust shared guardrail wording, keep prompt assembly unchanged, and update the runtime/architecture docs to describe the landed behavior accurately.

## Goals

- Add explicit built-in guidance for how Themion should size and structure ordinary human-facing responses.
- Prefer a direct 1–2 sentence answer when the user's request can be handled clearly without extra structure.
- Default to a Cowan-inspired 4±1 chunking heuristic for ordinary explanations, summaries, recommendations, and status reports that need more than 1–2 sentences.
- Make format escalation explicit so the model prefers plain prose first, treats bullets and headings as optional structure, and uses tables only when comparison clarity justifies them.
- Allow a Miller-inspired 7±2 chunk range only when the user explicitly asks for a fuller explanation and the answer would not fit comfortably within the default 4±1 structure.
- Keep the new behavior compatible with existing concise tool-summary guidance, coding guardrails, workflow guidance, and prompt-layer separation.
- Make the implementation target specific enough that it can be landed as a focused prompt-guidance and docs update in `themion-core`.

## Non-goals

- No change to provider APIs, tool contracts, workflow mechanics, history behavior, or TUI behavior.
- No requirement to count exact sentences, bullets, paragraphs, or tokens mechanically.
- No change to the existing rule that important tool-learned findings should be summarized into ordinary assistant chat text when useful.
- No attempt to turn cognitive-psychology heuristics into hard correctness constraints.
- No guarantee that every answer will fit a fixed number of top-level bullets if the user's request naturally needs a different shape.
- No new prompt message layer, no new backend abstraction, and no special transcript markup format.

## Background & Motivation

### Current state

Themion already uses a layered prompt assembly model with:

- a base system prompt
- predefined coding guardrails
- a predefined Codex CLI web-search instruction
- contextual instruction files such as `AGENTS.md`
- workflow context and phase instructions

The built-in guardrail layer already pushes the model toward concise, practical behavior in a few separate ways:

- be concise, direct, and helpful
- keep formatting light unless structure materially helps
- summarize important tool-learned findings into chat, usually in 1–2 sentences by default

That guidance is useful, but it still leaves a gap: there is no single explicit default policy for how the assistant should choose between a very short answer and a somewhat longer structured answer.

In practice, that can produce inconsistent behavior:

- short requests sometimes receive more structure than they need
- ordinary explanations sometimes contain too many same-level points to scan comfortably
- explicit requests for fuller explanation can be over-compressed because the model is trying to stay terse everywhere

### Why this belongs in built-in prompt guidance

The requested behavior is a product-level answer-style default, not a repository-local convention.

It should therefore live in the same built-in guardrail layer that already tells the model to:

- be direct and practical
- avoid unnecessary verbosity
- summarize important tool findings into ordinary transcript text

This keeps the behavior:

- consistent across repositories
- independent of whether a project has a strong `AGENTS.md`
- compatible with the repo's documented prompt layering, where built-in guardrails are separate from contextual instruction files

### Why these heuristics are useful

The requested heuristics suggest a practical three-tier policy:

- if the answer can be completed well in 1–2 sentences, prefer that
- if more explanation is needed, default to about 4±1 meaningful chunks
- if the user explicitly asks for deeper explanation and the answer does not fit the smaller structure, allow about 7±2 chunks instead

This improves readability without pretending the assistant should literally count memory slots, bullets, or sentences with mathematical precision. The value is in shaping the answer around a small number of scannable ideas.

**Alternative considered:** add only a blanket "be concise" rule. Rejected: too vague to tell the model when a longer answer should still be organized into a bounded number of digestible chunks.

## Design

### Design principles

- Keep the new behavior as a small built-in guardrail wording change rather than a runtime or protocol redesign.
- Treat 1–2 sentences, 4±1, and 7±2 as readability heuristics, not rigid quotas.
- Prefer semantic chunks such as short paragraphs, bullets, or numbered steps over literal counting of sentences or tokens.
- Treat headings, compact subsections, and table/comparison units as chunk-bearing structure rather than free decoration.
- Let explicit user intent override the default compactness when the user asks for a deeper explanation.
- Avoid conflict with existing tool-summary guidance by making this a broader answer-organization rule for the full human-facing response.
- Preserve prompt-layer separation exactly as documented today.

### 1. Add a built-in response-sizing rule to the predefined guardrails

Themion should implement this behavior by updating the shared built-in guardrail text in `crates/themion-core/src/predefined_guardrails.rs`.

The new wording should instruct the model to:

- answer in 1–2 sentences when that is enough to fully answer the user
- prefer plain direct prose for simple answers
- add bullets only when they materially improve scanning
- add section headings only when the answer genuinely has multiple distinct parts
- use tables only when comparing multiple items across the same dimensions is materially clearer than bullets
- otherwise organize the response into about 4±1 meaningful chunks by default
- count each major section, heading block, or comparison unit as a chunk when judging response size
- expand toward about 7±2 chunks only when the user explicitly asks for a fuller explanation and the answer would not fit well within the smaller default structure
- when the user mainly needs a recommendation or next action, lead with the answer first and keep supporting analysis secondary
- keep each chunk compact and easy to scan
- treat these numbers as heuristics for human readability rather than exact quotas

Prompt assembly in `crates/themion-core/src/agent.rs` should remain structurally unchanged:

- no new prompt message layer
- no special backend handling
- no new `ChatBackend` behavior
- no change to `AGENTS.md` injection semantics

The change is intentionally a focused wording update inside the existing predefined guardrail input.

**Alternative considered:** implement code-side output post-processing that tries to enforce sentence or bullet counts after generation. Rejected: brittle, unnatural, and directionally inconsistent with Themion's current prompt-first guidance model.

### 2. Define the three-tier policy precisely enough to implement

The guardrail wording should distinguish among three normal cases.

#### A. Immediate-answer case

When the user's request can be fully and clearly answered in 1–2 sentences, the assistant should prefer that shorter form and stop instead of expanding into bullets, headings, or tables.

Typical examples:

- yes/no answers with one brief reason
- a short factual clarification
- a quick progress/status update
- a narrow next-step confirmation

Implementation intent:

- this is the default for simple Q&A
- the assistant should not add multi-part structure just because structure is available

#### B. Default structured-answer case

When more than 1–2 sentences are needed, the assistant should default to about 4±1 meaningful chunks.

A chunk may be:

- one short paragraph
- one bullet
- one numbered item
- one compact subsection with a short heading and supporting sentence or two
- one comparison unit such as a compact table row/group when a table is genuinely the clearest format

Expected effect:

- keep the number of top-level ideas small
- reduce sprawling same-level lists
- encourage grouped, digestible points instead of a wall of text

This default should cover common responses such as:

- explanations
- recommendations
- change summaries
- review findings
- multi-step answers that are not full tutorials

#### C. Expanded structured-answer case

When the user explicitly asks for a fuller explanation and the answer would be cramped or incomplete within the smaller default structure, the assistant may expand toward about 7±2 meaningful chunks.

Typical triggers:

- "explain in detail"
- "walk me through it"
- "teach me"
- "give me the full reasoning"
- "compare several options"

Implementation intent:

- this is permission to expand, not a requirement to become verbose
- if the fuller explanation still fits naturally within 4±1 chunks, the assistant should stay smaller
- if correctness requires more detail than the chunk heuristic suggests, correctness wins and the heuristic remains advisory

**Alternative considered:** always jump to the larger structure whenever the user asks "why" or "explain." Rejected: that would over-trigger verbosity and fight the repo's existing concise-answer preference.

### 3. Keep the new rule compatible with existing tool-summary guidance

Themion already includes a narrower rule that important tool-learned findings should be preserved in ordinary assistant chat text, usually in 1–2 sentences by default.

The new response-sizing rule should not replace or blur that behavior.

Instead:

- tool-finding summaries remain concise by default
- the 4±1 or 7±2 heuristic applies to the overall final human-facing answer when a broader structured reply is warranted
- routine mechanical acknowledgements should still remain short
- the assistant should not force every tool-using response into a larger multi-chunk structure if a concise answer already solves the user's request

This preserves the useful narrower behavior from PRD-062 while adding a broader answer-shaping default.

**Alternative considered:** replace the explicit tool-summary rule with one unified chunking rule. Rejected: the tool-summary rule addresses a different transcript-preservation need and should stay independently explicit.

### 4. Keep the behavior product-level, not repository-local

This guidance should remain part of Themion's built-in guardrails rather than being pushed into project-local `AGENTS.md` files.

Reasoning:

- the requested behavior is about the product's general interaction style
- repositories may add local style preferences, but they should not be required to recreate a default answer-organization policy manually
- the repo already documents built-in guardrails and contextual instructions as separate prompt inputs with different responsibilities

Docs should describe the new heuristic as a built-in default that still obeys normal instruction precedence.

**Alternative considered:** ask each repository to restate this rule in `AGENTS.md`. Rejected: inconsistent, repetitive, and contrary to the product-level nature of the request.

### 5. Recommended wording shape for the guardrail text

The final guardrail wording does not need to quote psychology literature or mention author names. It should be short, practical, and model-readable.

A suitable shape is:

- prefer plain direct prose for simple answers
- if the user can be answered clearly in 1–2 sentences, do that
- add bullets only when they materially improve scanning
- add section headings only when the answer genuinely has multiple distinct parts
- use tables only when comparing multiple items across the same dimensions is materially clearer than bullets
- otherwise organize the response into about 4±1 meaningful chunks by default
- count each major section, heading block, or comparison unit as a chunk when judging response size
- if the user explicitly asks for a fuller explanation and the answer does not fit the smaller structure, you may expand toward about 7±2 chunks
- when the user mainly needs a recommendation or next action, lead with the answer first and keep supporting analysis secondary
- treat these as readability heuristics, not exact counting rules

This keeps the implementation readable and avoids making the prompt sound academic or over-specified.

**Alternative considered:** include explicit references to Nelson Cowan and George Miller in the guardrail text itself. Rejected: useful for PRD rationale, but unnecessary and potentially awkward inside the runtime prompt text.

### 6. Proposed exact guardrail text for implementation

To reduce handoff ambiguity, the implementation should add wording close to the following in `crates/themion-core/src/predefined_guardrails.rs`:

> When responding to the user, prefer the smallest clear answer shape that fully solves the request. Prefer plain direct prose for simple answers. If the user can be answered clearly in 1–2 sentences, do that. Add bullets only when they materially improve scanning. Add section headings only when the answer genuinely has multiple distinct parts. Use tables only when comparing multiple items across the same dimensions is materially clearer than bullets. If more structure is needed, organize the response into about 4±1 meaningful chunks by default. Count each major section, heading block, or comparison unit as a chunk when judging response size. If the user explicitly asks for a fuller explanation and the answer would not fit well in that smaller structure, you may expand toward about 7±2 meaningful chunks. When the user mainly needs a recommendation or next action, lead with the answer first and keep supporting analysis secondary. Treat these as readability heuristics, not exact counting rules.

Implementation notes for this exact text:

- `meaningful chunks` intentionally allows bullets, short paragraphs, numbered steps, compact subsections, or compact comparison units when those are actually the clearest format
- `smallest clear answer shape` keeps the wording aligned with the existing concise-answer preference
- the explicit prose/bullets/headings/tables guidance gives the model a clearer format-escalation ladder instead of only a length target
- `you may expand` is intentionally permissive rather than mandatory
- `not exact counting rules` protects correctness and keeps the instruction from sounding mechanical

Acceptable small wording variations are fine during implementation as long as all of the following remain explicit:

- plain prose first for simple answers
- 1–2 sentences when enough
- bullets/headings/tables only when their extra structure materially helps
- default about 4±1 chunks otherwise
- expand toward about 7±2 only for explicit fuller explanation that does not fit the smaller structure
- answer-first ordering for recommendation/next-action style replies
- heuristic guidance, not hard quotas

**Alternative considered:** leave the PRD at a conceptual level and let implementation invent the final sentence later. Rejected: avoidable ambiguity for a prompt-wording feature where the exact phrasing is a large part of the implementation.

### 7. Update runtime and architecture docs to match the actual implementation

Once implemented, the docs should describe the behavior where it really lives:

- `docs/engine-runtime.md` should say that the predefined guardrail layer includes a default answer-sizing and format-escalation rule: 1–2 sentences when enough, plain prose first, bullets/headings/tables only when they materially help, otherwise about 4±1 chunks, with expansion toward about 7±2 only for user-requested fuller explanation
- `docs/architecture.md` should mention the same behavior at a high level in the built-in guardrail description
- `docs/README.md` and this PRD should reflect the implemented status and release version when the change lands

The docs should not imply a new formatting engine, quota enforcer, or post-processor. This remains prompt guidance.

**Alternative considered:** document the behavior only in the PRD and not in runtime docs. Rejected: prompt-layer behavior is part of the product contract and should be discoverable in the main docs that describe prompt assembly.

### 8. Acceptance target for the first implementation

This PRD should be considered implemented when all of the following are true:

- `crates/themion-core/src/predefined_guardrails.rs` includes explicit wording for the response-sizing rule plus the format-escalation ladder
- that wording prefers plain prose and 1–2 sentence answers when sufficient, uses bullets/headings/tables only when they materially help, defaults to about 4±1 meaningful chunks otherwise, and allows expansion toward about 7±2 only for user-requested fuller explanation that does not fit the smaller structure
- that wording explicitly frames the counts as heuristics rather than exact quotas
- prompt assembly in `crates/themion-core/src/agent.rs` continues to inject the predefined guardrails as a separate prompt input without adding a new message layer
- the existing tool-summary guidance remains present and semantically compatible
- `docs/engine-runtime.md` and `docs/architecture.md` describe the new answer-shaping behavior consistently with the actual implementation
- `docs/README.md` and this PRD reflect the landed status/version accurately
- `cargo check -p themion-core` passes after the change
- `cargo check -p themion-core --all-features` passes after the change

This acceptance target keeps the implementation small, reviewable, and clearly bounded to prompt wording plus docs.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/predefined_guardrails.rs` | Add explicit built-in guardrail wording for the human-response sizing heuristic plus format-escalation ladder: plain prose first, 1–2 sentences when enough, bullets/headings/tables only when structure materially helps, otherwise about 4±1 meaningful chunks, with optional expansion toward about 7±2 for user-requested fuller explanation. |
| `crates/themion-core/src/agent.rs` | Keep the current prompt assembly structure unchanged; confirm the updated predefined guardrails continue to be injected as a separate prompt input. |
| `docs/engine-runtime.md` | Document the new built-in answer-sizing guidance in the prompt-input/runtime description. |
| `docs/architecture.md` | Update the high-level built-in guardrail description to mention the default answer-sizing heuristic. |
| `docs/README.md` | Keep the PRD index entry aligned with the PRD filename, status, scope, and eventual landed version. |
| `docs/prd/prd-066-guide-model-instructions-for-human-response-length-and-chunking.md` | Record the implementation-ready requirement and later update status/version when the work lands. |

## Edge Cases

- a user asks a simple question but casually adds "explain" → verify: the assistant may still answer briefly when a longer structure is unnecessary.
- a user asks for a complex explanation that naturally exceeds the default 4±1 chunk range → verify: the assistant may expand toward about 7±2 chunks without sounding forced.
- a user requests highly detailed technical output such as a multi-step implementation plan → verify: the heuristic guides organization but does not block correctness or completeness.
- a tool-heavy turn needs only one concise conclusion → verify: the assistant still uses the existing concise tool-summary behavior and does not force extra structure.
- repository-local instructions prefer a different tone or structure → verify: normal instruction precedence still applies and the built-in heuristic acts only as the default.
- a response needs nested structure, such as a short summary plus a few steps under one heading → verify: the assistant may use semantically grouped chunks rather than flattening everything into one rigid level.
- a long explanation would still be clearer as a table or ordered steps than as seven separate bullets → verify: the heuristic guides chunk count, not mandatory formatting form.
- a short answer could be made more decorative with headings or a tiny table → verify: the assistant keeps the simpler prose form unless the extra structure materially helps scanning or comparison.

## Migration

This is a prompt-guidance and documentation change with no database, config, or protocol migration.

Rollout guidance:

- update the shared predefined guardrails in `themion-core`
- keep the wording practical and heuristic-driven rather than academic or rigid
- update runtime and architecture docs so the prompt-layer description matches the implementation
- leave repository-local instruction files unchanged unless a future project explicitly wants to override the default

## Testing

- update `crates/themion-core/src/predefined_guardrails.rs` and inspect the resulting prompt path → verify: the new wording clearly states the response-sizing policy, the format-escalation ladder, and frames them as heuristic guidance.
- compare the implemented guardrail text against the PRD's proposed wording → verify: the landed text preserves the required semantics even if minor wording changes were made.
- submit a user prompt that can be answered clearly in 1–2 sentences → verify: the assistant prefers the short answer and does not pad it unnecessarily.
- submit a user prompt needing a normal multi-point explanation → verify: the answer is organized into about 4±1 meaningful chunks.
- submit a user prompt explicitly asking for a fuller explanation that does not fit the smaller structure → verify: the answer may expand toward about 7±2 chunks while remaining scannable.
- run a tool-using task with an important finding → verify: the existing concise tool-summary behavior still appears naturally when useful.
- run a task where a longer answer would be wrong or unnecessary despite the user using the word "explain" → verify: the assistant can still stay brief when the smaller answer is clearly sufficient.
- run `cargo check -p themion-core` after implementation → verify: the touched crate compiles cleanly in the default feature set.
- run `cargo check -p themion-core --all-features` after implementation → verify: the touched crate compiles cleanly with all features enabled.

## Implementation checklist

- [ ] update `crates/themion-core/src/predefined_guardrails.rs` with explicit three-tier response-sizing guidance
- [ ] ensure the final wording remains close to the PRD's proposed guardrail text or preserves the same semantics clearly
- [ ] ensure the wording clearly says the counts are heuristics, not exact quotas
- [ ] keep prompt assembly unchanged in `crates/themion-core/src/agent.rs`
- [ ] confirm the existing tool-summary guidance remains compatible and intact
- [ ] update `docs/engine-runtime.md` and `docs/architecture.md`
- [ ] update `docs/README.md` and this PRD status/version when the feature lands
- [ ] run `cargo check -p themion-core`
- [ ] run `cargo check -p themion-core --all-features`
