# docs/media — technical diagrams

Six diagram families: the product loop, the four of the docs audit (issue #113), and the wedge
pair behind `docs/COMPARISON.md`. Conventions:

- **Hand-crafted SVG, canonical.** Every diagram is a hand-crafted SVG (plan decision #3 set
  the style with the virtual-reference figure; maintainer review extended it to the rest).
  SVG is a text format, so each committed `.svg` is simultaneously the editable source and the
  export — there is no generated artifact to drift. Each has a Markdown page embedding it with
  a full-prose alt text.
- **Every figure is traceable.** Each diagram has a `*.notes.md` sidecar listing the committed
  artifact (bench file, load artifact, trace capture, test, prototype record) backing every
  number and every solid/dashed choice. No hand-typed figures.
- **Reader-facing words.** Milestone names (`M<n>`, "Phase-<n> exit") and ADR numbers belong in
  `docs/ROADMAP.md`, `docs/decisions/`, and PR/issue text. A diagram's labels, a screenshot's
  caption, and a document's first screen say what exists, in the reader's vocabulary; an ADR
  is cited as provenance in the sidecar or later in the body, never as the headline.
- **Theme legibility.** Every SVG paints its own opaque light background and carries an inline
  SPDX header, so it reads identically on GitHub light and dark themes (and is REUSE-clean
  both by header and by the `docs/**` aggregate annotation).

| Diagram | Page | SVG (canonical source) | Sidecar |
|---|---|---|---|
| The product loop (README hero) | [`product-loop.md`](product-loop.md) | [`product-loop.svg`](product-loop.svg) | [`product-loop.notes.md`](product-loop.notes.md) |
| Ingest-to-pixel flow, measured stage timings | [`ingest-to-pixel-flow.md`](ingest-to-pixel-flow.md) | [`ingest-to-pixel-flow.svg`](ingest-to-pixel-flow.svg) | [`ingest-to-pixel-flow.notes.md`](ingest-to-pixel-flow.notes.md) |
| Planner decision loop, real `plan.considered` | [`planner-decision-loop.md`](planner-decision-loop.md) | [`planner-decision-loop.svg`](planner-decision-loop.svg) | [`planner-decision-loop.notes.md`](planner-decision-loop.notes.md) |
| Legacy virtual-reference mechanism | [`virtual-reference.md`](virtual-reference.md) | [`virtual-reference.svg`](virtual-reference.svg) | [`virtual-reference.notes.md`](virtual-reference.notes.md) |
| Standards surfaces map | [`standards-map.md`](standards-map.md) | [`standards-map.svg`](standards-map.svg) | [`standards-map.notes.md`](standards-map.notes.md) |
| The wedge: capability ladders and the single-system frontier (embedded in `COMPARISON.md`) | — | [`wedge-a-quadrants.svg`](wedge-a-quadrants.svg), [`wedge-b-frontier.svg`](wedge-b-frontier.svg) | [`wedge.notes.md`](wedge.notes.md) |

`planner-trace.capture.json` is the committed capture (verbatim `GET /traces` SSE frames from
the fixture stack) that the planner diagram's embedded payload is copied from.
`qgis-xyz-connection.png` is the QGIS evidence capture for `docs/RECIPES.md` (issue #194) —
the one raster here, taken by hand because it is a third-party window.

## Screenshots

UI screenshots live in [`screenshots/`](screenshots/index.md) and are captured exclusively by
`just screenshots` (issue #112): the fixture compose stack, a pinned viewport/DPR,
deterministic filenames, and a second capture run that must reproduce every shot within a
perceptual-diff policy before the recipe passes. The index carries one-line captions and the
capture git sha; `shots.json` carries per-shot sha256 + diff policy. Never hand-edit or
hand-replace a shot — re-run the recipe.
