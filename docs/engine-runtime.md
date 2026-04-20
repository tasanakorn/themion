# Engine Runtime

This document explains how Themion's core harness/runtime works: how prompt inputs are assembled, how context is built, how tool calls are executed, how workflow state progresses, and how session history is stored.

## Scope

Most of the logic described here lives in `crates/themion-core/`. The CLI crate (`crates/themion-cli/`) is responsible for starting sessions, wiring the TUI, loading config, and passing the active project/session context into the core runtime.

When the optional `stylos` cargo feature is enabled for `themion-cli`, Stylos session startup and shutdown remain CLI-local runtime wiring rather than part of the core harness loop. In feature-enabled builds, Stylos starts by default unless config overrides disable it.

Relevant areas:

- `crates/themion-core/src/agent.rs`
- `crates/themion-core/src/client.rs`
- `crates/themion-core/src/client_codex.rs`
- `crates/themion-core/src/tools.rs`
- `crates/themion-core/src/db.rs`
- `crates/themion-core/src/workflow.rs`
- `crates/themion-cli/src/` for session startup, process-local agent descriptors, and UI integration

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

## Agent identity boundary

`themion-core::Agent` owns per-agent harness state such as session ID, project directory, workflow state, messages, and model/backend integration. `themion-cli` owns process-local descriptors such as `agent_id`, `label`, and `roles` that describe how a given core agent is used within one Themion process.

This keeps reusable harness behavior in core while allowing the CLI to publish process-level multi-agent status for Stylos.
