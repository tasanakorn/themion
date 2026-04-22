# Docs

## Architecture

- [architecture.md](architecture.md) — workspace structure, core design philosophy, component map, harness loop, context windowing, streaming, tools, persistent history, TUI behavior, and Stylos multi-agent status shape.
- [engine-runtime.md](engine-runtime.md) — detailed walkthrough of prompt inputs, `AGENTS.md` injection, context building, tool-calling flow, workflow runtime behavior, SQLite session storage, and the boundary between CLI-local agent roles and core harness state.
- [codex-integration-guide.md](codex-integration-guide.md) — Codex provider integration contract, endpoint usage, auth, `/models` metadata parsing, `/responses` translation, and rate-limit extraction behavior.

## Product Requirements Documents (PRDs)

| ID      | Title                                                                                                       | Status        | Version | Scope                                    |
| ------- | ----------------------------------------------------------------------------------------------------------- | ------------- | ------- | ---------------------------------------- |
| PRD-001 | [Config File + REPL Verbose Feedback](prd/prd-001-config-and-repl-feedback.md)                              | Implemented   | v0.1.0  | `themion-cli`, `themion-core`            |
| PRD-002 | [Persistent History, Multi-Agent Sessions, Context Window](prd/prd-002-persistent-history-multi-agent.md)   | Implemented   | v0.2.0  | `themion-core`, `themion-cli`, workspace |
| PRD-003 | [OpenAI Codex Subscription Provider](prd/prd-003-openai-codex-provider.md)                                  | Implemented   | v0.3.0  | `themion-core`, `themion-cli`, workspace |
| PRD-004 | [Direct Shell Command Prefix in the TUI](prd/prd-004-direct-shell-command-prefix.md)                        | Implemented   | v0.3.0  | `themion-cli`, `themion-core`            |
| PRD-005 | [Model Context Window Refresh and Statusline Display](prd/prd-005-model-context-window-refresh-and-statusline.md) | Implemented | v0.3.0 | `themion-core`, `themion-cli`, docs      |
| PRD-006 | [Workflow and Phase Model for the Harness Engine](prd/prd-006-workflow-and-phase-model-for-harness-engine.md) | Implemented | v0.4.0 | `themion-core`, `themion-cli`, docs      |
| PRD-007 | [Lite Workflow Activation and Runtime Structure](prd/prd-007-lite-workflow-activation-and-runtime-structure.md) | Implemented | v0.5.0 | `themion-core`, `themion-cli`, docs      |
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
| PRD-018 | [Stronger Short Commit-Message Guardrail](prd/prd-018-stronger-short-commit-message-guardrail.md) | Proposed | v0.9.1 | `themion-core`, docs |
| PRD-019 | [Basic Stylos Support in `themion-cli`](prd/prd-019-basic-stylos-support-in-themion-cli.md) | Implemented | v0.10.0 | `themion-cli`, docs |
| PRD-020 | [Stylos Git Metadata Cache and Remote URL Simplification](prd/prd-020-stylos-git-metadata-cache-and-remote-url-simplification.md) | Implemented | v0.10.1 | `themion-cli`, docs |
| PRD-021 | [Single-Process Multi-Agent Runtime and Multi-Agent Stylos Status Reporting](prd/prd-021-single-process-multi-agent-runtime-and-stylos-reporting.md) | Implemented | v0.11.0 | `themion-core`, `themion-cli`, docs |
| PRD-022 | [Stylos Queryables for Agent Presence, Availability, and Task Requests](prd/prd-022-stylos-queryables-for-agent-presence-availability-and-task-requests.md) | Implemented   | v0.12.1 | `themion-cli`, `themion-core`, docs |
| PRD-023 | [Use External Stylos Repository Instead of Vendored Workspace Copy](prd/prd-023-use-external-stylos-repository-instead-of-vendored-workspace-copy.md) | Implemented   | v0.13.0 | workspace, `themion-cli`, docs |
| PRD-024 | [Client-Side Git Repo Identity Normalization for Stylos Agent Git Queries](prd/prd-024-client-side-git-repo-identity-normalization-for-stylos-agent-git-queries.md) | Implemented   | v0.13.1 | `themion-core`, `themion-cli`, docs |
| PRD-025 | [Long-Session Chat History Navigation in the TUI](prd/prd-025-long-session-chat-history-navigation.md) | Implemented | v0.14.0 | `themion-cli`, docs |
| PRD-026 | [Stylos Talk Sender Identity, Prompt Wiring, Busy-Peer Reply Handling, and Lightweight Wait Tool](prd/prd-026-stylos-talk-sender-identity-and-reply-wiring.md) | Implemented | v0.15.0 | `themion-core`, `themion-cli`, docs |
| PRD-027 | [Sender-Side Stylos Talk `from` and `to` Identifier Semantics](prd/prd-027-stylos-talk-from-and-to-identifiers.md) | Implemented | v0.15.1 | `themion-cli`, docs |
| PRD-028 | [Receiver-Side Stylos Talk Logging Should Not Duplicate `hear` and `talk`](prd/prd-028-receiver-side-stylos-talk-logging-should-not-duplicate-hear-and-talk.md) | Implemented | v0.15.2 | `themion-cli`, docs |
| PRD-029 | [Stylos Notes Board Phase 1 — Replace Ephemeral Talk with Durable Note Intake and Board Sections](prd/prd-029-stylos-notes-board-phase-1.md) | Implemented | v0.16.0 | `themion-core`, `themion-cli`, docs |
| PRD-030 | [Stylos Notes Table Identifier Hardening and Human-Friendly Slugs](prd/prd-030-stylos-notes-table-uuid-and-slug.md) | Implemented | v0.16.1 | `themion-core`, `themion-cli`, docs |
| PRD-031 | [Rename Local Notes Tools from `stylos_` to `board_`](prd/prd-031-rename-local-note-tools-to-board-prefix.md) | Implemented | v0.17.0 | `themion-core`, `themion-cli`, docs |
| PRD-032 | [Stylos Network-Delivered Note Creation When `stylos` Feature Is Enabled](prd/prd-032-stylos-network-delivered-note-creation.md) | Implemented | v0.18.0 | `themion-core`, `themion-cli`, docs |
| PRD-033 | [Note Injection Should Present Note Identity and Metadata in the Initial Prompt](prd/prd-033-note-injection-metadata-first-prompting.md) | Implemented | v0.19.0 | `themion-core`, `themion-cli`, docs |
| PRD-034 | [Note-First Multi-Agent Collaboration and Done Mentions](prd/prd-034-note-first-multi-agent-collaboration-and-done-mentions.md) | Implemented | v0.20.0 | `themion-core`, `themion-cli`, docs |
| PRD-035 | [Add `blocked` Board Column with Cooldown-Aware Revisit Semantics](prd/prd-035-blocked-board-column-and-cooldown.md) | Proposed | v0.21.0 | `themion-core`, `themion-cli`, docs |

## Roadmap note

After `0.2.0`, themion will use themion to help develop itself.
