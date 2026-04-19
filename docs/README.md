# Docs

## Architecture

- [architecture.md](architecture.md) — component map, harness loop, context windowing, tool dispatch, TUI layout, build profiles.
- [core-ai-engine-loop.md](core-ai-engine-loop.md) — detailed walkthrough of prompt inputs, `AGENTS.md` injection, context building, tool-calling flow, and SQLite session storage.
- [codex-integration-guide.md](codex-integration-guide.md) — Codex provider integration contract, endpoint usage, auth, `/models` metadata parsing, `/responses` translation, and rate-limit extraction behavior.

## Product Requirements Documents (PRDs)

| ID      | Title                                                                                                       | Status        | Version | Scope                                    |
| ------- | ----------------------------------------------------------------------------------------------------------- | ------------- | ------- | ---------------------------------------- |
| PRD-001 | [Config File + REPL Verbose Feedback](prd/prd-001-config-and-repl-feedback.md)                              | Implemented   | v0.1.0  | `themion-cli`, `themion-core`            |
| PRD-002 | [Persistent History, Multi-Agent Sessions, Context Window](prd/prd-002-persistent-history-multi-agent.md)   | Implemented   | v0.2.0  | `themion-core`, `themion-cli`, workspace |
| PRD-003 | [OpenAI Codex Subscription Provider](prd/prd-003-openai-codex-provider.md)                                  | Implemented   | v0.3.0  | `themion-core`, `themion-cli`, workspace |
| PRD-004 | [Direct Shell Command Prefix in the TUI](prd/prd-004-direct-shell-command-prefix.md)                        | Implemented   | v0.3.0  | `themion-cli`, docs                      |
| PRD-005 | [Model Context Window Refresh and Statusline Display](prd/prd-005-model-context-window-refresh-and-statusline.md) | Implemented   | v0.3.0  | `themion-core`, `themion-cli`, docs      |
| PRD-006 | [Workflow and Phase Model for the Harness Engine](prd/prd-006-workflow-and-phase-model-for-harness-engine.md) | Implemented   | v0.4.0  | `themion-core`, `themion-cli`, docs      |

## Roadmap note

After `v0.2.0`, themion will use themion to help develop itself.
