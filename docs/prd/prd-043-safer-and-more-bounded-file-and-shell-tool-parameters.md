# PRD-043: Safer and More Bounded File and Shell Tool Parameters

- **Status:** Implemented
- **Version:** v0.27.0
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-23

## Summary

- The current filesystem and shell tools are too loose about payload format and output size.
- Add explicit `mode` parameters for file reads and writes so binary-safe usage is built in rather than ad hoc.
- Make `fs_read_file` bounded by default with `offset` and `limit` controls and a hard maximum of 2 MiB.
- Make `shell_run_command` bounded by default with a result-size limit and a default timeout.
- Keep the tools simple and compatible with existing usage patterns by defaulting file payloads to base64 and keeping shell behavior text-based.
- Do not redesign the broader tool system in this PRD; this is a targeted contract hardening for the most common local IO tools.

## Goals

- Make `fs_read_file` safe and explicit for both text and binary data.
- Make `fs_write_file` safe and explicit for both text and binary data.
- Bound default file-read size so one call cannot accidentally dump a very large file into the tool transcript.
- Add basic chunked-read support through `offset` and `limit` so large files remain accessible incrementally.
- Bound shell output size and execution time by default.
- Keep tool contracts clear enough that model behavior becomes more predictable and less error-prone.
- Update docs so the new tool semantics are documented as the expected runtime contract.

## Non-goals

- No redesign of unrelated tools.
- No change to `fs_list_directory` in this PRD.
- No introduction of a streaming file-transfer protocol.
- No redesign of shell sandboxing, environment handling, or working-directory semantics.
- No attempt to make `shell_run_command` binary-output aware in this slice; its contract remains bounded command output text.
- No provider-specific prompt changes beyond what the updated tool schema already implies.

## Background & Motivation

### Current state

Themion exposes local filesystem and shell tools from `themion-core/src/tools.rs`. The current architecture and runtime docs describe these tools at a high level, but the file and shell interfaces still leave a few practical gaps:

- file reads do not have a documented binary-safe default payload format
- file reads do not expose a standard offset/limit contract for chunked access
- file writes do not have an explicit content-encoding mode in the contract
- shell command output is not described with a default result-size cap and timeout in the user-facing tool contract

These gaps matter because the tools are transcript-visible and model-invokable. Loose or underspecified IO tools make it easier for one call to return too much data, mishandle binary content, or hang longer than intended.

### Why base64 should be the default file payload mode

For agent-facing tool contracts, a file tool should work predictably for both text and binary files. Raw text is convenient for many source files, but it is not a safe universal transport.

Defaulting file payloads to base64 gives the tool a single binary-safe behavior that works for text, images, archives, and partially read file segments. The caller can still opt into `raw` mode when plain text is actually preferred.

**Alternative considered:** keep raw text as the default because source-code editing is common. Rejected: source files are common, but the tool contract should still be binary-safe by default and avoid ambiguous encoding behavior.

### Why bounded reads and shell results should be built into the tool contract

Themion already emphasizes tool grounding and concise transcripts. Unbounded file reads or shell output work against that goal because they can flood the conversation context, slow the session, and make follow-up reasoning harder.

Adding defaults and hard ceilings directly to the tool schema makes the safe path the normal path:

- `fs_read_file` defaults to `offset=0` and `limit=128 KiB`
- `fs_read_file` rejects limits above `2 MiB`
- `shell_run_command` defaults to a `16 KiB` result limit
- `shell_run_command` defaults to a `5 minute` timeout

This keeps the tools useful for normal coding work while preventing accidental oversized responses or indefinitely long shell calls.

**Alternative considered:** keep the tools permissive and rely on prompt guidance to ask the model to be careful. Rejected: the contract should enforce reasonable bounds rather than depending only on model behavior.

### Why chunked file access is better than one large read

Large files still need to be inspectable. The right first-step behavior is not to remove access, but to make it incremental.

A simple `offset` plus `limit` contract is enough for practical chunked reading and matches how agents already reason about partial inspection. It also avoids inventing a more complex cursor or session protocol.

**Alternative considered:** add a special paged-read session token or cursor-based API. Rejected: unnecessary complexity for a first bounded-read contract.

## Design

### Extend `fs_read_file` with explicit mode, offset, and limit

`fs_read_file` should accept these arguments:

