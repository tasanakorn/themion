# PRD-009: Domain-Prefixed Tool Naming Convention

- **Status:** Implemented
- **Version:** v0.5.1
- **Scope:** `themion-core` (tool definitions, dispatch, prompt references, provider translation compatibility), docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Standardize the model-visible tool surface around a domain-prefixed naming convention.
- Replace generic or implementation-leaking tool names with names that describe domain and action clearly.
- Make the tool list easier for both humans and models to scan as the available tool surface grows.
- Reduce ambiguity and future naming collisions by grouping tools under stable domains such as `fs`, `shell`, `history`, and `workflow`.
- Preserve the existing tool capabilities and behavioral semantics while changing only the exposed naming contract.
- Keep the naming policy in `themion-core` and align documentation with the new public tool names.

## Non-goals

- No change in this PRD to the underlying behavior, permissions, or implementation semantics of existing tools.
- No redesign of tool argument schemas beyond what is required to support renamed tool identifiers.
- No introduction of nested namespaces or non-OpenAI-style tool metadata beyond the current function-schema model.
- No removal of current workflow control semantics or retry behavior.
- No addition of new tool categories unless needed to keep the renamed set coherent.

## Background & Motivation

### Current state

Themion exposes model-visible tools through `crates/themion-core/src/tools.rs` using short snake_case names. The currently documented built-in tool surface includes:

- `read_file`
- `write_file`
- `list_directory`
- `bash`
- `recall_history`
- `search_history`
- `get_workflow_state`
- `set_workflow`
- `set_workflow_phase`
- `set_phase_result`
- `complete_workflow`

This surface is functional, but its naming style is mixed.

Some tools follow a verb-object style based on what the user is trying to do, such as `read_file` and `search_history`. Others expose runtime or implementation detail, most notably `bash`, which names the shell mechanism rather than the action being requested. Workflow tools are grouped conceptually but not lexically, so they are only adjacent if the reader already knows their names.

The current architecture documentation also presents these names directly as part of the public harness contract. That means the tool names are not merely internal code details; they shape model behavior, influence prompt readability, and become part of the documented interface.

### Why a domain prefix helps

As the tool surface grows, short ungrouped names become harder to scan quickly. A domain prefix makes each tool answer two questions immediately:

- what subsystem it belongs to
- what action it performs inside that subsystem

For example:

- `fs_read_file` clearly identifies a filesystem operation
- `shell_run_command` describes the action without exposing the shell implementation detail
- `workflow_set_phase` reads as a workflow control action rather than a generic setter

This improves both:

- human readability in docs, logs, and tool traces
- model readability when choosing among available tools in a prompt

### Why this is a naming-contract change rather than only a cosmetic refactor

Tool names are part of the model-facing API contract. The runtime sends them on every tool-enabled request, prompt guidance references them by name, and provider translation layers preserve them in outgoing tool definitions and incoming tool calls.

Renaming them therefore has compatibility impact across:

- tool schema generation
- tool dispatch in the harness
- prompt and workflow guidance that mentions specific tool names
- architecture and runtime documentation
- any tests that assert exact tool names

The change should be specified explicitly so implementation can preserve behavior while updating all name-bearing surfaces coherently.

## Design

### Naming convention

Themion should adopt a model-visible tool naming convention of:

- `<domain>_<action>_<object>` when an object noun is needed
- `<domain>_<action>` when the domain and action already imply the target clearly

Normative rules:

- names remain lowercase snake_case
- the first segment is the stable domain prefix
- the remaining segment or segments describe user-visible intent, not implementation detail
- names should prefer verbs that describe what the tool does from the assistant's perspective
- names should avoid leaking backend mechanisms when the mechanism is not part of the contract

Recommended built-in domains in the first version:

- `fs` for filesystem tools
- `shell` for shell-command execution tools
- `history` for persistent conversation history tools
- `workflow` for workflow inspection and control tools

**Alternative considered:** keep the current short names and only rename `bash`. Rejected: that fixes the most obvious inconsistency but leaves the broader tool surface without a scalable grouping convention.

### Renamed built-in tool set

The built-in tool surface should be renamed as follows:

| Current name | New name |
| --- | --- |
| `read_file` | `fs_read_file` |
| `write_file` | `fs_write_file` |
| `list_directory` | `fs_list_directory` |
| `bash` | `shell_run_command` |
| `recall_history` | `history_recall` |
| `search_history` | `history_search` |
| `get_workflow_state` | `workflow_get_state` |
| `set_workflow` | `workflow_set_active` |
| `set_workflow_phase` | `workflow_set_phase` |
| `set_phase_result` | `workflow_set_phase_result` |
| `complete_workflow` | `workflow_complete` |

These new names should be treated as the canonical public tool names exposed to models.

Rationale for notable choices:

- `shell_run_command` replaces `bash` because the contract is “run a shell command,” not “use bash specifically.”
- `history_recall` and `history_search` keep the distinction between sequential retrieval and query-based lookup while grouping them under the same domain.
- `workflow_set_active` is preferred over `workflow_set` because it describes what is actually being set and avoids an overly generic setter name.

**Alternative considered:** use dotted names such as `fs.read_file` or `workflow.set_phase`. Rejected: the current tool schema and surrounding model ecosystem already assume flat function names, and snake_case names remain more consistent with the rest of Themion's tool surface.

### Behavioral compatibility

The rename should not change tool behavior.

The following must remain stable except for the exposed name:

- parameter shapes
- return payload shapes and error formatting where currently relied upon
- workflow-state validation rules
- shell execution behavior and working-directory semantics
- history query semantics

