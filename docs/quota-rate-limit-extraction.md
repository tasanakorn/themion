# Reading and extracting quota / rate-limit windows

## Source of truth

Use the backend RPC:

- `account/rateLimits/read`

In the TUI this is fetched via `ClientRequest::GetAccountRateLimits` and converted into `RateLimitSnapshot` values.

## Goal

Turn backend rate-limit data into a stable, UI-friendly structure that answers:

- how much quota is left
- what bucket/window it belongs to
- when it resets
- whether credits are enabled / unlimited / depleted

## Data model to read

Each `RateLimitSnapshot` may contain:

- `primary: RateLimitWindow`
- `secondary: RateLimitWindow`
- `credits: CreditsSnapshot`

Each `RateLimitWindow` provides:

- `used_percent: f64`
- `window_minutes: Option<i64>`
- `resets_at: Option<i64>` — Unix timestamp seconds

Expected `CreditsSnapshot` fields:

- `has_credits: bool`
- `unlimited: bool`
- `balance: Option<...>`

The exact numeric type for `balance` is less important than preserving a display-safe value.

## Which windows matter

Interpret windows as:

- primary = short rolling window, usually `5h`
- secondary = longer plan window, usually `weekly`

Other possible durations:

- hourly-ish/day-ish values render as `Nh`
- up to 7 days → `weekly`
- up to 30 days → `monthly`
- above that → `annual`

Duration label mapping is currently:

- `<= 24h + 3m` => `"{hours}h"`
- `<= 7d + 3m` => `weekly`
- `<= 30d + 3m` => `monthly`
- else => `annual`

Examples:

- `300` → `5h`
- `10080` → `weekly`

## How to extract remaining quota

For each present window:

- `percent_left = 100.0 - used_percent`

Clamp if desired to `[0, 100]` for display safety.

Recommended clamp:

- `percent_left = max(0.0, min(100.0, 100.0 - used_percent))`

This protects against backend rounding drift, overages, or malformed values.

## How to extract reset time

If `resets_at` is present:

- parse as Unix timestamp seconds
- convert to local time for UI display
- preserve original Unix timestamp in normalized output

Example display target:

- `95% left (resets 00:01 on 19 Apr)`

If `resets_at` is absent:

- show only remaining percent
- example: `95% left`

If the timestamp is invalid or unparseable:

- keep `resets_at_unix` if available
- omit the human string or fall back to percent-only display

## Credits extraction rules

Treat credits separately from rate-limit windows.

Recommended logic:

1. If `credits` is missing:
   - output `credits: null` or omit the field
2. If `has_credits == false`:
   - treat credits as disabled / not applicable
3. If `unlimited == true`:
   - show `Credits: Unlimited`
4. Else if `balance` is present:
   - format and surface the balance
5. Else:
   - show credits as enabled but unknown balance

Suggested normalized shape:

```json
{
  "enabled": true,
  "unlimited": false,
  "balance": "17.5",
  "display": "Credits: 17.5"
}
```

If `has_credits == false`, prefer:

```json
{
  "enabled": false,
  "unlimited": false,
  "balance": null,
  "display": null
}
```

## Recommended extraction algorithm

For each `RateLimitSnapshot`:

1. Read `primary`, if present
   - derive duration label from `window_minutes` or default to `5h`
   - compute `percent_left = 100 - used_percent`
   - clamp to `[0, 100]`
   - format reset timestamp if present
2. Read `secondary`, if present
   - derive duration label from `window_minutes` or default to `weekly`
   - compute `percent_left = 100 - used_percent`
   - clamp to `[0, 100]`
   - format reset timestamp if present
3. Optionally read `credits`
   - if `has_credits == false`, ignore or emit disabled state
   - if `unlimited == true`, show `Credits: Unlimited`
   - else if `balance` present, show formatted balance
4. Return a normalized result that preserves both machine-readable and display-ready forms

## Output contract

Normalized extracted structure:

