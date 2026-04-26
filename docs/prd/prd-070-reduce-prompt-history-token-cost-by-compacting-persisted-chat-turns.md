# PRD-059: Reduce Prompt-Build History Token Cost by Compacting Persisted Chat Turns

- **Status:** Draft
- **Version:** >v0.36.0 +minor
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Themion's persisted chat history can contain extremely large tool results and large assistant tool-call payloads, and those records can dominate prompt-building cost if recalled too literally.
- Local database analysis found one session where a single turn occupied about 1.06 MB of serialized history, with one tool message alone consuming about 1.02 MB.
- The main culprits are broad raw search output, especially output that includes irrelevant paths such as `target/`, and assistant tool-call JSON that embeds whole write payloads.
- Keep persistent history useful for debugging and auditability, but store or replay large tool traffic in a more compact form by default.
- Treat prompt-building history as a compact semantic transcript, not as a raw log replay of every byte ever returned by tools.
- Tool-reason guidance, recording, and chat visibility now belong to PRD-058; this PRD covers the remaining history-compaction and replay-shaping work.

## Goals

- Reduce prompt token cost caused by oversized persisted history entries during prompt building.
- Preserve the semantic value of prior turns while avoiding re-injecting massive raw tool output that the model does not need.
- Define a clear policy for how tool results and assistant tool-call payloads should be compacted before they become durable prompt-history inputs.
- Keep debugging and inspection workflows possible without requiring full raw payload replay in ordinary prompt construction.
- Build on PRD-056's result-shape discipline and PRD-058's tool-reason experiment by extending size discipline to persisted history and prompt reconstruction.

## Non-goals

- No attempt to remove persistent history entirely.
- No requirement that every raw tool result become unavailable for debugging.
- No redesign of provider APIs, model streaming, or workflow semantics.
- No requirement in this PRD to invent a full external blob store unless implementation pressure clearly justifies it.
- No change to user-visible meaning of successful tool operations beyond making history leaner.
- No attempt to re-specify the optional tool-reason UI and recording behavior covered by PRD-058.

## Background & Motivation

### Current state

Themion stores session history in SQLite tables such as `agent_turns` and `agent_messages`. Prompt building can recall prior turns and serialize them back into model-facing context. That means persisted message size is not just a storage concern; it is also a prompt-budget concern.

PRD-056 already tightened tool result shapes for mutation tools by making acknowledgements compact by default. PRD-058 separately scopes optional tool reasons so Themion can evaluate whether concise intent text improves readability and later recall. Even with those improvements, local history analysis shows that a larger remaining problem is oversized persisted tool traffic and tool-call payloads.

### Empirical findings from local history

A local database inspection of `~/.local/share/themion/system.db` found a representative pathological session:

- session `1181d50f-ebe1-468b-addf-ad133415dbcc`
- `16` turns
- `165` messages
- about `2,060,456` bytes of serialized message data

That session contains a single dominant turn:

- turn `14`
- `5` messages
- about `1,058,447` bytes total
- about `51.4%` of the entire session's serialized message volume

Within that turn, one tool message dominates:

- `message_id=8955`
- role `tool`
- about `1,022,677` bytes

The stored content begins as relevant search output about note-related schema, but it expands into a huge grep-style dump and ends with irrelevant binary-match lines from `target/release/deps/...`. If replayed literally into prompt history, that one tool message alone can consume on the order of hundreds of thousands of tokens, depending on tokenizer and serialization overhead.

The same session also shows a secondary bloat source:

- assistant messages with large `tool_calls_json`
- especially `fs_write_file` calls that embed whole file contents in the stored call arguments
- several individual assistant tool-call records exceed `24 KB`, with the largest over `41 KB`

Role totals in the analyzed session were heavily skewed toward tool traffic:

- tool messages: about `1,789,665` bytes
- assistant messages plus tool-call JSON: about `269,465` bytes
- user messages: about `1,326` bytes

This confirms that prompt-history bloat is primarily a tool-output persistence problem, not a natural user-dialogue growth problem.

### Why this matters

Prompt construction should preserve what the model needs to continue the task:

- what the user asked
- what the assistant decided
- what tools were called at a semantic level
- what important results or failures occurred
- optional tool-use reasons when they exist and are useful

Prompt construction usually does not need:

