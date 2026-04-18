# Reading and extracting quota / rate-limit windows

## Source of truth

For the TUI status-line quota display, the source of truth is **response headers**.

Use header-based parsing for the `codex` bucket:

- `x-codex-primary-used-percent`
- `x-codex-primary-window-minutes`
- `x-codex-primary-reset-at`
- `x-codex-secondary-used-percent`
- `x-codex-secondary-window-minutes`
- `x-codex-secondary-reset-at`
- `x-codex-credits-has-credits`
- `x-codex-credits-unlimited`
- `x-codex-credits-balance`
- `x-codex-active-limit`

The TUI displays **remaining percent**, not used percent.

## Status-line mapping

For the main `codex` bucket:

- `primary` → short window, typically shown like `5h`
- `secondary` → longer window, typically shown like `weekly`

Display calculation:

- `remaining_percent = clamp(100.0 - used_percent, 0.0, 100.0)`

So:

- `five-hour-limit` comes from `codex.primary.used_percent`
- `weekly-limit` comes from `codex.secondary.used_percent`

## Per-call extraction

### `POST /responses`

Read rate-limit snapshots from the **HTTP response headers**.

If streamed updates also exist, they may refine the snapshot later, but the immediate status extraction is header-based.

### `429` responses

Read:

- `x-codex-active-limit`

Then parse the matching limit family from headers.

## What not to use here

Do **not** treat `/models` or a usage JSON body as the documented source of truth for this TUI status-line value.
