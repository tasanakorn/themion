# Tool Design and Implementation Guide

This guide defines how to design and implement tools in `themion`.

It complements `AGENTS.md` and the existing PRD history. Use it when adding a new tool, changing a tool schema, or reviewing whether a new capability should be a new tool at all.

## Goals

- keep the tool surface small
- keep each tool contract simple
- make tool schemas easier for AI models to use correctly
- reduce prompt overhead from tool definitions
- reduce the number of tool calls needed to complete common tasks
- keep implementation ownership in the correct crate and layer

## Core principles

### 1. Prefer fewer tools

Add a new tool only when an existing tool cannot express the capability cleanly.

Prefer:

- extending an existing tool when the capability is the same operation family
- one tool with a clean general contract over several tiny overlapping tools
- one read tool that supports the common useful query shapes over several nearly identical read tools

Avoid:

- splitting one concept into many narrow tools just because the implementation has multiple helper functions
- creating separate tools that differ only by one small parameter or one trivial preset
- exposing migration-era compatibility shapes as permanent tool-surface complexity unless they are truly required

Working rule:

- optimize for the smallest stable tool set that still preserves clarity

### 2. Prefer fewer parameters

Every parameter increases prompt cost and model decision cost.

Prefer:

- required parameters only when truly required
- one canonical parameter shape instead of multiple overlapping ways to say the same thing
- arrays or structured values when they remove repeated tool calls cleanly
- sensible defaults when the behavior is safe and unsurprising

Avoid:

- paired legacy and new parameters in the long-term public contract when one parameter can replace both
- parameters that merely expose implementation details
- optional parameters that almost no caller should set
- verbose descriptive fields that exist only to restate obvious meaning

Working rule:

- if a parameter does not materially improve correctness, safety, or round-trip reduction, do not add it

### 3. Optimize for AI-model use, not human-style prose

Tool descriptions are primarily for model execution quality, not for human-friendly documentation style.

Prefer descriptions that are:

- short
- explicit
- systematic
- literal
- unambiguous
- close to the actual contract

Prefer wording that is:

- direct over conversational
- exact over verbose
- programmatic over rhetorical
- math-like or rule-like when that is clearer

Examples of good style:

- "List board notes filtered by target and optional columns."
- "Read one file segment."
- "Sleep for bounded milliseconds."

Avoid:

- marketing tone
- narrative explanation
- soft or vague wording
- long examples inside the schema unless required for safe use
- human-helpdesk phrasing when a short contract sentence is enough

### 4. Reduce tool-call rounds

Tool design should reduce the number of calls a model must make for common tasks.

Prefer tool contracts that let one call answer one real question.

Prefer:

- multi-item query support when callers commonly need combined views
- filters that match real decision tasks
- result shapes that are complete enough to avoid immediate follow-up calls when practical

Avoid:

- forcing the model to fan out into many nearly identical calls and merge the answers client-side
- tool shapes that require a second call only because the first call was made artificially narrow
- fragmentation where one logical operation is split across multiple tools without a strong reason

Tradeoff rule:

- do not add parameter complexity just to save a rare extra call
- do add carefully chosen expressive power when it removes frequent multi-call patterns

### 5. Keep tool schemas compact

Shorter schemas reduce static prompt overhead.

Prefer:

- one short purpose sentence for the tool description
- one short constraint sentence only when needed
- short literal parameter descriptions
- exact bounds and special tokens only where they matter

Avoid:

- repeating the same policy across many tools
- long prose in parameter descriptions
- duplicating repository-level guidance inside every tool schema

If guidance is important but not contract-critical, prefer putting it in docs or higher-level instructions instead of repeating it in every tool definition.

## Tool-shape rules

### Canonical parameter design

Prefer one canonical parameter shape per concept.

Examples:

- prefer `columns: [..]` over both `column` and `columns` in the long-term contract
- prefer one target selector format over multiple equivalent aliases
- prefer one bounded timeout field over several overlapping timing knobs

If compatibility requires a temporary shim:

- keep the implementation shim narrow
- document the canonical contract clearly
- avoid letting migration compatibility permanently bloat tool descriptions

### Read tools

Read tools should answer complete inspection questions with minimal follow-up.

Prefer:

- useful filters
- stable ordering when ordering matters
- result shapes that include enough context to avoid immediate extra lookup calls

Avoid:

- read tools that only expose one trivial preset when a slightly more general filter would be cleaner
- unnecessary split between "list" and "search" when one clearly designed query tool would suffice

### Mutation tools

Mutation tools should stay narrow and explicit.

Prefer:

- one mutation per tool call
- direct required identifiers
- minimal optional fields
- acknowledgements that confirm what changed

Avoid:

- mutation tools with broad mixed behavior controlled by many flags
- overloaded mutation tools that act like mini command languages

### Special tokens and bounds

Keep exact special semantics only when they are required for safe or correct use.

Examples:

- `SELF`
- `[GLOBAL]`
- `*`
- max byte or timeout bounds

State them briefly and consistently.

## Description-writing rules

### Tool description template

Preferred shape:

- sentence 1: what the tool does
- sentence 2: one key constraint only if needed

Examples:

- "List board notes filtered by target and optional columns."
- "Run a shell command and return bounded stdout+stderr."
- "Search Project Memory nodes in one project context."

### Parameter description template

Preferred shape:

- literal meaning first
- format or bound second
- default only when it matters

Examples:

- "Max returned bytes. Default 16384."
- "Project context. Use [GLOBAL] for Global Knowledge."
- "Sleep duration in ms. Max 30000."

Avoid:

- restating the tool description in every parameter
- long examples unless the field is otherwise ambiguous
- explanation of motivation when the contract is enough

## Round-trip reduction guidance

Before adding a tool or parameter, ask:

1. what user/model task does this help complete?
2. how many tool calls does the common path need today?
3. can one clearer schema reduce those calls?
4. does the change stay simple enough to remain model-friendly?

Good reasons to expand a tool contract:

- the common workflow currently needs repeated near-identical calls
- callers must merge results externally for a naturally single query
- the current schema hides the actual useful unit of work

Bad reasons to expand a tool contract:

- exposing low-level implementation options that most callers do not need
- saving one rare extra call while making every invocation harder to understand
- adding permanent compatibility complexity instead of converging on one clean contract

## Implementation ownership

- define and expose tool schemas in `crates/themion-core/src/tools.rs`
- keep reusable tool behavior in `themion-core`
- do not move tool-contract ownership into TUI code
- update CLI presentation only when transcript or debug labeling needs to reflect the changed contract
- when a tool change affects docs or prompt-visible semantics, update docs in the same task

## Review checklist for tool changes

When reviewing a tool design or implementation, check:

- is a new tool truly needed?
- can an existing tool absorb the capability more cleanly?
- is the parameter set minimal?
- is there one canonical way to express the request?
- is the description short, exact, and model-friendly?
- does the design reduce common tool-call rounds?
- does the result shape avoid unnecessary immediate follow-up calls?
- are special tokens and bounds preserved where required?
- is the implementation in the correct layer/crate?
- are docs and PRD notes updated if the contract changed?

## Relationship to existing repo guidance

This guide extends, and does not replace:

- `AGENTS.md` architecture and validation rules
- `docs/prd/PRD_AUTHORING_GUIDE.md`
- PRD-071 guidance on reducing tool-schema verbosity

When there is tension between adding compatibility surface and keeping the contract small, prefer the smallest long-term model-facing contract and treat compatibility as a migration concern unless product requirements explicitly require both forms to remain public.