- megabytes of raw grep output
- binary-match noise
- whole file payloads repeated inside write-tool arguments
- large unchanged blobs when the assistant's follow-up already summarized the result

When history is replayed too literally, Themion wastes context window budget, increases provider cost, and raises the risk that a few pathological turns crowd out much more relevant recent reasoning.

## Design

### Design principles

- Persist enough information to reconstruct intent and outcome, not every raw byte by default.
- Separate durable prompt history from full-fidelity debugging data when those needs diverge.
- Prefer compact structured summaries over arbitrary truncation when practical.
- Keep read/query tools and explicit inspection paths as the canonical way to fetch detail on demand.
- Exclude obvious low-value noise such as build-artifact binary matches from durable prompt-facing history.
- Preserve optional tool reasons from PRD-058 when they are available and useful, but do not depend on them for correctness.

### 1. Introduce prompt-facing history compaction rules

Before a message is persisted for later prompt reconstruction, or before a persisted message is replayed into prompt history, Themion should classify it by role and content kind and apply role-appropriate compaction.

Expected rules:

- user messages remain effectively unchanged except for any existing normalizations.
- assistant natural-language replies remain effectively unchanged.
- assistant tool-call records should keep the tool name and compact argument summary, not always the full original argument blob.
- tool result messages should keep a compact structured summary by default when raw payloads exceed a bounded threshold.
- optional tool reasons should be retained when available, but omitted cleanly when absent.

A compacted history entry should preserve at least:

- role
- tool name or tool call identifier when relevant
- success/failure status when known
- key scalar result fields when they are small and meaningful
- bounded preview text when needed
- optional tool reason when available
- explicit metadata indicating that compaction occurred
- original byte count and kept byte count for debugging transparency

**Alternative considered:** hard truncate oversized strings without metadata. Rejected: this is simpler but makes history less interpretable and can hide whether an omitted section contained important result structure.

### 2. Treat tool history as semantic summaries, not raw logs

For large tool results, Themion should store or replay a summary-shaped history representation such as:

- tool name
- result kind
- whether the command succeeded
- a bounded preview
- counts such as line count, file count, match count, or bytes omitted when available
- a note that the full raw output was omitted from prompt-facing history
- optional tool reason when it helps preserve intent

Examples:

- large grep/search result → store match count, first few relevant lines, omitted-byte count, and whether excluded paths such as `target/` were encountered
- large shell output → store exit status, first/last bounded preview, omitted-byte count, and whether output was truncated
- write tool → store path, byte count written, and whether the original payload was compacted from history

This should be applied to the persisted history representation that prompt building consumes, even if a fuller raw record is optionally kept elsewhere for inspection.

### 3. Compact assistant tool-call arguments for large write operations and similar payloads

Assistant messages currently can store large `tool_calls_json` payloads containing full arguments for operations such as `fs_write_file`. For prompt history, this is usually much more detail than needed.

For prompt-facing persistence or replay, assistant tool-call records should prefer a compact argument summary such as:

- tool name
- key identifying arguments, for example `path` or `command`
- bounded scalar options such as `mode`, `offset`, `limit`, `timeout_ms`
- content length rather than full content for write-like tools
- optional short reason text when it is available from PRD-058's recording path
- a short preview only when it adds real semantic value

For example, instead of replaying the entire file content inside an `fs_write_file` call, history should capture something like:

- `fs_write_file(path="crates/themion-core/src/tools.rs", mode="raw", content_bytes=31244)`

And when a reason exists:

- `shell_run_command(command="cargo check -p themion-core", timeout_ms=300000) — reason: validate touched crate builds cleanly`

**Alternative considered:** preserve full tool-call arguments for exact replay fidelity. Rejected: this helps debugging but imposes heavy prompt cost and is rarely necessary for the model to continue correctly after the tool already succeeded.

### 4. Keep full-detail inspection as an explicit opt-in path

The compacted prompt-history representation should not eliminate observability. Themion should preserve a way to inspect full raw records when they genuinely matter.

Reasonable options include:

- storing both compact prompt-facing content and optional raw archival content
- storing raw content only when it fits within bounds, otherwise keeping a compact summary and dropping raw payload
- storing a digest, byte count, and retrieval-unavailable marker when full archival retention is intentionally not supported

