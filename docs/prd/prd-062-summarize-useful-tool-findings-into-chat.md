# PRD-062: Prompt Guidance for Summarizing Useful Tool-Learned Information into Chat

- **Status:** Implemented
- **Version:** v0.40.0
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-27

## Summary

- Themion already lets the model inspect files, run shell commands, and query history, but the most valuable facts discovered through those tools often remain trapped inside raw `tool_result` payloads.
- Those raw tool results are not a reliable long-term memory surface: large payloads can be trimmed, compacted, truncated, or omitted in future prompt windows and summaries.
- Themion should strengthen its shared coding/tool-use prompt guidance so that when a tool call returns user-useful information, the model restates the important facts in normal assistant chat text.
- By default, that preservation summary should stay to 1–2 sentences and should expand only when the importance or complexity of the finding materially justifies it.
- This is a prompt-behavior improvement, not a new memory database feature: useful facts should first survive in the ordinary transcript before any later history recall, compaction, or memory workflows build on them.

## Goals

- Instruct the model to summarize user-useful information learned from tools in normal assistant chat text.
- Improve continuity when raw tool results are later truncated, compacted, or omitted from future prompt windows.
- Preserve the most important findings in a model-readable and human-readable form inside the transcript.
- Keep the behavior concise, with 1–2 sentences as the default summary size, and expand only when a longer summary is genuinely important.
- Clarify that the summary should capture conclusions and relevant facts, not mechanically restate every byte of tool output.
- Make the implementation target explicit enough that the change can be landed as a focused update to shared guardrail text plus corresponding docs.

## Non-goals

- No requirement to echo every tool result back to the user.
- No requirement to expand beyond a brief 1–2 sentence summary unless the information is actually important enough to justify it.
- No new structured `tool_result` metadata flag such as `historical=true` or `preserve_for_memory=true`.
- No replacement of Project Memory, board notes, or history tools with transcript summaries.
- No requirement in this slice to redesign compaction, durable storage, or tool result schemas.
- No requirement to summarize purely mechanical acknowledgements such as a simple successful write when no user-useful fact was learned.
- No requirement to add a new prompt message layer or special summary markup format.

## Background & Motivation

### Current state

Themion's harness loop appends structured tool calls and tool results to the conversation, and those records are useful in the moment. But the repository already treats prompt inputs and later history differently from live execution details:

- the recent prompt window is bounded
- older history may be summarized or omitted from future model input
- large tool payloads are intentionally bounded or truncated
- later history recall and search work best when important facts appear in ordinary language rather than only inside raw tool protocol detail

This creates a continuity risk: the model may successfully inspect a file or run a command, but the key conclusion remains buried inside a tool result that will not reliably survive future compaction or narrow prompt windows.

### Why prompt-level guidance is the right first step

The simplest reliable place to preserve a useful discovery is the normal assistant transcript.

If the assistant learns something that matters to the user's request, later turns benefit when that information appears as plain chat text such as:

- what was found
- what changed
- what failed
- what remains blocked
- what conclusion the assistant is acting on next

That transcript text is easier for both humans and future prompt summaries to retain than raw low-level tool output alone.

A short summary is usually enough. Most tool-learned facts should be preserved in 1–2 sentences rather than a long replay of the evidence. Longer explanation should be reserved for cases where the finding is unusually important, subtle, or necessary for the user to understand the next decision.

This PRD therefore focuses on prompt/instruction guidance rather than a storage redesign. Themion should first teach the model to restate important findings in chat, then later compaction or memory features can preserve that higher-value summary more naturally.

**Alternative considered:** add a new storage or schema flag to mark certain tool results as durable memory. Rejected for this slice: it adds protocol complexity without solving the more immediate behavior gap that the assistant often fails to translate raw tool output into a concise conclusion for the user.

## Design

### Design principles

- Prefer a prompt-behavior change over a protocol redesign.
- Preserve useful findings in ordinary assistant language close to when they were learned.
- Keep summaries selective and concise.
- Default to 1–2 sentences and expand only when greater detail is materially justified.
- Distinguish between raw evidence from tools and the user-relevant conclusion drawn from that evidence.
- Avoid forcing repetitive narration for trivial or obvious tool results.
- Keep the implementation local to the existing shared guardrail/prompt assembly path rather than introducing a new subsystem.

### 1. Add explicit instruction to the shared predefined guardrails

Themion should implement this behavior by updating the shared predefined coding/tool-use guardrails in `crates/themion-core/src/predefined_guardrails.rs`.

That text is already injected as its own prompt input from `crates/themion-core/src/agent.rs` alongside:

- the base system prompt
- the predefined Codex CLI web-search instruction
- injected contextual instructions such as `AGENTS.md`
- workflow context and phase instructions

This PRD does **not** require a new message type, a new prompt assembly phase, or any change to the `ChatBackend` abstraction. The target is a focused wording change to the existing predefined guardrail string so the behavior applies consistently across supported backends.

