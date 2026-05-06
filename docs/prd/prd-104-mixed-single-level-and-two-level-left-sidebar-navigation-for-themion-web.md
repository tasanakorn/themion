# PRD-104: Mixed Single-Level and Two-Level Left Sidebar Navigation for Themion Web

- **Status:** Implemented
- **Version:** v0.63.0
- **Scope:** `themion-web`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-06

## Summary

- The current web sidebar is too flat for a growing product.
- Update the left sidebar so it supports both single-level items and grouped two-level items.
- The first grouped section is `Knowledge` with two child items: `Stats` and `Query`.
- Standalone items such as `Example` must remain valid top-level sidebar items.
- Keep the change focused on navigation structure, active states, and direct links.

## Implementation notes

- Landed in `themion-web` with one typed sidebar model that supports standalone items plus grouped `Knowledge` child destinations.
- The first implementation uses direct-linkable `/?page=knowledge&view=stats` and `/?page=knowledge&view=query` URL state for the grouped knowledge pages.
- Standalone pages such as `Example`, `Agent`, and `Shell` remain direct top-level destinations.

## Goals

- Add a sidebar navigation model in `themion-web` that supports both standalone items and grouped items.
- Group related destinations under one parent section such as `Knowledge`.
- Keep standalone top-level items valid when they do not need child items.
- Make `Knowledge` → `Stats` and `Knowledge` → `Query` clear in the sidebar.
- Keep navigation easy to scan, easy to understand, and direct-link friendly.

## Non-goals

- Do not force every sidebar item into a grouped parent/child structure.
- Do not redesign existing knowledge content beyond what the new navigation needs.
- Do not add third-level nesting or a tree-style navigation system.
- Do not add runtime-control or agent-control features as part of this sidebar change.
- Do not ship placeholder-only nav items that lead nowhere.

## Background & Motivation

### Current state

`themion-web` now has more than one workflow inside the knowledge area. Today those workflows are clearer inside the page than from the sidebar.

A flat sidebar makes every destination look unrelated. That gets harder to scan as the web surface grows.

### Why this matters now

Not every page needs children, but some areas clearly do. The knowledge area already has two clear modes:

- `Stats`
- `Query`

These should read as one grouped area, not as unrelated top-level pages. At the same time, simple standalone items should remain simple.

**Alternative considered:** force every sidebar item into a two-level structure. Rejected: the product should support grouping where useful without making simple items artificially nested.

## Design

### Navigation structure

The left sidebar should support a mixed structure:

- single-level top-level items
- grouped top-level items with child destinations underneath

Required behavior:

- keep the sidebar at no more than two levels deep
- allow a top-level item to be either:
  - a direct destination, or
  - a parent section with child destinations
- make standalone items and grouped items visually distinct
- use child items as the real navigation targets for grouped sections

Implementation requirement:

- the sidebar data model must represent both shapes directly instead of faking standalone items as one-child groups

Example structure:

- `Knowledge`
  - `Stats`
  - `Query`
- `Example`

### Knowledge section

The sidebar should show the knowledge area as two child destinations:

- `Stats` for the summary-oriented view
- `Query` for the search-oriented view

Implementation requirement:

- `Stats` and `Query` must be separate navigable destinations in the sidebar, even if the first implementation still renders them from one route with an explicit view parameter

Allowed URL shapes for the first implementation:

- separate routes such as `/knowledge/stats` and `/knowledge/query`
- one route with an explicit view parameter such as `/knowledge?view=stats` and `/knowledge?view=query`

Each child destination must be directly addressable.

### Active state and direct links

Required behavior:

- the active standalone item must be clearly highlighted
- the active child item must be clearly highlighted
- the parent section for the active child must appear active or expanded
- direct navigation by URL must restore the correct active state
- browser back and forward must keep the sidebar state in sync with the page

Prefer always-expanded parent groups for the first implementation unless a larger number of sections makes collapsing necessary.

### Navigation data model

`themion-web` should define the sidebar from one small typed navigation model instead of scattered hard-coded branches.

That model should make standalone items, parent sections, child items, labels, and routes easy to review in one place.

Implementation requirement:

- active-state matching rules must also live in that model or next to it, so sidebar selection logic is not duplicated across render paths

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-104-mixed-single-level-and-two-level-left-sidebar-navigation-for-themion-web.md` | Define the mixed single-level and two-level sidebar requirement for `themion-web`. |
| `docs/README.md` | List PRD-104 with the corrected title, status, version, and link. |
| `crates/themion-web/src/main.rs` | Update routing and app-shell rendering for standalone items plus parent/child navigation. |
| `crates/themion-web/src/components/` | Add or adjust sidebar rendering helpers and structure. |
| `crates/themion-web/style/` | Add styling for standalone items, grouped items, and active states. |
| `crates/themion-web/README.md` | Update docs when the new sidebar structure lands. |

## Edge Cases

- direct URL to `Knowledge` → `Query` → verify: `Knowledge` is expanded or active and `Query` is selected.
- direct URL to `Knowledge` → `Stats` → verify: the correct child is selected on first render.
- direct URL to a standalone top-level item such as `Example` → verify: the standalone item is selected without requiring a parent section.
- only one child exists under a parent section → verify: the grouped layout still looks intentional.
- a future section is not ready yet → verify: no dead placeholder child item is shown.

## Migration

This is a navigation change only.

- existing web features stay available
- old knowledge links should continue to reach the closest matching destination when practical
- no database or runtime migration is required

## Testing

- open `themion-web` with the new sidebar → verify: the sidebar supports both standalone items and grouped parent/child sections.
- click `Knowledge` → `Stats` and `Knowledge` → `Query` → verify: page content and active sidebar state both change correctly.
- open a grouped child destination directly by URL → verify: the correct parent and child state render on first load.
- open a standalone destination directly by URL → verify: the standalone item is selected correctly on first load.
- use browser back and forward between standalone and grouped destinations → verify: sidebar state stays synchronized.
- verify unimplemented sections are not shown as dead placeholders → verify: every visible item leads to a real destination.
- verify standalone items are not modeled or rendered as fake one-child groups → verify: the typed navigation model preserves both shapes cleanly.

## Implementation checklist

- [ ] define a typed sidebar navigation model that supports standalone items and grouped items as separate shapes
- [ ] render standalone top-level items and parent/child groups in the left sidebar
- [ ] make `Knowledge` → `Stats` and `Knowledge` → `Query` explicit sidebar destinations
- [ ] keep standalone items valid direct destinations instead of wrapping them as fake groups
- [ ] ensure each destination has a stable direct URL or explicit view parameter
- [ ] keep active state correct for both standalone items and grouped items
- [ ] keep sidebar selection logic sourced from one navigation model
- [ ] update docs when the sidebar structure lands