```json
{
  "limits": [
    {
      "kind": "primary",
      "label": "5h",
      "window_minutes": 300,
      "used_percent": 5.0,
      "percent_left": 95.0,
      "resets_at_unix": 1713484860,
      "display": "95% left (resets 00:01 on 19 Apr)"
    },
    {
      "kind": "secondary",
      "label": "weekly",
      "window_minutes": 10080,
      "used_percent": 38.0,
      "percent_left": 62.0,
      "resets_at_unix": 1714000000,
      "display": "62% left (resets 12:30 on 25 Apr)"
    }
  ],
  "credits": {
    "enabled": true,
    "unlimited": false,
    "balance": "17.5",
    "display": "Credits: 17.5"
  }
}
```

## Minimal pseudocode

```text
for snapshot in rate_limit_snapshots:
    if snapshot.primary exists:
        label = label_from_minutes(snapshot.primary.window_minutes, default="5h")
        left = max(0, min(100, 100 - snapshot.primary.used_percent))
        text = format("{left:.0f}% left")
        if snapshot.primary.resets_at exists and timestamp_is_valid(snapshot.primary.resets_at):
            text += format(" (resets {local_time(snapshot.primary.resets_at)})")
        emit(kind="primary", label, left, snapshot.primary.resets_at, text)

    if snapshot.secondary exists:
        label = label_from_minutes(snapshot.secondary.window_minutes, default="weekly")
        left = max(0, min(100, 100 - snapshot.secondary.used_percent))
        text = format("{left:.0f}% left")
        if snapshot.secondary.resets_at exists and timestamp_is_valid(snapshot.secondary.resets_at):
            text += format(" (resets {local_time(snapshot.secondary.resets_at)})")
        emit(kind="secondary", label, left, snapshot.secondary.resets_at, text)

    if snapshot.credits exists:
        if !snapshot.credits.has_credits:
            credits = { enabled: false, unlimited: false, balance: null, display: null }
        else if snapshot.credits.unlimited:
            credits = { enabled: true, unlimited: true, balance: null, display: "Credits: Unlimited" }
        else:
            credits = {
                enabled: true,
                unlimited: false,
                balance: snapshot.credits.balance,
                display: format_balance(snapshot.credits.balance)
            }
```

## Suggested helper behavior

### `label_from_minutes(minutes, default)`

Use the backend-provided `window_minutes` when present.

Suggested behavior:

- if `minutes` is missing, return the supplied default
- if `minutes <= 24h + 3m`, round to whole hours and render `"{hours}h"`
- else if `minutes <= 7d + 3m`, return `weekly`
- else if `minutes <= 30d + 3m`, return `monthly`
- else return `annual`

### `format_balance(balance)`

Keep balance formatting conservative:

- preserve precision if already provided as a string
- otherwise use a compact decimal form without unnecessary trailing zeros
- avoid locale-dependent formatting unless the whole UI is localized

### `format_reset(ts)`

Suggested display format:

- `HH:MM on D Mon`

But the exact UI string can vary as long as:

- the machine-readable Unix timestamp is preserved
- local timezone conversion is applied consistently

## Edge cases

### Missing `window_minutes`

Use defaults:

- primary default label: `5h`
- secondary default label: `weekly`

### Missing `resets_at`

Still emit the limit row; only omit the reset text.

### `used_percent < 0` or `used_percent > 100`

Clamp display values. If you retain raw values for debugging, do so separately from the displayed percentage.

### Unknown extra windows

If future schemas add additional buckets beyond `primary` and `secondary`:

- preserve current extraction for known fields
- avoid breaking when new fields appear
- optionally extend normalized output later with a generic `other_limits` array

### Multiple snapshots

If the RPC returns multiple `RateLimitSnapshot` items:

- apply the same normalization per snapshot
- do not merge unrelated snapshots unless the caller has a documented reason to do so

## Practical user-facing rule

If someone asks, "how do I get quota / limit?":

- call `account/rateLimits/read`
- read `primary` and `secondary`
- convert `used_percent` into remaining quota with `100 - used_percent`
- clamp to `[0, 100]` for display
- use `window_minutes` to label the bucket (`5h`, `weekly`, `monthly`, etc.)
- use `resets_at` to show when it resets
- read `credits` separately for unlimited / balance status

## Confidence / gaps

This document is based on the currently observed field names and TUI naming described above.

Still worth confirming in code or live payloads:

- whether `primary` / `secondary` are optional in all cases
- the exact type of `credits.balance`
- whether `account/rateLimits/read` returns a single snapshot or a list of snapshots
- whether any providers omit `window_minutes` or `resets_at` systematically
