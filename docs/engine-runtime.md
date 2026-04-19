# Engine Runtime

This document explains how Themion's core harness/runtime works: how prompt inputs are assembled, how context is built, how tool calls are executed, how workflow state progresses, and how session history is stored.

## Scope

Most of the logic described here lives in `crates/themion-core/`. The CLI crate (`crates/themion-cli/`) is responsible for starting sessions, wiring the TUI, loading config, and passing the active project/session context into the core runtime.

Relevant areas:

- `crates/themion-core/src/agent.rs`
- `crates/themion-core/src/client.rs`
- `crates/themion-core/src/client_codex.rs`
- `crates/themion-core/src/tools.rs`
- `crates/themion-core/src/db.rs`
- `crates/themion-core/src/workflow.rs`
- `crates/themion-cli/src/` for session startup and UI integration

## High-level flow

A single user turn follows this shape:

1. The CLI starts or resumes a harness session.
2. The user submits input.
3. The harness records a new turn and persists the user message.
4. The harness builds the model input from:
   - the base system prompt
   - predefined built-in coding guardrails
   - predefined Codex CLI web-search instruction
   - injected contextual instructions such as `AGENTS.md`
   - workflow context and phase instructions
   - an optional history recall hint
   - the recent conversation window
5. The active backend streams the assistant response.
6. If the model requests tools, the harness executes them and appends tool results to the conversation.
7. The harness calls the model again with the updated conversation.
8. Workflow tools may also inspect or mutate the current workflow state between model calls.
9. This repeats until the model returns a normal assistant response with no more tool calls, or the loop limit is reached.
10. The turn is finalized in SQLite with message, workflow, and token metadata.

## Prompt inputs

Themion keeps different instruction sources separate instead of flattening them into one blob.

### 1. Base system prompt

The base system prompt comes from configuration. It establishes the assistant's default behavior and is always part of the prompt sent to the model.

This is the top-level instruction layer.

### 2. Predefined coding guardrails

Themion injects a short built-in coding-guardrail instruction layer after the base system prompt and before repository-local instructions. This layer is inspired by the commonly shared “Karpathy's `CLAUDE.md`” idea set, but Themion adopts only a minimal behavioral subset rather than Anthropic's full Claude Code mechanism.

The built-in topics are:

- avoid making important assumptions silently
- prefer the simplest solution that cleanly solves the task
- make targeted changes and avoid unrelated refactors
- run the narrowest useful validation and report the result
- do not create commits or branches unless explicitly asked, and when the user explicitly asks for a commit, write a brief specific message naming the actual change rather than a vague placeholder

This layer remains separate from both the base system prompt and repository-local instruction files.

### 3. Predefined Codex CLI web-search instruction

Themion injects a short built-in instruction layer telling the agent to use Codex CLI via the shell as the preferred path for web-search-style research when the task requires current external information that is not available in the local repository.

This layer is separate from both the predefined coding guardrails and repository-local instructions. It is intentionally narrow: it guides the model toward Codex CLI for focused external research, not arbitrary external-tool delegation.

The instruction also tells the model to report clearly when Codex CLI is unavailable or fails, rather than pretending certainty about external facts.

### 4. Contextual instruction files

Repository or workspace instructions such as `AGENTS.md` are treated as separate injected prompt inputs, not as text concatenated into the base system prompt.

That separation matters because:

- it preserves the distinction between global assistant behavior and repository-local instructions
- it matches the repository's prompt assembly expectations
- it keeps compatibility with both chat-completions-style backends and the Codex Responses backend

In practice, the model sees both the base system prompt and the injected contextual instructions, but they remain separate prompt components.

### 5. Workflow context and phase instructions

Workflow runtime state is injected as another separate prompt component.

The runtime includes a compact workflow summary such as:

- active workflow name
- current phase
- workflow status
- current phase result
- activation source
- allowed next phases
- retry counters and limits
- phase entry kind

For example, the engine injects a line in this shape:

