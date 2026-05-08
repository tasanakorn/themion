# themion-web

> Obsolete migration residue.
>
> The product direction has moved to `themion-cli --web`.
> Do not treat this crate as the target architecture for new browser work.

## Status

This crate remains only as temporary migration source material while browser behavior is absorbed into `themion-cli`.

Use PRD-106 as the active requirement:
- `docs/prd/prd-106-web-interface-as-themion-cli-web-io-layer.md`

## Current repository policy

- new browser/runtime work should target `themion-cli --web`
- `themion-web` should not gain a new long-term runtime ownership role
- code here may still be referenced, migrated, or retired as the CLI-owned web mode grows

## Historical note

This crate currently contains older browser-specific pieces such as:
- Axum/Leptos web serving
- websocket transport
- browser shell / PTY support
- direct database-backed read-only pages

Those pieces are historical implementation material, not the intended final ownership model.


## Remaining migration residue

The main unfinished migration areas are:
- shared websocket transport (`/api/ws`)
- richer browser shell / PTY UI and protocol code beyond the minimal CLI-owned shell page already migrated into `themion-cli --web`
- Leptos UI pages that still assume the standalone `themion-web` binary

Until those pieces are absorbed or retired, this crate remains obsolete residue rather than a supported product path.