Normative behavior to add:

- after receiving tool output with meaningful findings, the assistant should restate the important facts in a brief assistant message or in the next natural assistant response
- the default preservation summary should stay within 1–2 sentences
- the assistant should expand beyond that only when the importance, complexity, or user impact of the finding materially justifies more detail
- the summary should focus on what matters for the task, not on replaying the full tool payload
- the assistant should prefer concrete findings, conclusions, blockers, and next-step implications
- the assistant should do this even when the tool result itself was already visible in the transcript, because raw tool output may not remain available in future prompt windows

Recommended implementation wording shape for the guardrail text:

- tell the model that if a tool call reveals information that is useful to the user or likely to matter later, it should preserve that finding in normal assistant chat text
- state that 1–2 sentences is the default size for that preservation summary
- state that longer summaries are for materially important or complex findings only
- explain briefly that raw tool results may later be trimmed, compacted, truncated, or omitted

Examples of information that should usually be summarized:

- a file contains or does not contain the expected implementation
- a command failed and revealed the root cause
- a search found the relevant location or confirmed absence
- a validation run passed or failed in a way that affects the next action
- a document or config file established a project-specific requirement

**Alternative considered:** instruct the model only to summarize final outcomes at the very end of the turn. Rejected: some tool-learned facts are needed mid-turn for later tool choices or model reasoning, and waiting until the very end makes those facts easier to lose.

### 2. Define what counts as "useful" versus "mechanical"

The prompt guidance should make clear that not every tool result deserves a chat summary.

Useful results usually include:

- discovered facts that affect the task
- constraints or requirements learned from docs, config, or source
- failed validations or command errors that change the plan
- successful validations whose outcome materially supports the answer
- comparisons, counts, or findings that the assistant expects to rely on later

Mechanical results that usually do not need a separate summary include:

- a routine `fs_write_file` acknowledgement with no additional insight
- a directory listing used only to navigate one step deeper
- a sleep completion acknowledgement
- a trivial read whose only role was to obtain text already immediately quoted in the assistant response
- a successful mutation acknowledgement when the assistant's ordinary reply already states what changed

The instruction should encourage judgment: summarize the information value, not the mere fact that a tool was called.

Implementation note:

- this distinction should be documented in both the guardrail text and the runtime/docs updates so the intended behavior is clear to future maintainers
- no tool-specific allowlist or code-side heuristic is required in this slice; the model behavior remains prompt-driven

**Alternative considered:** require a summary after every tool result. Rejected: that would create repetitive transcript noise and would train the model to produce low-value filler instead of preserving only the important facts.

### 3. Keep summaries in normal assistant voice, not as pseudo-tool protocol

The preserved information should appear as ordinary assistant chat text rather than as a new special markup format.

Expected style:

- concise
- task-relevant
- naturally integrated into the answer or progress update
- 1–2 sentences by default
- written as conclusions or observations, not as raw JSON or protocol replay

Examples:

- `I checked docs/README.md and the current PRD table stops at PRD-061, so PRD-062 is the next available number.`
- `cargo check failed in themion-cli because the new field is referenced from the default build but only defined behind the stylos feature.`
- `The compaction path preserves older tool activity only as plain summary text, so important findings should be restated in chat if we want them to survive later trimming.`

This keeps the transcript readable to humans and easy for later summarization layers to reuse.

**Alternative considered:** invent a dedicated inline tag such as `[tool-summary]`. Rejected: a special format adds new prompt surface area without clear evidence that it works better than plain assistant prose.

### 4. Update the prompt/runtime docs to match the actual implementation location

The docs changes should describe the behavior where it actually lives today:

- `docs/engine-runtime.md` should state that the predefined coding/tool-use guardrails ask the model to preserve important tool-learned findings in ordinary assistant transcript text because raw tool results are not guaranteed to survive future prompt windows
- `docs/architecture.md` should mention this at the high level in the design philosophy or prompt-input sections, without over-describing implementation detail
- `docs/README.md` should keep the PRD entry current; once implemented, the PRD status/version should also be updated to reflect landing

The docs should avoid implying that Themion introduced a special transcript-summary subsystem. This feature remains a prompt-level guardrail behavior.

**Alternative considered:** put the behavior only in docs and not in the guardrail text. Rejected: the user-visible behavior depends on what the model is told during execution, so docs-only wording would not make the feature real.

### 5. Acceptance target for the first implementation

This PRD should be considered implemented when all of the following are true:

