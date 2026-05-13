# PRD-126: Tighten Themion File-Editing Tool Descriptions and System Prompt Guidance

- **Status:** Implemented
- **Version:** v0.75.1
- **Scope:** `themion-core`, tool schemas/descriptions, Themion runtime system prompt
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-12

## Summary

- Models still misuse file-edit tools in two common ways: they rewrite existing files with `fs_write_file`, or they jump straight to shell-based file mutation.
- Tighten Themion's built-in system prompt and model-facing tool descriptions so `fs_write_file` is for creating new files only.
- Make `fs_patch` the normal and primary tool for modifying existing text files in Themion's app-level guidance.
- Document the current `fs_patch` path and format contract clearly in the tool description: it accepts standard unified diffs with project-relative targets, and rejects absolute paths or `..` traversal outside the project directory.
- Keep shell-based file mutation available only as a last-resort fallback when repository tools cannot complete the change cleanly.

## Goals

- Reduce incorrect existing-file rewrites caused by whole-file `fs_write_file` usage.
- Make `fs_patch` the clear default for source-code and text-file modification.
- Document `fs_patch` path limits in the model-facing tool description and app-level prompt guidance.
- Improve Themion's tool description text so it teaches the supported `fs_patch` unified-diff format directly.
- Discourage shell-based file mutation without banning it when it is truly the only workable path.
- Keep the system prompt and tool descriptions short, direct, and easy for models to follow.

## Non-goals

- Do not change the underlying `fs_write_file` or `fs_patch` runtime implementation in this PRD.
- Do not remove shell-based editing from the system entirely.
- Do not change tool parameter schemas in this PRD. Only tool descriptions and app-level prompt guidance are in scope.
- Do not broaden `fs_patch` to support absolute paths, out-of-project traversal, binary edits, or non-text patch formats.
- Do not treat root `AGENTS.md` or project-specific repository instructions as the primary product surface for this fix.

## Background & Motivation

### Current state

Themion already has the right file-edit tool split in product behavior:

- `fs_write_file` writes a whole file body
- `fs_patch` applies targeted unified-diff edits to existing text files

However, model behavior is still uneven. Two failure patterns keep recurring:

- the model uses `fs_write_file` to modify an existing file and sends incorrect or incomplete full-file content
- the model skips file tools and uses shell commands to mutate files directly even when `fs_patch` would be safer and clearer

The problem is not mainly missing capability. It is missing instruction priority in Themion's built-in system prompt and missing explicit guardrails in the tool descriptions the model sees at tool-call time.

### Why this matters now

`fs_patch` was added specifically to avoid risky whole-file rewrites for small edits. If prompt and guide text still leave room for `fs_write_file` as a normal existing-file edit tool, models will keep taking the higher-risk path.

There is also a format clarity issue. A small live smoke test showed that `fs_patch` rejects the common OpenAI-style patch wrapper:

```text
*** Begin Patch
*** Update File: path
...
*** End Patch
```

The tool expects a standard unified diff with `--- a/path`, `+++ b/path`, and `@@` hunks. Current tool description wording that says only “Unified-diff patch text” is technically correct, but it is not enough to prevent this failure for models that have learned the other patch format.

Shell mutation has a similar issue. It is sometimes necessary, but if the guidance does not clearly rank it below dedicated file tools, models may choose a less reliable and less reviewable path.

## Design

### 1. Make `fs_patch` the primary edit tool for existing text files

Model-facing guidance must say plainly:

- use `fs_patch` as the primary tool for modifying existing source files or other text-based files
- prefer `fs_patch` for localized edits and multi-hunk edits
- use `fs_patch` before considering shell-based file mutation for normal text edits
- pass a true unified diff, not the `*** Begin Patch` / `*** Update File` patch format

This should appear in Themion's built-in system prompt or equivalent app-level prompt guidance, and in the `fs_patch` tool description itself. Project-local instruction files may repeat the rule, but they are not the main target of this PRD.

### 2. Reserve `fs_write_file` for creating new files

Model-facing guidance must say plainly:

- use `fs_write_file` to create a new file
- do not use `fs_write_file` as the normal tool for modifying an existing text file
- if an existing text file must change, prefer `fs_patch`
- avoid `fs_write_file` payloads over about 2K characters because current tool-call capture can truncate larger arguments before execution

The intent is instructional, not a runtime behavior change in this PRD. The underlying tool may still support whole-file replacement, but the canonical model-facing rule should reserve it for new-file creation.

For larger new-file content, guidance should prefer one of these paths instead of one large `fs_write_file` body:

- use `fs_write_file` only when the new file content stays within the safe small-payload range
- if a new text file would exceed about 2K characters, prefer creating a minimal starter file and then extending it with `fs_patch`
- if the change cannot be completed cleanly through repository file tools, use shell-based creation only as the documented last resort

### 3. Document `fs_patch` path limits explicitly

Guidance must state the current path contract clearly:

- `fs_patch` supports only paths that resolve relative to the current project directory
- `fs_patch` does not support absolute paths
- `fs_patch` does not support `..` traversal outside the project directory

This rule should be visible in the `fs_patch` tool description because incorrect path assumptions lead to wasted turns and failed edits.

### 4. Add an explicit `fs_patch` success example

Guidance should include one small canonical example because examples reduce patch-format drift better than prose alone.

Required successful shape:

```diff
--- a/tmp/example.txt
+++ b/tmp/example.txt
@@ -1 +1 @@
-hello
+hello patched
```

