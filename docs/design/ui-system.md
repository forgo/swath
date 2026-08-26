# The Swath UI system — one shell, tokens, shadow-DOM primitives

_Design note for issue #277 (M10 — UX product structure and surfacing value), the contract every
M10 PR is reviewed against and the freeze M12 (design language) inherits. Companion code studied:
`web/src/*.ts` (the M5–M9 elements), `web/demo/index.html` + `main.ts` (the shell), the Playwright
and screenshot harnesses. Decisions recorded in ADR 0021. August 2026._

## 1. Where the M5–M9 UI landed, and where it ceilings

The current web surface does the right things for its era: six vanilla custom elements, MapLibre
as the only runtime dependency (ADR 0005), the URL as the share link (`view-state.ts`), lazy
panels that fetch nothing until opened, every pixel through the engine (ADR 0019), and a
screenshot suite that proves what the docs claim. Its shape, though, is a **rail of disclosures**
beside a map, and its construction is **six copies of the same scaffolding**:

- Zero CSS custom properties. Colours, font stacks and sizes are literals — ~330 colour literals
  and 67 font-stack literals across six CSS template strings and the demo page.
- `injectStyles` + `STYLE_ELEMENT_ID` + `defineSwathX()` copy-pasted per element; imperative
  `replaceChildren` re-rendering; bare `fetch()` at ~15 sites with two ad-hoc `fetchImpl` test
  seams; no router; no icons beyond CSS `content: "▸"`; the x-ray readouts, trace feed and time
  slider each hand-positioned inside the map.
- ADR 0005 promised "a tiny in-house reactive layer". It was never built; each element re-derives
  reactivity from `observedAttributes` by hand.

The ceiling is structural, not cosmetic: any retheme today is a six-file search-and-replace, and
any new surface (catalog, analytics, a DAG editor) would be a seventh copy. This note fixes the
structure so the design-language milestone becomes a **token-value swap**.

## 2. What was taken from the Stitch exploration, and what was fenced out

The maintainer explored Google Stitch for layout ideas. Its output is Tailwind-CDN + Google-Fonts
HTML — none of it is reusable (both are dependencies and external hosts this project rejects) —
but several structural ideas are.

| Adopted (structure) | Fenced out (recorded so it is not re-litigated per PR) |
| --- | --- |
| Top bar: brand · mode navigation · command palette (⌘K) · x-ray toggle | Four separate pages (Workbench / Graph Lab / Catalog / Dashboard) — the map is the product and never leaves |
| Collapsible rail (248 px ↔ 56 px icon rail) | "Execute Pipeline" / batch buttons — no jobs in the bounded profile (ADR 0010) |
| HUD cards docked to map corners and edges, one visual vocabulary | Fabricated telemetry: GPU %, `$/op`, worker fleets by region, uptime, "compiling shader" |
| Status bar: lat/lon · zoom · CRS · ingest→pixel | User avatar / sign-in — auth is a later era (ROADMAP §3) |
| Layer row: eye · name · meta · opacity · kebab · active accent | Cloud-cover filter, sensor type, Float32/Int16 output picker — not in the catalog domain |
| Catalog: filter rail + granule cards + grid/list + count/sort | A free-form node graph with nothing to join — the DAG is earned server-side in M11 (ADR 0021 §"M11") |
| Master/detail authoring: canvas + properties inspector + per-node log + live preview | Glassmorphism, neon palette, JetBrains Mono — M12 concerns; only the token *taxonomy* shape is used now |
| Dashboard of real telemetry (p50/p95, decision mix, cache hit rate, ingest→pixel, recent ingests, trace log) | Anything that would need a second render path or a dependency (ADR 0005, 0019) |

## 3. The layered system

Four layers, each allowed to depend only downward. `web/src/ui/` (L0–L2) never imports from
`../`; organisms import primitives and pure models; the shell imports organisms. The rule is
enforced by the DRY gate (§9).

| Layer | What | Where |
| --- | --- | --- |
| L0 Foundations | tokens, base sheet, `SwathElement`, events, API client, app state, icons, breakpoints | `web/src/ui/` (+ `api.ts`, `app-state.ts` beside `view-state.ts`) |
| L1 Atoms | icon, button, toggle, slider, field, pill/badge, disclosure, log | `web/src/ui/swath-*.ts` |
| L2 Molecules | card, menu, drawer, HUD dock + card, status bar, rail, command palette, canvas primitives | `web/src/ui/`, `web/src/canvas/` |
| L3 Organisms | layer list, catalog, add-data, author mode + inspector, x-ray chrome, time slider, compare | `web/src/swath-*.ts` (flat, as today) |
| L4 Shell | `<swath-shell>`: regions, responsive reflow, live region, deep links | `web/src/swath-shell.ts`; `web/demo/index.html` reduces to it |