- `crates/themion-core/src/predefined_guardrails.rs` includes explicit wording that useful tool-learned information should be summarized in normal assistant chat text
- that wording says the default summary size is 1–2 sentences and that longer summaries are only for materially justified cases
- that wording distinguishes meaningful findings from routine mechanical acknowledgements
- prompt assembly in `crates/themion-core/src/agent.rs` continues to inject the predefined guardrails as a separate prompt input without introducing a new message layer
- `docs/engine-runtime.md` and `docs/architecture.md` describe the new behavior consistently with the actual implementation
- `docs/README.md` and this PRD's status/version notes reflect the landed state
- `cargo check -p themion-core` passes after the change
- `cargo check -p themion-core --all-features` passes after the change

This acceptance target intentionally keeps the work small and implementable. It does not require automated transcript-behavior assertions unless the implementation naturally adds a narrow prompt-construction test.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/predefined_guardrails.rs` | Add explicit shared guardrail wording that when tool calls return user-useful information, the assistant should preserve the important findings in normal chat text, defaulting to 1–2 sentences and expanding only when materially justified. |
| `crates/themion-core/src/agent.rs` | Keep existing prompt assembly structure; no new prompt layer is required, but the landed behavior should remain accurate with how predefined guardrails are injected as a separate prompt input. |
| `docs/engine-runtime.md` | Document that predefined coding/tool-use guardrails now ask the model to preserve important tool-learned findings in ordinary assistant transcript text because raw tool results are not a guaranteed long-term prompt surface. |
| `docs/architecture.md` | Update prompt/guardrail documentation to mention this transcript-preservation behavior at a high level. |
| `docs/README.md` | Keep the PRD table entry aligned with the current filename, status, and later implementation state. |

## Edge Cases

- a tool result is very large but only one conclusion matters → verify: the assistant preserves the conclusion in 1–2 sentences without dumping the full payload.
- several tools are used in sequence during one turn → verify: the assistant summarizes the important cumulative finding rather than narrating each trivial intermediate step.
- a command fails with noisy stderr output → verify: the assistant states the actionable error or blocker in chat rather than relying on the raw command output alone.
- a tool result is purely mechanical and adds no new fact → verify: the assistant does not create unnecessary filler summary text.
- the assistant learns a project rule from `AGENTS.md` or docs via file reads → verify: the transcript captures the rule if it materially affects the plan or answer.
- the finding is unusually important or subtle → verify: the assistant may go beyond 1–2 sentences, but only when that added detail materially helps the user or later reasoning.
- the assistant already explains the important outcome in its ordinary reply → verify: no duplicate pseudo-summary line is needed.
- a later turn uses history recall after the original raw tool result has fallen out of the active prompt window → verify: the plain-language transcript summary is easier to recover and reason over than the raw tool payload alone.

## Migration

This is a prompt/instruction change with no schema or storage migration.

Rollout guidance:

- update the shared predefined guardrails in `themion-core`
- update docs that describe prompt behavior and transcript/tool interaction
- observe whether the model produces concise, useful summaries rather than noisy narration
- keep any later storage or compaction changes as follow-on work instead of coupling them into this slice

## Testing

- update `crates/themion-core/src/predefined_guardrails.rs` and inspect the resulting prompt path → verify: the new instruction clearly tells the assistant to preserve useful tool findings in normal chat text and to keep those summaries to 1–2 sentences by default.
- run a tool-using task that inspects repository docs → verify: the assistant restates the important doc finding in normal chat text, usually within 1–2 sentences, rather than relying only on the file-read result.
- run a task that uses shell validation and receives a meaningful failure → verify: the assistant summarizes the failure and its implication in chat.
- run a task with several routine tool calls but only one meaningful conclusion → verify: the transcript contains the useful conclusion without narrating every mechanical step.
- run a task with only trivial mutation acknowledgements → verify: the assistant does not add repetitive filler summaries for low-value tool results.
- run a task where the finding is unusually important or subtle → verify: the assistant may exceed 1–2 sentences only when the extra detail is materially justified.
- run `cargo check -p themion-core` after implementation → verify: prompt/instruction changes compile cleanly in the touched crate.
- run `cargo check -p themion-core --all-features` after implementation → verify: feature-enabled builds of the touched crate still compile cleanly.

## Implementation checklist

- [x] update `crates/themion-core/src/predefined_guardrails.rs` with shared guardrail text that tells the model to summarize useful tool-learned information in normal assistant chat text
- [x] explain in that guardrail text that raw tool results may later be trimmed, compacted, truncated, or omitted, so important findings should not live only inside tool output
- [x] set the default preservation summary size to 1–2 sentences and clarify that longer summaries are only for genuinely important cases
- [x] clarify that routine mechanical acknowledgements usually do not need separate summary narration
- [x] confirm `crates/themion-core/src/agent.rs` prompt assembly still injects predefined guardrails as a separate prompt input without adding a new prompt layer
- [x] update `docs/engine-runtime.md` and `docs/architecture.md` to describe the new transcript-preservation guidance once implemented
- [x] update `docs/README.md` and this PRD status/version when the feature lands
- [x] run `cargo check -p themion-core`
- [x] run `cargo check -p themion-core --all-features`