- `path: string`
- `mode: "raw" | "base64"` with default `"base64"`
- `offset: integer` with default `0`
- `limit: integer` with default `131072` bytes

Normative direction:

- `limit` must be interpreted in bytes
- values above `2097152` bytes must be rejected
- negative `offset` or non-positive `limit` should be rejected clearly
- `raw` mode should return direct file text only when the selected byte slice can be represented as valid UTF-8; otherwise the tool should return an error instructing the caller to use `base64`
- `base64` mode should return the selected byte slice encoded as base64 text
- the tool result should make the returned range explicit so callers can continue reading predictably

This keeps file reads binary-safe, bounded, and incrementally navigable.

**Alternative considered:** silently coerce invalid UTF-8 in `raw` mode with replacement characters. Rejected: that would lose byte fidelity and make the tool misleading for binary or mixed-encoding data.

### Extend `fs_write_file` with explicit payload mode

`fs_write_file` should accept these arguments:

- `path: string`
- `content: string`
- `mode: "raw" | "base64"` with default `"base64"`

Normative direction:

- in `raw` mode, the tool writes the provided string bytes as-is
- in `base64` mode, the tool decodes the provided content from base64 and writes the resulting bytes
- invalid base64 input must produce a clear error
- the tool should keep its current overwrite semantics unless and until a separate PRD changes them

This makes write behavior explicit and symmetric with read behavior without changing the basic simplicity of the tool.

**Alternative considered:** add separate tools such as `fs_write_file_raw` and `fs_write_file_base64`. Rejected: one tool with an explicit mode is easier to document and keeps the tool surface smaller.

### Bound `shell_run_command` result size and execution time by default

`shell_run_command` should accept these additional arguments:

- `command: string`
- `result_limit: integer` with default `16384` bytes
- `timeout_ms: integer` with default `300000`

Normative direction:

- `result_limit` applies to the combined returned command output payload
- when output exceeds the limit, the tool should truncate deterministically and indicate truncation in the result
- `timeout_ms` should be enforced by the tool runtime rather than advisory text only
- if the command times out, the tool should terminate or stop waiting for the command and return a clear timeout result
- if callers need larger outputs or longer execution, they may request them explicitly within any enforced implementation-side maximums

This keeps shell usage practical for coding work while reducing transcript blowups and stalled turns.

**Alternative considered:** keep shell output unbounded but summarize after capture. Rejected: the runtime should avoid capturing arbitrarily large output in the first place.

### Update tool schemas and docs together

The JSON tool definitions and the human-readable docs should be updated in the same change.

Normative direction:

- `themion-core/src/tools.rs` should expose the new parameter shapes and defaults in tool definitions
- runtime behavior should match those definitions exactly
- `docs/architecture.md` and `docs/engine-runtime.md` should describe the new bounded and mode-aware semantics at a level useful to readers
- any docs that mention these tools should avoid implying unbounded or text-only behavior once the new contract lands

This keeps prompt-visible schema and repository documentation aligned.

**Alternative considered:** update only the schema and trust it to be self-documenting. Rejected: repository docs should still describe the behavioral contract and rationale.

### Prefer explicit defaults over implicit compatibility assumptions

The tool contract should specify defaults directly rather than expecting every caller to pass them.

Normative direction:

- omitted `mode` on file tools means `base64`
- omitted `offset` on reads means `0`
- omitted `limit` on reads means `131072`
- omitted `result_limit` on shell commands means `16384`
- omitted `timeout_ms` on shell commands means `300000`

This ensures the default behavior is stable and model-visible.

**Alternative considered:** require the caller to always pass every new field. Rejected: defaults keep the tools easy to use while still hardening behavior.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Extend `fs_read_file` schema and implementation with `mode`, `offset`, and `limit`; default to base64 and bounded reads; reject invalid ranges and over-limit requests. |
| `crates/themion-core/src/tools.rs` | Extend `fs_write_file` schema and implementation with `mode`; default to base64 and reject invalid base64 input cleanly. |
| `crates/themion-core/src/tools.rs` | Extend `shell_run_command` schema and implementation with bounded `result_limit` and `timeout_ms` defaults and clear truncation/timeout reporting. |
| `docs/architecture.md` | Update the tools section to describe bounded file reads, mode-aware file writes, and bounded shell command behavior. |
| `docs/engine-runtime.md` | Document the new file/shell tool contracts and defaults so runtime behavior is explicit. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Edge Cases

