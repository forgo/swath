# ADR 0021 — The UI system: shadow-DOM primitives on design tokens, one shell, modes in the URL

**Status:** Accepted · **Date:** 2026-08-26 · **Refs:** issue #277 (M10 — UX product structure),
`docs/design/ui-system.md` (the contract), ADR 0005 (frontend), ADR 0011 (embedded UI), ADR 0019
(every pixel through the engine)

## Context

ADR 0005 chose vanilla Web Components with "a tiny in-house reactive layer" and accepted the cost
of building reactivity, state and routing ourselves. Six elements later the reactive layer does
not exist: each element re-derives it by hand, styles are literals duplicated across six template
strings (zero custom properties in the tree), and the page is a rail of disclosures beside the map.
A design-language pass on that base is a six-file search-and-replace; every new surface is a
seventh copy. M10 fixes the structure so that M12 (design language) is a token-value swap.

The choices that shape everything else — how styles are scoped, how the app is navigated, and
whether the map ever leaves the screen — are decisions, not implementation details, and belong
here so they are not re-litigated per PR.

## Decision

1. **One shell; the map is always present.** `<swath-shell>` owns rail, top bar, main (the map,
   slotted), inspector, HUD dock and status bar. The rail switches *mode* — Layers · Data ·
   Author · X-ray/Analytics; analytics is a mode over the same trace stream, not a page. The
   separate-pages model (workbench / catalog / graph lab / dashboard) is rejected: the product's
   claim is a single pane of glass (REQUIREMENTS R2), and the glass-box demo needs the map under
   every mode.
2. **Primitives are shadow-DOM custom elements on constructable stylesheets**, themed through CSS
   custom properties (`--swath-<category>-<role>`) defined once in `web/src/ui/tokens.css` and
   `::part()` escape hatches. `<swath-map>` alone stays light DOM (MapLibre needs document scope)
   and is slotted into the shell. `SwathElement` (`web/src/ui/element.ts`) is ADR 0005's reactive
   layer: attribute↔property reflection, batched `requestUpdate`, stylesheet adoption, composed
   `emit()` — and nothing more (no templating, store, DI, directives).
3. **Modes live in the URL as query params** (`view=`, `sel=`; `rail=` read-only), written with
   `replaceState` only, exactly as `view-state.ts` writes the view. ADR 0011 forbids SPA path
   rewrites (unknown paths stay 404); this decision keeps the one-shareable-URL property and
   accepts no back-button between modes.
4. **No new runtime dependency and no external host**: no Tailwind, no icon library, no web fonts
   from a CDN — the UI ships inside the binary (ADR 0011). Icons are one inline SVG symbol sheet.
5. **The DRY gate is part of `just check`**: raw colour/font literals, style injection, element
   definition, `new CustomEvent` and `fetch` each have exactly one home in the tree.

### Rejected alternatives

- **Lit (or any micro-framework).** Solves reflection/templating well, but ADR 0005's budget is
  one runtime dependency and its reasoning (framework churn) has not changed; the ~150-line base
  covers what the six elements actually need.
- **Light DOM + cascade layers + tag-scoped selectors.** Simplest a11y and MapLibre interop, but
  isolation is discipline, not mechanism — the very failure mode this ADR exists to end.
- **Path-based routing** (`/author`, `/catalog`). Contradicts ADR 0011's no-rewrite rule and would
  make bookmarked deep links depend on the server's fallback.
- **Separate pages per surface.** See decision 1.

## Consequences

- M12 is mechanically a value swap: its PRs may touch only `tokens.css`, `base.css`, `icons.svg`,
  `theme-*.css` and screenshots (a CI path check enforces it); token *names*, icon *names*, parts,
  slots, attributes, events and the region layout are frozen in the design note's §8.
- Every event must be `composed`; `emit()` owns that and a test pins it.
- Playwright CSS locators pierce shadow roots; `page.evaluate(document.querySelector…)` does not —
  the handful of such sites move to `locator.evaluate`. The shell re-pins the viewport once so the
  map canvas keeps its 1280-wide geometry and the screenshot gate's `RAIL_WIDTH = 248` holds.
- `aria-labelledby` cannot cross shadow roots; primitives take `label` strings and composites use
  `ElementInternals` for host semantics.
- A bare `<swath-map>` without the shell keeps working (in-map fallbacks), so the element stays
  usable as a component, not only as the app.

### M11 — earning the DAG

The authoring surface becomes a constrained DAG editor in M11. Its server-side prerequisite — a
typed two-cube join, since today's process compiler admits only a single-cube chain — is a
separate decision with its own design note (`docs/design/authoring-dag.md`) and ADR, filed as
issue #294. This ADR only fixes that the editor is built on M10's canvas primitives and author
mode, so the swap is a content change, not a layout change.

## Reopen / supersede conditions

- A second consumer of the primitives outside this repository (ADR 0007's deferred
  `custom-elements.json` manifest) would justify publishing `web/src/ui/` as a package.
- A real need for back-button navigation between modes reopens decision 3 (`pushState`), as would
  a change to ADR 0011's no-rewrite rule.
- A primitive that cannot be built without a framework feature (e.g. scoped custom element
  registries landing in every browser) is a reason to revisit the base class, not to add a
  dependency.
