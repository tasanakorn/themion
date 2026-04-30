# PRD-071: Reduce Tool-Schema Verbosity to Lower Static Prompt Overhead

- **Status:** Implemented
- **Version:** v0.46.1
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-29

## Implementation status

Landed in `v0.46.1` as schema-trimming work across filesystem, shell, Project Memory, board/workflow, and Stylos tool descriptions. The shipped behavior reduces static tool-definition overhead without changing tool names or tool availability. Build validation is complete; `/context` reduction verification remains the product check for follow-up measurement on a live session.

## Summary

- Recent `/context` inspection now shows that tool definitions can dominate startup prompt cost, often consuming more than half of prompt tokens before meaningful conversation history is added.
- In one current observed startup snapshot, `tool definitions` consumed about `3,889` of `5,879` estimated prompt tokens, roughly two-thirds of the total prompt before real conversation growth.
- The current issue is not that tools exist, but that their JSON schema descriptions are verbose enough to become a major static prompt budget consumer on every round.
- This PRD reduces tool-schema verbosity in `themion-core` while preserving the current tool surface, safety bounds, and behavioral contracts.
- The main product goal is to cut static prompt overhead from tool definitions without changing the meaning of tools, changing tool availability, or moving tool logic into the TUI.
- The work should focus on trimming unnecessary prose, repeated explanations, and overly wordy parameter descriptions before considering more invasive follow-up work such as dynamic tool-surface partitioning.

## Goals

- Reduce the token cost of tool definitions sent on every provider round.
- Preserve the existing tool surface, tool names, and tool contracts while making schema descriptions more compact.
- Keep safety-critical constraints and important semantics visible, but state them more concisely.
- Improve PRD-067 replay budgeting and PRD-069 `/context` usefulness indirectly by shrinking static tool-definition overhead.
- Keep implementation in `themion-core`, where tool definitions already live.
- Make reductions measurable through `/context`, especially the `tool definitions` section.
- Achieve a meaningful startup overhead reduction through schema trimming alone, without requiring tool-family partitioning in this PRD.

## Non-goals

- No removal of major tools or tool families in this PRD.
- No dynamic tool-subset selection in this PRD.
- No redesign of tool runtime behavior, permissions, or execution semantics.
- No migration of tool-definition ownership into `themion-cli`.
- No attempt to solve provider-side hidden framing or exact billing accuracy in this PRD.
- No change to tool result-shape policy except where shorter schema wording references current behavior.
- No attempt to make tool descriptions cryptic or keyword-only just to save tokens.

## Background & Motivation

### Current state

PRD-069 and PRD-070 made prompt budgeting visible and more accurate. That visibility now shows a different bottleneck from the original history-focused concern: tool definitions are often the single largest prompt section.

Observed `/context` output in the current implementation shows examples like:

- total prompt estimate around `5,879` tokens before meaningful conversation growth
- tool definitions around `3,889` tokens
- tool definitions therefore accounting for roughly two-thirds of startup prompt tokens in a short session

That means static tool overhead is currently more expensive than conversation history in many ordinary runs.

The current tool-definition implementation in `crates/themion-core/src/tools.rs` includes many long descriptions and parameter notes, for example:

- repeated “Strongly prefer including a short concrete `reason`...” wording across filesystem and shell tools
- long explanatory descriptions for Project Memory context rules, Global Knowledge semantics, and retrieval modes
- repeated default/max-value phrasing embedded in many parameter descriptions
- verbose descriptions for board, workflow, and memory mutation acknowledgements
- feature-gated Stylos descriptions that restate targeting and status semantics at length

Much of that information is useful, but not all of it needs to be repeated in long natural-language form inside every tool schema on every round.

### Codex-style schema-writing principle

A review of the sibling `../codex` codebase shows a simpler built-in tool-description style:

- lead with what the tool does in one short sentence
- add at most one short extra sentence for an essential constraint
- keep parameter descriptions literal and compact
- keep only contract-critical rules in the schema itself
- move broader usage policy and examples out of per-tool schema text unless they are required for safe tool use

Themion should adopt the same principle for this PRD.

Working rule for schema text in this repository:

- tool description: one short purpose sentence, plus one short constraint sentence only when needed
- parameter description: literal field meaning plus compact bound/format note when needed
- keep: exact limits, exact special tokens such as `SELF`, `[GLOBAL]`, and `*`, and other safety-critical semantics
- remove or shorten: repeated narrative, repeated examples, and policy text that can live in higher-level prompt guidance or docs instead