## 4. Foundations (L0)

### 4.1 Tokens — `web/src/ui/tokens.css`

The **only** file in `web/` allowed to contain raw colour, font-stack or size values. Declared on
`:root` as custom properties and adopted once per document (`adoptedStyleSheets`, guarded by a
`WeakSet<Document>`); custom properties inherit through shadow boundaries, so every shadow sheet
writes `var(--swath-…)` and nothing else.

Naming: `--swath-<category>-<role>[-<state>]`. Roles are **semantic**, never palette names.

| Category | Roles (M10 names; M12 changes values, never names) |
| --- | --- |
| `color` | `bg`, `bg-raised`, `bg-hud`, `fg`, `fg-muted`, `line`, `accent`, `accent-bg`, `accent-border`, `warn`, `danger`, `info`, `udf`, `compare`, `decision-live`, `decision-overview`, `decision-cache`, `focus` |
| `font` | `ui` (prose), `mono` (ids, telemetry, coordinates) |
| `text` / `leading` / `tracking` | `xs`, `sm`, `md`, `lg`; `tight`, `normal`; `wide` (uppercase headers) |
| `space` | `0`, `1`…`8` on a 4 px grid |
| `radius` | `sm`, `md`, `pill` |
| `border` / `shadow` | `hairline`; `hud` (flat by default — depth is an M12 decision) |
| `size` | `rail: 248px`, `rail-icon: 56px`, `inspector: 320px`, `topbar: 44px`, `statusbar: 24px`, `target: 44px`, `icon: 16px` |
| `z` | `overlay: 1`, `controls: 2`, `hud: 3`, `drawer: 4`, `palette: 5` |
| `motion` | `fast`, `normal`, `ease` (zeroed under `prefers-reduced-motion`) |

Two places cannot read custom properties and get explicit bridges: MapLibre paint properties
(`readToken(name)` — a `getComputedStyle(documentElement)` lookup at paint time, used by
`granule-footprints.ts`), and `@media` queries (breakpoints live in `breakpoints.ts` as TS
constants — 640 / 1024 / 1280 — mirrored in a comment at the top of `tokens.css`; the shell uses
container queries where a value must be shared with CSS).

`base.css` is the second shared sheet, adopted into every shadow root: box-sizing, `[hidden]`,
the focus ring (`:focus-visible { outline: var(--swath-border-focus) }` — no element writes its
own outline), reduced-motion zeroing.

### 4.2 `SwathElement` — `web/src/ui/element.ts`

ADR 0005's reactive layer, finally built, and kept deliberately small (≤ ~150 lines):

```ts
export abstract class SwathElement extends HTMLElement {
  static tagName: string;                              // required by define()
  static styles: readonly CSSStyleSheet[] = [];        // subclasses: static override styles = [css`…`]
  static properties: Record<string, PropSpec> = {};    // {type, attribute?, reflect?}; observedAttributes derived
  static shadowOptions: ShadowRootInit = { mode: "open" };
  static define(): void;                               // idempotent customElements.define
  protected readonly renderRoot: ShadowRoot;           // adoptedStyleSheets = [base, ...styles]
  protected readonly disconnected: AbortSignal;        // listeners added with {signal} die on disconnect
  requestUpdate(): void;                               // microtask-batched; coalesces attribute storms
  readonly updateComplete: Promise<void>;              // tests: await el.updateComplete
  protected abstract render(): void;                   // imperative DOM into renderRoot
  protected emit<K extends keyof SwathEventMap>(type: K, detail: SwathEventMap[K]): boolean;
}
```

Properties are a static table because `tsconfig` has `erasableSyntaxOnly` (no decorators); the
base installs accessors at `define()` time (coerce per `type`, reflect if asked, `requestUpdate`).
Form participation is opt-in: `static formAssociated = true` + `attachInternals()` in the subclass.

**What it deliberately does not do:** templating or diffing (rendering stays imperative, with a
one-function `el()` helper in `dom.ts`), state store, dependency injection, directives, SSR,
i18n, a theming API beyond tokens + `::part`. If it needs more, it is doing too much.

### 4.3 Events — `web/src/ui/events.ts`