This PRD does not require choosing the most elaborate archival design immediately. The key requirement is that ordinary prompt building must not depend on replaying raw oversized payloads.

### 5. Improve source-side noise reduction where practical

History compaction should not be the only defense. Source-side reductions should also be applied where the tool itself can avoid producing low-value noise.

Examples:

- prefer searches that exclude `target/`, `.git/`, and other obvious generated or irrelevant directories by default when repository-aware behavior is intended
- suppress binary-match spam in search output when the caller asked for text inspection
- prefer bounded or structured search/report commands over unconstrained recursive dumps

This reduces both immediate transcript noise and downstream history compaction pressure.

## Changes by Component

| Component / file area | Change |
| --- | --- |
| `crates/themion-core/src/` prompt assembly and history replay paths | Define a compact prompt-history representation for oversized assistant tool-call and tool-result messages. |
| `crates/themion-core/src/` history persistence helpers | Add history-compaction helpers close to prompt assembly and history persistence code. |
| `crates/themion-core/src/` history metadata handling | Preserve explicit metadata that a message was compacted, including omitted-byte counts and optional retained tool reason text. |
| `crates/themion-core/src/` assistant tool-call persistence/replay | Compact large `tool_calls_json` payloads into summary form for prompt-facing history reconstruction. |
| `crates/themion-core/src/` tool result replay logic | Compact oversized tool results into bounded structured summaries for prompt-facing history reconstruction. |
| `crates/themion-cli/src/` tool and diagnostic presentation paths | Where CLI-local flows or tools produce especially noisy output, prefer bounded summaries over raw dumps when that does not reduce correctness. |
| `docs/architecture.md`, `docs/engine-runtime.md`, `docs/README.md`, this PRD | Document the distinction between durable raw-ish history storage and prompt-facing compact history if both exist, and keep PRD references aligned. |

## Edge Cases

- Some tool outputs are large because they are the result the user explicitly asked to inspect. In those cases, the full content may still belong in the immediate visible transcript, but prompt-facing persistence should remain bounded unless later recall explicitly asks for detail.
- Some tools return structured JSON where dropping fields may remove important semantics. Prefer field-aware compaction rather than string truncation for known structured payloads.
- Failure cases may require slightly more detail than success cases, especially when the error message itself is the actionable result. Keep error summaries richer within bounds.
- If compaction metadata is persisted, prompt assembly should avoid replaying internal bookkeeping verbatim when a simpler human-readable summary is sufficient.
- If a future feature needs exact tool-call replay for debugging, that should be an explicit debug path rather than the default prompt-history path.
- Tool reasons from PRD-058 may be missing, low-quality, or absent in older history rows; compaction should tolerate that without reducing correctness.

## Migration

- Existing oversized history rows in user databases may remain as-is unless a later migration or background compaction tool is added.
- New compaction rules should apply to newly persisted history first.
- If prompt assembly can detect legacy oversized rows, it should still apply bounded replay rules so old data does not continue to poison prompt budgets.
- If PRD-058 lands first, replay compaction should preserve optional recorded tool reasons when available but should not require them.

## Testing

- persist a large grep-like tool result with noisy trailing binary-match lines → verify: prompt-facing history stores or replays a bounded compact summary rather than the full raw payload
- persist a large `fs_write_file` tool call containing full file content → verify: prompt-facing history retains tool identity and content length but not the full embedded content blob
- persist a tool call with an optional PRD-058 reason plus oversized output → verify: compact replay keeps the useful reason while still bounding the bulky payload
- recall prompt history for a session containing one pathological oversized turn → verify: reconstructed prompt size stays bounded and the semantic narrative of the turn remains understandable
- persist a small tool result and a small tool-call payload → verify: small records remain unchanged or nearly unchanged
- replay legacy oversized history rows through prompt assembly → verify: bounded compaction still applies even if the raw row predates the new persistence rules

## Implementation checklist

- [ ] define prompt-facing compaction rules for oversized assistant tool-call and tool-result history
- [ ] compact replay of large `tool_calls_json` payloads into bounded summaries
- [ ] compact replay of oversized tool-result messages into bounded summaries with compaction metadata
- [ ] preserve optional tool reasons from PRD-058 when present in replayable history
- [ ] document replay/persistence policy updates in the relevant runtime and architecture docs
- [ ] validate bounded replay behavior against both new and legacy oversized history rows
