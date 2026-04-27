# Docs

## Architecture

- [architecture.md](architecture.md) — workspace structure, core design philosophy, component map, process/thread model, harness loop, context windowing, streaming, tools, persistent history, TUI behavior, and Stylos multi-agent status shape.
- [engine-runtime.md](engine-runtime.md) — detailed walkthrough of prompt inputs, `AGENTS.md` injection, context building, tool-calling flow, workflow runtime behavior, SQLite session storage, and the boundary between CLI-local agent roles and core harness state.
- [codex-integration-guide.md](codex-integration-guide.md) — Codex provider integration contract, endpoint usage, auth, `/models` metadata parsing, `/responses` translation, and rate-limit extraction behavior.

## Product Requirements Documents (PRDs)

Start PRD authoring with [prd/PRD_AUTHORING_GUIDE.md](prd/PRD_AUTHORING_GUIDE.md). The `docs/prd/README.md` file is the directory entrypoint and links back to that guide. Older PRDs 001–050 now live under [`docs/prd/archive/`](prd/archive/).

| ID      | Title                                                                                                       | Status        | Version | Scope                                    |
| ------- | ----------------------------------------------------------------------------------------------------------- | ------------- | ------- | ---------------------------------------- |
| PRD-001 | [Config File + REPL Verbose Feedback](prd/archive/prd-001-config-and-repl-feedback.md)                              | Implemented   | v0.1.0  | `themion-cli`, `themion-core`            |
| PRD-002 | [Persistent History, Multi-Agent Sessions, Context Window](prd/archive/prd-002-persistent-history-multi-agent.md)   | Implemented   | v0.2.0  | `themion-core`, `themion-cli`, workspace |
| PRD-003 | [OpenAI Codex Subscription Provider](prd/archive/prd-003-openai-codex-provider.md)                                  | Implemented   | v0.3.0  | `themion-core`, `themion-cli`, workspace |
| PRD-004 | [Direct Shell Command Prefix in the TUI](prd/archive/prd-004-direct-shell-command-prefix.md)                        | Implemented   | v0.3.0  | `themion-cli`, `themion-core`            |
| PRD-005 | [Model Context Window Refresh and Statusline Display](prd/archive/prd-005-model-context-window-refresh-and-statusline.md) | Implemented | v0.3.0 | `themion-core`, `themion-cli`, docs      |
| PRD-006 | [Workflow and Phase Model for the Harness Engine](prd/archive/prd-006-workflow-and-phase-model-for-harness-engine.md) | Implemented | v0.4.0 | `themion-core`, `themion-cli`, docs      |
| PRD-007 | [Lite Workflow Activation and Runtime Structure](prd/archive/prd-007-lite-workflow-activation-and-runtime-structure.md) | Implemented | v0.5.0 | `themion-core`, `themion-cli`, docs      |
| PRD-008 | [Workflow Phase Retry and Recovery Policy](prd/archive/prd-008-workflow-phase-retry-and-recovery-policy.md)         | Implemented | v0.5.0 | `themion-core`, `themion-cli`, docs      |
| PRD-009 | [Domain-Prefixed Tool Naming Convention](prd/archive/prd-009-domain-prefixed-tool-naming-convention.md)             | Implemented | v0.5.1 | `themion-core`, docs                     |
| PRD-010 | [Rename Persistent Database File from `history.db` to `system.db`](prd/archive/prd-010-rename-database-file-to-system-db.md) | Implemented | v0.5.2 | `themion-core`, `themion-cli`, docs      |
| PRD-011 | [Softer, More Verbose Harness Status Events](prd/archive/prd-011-softer-more-verbose-harness-status-events.md) | Implemented | v0.6.0 | `themion-core`, `themion-cli`, docs      |
| PRD-012 | [Human-Friendly Statusline Token Units](prd/archive/prd-012-human-friendly-statusline-token-units.md) | Implemented | v0.6.1 | `themion-cli`, docs      |
| PRD-013 | [Minimal Karpathy-Inspired Predefined Coding Guardrails](prd/archive/prd-013-minimal-karpathy-inspired-system-prompt-guardrails.md) | Implemented | v0.7.0 | `themion-core`, docs |
| PRD-014 | [Codex CLI Web-Search Instruction Injection](prd/archive/prd-014-codex-cli-web-search-instruction-injection.md) | Implemented | v0.8.0 | `themion-core`, docs |
| PRD-015 | [User-Feedback-Required Phase Result](prd/archive/prd-015-user-feedback-required-phase-result.md) | Implemented | v0.8.0 | `themion-core`, `themion-cli`, docs |
| PRD-016 | [Commit-When-Asked Guardrail for Useful Brief Commit Messages](prd/archive/prd-016-commit-when-asked-brief-summary-guardrail.md) | Implemented | v0.8.1 | `themion-core`, docs |
| PRD-017 | [Press `Esc` to Interrupt an In-Progress Agent Turn](prd/archive/prd-017-press-esc-to-interrupt-agent.md) | Implemented | v0.9.0 | `themion-cli`, `themion-core`, docs |
| PRD-018 | [Stronger Short Commit-Message Guardrail](prd/archive/prd-018-stronger-short-commit-message-guardrail.md) | Implemented | v0.9.1 | `themion-core`, docs |
| PRD-019 | [Basic Stylos Support in `themion-cli`](prd/archive/prd-019-basic-stylos-support-in-themion-cli.md) | Implemented | v0.10.0 | `themion-cli`, docs |
| PRD-020 | [Stylos Git Metadata Cache and Remote URL Simplification](prd/archive/prd-020-stylos-git-metadata-cache-and-remote-url-simplification.md) | Implemented | v0.10.1 | `themion-cli`, docs |
| PRD-021 | [Single-Process Multi-Agent Runtime and Multi-Agent Stylos Status Reporting](prd/archive/prd-021-single-process-multi-agent-runtime-and-stylos-reporting.md) | Implemented | v0.11.0 | `themion-core`, `themion-cli`, docs |
| PRD-022 | [Stylos Queryables for Agent Presence, Availability, and Task Requests](prd/archive/prd-022-stylos-queryables-for-agent-presence-availability-and-task-requests.md) | Implemented   | v0.12.1 | `themion-cli`, `themion-core`, docs |
| PRD-023 | [Use External Stylos Repository Instead of Vendored Workspace Copy](prd/archive/prd-023-use-external-stylos-repository-instead-of-vendored-workspace-copy.md) | Implemented   | v0.13.0 | workspace, `themion-cli`, docs |
| PRD-024 | [Client-Side Git Repo Identity Normalization for Stylos Agent Git Queries](prd/archive/prd-024-client-side-git-repo-identity-normalization-for-stylos-agent-git-queries.md) | Implemented   | v0.13.1 | `themion-core`, `themion-cli`, docs |
| PRD-025 | [Long-Session Chat History Navigation in the TUI](prd/archive/prd-025-long-session-chat-history-navigation.md) | Implemented | v0.14.0 | `themion-cli`, docs |
| PRD-026 | [Stylos Talk Sender Identity, Prompt Wiring, Busy-Peer Reply Handling, and Lightweight Wait Tool](prd/archive/prd-026-stylos-talk-sender-identity-and-reply-wiring.md) | Implemented | v0.15.0 | `themion-core`, `themion-cli`, docs |
| PRD-027 | [Sender-Side Stylos Talk `from` and `to` Identifier Semantics](prd/archive/prd-027-stylos-talk-from-and-to-identifiers.md) | Implemented | v0.15.1 | `themion-cli`, docs |
| PRD-028 | [Receiver-Side Stylos Talk Logging Should Not Duplicate `hear` and `talk`](prd/archive/prd-028-receiver-side-stylos-talk-logging-should-not-duplicate-hear-and-talk.md) | Implemented | v0.15.2 | `themion-cli`, docs |
| PRD-029 | [Stylos Notes Board Phase 1 — Replace Ephemeral Talk with Durable Note Intake and Board Sections](prd/archive/prd-029-stylos-notes-board-phase-1.md) | Implemented | v0.16.0 | `themion-core`, `themion-cli`, docs |
| PRD-030 | [Stylos Notes Table Identifier Hardening and Human-Friendly Slugs](prd/archive/prd-030-stylos-notes-table-uuid-and-slug.md) | Implemented | v0.16.1 | `themion-core`, `themion-cli`, docs |
| PRD-031 | [Rename Local Notes Tools from `stylos_` to `board_`](prd/archive/prd-031-rename-local-note-tools-to-board-prefix.md) | Implemented | v0.17.0 | `themion-core`, `themion-cli`, docs |
| PRD-032 | [Stylos Network-Delivered Note Creation When `stylos` Feature Is Enabled](prd/archive/prd-032-stylos-network-delivered-note-creation.md) | Implemented | v0.18.0 | `themion-core`, `themion-cli`, docs |
| PRD-033 | [Note Injection Should Present Note Identity and Metadata in the Initial Prompt](prd/archive/prd-033-note-injection-metadata-first-prompting.md) | Implemented | v0.19.0 | `themion-core`, `themion-cli`, docs |
| PRD-034 | [Note-First Multi-Agent Collaboration and Done Mentions](prd/archive/prd-034-note-first-multi-agent-collaboration-and-done-mentions.md) | Implemented | v0.20.0 | `themion-core`, `themion-cli`, docs |
| PRD-035 | [Add `blocked` Board Column with Cooldown-Aware Revisit Semantics](prd/archive/prd-035-blocked-board-column-and-cooldown.md) | Implemented | v0.21.0 | `themion-core`, `themion-cli`, docs |
| PRD-036 | [Prompt Guidance for Self-Note Creation Beyond Simple Q&A](prd/archive/prd-036-prompt-guidance-for-self-note-creation-beyond-simple-qa.md) | Implemented | v0.22.0 | `themion-core`, docs |
| PRD-037 | [Remove the Hard-Coded 10-Round Harness Loop Limit and Rely on State-Based Termination](prd/archive/prd-037-replace-hard-coded-harness-loop-limit.md) | Implemented | v0.23.0 | `themion-core`, docs |
| PRD-038 | [Center Trim Tool Call Chat Labels](prd/archive/prd-038-center-trim-tool-call-chat-labels.md) | Implemented | v0.26.0 | `themion-core`, `themion-cli`, docs |
| PRD-039 | [Refactor Board and Note Naming Toward Local-Board-First Semantics](prd/archive/prd-039-refactor-board-and-note-naming-toward-local-board-first-semantics.md) | Implemented | v0.24.0 | `themion-core`, `themion-cli`, docs |
| PRD-040 | [Debug Command for Themion Process, Thread, and Task Utilization](prd/archive/prd-040-debug-runtime-recent-window-reporting.md) | Implemented | v0.25.0 | `themion-cli`, docs |
| PRD-041 | [Fix `/debug runtime` Recent-Window Counter and Rate Reporting](prd/archive/prd-041-fix-debug-runtime-recent-window-reporting.md) | Implemented | v0.25.1 | `themion-cli`, docs |
| PRD-042 | [Dirty-Region and Partial TUI Redraws](prd/archive/prd-042-dirty-region-and-partial-tui-redraws.md) | Implemented | v0.26.0 | `themion-cli`, docs |
| PRD-043 | [Safer and More Bounded File and Shell Tool Parameters](prd/archive/prd-043-safer-and-more-bounded-file-and-shell-tool-parameters.md) | Implemented | v0.27.0 | `themion-core`, docs |
| PRD-044 | [Fix Multiline Input Newline and Wrapped-Cursor Tracking](prd/archive/prd-044-fix-multiline-input-newline-and-wrapped-cursor-tracking.md) | Implemented | v0.26.1 | `themion-cli`, docs |
| PRD-045 | [Project-Scoped History Recall and Search Across Sessions](prd/archive/prd-045-project-wide-history-recall-and-search-across-sessions.md) | Implemented | v0.28.0 | `themion-core`, docs |
| PRD-046 | [Lightweight Long-Term Memory Knowledge Base with Hashtag-Based Organization](prd/archive/prd-046-lightweight-unified-memory-graph-with-hashtag-based-organization.md) | Implemented | v0.29.1 | `themion-core`, `themion-cli`, docs |
| PRD-047 | [Prefer `note_slug` in User-Facing Board Note Chat Events](prd/archive/prd-047-prefer-note-slug-in-chat-events.md) | Implemented | v0.29.2 | `themion-cli`, docs |
| PRD-048 | [Remove Long Navigation Shortcut Hints from the TUI Statusline](prd/archive/prd-048-remove-navigation-shortcut-hints-from-statusline.md) | Implemented | v0.29.3 | `themion-cli`, docs |
| PRD-049 | [Project Memory and Global Knowledge Naming for Durable Knowledge Tools](prd/archive/prd-049-project-memory-and-global-knowledge-naming.md) | Implemented | v0.30.0 | `themion-core`, `themion-cli`, docs |
| PRD-050 | [Reorganize Tokio Runtime Execution into Domain-Specific Pools](prd/archive/prd-050-reorganize-tokio-runtime-pools.md) | Implemented | v0.31.0 | `themion-cli`, `themion-core`, docs |
| PRD-051 | [Separate Shared Application Runtime from TUI Presentation and Introduce Headless Mode](prd/prd-051-separate-shared-application-runtime-from-tui-presentation-and-introduce-headless-mode.md) | Implemented | v0.32.0 | `themion-cli`, docs |
| PRD-052 | [Local System Inspection Tool for Runtime, Tooling, and Provider Readiness](prd/prd-052-tool-and-model-self-healthcheck.md) | Implemented | v0.33.0 | `themion-core`, `themion-cli`, docs |
| PRD-053 | [Tighten Tokio Runtime Topology Semantics and Remove Remaining TUI-Orchestration Leakage](prd/prd-053-tighten-tokio-runtime-topology-and-tui-layering.md) | Implemented | v0.34.0 | `themion-cli`, docs |
| PRD-054 | [Rename Shared CLI Application Runtime Type to `AppState`](prd/prd-054-rename-shared-cli-app-runtime-to-app-state.md) | Implemented | v0.34.1 | `themion-cli`, docs |
| PRD-055 | [Fix TUI Input Dirty Detection for Non-ASCII Typing and Paste-Burst Flushes](prd/prd-055-fix-tui-input-dirty-detection-for-non-ascii-and-paste-burst.md) | Implemented | v0.34.2 | `themion-cli`, docs |
| PRD-056 | [Right-Size Tool Result Payloads and Standardize Mutation Acknowledgements](prd/prd-056-right-size-tool-result-payloads-and-standardize-mutation-acks.md) | Implemented | v0.35.0 | `themion-core`, `themion-cli`, docs |
| PRD-057 | [Store Turn-Level Runtime Metadata as JSON in `agent_turns.meta`](prd/prd-057-store-turn-level-runtime-metadata-as-json-in-agent-turns-meta.md) | Implemented | v0.35.1 | `themion-core`, docs |
| PRD-058 | [Optional Tool-Reason Guidance Recording and Chat Visibility](prd/prd-058-optional-tool-reason-guidance-recording-and-chat-visibility.md) | Implemented | v0.36.0 | `themion-core`, `themion-cli`, docs |
| PRD-059 | [Add Vector Embedding and Semantic Search for Project Memory](prd/prd-059-add-vector-embedding-and-semantic-search-for-project-memory.md) | Implemented | v0.37.0 | phased delivery: Phase 1 spike artifact and evaluation plus Phase 2 feature-flagged production integration for `themion-core`/`themion-cli`, with later optimization follow-ons if warranted |
| PRD-060 | [Replace `tui-textarea` with a Themion-Owned Composer Text Buffer Inspired by `codex-rs`](prd/prd-060-replace-tui-textarea-with-local-composer-buffer.md) | Implemented | v0.38.0 | `themion-cli`, docs |