One `SwathEventMap` interface plus a `declare global { interface HTMLElementEventMap }`
augmentation, so listeners are typed without casts. Names are `swath-<subject>-<verb>` with verbs
from `select | change | toggle | open | close | move | drop | connect | action | activate |
request`. `emit()` hardcodes `bubbles: true, composed: true` — an event that is not composed dies
at the first shadow boundary, so a vitest pins it. The existing `layerchange` becomes
`swath-layer-change`; the map dispatches both for one milestone.

### 4.4 API client — `web/src/api.ts`

`SwathApi { base, url(), json(), blob(), capabilities(), events() }` — one injectable seam
replacing every bare `fetch()` and both `fetchImpl` test seams. `capabilities()` caches the
`GET /` promise and invalidates it on failure (the add-data panel's re-open-retries rule, kept).
`ApiProblem` parses RFC 7807 bodies (and degrades honestly on non-JSON); `fieldFor()` generalises
`add-data-model.ts`'s `mapProblem` so every organism maps server diagnostics onto fields the same
way. The basemap style cache stays outside (a foreign URL) and is allow-listed in the gate.

### 4.5 App state and routing — `web/src/app-state.ts`

A pure module beside `view-state.ts`, same shape (parse / format / equal / resolve / save / load),
storage key `swath.app-state.v1`. The contract extends view-state's and never contradicts it:

- `view=` is the mode (`layers | data | author | xray`). Absent = `layers`; never written when
  default, so a bare `/` stays bare. Unknown values degrade to `layers`.
- `sel=<node-id>` is meaningful only under `view=author`; parsed otherwise → dropped.
- `rail=collapsed` is honoured when present in a URL (a deep link is never rewritten) but is
  **never written** by interaction — collapse is a device preference, persisted in storage only.
- `inspector` is derived (`author` and a selection), never serialised.
- The overlay flag `xray` (view-state's) and the analytics mode `view=xray` are distinct: entering
  the mode turns the overlay on (a user act, so `xray` is written); leaving leaves it alone.
- Writes compose `withViewState` then `withAppState` on one search string through the same
  interaction gate and **`history.replaceState`** — no `pushState`, no path rewrite (ADR 0011:
  unknown paths stay 404). No back-button between modes; that is the same trade view-state made.
- `stac=` and `basemap=` remain pass-through page config.

### 4.6 Icons — `web/src/ui/icons.svg` + `<swath-icon>`

One inline SVG symbol sheet, imported `?raw`. `<use href="#id">` cannot cross a shadow boundary,
so `<swath-icon name>` parses the sheet once (`DOMParser`), caches symbols, and clones the chosen
one into its own shadow root with `fill`/`stroke: currentColor` and `--swath-size-icon`. `label`
→ `role="img"` + `aria-label`; otherwise `aria-hidden`. A vitest pins the name list against the
sheet: M12 may redraw every glyph; it may not rename one.

## 5. Primitives (L1/L2) — only what M10 organisms consume

Each primitive is specified by its tag, attributes, slots, parts, events, keyboard and touch
behaviour; the table is the contract, the issues carry the ACs.

| Tag | Attributes | Slots / parts | Events | Keyboard · touch | First consumer |
| --- | --- | --- | --- | --- | --- |
| `swath-icon` | `name`, `label?`, `size?` | `svg` | — | — | everything |
| `swath-button` | `variant` (ghost/solid/accent/danger), `size`, `icon?`, `label`, `pressed?`, `disabled`, `href?` | default, `icon` / `base label icon` | `click`; `swath-toggle` when `pressed` used | Space/Enter · ≥44 px on coarse pointers | Share, rail |
| `swath-toggle` | `checked`, `label`, `disabled`, `name` (form-associated) | `label` / `base control track thumb` | `swath-change` | Space · tap, 44 px | layer eye |
| `swath-slider` | `value/min/max/step`, `label`, `format?` | — / `base control value` | `swath-input` (live), `swath-change` (commit) | arrows/Home/End · native range, `touch-action: pan-y` | opacity, compare |
| `swath-menu` | `open`, `label`, `items` prop | `trigger` / `base trigger list item` | `swath-menu-select`, `swath-drawer-close` | ↑↓ Enter Esc, typeahead · bottom sheet on narrow | layer kebab, canvas context |
| `swath-field` | `label`, `help?`, `error?`, `type`, `name`, `value`, `options` (form-associated) | default, `help` / `base label control help error` | `swath-input`, `swath-change` | native | add-data, inspector |
| `swath-card` | `title?`, `dense`, `selected`, `interactive` | default, `header`, `media`, `footer` / same | `swath-activate` | Enter/Space · tap, long-press | granule cards |
| `swath-drawer` | `edge` (right/bottom), `open`, `size`, `resizable`, `modal`, `snap` | default, `header`, `footer` / `base header body handle scrim` | `swath-drawer-close`, `swath-change` | Esc; focus trap when modal · handle drag, snap points, swipe-down | inspector, author dock, narrow rail |
| `swath-hud-dock` | `collapsed` | `top-left` … `bottom-right` (8) / `base corner` | — | — · horizontal strip on narrow | x-ray chrome, landing, slider |
| `swath-hud-card` | `title?`, `collapsible`, `collapsed`, `dense` | default, `actions` / `base header body` | `swath-toggle` | Enter on header · tap | readouts, feed, analytics |
| `swath-status-bar` + `-cell` | `label`, `value`, `mono` | default / `base label value` | — | — · one chip in the dock on narrow | shell |
| `swath-rail` | `collapsed`, `mode`, `items` prop | default, `brand`, `footer` / `base nav item content` | `swath-mode-change` | ↑↓ Enter · bottom tab bar on narrow | shell |
| `swath-command-palette` | `open`, `commands` prop | — / `base input list item hint` | `swath-command` | ⌘K/Ctrl-K, ↑↓ Enter Esc · sheet on narrow | shell |
| `swath-layer-item` | `layer-id`, `title`, `active`, `visible`, `opacity`, `kind` | — / `base row eye opacity menu` | `swath-layer-select/-visibility/-opacity/-action` | own tab stops per control · tap row, expand slider | layer list |

Canvas primitives (`web/src/canvas/`, interaction only — graph semantics belong to M11's
design note): `swath-canvas` (viewport `x/y/k`, grid, `edges` prop rendered in an SVG layer with
12 px hit twins, marquee, `fit()`, `toCanvas()`, `portAnchor()`), `swath-canvas-node` (drag with
an 8 px threshold, arrow nudge, activate, delete-request), `swath-canvas-port` (drag-to-connect
and a tap-to-connect state machine: first tap arms, second completes, Esc cancels; 44 px hit on
coarse pointers), and pure `canvas-geometry.ts` (anchors, cubic paths, hit test). Touch: one
finger pans, two-finger pinch zooms around the midpoint via pointer-event pairs (no gesture
events), long-press (500 ms, < 8 px drift) opens context, `touch-action: none`.

Frozen part vocabulary: `base label icon control track thumb header body footer item handle`.
Composites forward inner parts with `exportparts`.

## 6. Organisms (L3) and the shell (L4)

Organisms are rebuilt on primitives, keeping their pure models (`view-state`, `authoring-model`,
`add-data-model`, `xray-analytics`, `tms`) and their public events. The catalog's thumbnails are
`POST /result` previews (ADR 0014's bounded tile) rendered to `<img>` — through the engine, never a
client decode (ADR 0019); a refused preview explains itself in plain words.

`<swath-shell>` owns five regions; the map is **slotted, light DOM**, so MapLibre keeps document
scope and every `document.querySelector("swath-map")` in the e2e suite keeps working:

```
┌ rail (248 / 56 / tab bar) ┬ top bar (44; spans right of the rail) ─────────────────────┐
│ brand · Share             │ mode title · search (⌘K) · x-ray toggle                    │
│ mode switcher (4)         ├ main ───────────────────────────────────┬ inspector (320) ─┤
│ mode content              │  <slot name="map">  — swath-map          │ author only      │
│  layers|data|author|xray  │  swath-hud-dock (overlay, 8 slots)       │ (drawer, right)  │
│ footer                    │  [author: drawer, bottom — the canvas]   │                  │
│                           ├ status bar (24) ────────────────────────┴──────────────────┤
└───────────────────────────┴ lat/lon · zoom · CRS · ingest→pixel ──────────────────────┘
```

The top bar spans only right of the rail so `tests/screenshots/verify_stable.py`'s
`RAIL_WIDTH = 248` and its "content right of the rail" gate stay valid; the shell issue (#284)
is the single viewport re-pin (1528 × 788 e2e, 1528 × 928 screenshots) that keeps the canvas at
1280 × 720 / 860.

HUD dock assignment: `top-center` landing card; `top-right` x-ray / compare / zoom-to-data;
`bottom-center` time slider; `bottom-left` readouts (ingest, analytics, legend); `bottom-right`
trace feed; `right` the why-view inspector. Badges (positioned in map pixels) stay inside
`swath-map`; only chrome moves. The dock is `pointer-events: none`, cards `auto`; the dock sits
outside the map element so it never joins the map's internal stacking order.

Responsive reflow (container queries on the shell; constants in `breakpoints.ts`):

| Width | Rail | Mode content / inspector | HUD / status |
| --- | --- | --- | --- |
| ≥ 1280 | expanded (unless collapsed by preference) | in the rail / right column | as drawn |
| 1024–1279 | icon rail | right drawers, stacked | as drawn |
| 640–1023 | icon rail | modal drawers (scrim, focus trap) | cards default collapsed |
| < 640 | bottom tab bar (+ safe-area inset) | bottom sheets, snap 40 / 90 %; map `inert` under 90 % | strip above the tab bar; status → one chip |

Touch parity everywhere, the DAG editor included: `(pointer: coarse)` bumps every hit box to
`--swath-size-target`; long-press opens the same menus as kebab/right-click; sheets drag by
handle and swipe down to close.

## 7. Accessibility rules

- Primitives that wrap a native control render the real `<button>` / `<input>` inside the shadow
  root (roles and keyboard come free) with `delegatesFocus: true` on the host.
- `aria-labelledby` cannot cross shadow roots: primitives take a `label` string and apply
  `aria-label` internally; composites set host semantics through `ElementInternals`
  (`role`, `ariaLabel`).
- One `role="status"` live region per shell, fed by `announce(text)`, replaces ad-hoc status nodes.
- Roving `tabindex` in lists and on the canvas; Esc closes the topmost drawer / menu / palette;
  the map region is `inert` while a full-height sheet is open.
- `prefers-reduced-motion` zeroes the motion tokens; the landing loop already honours it.

## 8. What M12 may touch — the freeze

M12 changes **values, glyphs and themes**, not structure: `web/src/ui/tokens.css`, `base.css`,
`icons.svg`, an optional `theme-*.css` (light / high-contrast under the same token names) adopted
after tokens, and a one-time regeneration of `docs/media/screenshots/`. Frozen for M12: every
token *name* (`tokens.test.ts`), every icon *name*, the part vocabulary, slot names, attributes
and events in §5, and the region layout in §6. A CI path check on M12 PRs makes it mechanical: a
diff outside those files fails.

## 9. The DRY gate — `web/scripts/check-ui-dry.mjs`

Runs in `pnpm run lint`, therefore in `just lint-web` and `just check`. Forbids, outside their
one home: hex/`rgb(`/`hsl(` and font-stack literals (home: `tokens.css`); `injectStyles`,
`STYLE_ELEMENT_ID`, `customElements.define(` (home: `ui/element.ts`; the MapLibre stylesheet in
`swath-map.ts` allow-listed with a reason); `new CustomEvent(` (home: `ui/events.ts`); `fetch(`
(home: `api.ts`; the basemap cache allow-listed); `from "../` inside `web/src/ui/`. Advisory
when introduced (#278); blocking per file as each organism migrates; the allow-list is empty by
the author-mode issue (#291), and a stale allow-list entry fails the gate so the escape hatch can
only shrink.

## 10. Honest risks

- **Shadow DOM vs the harnesses.** Playwright CSS locators pierce shadow roots;
  `page.evaluate(document.querySelector…)` does not. Five sites in `web/screenshots/capture.ts`
  and a similar number in `swath-xray.e2e.ts` move to `locator.evaluate` in the x-ray issue.
- **Baseline churn is real and budgeted.** Nine of the seventeen M10 PRs change visible chrome and
  must commit `just screenshots`; the geometry re-pin happens exactly once (#284).
- **`composed` is easy to forget** and fails silently across the shell — `emit()` owns it and a
  test pins it.
- **The client re-encodes layout semantics** (rail width, canvas size) that the screenshot gate
  also pins; the shell reads them from tokens so there is one number, not two.
- **Standalone `<swath-map>`** must keep working without the shell (in-map fallbacks for the
  landing card and time slider); a vitest mounts it bare.
- **Cinematic hover-pause** listens on the map host; dock cards sit outside it, so reaching for a
  card pauses the loop — documented, revisitable.
- **Bundle growth**: no new dependencies; base + primitives target < 30 kB minified, reported per PR.

## 11. Maintainer: contract accepted

_Check to freeze the token names (§4.1), the part vocabulary and primitive table (§5), and the
region layout (§6) for M10–M12; implementation issues #278–#293 are filed against this note._

- [ ] **Accepted as written**
- [ ] **Accepted with amendments** (describe in a comment on #277)
