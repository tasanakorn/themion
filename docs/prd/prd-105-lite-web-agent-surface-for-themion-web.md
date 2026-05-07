# PRD-105: Web Interface Delivered Through `themion-cli --web`

- **Status:** Canceled
- **Version:** v0.64.0
- **Scope:** historical `themion-web` direction, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-06

## Summary

- This PRD is canceled as a forward product direction.
- The earlier direction assumed a separate `themion-web` product/runtime path.
- The team has decided the web interface should instead plug into `themion-cli` as another local I/O surface enabled by `--web`.
- Existing useful web work should be reused where practical, but future design and implementation should follow the new CLI-owned web-interface direction.
- A replacement PRD defines that new direction.

## Cancellation note

PRD-105 described a lightweight web-agent surface built around `themion-web` as a separate product/runtime path. After implementation and review, the team decided this is the wrong long-term shape for Themion.

The main issue is architectural overlap. `themion-cli` already owns the local runtime, agent roster, app-state coordination, and other behavior that the web surface needs. Re-inventing that runtime again in a separate web product creates duplicated orchestration, duplicated bootstrap, and extra drift risk.

Future web work should treat the browser as another I/O layer for the existing `themion-cli` runtime. In the same way the TUI is a surface over runtime-owned state, the web interface should become a surface over CLI-owned runtime/app-state behavior. PTY and browser transport can still have their own adapters, but they should not imply a second product runtime.

Implemented work from this PRD remains historical context and may still supply reusable UI, transport, or browser-specific pieces. However, this PRD should no longer be used as the active design basis for future web direction.

## Replacement

Use PRD-106 as the active requirement for future web-interface work.