Required lesson learned:

- `fs_patch` input is the unified diff itself.
- File paths in the diff header must be project-relative through the `a/` and `b/` prefixes.
- Hunks must include standard `@@` ranges.
- Do not send `*** Begin Patch`, `*** Update File`, or `*** End Patch` wrappers.

The guidance should also include a short failure example so the model can recognize the wrong pattern:

```text
*** Begin Patch
*** Update File: tmp/example.txt
@@
-hello
+hello patched
*** End Patch
```

This PRD does not require runtime support for that alternate wrapper. The goal is to reduce avoidable tool-call failures by teaching the supported format clearly.

### 5. Keep shell-based file mutation as last-resort fallback guidance

Model-facing guidance must rank shell mutation below dedicated file tools.

Required guidance:

- do not prefer shell commands for normal file modification when `fs_patch` can express the change
- shell-based file editing is allowed only when it is the only practical way to complete the change cleanly
- when shell mutation is used, keep it targeted and explain the reason briefly

This preserves needed flexibility without teaching the shell path as a normal editing workflow.

### 6. Update app-level prompt guidance and tool descriptions

This guidance change should land in the Themion product surfaces that shape model behavior most directly:

- Themion's built-in base system prompt or equivalent default runtime instruction source
- the `fs_write_file` and `fs_patch` model-facing tool descriptions
- runtime/tool docs that describe the app behavior and explain why the description text is intentionally explicit

The goal is consistent instruction priority in the Themion app itself. Repository-local project instructions are secondary examples, not the product fix.

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-126-tighten-file-editing-tool-guidance.md` | Add the durable PRD for the guidance change. |
| `crates/themion-core` tool definitions | Update `fs_write_file` and `fs_patch` descriptions so the model sees the correct edit-tool choice and supported patch format at tool-call time. |
| Themion system prompt source | Add short default guidance in the CLI default system prompt and core predefined guardrails that reserves `fs_write_file` for new-file creation, makes `fs_patch` primary for existing text-file edits, and allows shell mutation only as last resort. |
| `docs/engine-runtime.md` or tool docs | Clarify the app-level runtime/tool guidance so future changes preserve the intended tool description and prompt behavior. |
| `docs/README.md` | Keep the PRD entry aligned with the corrected title, status, and scope. |

## Edge Cases

- model needs to create a brand new text file → verify: guidance points to `fs_write_file`.
- model needs to create a brand new file larger than about 2K characters → verify: guidance avoids one large `fs_write_file` payload and prefers a safer staged path.
- model needs to change one line in an existing Rust file → verify: guidance points to `fs_patch`.
- model needs to edit several hunks in one existing text file → verify: guidance still points to `fs_patch`.
- model prepares an `fs_patch` request with an absolute path → verify: guidance already says that path shape is unsupported.
- model prepares an `fs_patch` request with `../` that escapes the project directory → verify: guidance already says that path shape is unsupported.
- model prepares an `fs_patch` request using `*** Begin Patch` / `*** Update File` wrappers → verify: guidance shows that this format is unsupported.
- model prepares an `fs_patch` request using standard unified diff headers and `@@` hunks → verify: guidance shows this as the supported format.
- model considers using `sed`, `python`, or redirect-based shell mutation for a normal text edit → verify: guidance says not to prefer shell mutation when `fs_patch` can do the job.
- model has a rare edit case that cannot be expressed safely through `fs_patch` → verify: shell mutation remains allowed as documented fallback guidance.

## Migration

No database or schema migration is required.

This PRD changes Themion's built-in prompt guidance and model-facing tool descriptions. It does not change tool names, parameter schemas, or runtime patch semantics.

Patch-version scope is appropriate because the intended landing change is tool-description and system-prompt tightening in an existing feature area rather than a new user-visible capability.

## Testing

- inspect Themion's generated/system prompt text → verify: it says `fs_write_file` is for new-file creation and `fs_patch` is the primary existing-file edit tool.
- inspect `fs_write_file` tool description → verify: it discourages using the tool for normal existing-file edits and points to `fs_patch`.
- inspect `fs_patch` tool description → verify: it requires standard unified diff format, includes or clearly implies `--- a/path`, `+++ b/path`, and `@@` hunks, and rejects `*** Begin Patch` style wrappers.
- inspect updated app runtime/tool docs → verify: they explain the prompt/tool-description behavior and discourage shell-based file mutation except as last resort.
- inspect updated runtime docs → verify: they describe the current `fs_patch` path and unified-diff format limits explicitly.
- inspect the new PRD and `docs/README.md` entry → verify: the PRD is indexed with the right number, title, status, and scope.

## Implementation checklist

- [x] add PRD-126 with the final guidance rules and scope
- [x] update Themion's default system prompt or equivalent app-level instruction source
- [x] update `fs_write_file` model-facing tool description
- [x] update `fs_patch` model-facing tool description with standard unified-diff guidance and unsupported wrapper warning
- [x] update runtime/tool docs only where they explain the Themion app behavior
- [x] update `docs/README.md` PRD table

## Implementation Notes

Implemented in v0.75.1 scope by updating `crates/themion-cli/src/config.rs`, `crates/themion-core/src/predefined_guardrails.rs`, `crates/themion-core/src/tools.rs`, and `docs/engine-runtime.md`. The runtime patch parser behavior is unchanged; the change is model-facing prompt and tool-description guidance.