> Workflow context: flow=LITE phase=CLARIFY status=running phase_result=pending agent=main activation_source=user_input allowed_next=EXECUTE retry_current=0/3 retry_previous=0/3 entered_via=normal

The runtime also injects phase-specific guidance from `workflow.rs`. For the built-in `LITE` workflow:

- `CLARIFY` tells the model to produce a compact brief, state assumptions, and ask only when ambiguity is genuinely blocking
- `EXECUTE` tells the model to implement the smallest working slice and keep scope narrow
- `VALIDATE` tells the model to check success criteria and return pass or fail

### 6. Recall hint for trimmed history

When the in-memory conversation is longer than the configured context window, the harness adds a synthetic system message explaining that earlier turns are still available in persistent history.

Example shape:

> Note: N earlier turn(s) (seq 1–N) are stored in history. Use `history_recall` to load a range or `history_search` to find a keyword.

This gives the model a way to recover older context without sending the full conversation every time.

## Context building

The harness keeps the full conversation in memory, but only sends a bounded recent window to the model.

### Full in-memory history

`Agent` owns a complete `Vec<Message>` for the active session. Messages are not trimmed out of memory during the session.

This full history includes:

- user messages
- assistant messages
- tool results

### Windowed model context

For each model request, the harness constructs a smaller prompt window. Conceptually it looks like this:

```text
[system prompt]
[predefined coding guardrails]
[predefined Codex CLI web-search instruction]
[injected contextual instructions, e.g. AGENTS.md]
[workflow context + phase instructions]
[recall hint, if older turns were omitted]
[recent turns only]
```

`Agent.window_turns` controls how many recent turns are included. Older turns remain in memory and in SQLite, but are not sent unless recovered through history tools.

This design gives a few benefits:

- lower token usage on long sessions
- stable prompt size
- recoverability of old context through explicit tool use

## Workflow runtime

Themion has explicit workflow and phase runtime state, separate from plain conversational history.

### Built-in workflows

The current built-in workflows are:

- `NORMAL`
  - start phase: `EXECUTE`
  - used for the default one-turn direct execution path
- `LITE`
  - start phase: `CLARIFY`
  - uses a compressed `CLARIFY -> EXECUTE -> VALIDATE` flow with retry-aware recovery

Sessions still default to `NORMAL`, and the runtime may return to `NORMAL` / `IDLE` behavior after a workflow completes.

### Workflow state shape

The runtime tracks state including:

- workflow name
- phase name
- workflow status: `running`, `waiting_user`, `completed`, `failed`, or `interrupted`
- phase result: `pending`, `passed`, `failed`, or `user_feedback_required`
- agent label
- last updated turn sequence
- retry state
  - current-phase retries and limit
  - previous-phase retries and limit
  - how the phase was entered: `normal`, `retry_current_phase`, or `retry_previous_phase`

### Workflow control tools

Workflow state is model-visible through dedicated tools:

- `workflow_get_state`
- `workflow_set_active`
- `workflow_set_phase`
- `workflow_set_phase_result`
- `workflow_complete`

Important runtime rules:

- `workflow_set_active` always resets the phase to that workflow's start phase
- `workflow_set_phase` is validated against the active workflow's allowed transitions
- `workflow_set_phase` requires the current `phase_result` to be `passed`
- `workflow_complete` with outcome `completed` also requires current `phase_result=passed`
- `workflow_set_phase_result(result="user_feedback_required")` pauses the workflow in `waiting_user` and ends the turn without auto-retry
- runtime validation stays authoritative even when the model requests a change

`workflow_get_state` returns not only workflow and phase, but also retry information, previous phase info, phase instructions, and allowed next phases.

## Workflow state diagram

The built-in workflow graph is small enough to document directly.

```mermaid
stateDiagram-v2
    [*] --> NORMAL_IDLE: session start / completed default turn

    state "NORMAL" as NORMAL {
        NORMAL_IDLE: IDLE
        NORMAL_EXECUTE: EXECUTE
        NORMAL_IDLE --> NORMAL_EXECUTE: user turn starts