### Why this matters now

PRD-067 replay budgeting and PRD-069 `/context` both improved the ability to manage dynamic prompt growth. PRD-070 then improved estimate quality and made it easier to see where tokens are actually going.

The next clear optimization target is therefore not more history trimming. It is static tool-schema overhead.

This PRD intentionally chooses the least disruptive optimization first:

- keep the current tool set
- keep safety bounds
- keep current tool behavior
- reduce verbosity in schema descriptions

That makes this a good first optimization before higher-risk or broader-scope changes such as:

- dynamic tool-surface partitioning
- per-turn tool-family selection
- deeper prompt-layout redesign

**Alternative considered:** jump immediately to dynamic tool-surface partitioning. Rejected for this PRD: that is likely useful later, but shortening schema prose is a smaller, clearer, and lower-risk first optimization.

## Design

### 1. Shorten repetitive description prose across the tool surface

Themion should reduce repeated explanatory prose in tool descriptions and parameter descriptions wherever the same idea appears many times.

Required behavior:

- shorten repeated boilerplate without removing the underlying meaning
- prefer compact phrases over paragraph-style wording inside tool schemas
- keep important constraints but avoid restating the same advice in multiple long forms
- prefer a reusable compact wording pattern when many tools share the same note type
- follow the Codex-like pattern of one short purpose sentence plus one short constraint sentence only when needed

Examples of likely reductions:

- shorten repeated optional `reason` guidance on file/shell tools
- shorten repeated “defaults to..., max ...” wording where compact phrasing can say the same thing
- shorten repeated acknowledgement/result-shape wording on mutation tools
- shorten repeated “local-only / current-project-only” phrasing where existing parameter names and one short sentence are enough

**Alternative considered:** preserve long descriptions for readability because tokens are cheap compared with correctness. Rejected: current `/context` output shows tool-schema tokens are no longer cheap enough to ignore.

### 2. Preserve safety and contract-critical details, but express them more compactly

Not all description text is equally expendable. Some details carry important behavior or safety meaning.

Required behavior:

- preserve safety-critical limits such as byte limits, timeouts, and maximum sleep durations
- preserve meaning for special semantics such as `SELF`, `[GLOBAL]`, session scoping, and feature-gated behavior
- prefer concise declarative wording over narrative explanation where possible
- remove repetition before removing meaning

For example:

- keep exact numeric bounds for read limits, shell output limits, and timeout behavior
- keep the fact that `session_id="*"` widens history scope within the current project
- keep the fact that `[GLOBAL]` is virtual Project Memory context, not a filesystem path
- keep `SELF` semantics, but shorten repeated multi-sentence explanations where the same meaning can be conveyed more briefly

**Alternative considered:** remove special-semantics explanations from schemas entirely and rely only on prompt guidance. Rejected: the tool schema still needs enough meaning to be safely usable on its own.

### 3. Prioritize highest-cost tool families first

This PRD should focus first on the tool families most likely to contribute the most schema text.

Priority order:

1. Project Memory tools
2. board/workflow coordination tools
3. filesystem and shell tools
4. Stylos tool family when enabled

Required behavior:

- trim the largest and wordiest tool descriptions first
- prefer reducing the biggest token sources rather than chasing tiny per-tool savings evenly
- preserve consistent style after trimming
- use the existing `tool definitions` section in `/context` as the main before/after product metric

This prioritization is grounded in the current source layout and the verbosity visible in `tools.rs`.

**Alternative considered:** trim all tools evenly in one pass. Rejected: optimizing the largest schemas first should produce clearer wins with less churn.

### 4. Keep tool schemas machine-usable and human-scannable

Shorter must not become cryptic.

Required behavior:

- keep tool descriptions explicit enough for reliable model use
- keep parameter names and compact descriptions understandable in isolation
- avoid replacing clear prose with obscure abbreviations
- prefer one short clear sentence over multiple explanatory sentences

The goal is not ultra-minimalism at the cost of correctness. The goal is to remove unnecessary verbosity while preserving tool usability.

**Alternative considered:** aggressively compress every description to near-keyword-only form. Rejected: that risks harming model tool selection and parameter quality.

### 5. Measure reduction through `/context` and treat startup overhead as the success metric

The success metric for this PRD should be visible in the existing prompt inspection path.