Implementation may keep internal helper function names and module organization unchanged if that is the smallest clean change. The normative requirement is that the model-visible tool names, dispatch mapping, docs, and prompt guidance all use the new names consistently.

Implementation note: canonical tool definitions now expose only the domain-prefixed names. Dispatch continues accepting the legacy names as deprecated aliases for compatibility during the transition window.

**Alternative considered:** use the rename to also redesign argument schemas for consistency. Rejected: that would expand scope and mix naming cleanup with behavior changes that deserve separate review.

### Prompt and instruction references

Because prompt content and workflow guidance mention tool names directly, all injected instructions should be updated to use the new canonical names.

This includes at minimum:

- workflow guidance in `agent.rs`
- recall hints that currently mention history tool names
- architecture/runtime docs that enumerate tools or describe how the model should use them

For example, workflow guidance that currently instructs the model to use `get_workflow_state` or `set_workflow_phase` should be updated to `workflow_get_state` and `workflow_set_phase` respectively.

Prompt assembly should continue treating these guidance layers as separate injected inputs rather than merging them into the base system prompt.

### Compatibility and migration policy

This rename changes the model-visible public tool contract. The runtime therefore needs an explicit compatibility policy.

Preferred policy for implementation:

- expose the new domain-prefixed names as canonical names in tool definitions
- continue accepting the old names in dispatch for a short compatibility period when practical
- update docs and prompt guidance immediately to the new names
- treat the old names as deprecated aliases rather than equal first-class names

This approach reduces breakage risk for:

- persisted or replayed model traces
- provider-specific tests that may still reference old names
- in-flight prompt examples during the transition window

If alias support is implemented, it should be internal-only and should not keep old names listed in `tool_definitions()` once the new contract lands.

**Alternative considered:** hard cutover with no alias handling. Rejected: acceptable for a private prototype, but unnecessary brittleness for a user-facing runtime whose docs and tests may lag briefly.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Rename the canonical tool definitions to domain-prefixed names, update dispatch matching, and preserve deprecated aliases internally for compatibility. |
| `crates/themion-core/src/agent.rs` | Update workflow guidance, recall hints, and tool-name-specific prompt text to reference the new canonical names while still recognizing legacy aliases in local workflow-result handling and tool-call display labels. |
| `crates/themion-core/src/client.rs` | Provider request shape continues to use `tool_definitions()` output, so renamed canonical names pass through without extra translation changes. |
| `crates/themion-core/src/client_codex.rs` | Provider translation continues to pass through the renamed tool names correctly because it preserves function names from the canonical tool schema. |
| `crates/themion-cli/src/config.rs` | Update the default system prompt guidance to reference `history_recall` and `history_search`. |
| `docs/architecture.md` | Replace the documented built-in tool list with the domain-prefixed names and explain the grouping convention at a high level. |
| `docs/engine-runtime.md` | Update the runtime/tool-calling documentation and examples to mention the renamed tools consistently. |
| `docs/README.md` | Mark this PRD implemented and keep the docs index aligned with the landed contract. |

## Edge Cases

- A model emits an old tool name such as `bash` during the transition window → handled via deprecated alias mapping while exposing only `shell_run_command` in the canonical schema.
- A prompt or workflow instruction still references an old tool name after the rename → treat this as a documentation and prompt-assembly bug because it trains the model toward a stale contract.
- Tool traces in the TUI or persisted history contain older names from earlier sessions → preserve them as historical records rather than rewriting old stored content.
- Provider-specific request translation preserves tool names exactly → verify renamed names pass through unchanged and are not normalized unexpectedly.
- The workflow instruction block references several workflow tools in one sentence → update all of them together so the model does not receive mixed old/new naming in the same prompt.
- Future tool additions do not fit the initial domains cleanly → allow new domain prefixes, but require the same domain-first naming structure instead of falling back to unprefixed generic names.
- A compatibility alias and a canonical name both point at the same handler → ensure usage metrics and logs remain interpretable and do not imply distinct behaviors.

## Migration

This change is a model-visible API migration rather than a database migration.

User-facing transition expectations:

- docs should switch to the new names immediately once implemented
- new prompts and examples should use only the new names
- old persisted transcripts may still contain historical tool names and should remain readable as-is
- deprecated alias handling remains transitional and may be removed in a later version once the new names are fully established

No SQLite schema change is required purely for the naming convention update.

## Testing

- inspect `tool_definitions()` after the rename → verify: the canonical exposed tool names all follow the domain-prefixed convention and old names are not listed as primary definitions.
- submit a tool-using request that reads and writes files → verify: the model can call `fs_read_file` and `fs_write_file` successfully with unchanged behavior.
- submit a shell-execution request → verify: `shell_run_command` runs in the project directory and returns stdout plus stderr as before.
- trigger history recovery behavior when the context window is exceeded → verify: prompt guidance refers to `history_recall` and `history_search`, and those tools work with existing history behavior.
- run a workflow-aware turn → verify: workflow guidance references `workflow_get_state`, `workflow_set_active`, `workflow_set_phase`, `workflow_set_phase_result`, and `workflow_complete` consistently.
- if deprecated aliases are kept, issue an old tool name such as `read_file` or `bash` through a direct dispatch test → verify: the handler still executes correctly while canonical definitions expose only the new names.
- inspect provider request payloads for chat-completions and codex backends → verify: translated tool definitions preserve the renamed canonical names exactly.
- inspect docs after the update → verify: architecture and runtime docs enumerate the renamed tools consistently with no stale old-name references in normative guidance.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: renamed tool definitions, dispatch, prompt references, and docs-linked code paths compile cleanly.