- `fs_read_file` is called on a binary file with `mode="raw"` → verify: the tool returns a clear error telling the caller to use `base64` instead of returning corrupted text.
- `fs_read_file` is called with `offset` beyond EOF → verify: the tool returns an empty slice or a clear bounded success result rather than failing ambiguously.
- `fs_read_file` is called with `limit` above `2 MiB` → verify: the tool rejects the call clearly.
- `fs_read_file` is called repeatedly with increasing offsets → verify: callers can reconstruct a large file incrementally without overlap ambiguity.
- `fs_write_file` receives invalid base64 in default mode → verify: the tool rejects the write and does not create corrupted output.
- `fs_write_file` is used in `raw` mode for a normal UTF-8 source file → verify: the file is written exactly as provided.
- `shell_run_command` produces more than the default output cap → verify: the returned output is truncated deterministically and clearly labeled as truncated.
- `shell_run_command` exceeds the default timeout → verify: the tool returns a timeout result rather than hanging indefinitely.
- existing callers omit all new optional fields → verify: behavior follows the documented defaults without requiring every call site to change immediately.

## Migration

This is an additive tool-contract change with defaulted parameters rather than a new tool family.

Expected rollout shape:

- keep the existing tool names
- add defaulted optional parameters for mode, offsets, limits, and timeout
- let existing tool callers continue working when they omit the new fields
- update docs and tool schemas together so model-visible behavior changes land consistently

The main compatibility consideration is semantic rather than syntactic: callers that previously assumed raw text file reads or effectively unbounded shell output may need to opt into `raw` or request larger bounds explicitly once the implementation lands.

## Testing

- call `fs_read_file` on a normal UTF-8 source file with no optional arguments → verify: the result is base64 for bytes `0..131072` by default.
- call `fs_read_file` with `mode="raw"` on a UTF-8 source file → verify: the tool returns the requested plain-text slice.
- call `fs_read_file` with `mode="raw"` on binary content → verify: the tool returns a clear error directing the caller to `base64`.
- call `fs_read_file` with `offset` and `limit` across multiple chunks → verify: returned ranges are predictable and reconstruct the file content correctly.
- call `fs_read_file` with `limit` above `2097152` → verify: the request is rejected clearly.
- call `fs_write_file` with default mode using base64 content → verify: decoded bytes are written correctly.
- call `fs_write_file` with `mode="raw"` using source text → verify: the file contents match the provided raw string exactly.
- call `shell_run_command` with no optional arguments on a short command → verify: the command succeeds under the default `300000` ms timeout and returns bounded text output.
- call `shell_run_command` on a command that emits output beyond `16384` bytes → verify: the result is truncated and labeled clearly.
- call `shell_run_command` on a command that runs longer than the timeout → verify: the result reports timeout rather than waiting indefinitely.

## Implementation checklist

- [x] extend `fs_read_file` tool schema with `mode`, `offset`, and `limit`
- [x] implement bounded read slicing with default `offset=0`, default `limit=128 KiB`, and max `2 MiB`
- [x] make `fs_read_file` default to base64 output and reject invalid `raw` decoding cases clearly
- [x] extend `fs_write_file` tool schema with `mode`
- [x] implement base64-decoding write behavior with clear invalid-input errors
- [x] extend `shell_run_command` tool schema with `result_limit` and `timeout_ms`
- [x] enforce default `16 KiB` shell output limiting and default `5 minute` timeout with clear reporting
- [x] update `docs/architecture.md` and `docs/engine-runtime.md`
- [x] update `docs/README.md` with the new PRD entry

## Implementation notes

The implemented slice landed with these concrete behaviors:

- `crates/themion-core/src/tools.rs` now exposes mode-aware file read and write contracts and bounded shell command defaults in the tool schema
- `fs_read_file` now returns structured JSON with `content` plus range metadata, defaults to base64, and supports chunked reads through `offset` and `limit`
- `fs_read_file` rejects over-limit reads and invalid `raw` decoding for non-UTF-8 byte ranges
- `fs_write_file` now defaults to base64-decoded writes while preserving `raw` mode for direct text writes
- `shell_run_command` now enforces default timeout and result-size limits with explicit timeout and truncation reporting
- `docs/architecture.md` and `docs/engine-runtime.md` now describe the bounded local IO tool contracts