Required behavior:

- `/context` should show a lower `tool definitions` token total after the change
- the reduction should be measurable in normal startup state before conversation history grows
- implementation should report the before/after impact in validation notes when practical
- success should be evaluated primarily on startup prompt reduction, not only on source-text diff size

A good result for this PRD is not just “some wording got shorter.” A good result is “startup prompt overhead dropped enough to be noticeable in `/context`.”

**Alternative considered:** treat any shorter source text as success without measuring prompt impact. Rejected: the product issue is prompt overhead, so prompt impact must remain the success metric.

### 6. Keep this PRD focused on verbosity reduction rather than tool-surface selection

This PRD should stay narrow enough to implement safely.

Required behavior:

- reduce schema verbosity without changing which tools are exposed in a given build
- do not mix this PRD with dynamic tool-subset selection or per-turn routing decisions
- if implementation reveals that schema trimming alone is insufficient, report that clearly rather than expanding scope silently

This keeps the change targeted, measurable, and low-risk.

**Alternative considered:** combine verbosity reduction and dynamic tool partitioning in one PRD. Rejected: that would mix two different product decisions and make validation less clear.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Shorten verbose tool and parameter descriptions while preserving safety-critical limits and special semantics. |
| `crates/themion-core/src/agent.rs` | No behavior redesign required, but `/context` will naturally reflect lower tool-definition cost through the existing prompt report path. |
| `docs/engine-runtime.md` | Update tool-schema or prompt-budget wording if needed so docs reflect the more compact schema style. |
| `docs/architecture.md` | Update the tools/context-windowing discussion if needed to reflect the reduced static tool-schema overhead emphasis. |
| `docs/README.md` | Add this PRD entry to the PRD index. |

## Edge Cases

- a shortened description accidentally removes a safety-critical constraint → verify: numeric bounds and special semantics remain present after trimming.
- repeated compacting makes two similar tools harder to distinguish → verify: each tool remains understandable enough in isolation.
- feature-gated Stylos tools are enabled → verify: trimmed schemas still describe remote-instance, agent-target, and task-status semantics clearly enough.
- Project Memory tools still need `[GLOBAL]` semantics → verify: the virtual-context rule remains present, even if phrased more compactly.
- board tools still need `SELF` semantics when Stylos is enabled → verify: the compact wording still preserves exact-self-target meaning.
- a short session is inspected with `/context` before and after the change → verify: the `tool definitions` section shows a meaningful token reduction.
- schema descriptions become shorter but startup token cost barely moves → verify: implementation reports that outcome clearly so follow-up work can decide whether tool-surface partitioning is needed.

## Migration

This is a prompt-schema optimization and documentation change only.

- no database migration is required
- no provider contract change is required beyond smaller prompt payloads
- no tool-name migration is required
- no TUI migration is required

## Testing

- run `/context` before and after the implementation on the same profile/model → verify: the `tool definitions` token total decreases measurably.
- inspect trimmed filesystem/shell tool schemas → verify: optional `reason`, bounds, and mode semantics remain clear.
- inspect trimmed memory tool schemas → verify: `[GLOBAL]`, retrieval-mode, and node-type semantics remain understandable.
- inspect trimmed board/workflow tool schemas → verify: `SELF`, workflow transition, and status/result semantics remain understandable.
- inspect trimmed Stylos tool schemas when the feature is enabled → verify: target-instance, agent-target, and task semantics remain understandable.
- run representative tool-using turns after the schema trim → verify: tool selection and argument quality do not regress obviously.
- compare startup `/context` snapshots before and after trimming → verify: the reduction is large enough to matter in product terms, not only as a source-level cleanup.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates build cleanly.
- run `cargo check -p themion-core --all-features` after implementation → verify: `themion-core` still builds cleanly across feature combinations.
- run `cargo check -p themion-cli --all-features` after implementation → verify: `themion-cli` still builds cleanly across feature combinations.

## Implementation checklist

- [x] audit tool-definition text for the highest-cost verbose descriptions
- [x] shorten repetitive boilerplate across filesystem and shell tools
- [x] shorten high-cost memory tool descriptions and parameter notes
- [x] shorten board/workflow tool description text while preserving semantics
- [x] trim feature-gated Stylos tool descriptions where needed
- [ ] verify `/context` shows a meaningfully lower `tool definitions` token total
- [x] update relevant docs and the PRD index
