# Docs

## Architecture

- [architecture.md](architecture.md) — component map, harness loop, context windowing, tool dispatch, TUI layout, build profiles.
- [core-ai-engine-loop.md](core-ai-engine-loop.md) — detailed walkthrough of prompt inputs, `AGENTS.md` injection, context building, tool-calling flow, workflow runtime behavior, and SQLite session storage.
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
| PRD-007 | [Lite Workflow Activation and Runtime Structure](prd/prd-007-lite-workflow-activation-and-runtime-structure.md) | Implemented   | v0.5.0  | `themion-core`, `themion-cli`, docs      |
| PRD-008 | [Workflow Phase Retry and Recovery Policy](prd/prd-008-workflow-phase-retry-and-recovery-policy.md)         | Implemented   | v0.5.0  | `themion-core`, `themion-cli`, docs      |
| PRD-009 | [Domain-Prefixed Tool Naming Convention](prd/prd-009-domain-prefixed-tool-naming-convention.md)             | Implemented   | v0.5.1  | `themion-core`, docs                     |
| PRD-010 | [Rename Persistent Database File from `history.db` to `system.db`](prd/prd-010-rename-database-file-to-system-db.md) | Implemented | v0.5.2 | `themion-core`, `themion-cli`, docs      |
| PRD-011 | [Softer, More Verbose Harness Status Events](prd/prd-011-softer-more-verbose-harness-status-events.md) | Implemented | v0.6.0 | `themion-core`, `themion-cli`, docs      |
| PRD-012 | [Human-Friendly Statusline Token Units](prd/prd-012-human-friendly-statusline-token-units.md) | Implemented | v0.6.1 | `themion-cli`, docs      |
| PRD-013 | [Minimal Karpathy-Inspired Predefined Coding Guardrails](prd/prd-013-minimal-karpathy-inspired-system-prompt-guardrails.md) | Implemented | v0.7.0 | `themion-core`, docs |
| PRD-014 | [Codex CLI Web-Search Instruction Injection](prd/prd-014-codex-cli-web-search-instruction-injection.md) | Implemented | v0.8.0 | `themion-core`, docs |
| PRD-015 | [User-Feedback-Required Phase Result](prd/prd-015-user-feedback-required-phase-result.md) | Proposed | v0.8.0 | `themion-core`, `themion-cli`, docs |
| PRD-016 | [Commit-When-Asked Guardrail for Useful Brief Commit Messages](prd/prd-016-commit-when-asked-brief-summary-guardrail.md) | Implemented | v0.8.1 | `themion-core`, docs |
| PRD-017 | [Press `Esc` to Interrupt an In-Progress Agent Turn](prd/prd-017-press-esc-to-interrupt-agent.md) | Implemented | v0.9.0 | `themion-cli`, `themion-core`, docs |

## Roadmap note

After `v0.2.0`, themion will use themion to help develop itself.
