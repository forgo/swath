# docs/media — technical diagrams

Four diagrams (docs audit ranks 2–5, issue #113). Conventions:

- **Hand-crafted SVG, canonical.** All four diagrams are hand-crafted SVGs (plan decision #3
  set the style with the virtual-reference figure; maintainer review extended it to all four).
  SVG is a text format, so each committed `.svg` is simultaneously the editable source and the
  export — there is no generated artifact to drift. Each has a Markdown page embedding it with
  a full-prose alt text.
- **Every figure is traceable.** Each diagram has a `*.notes.md` sidecar listing the committed
  artifact (bench file, load artifact, trace capture, test, prototype record) backing every
  number and every solid/dashed choice. No hand-typed figures.
- **Theme legibility.** Every SVG paints its own opaque light background and carries an inline
  SPDX header, so it reads identically on GitHub light and dark themes (and is REUSE-clean
  both by header and by the `docs/**` aggregate annotation).

| Diagram | Page | SVG (canonical source) | Sidecar |
|---|---|---|---|
| Ingest-to-pixel flow, measured stage timings | [`ingest-to-pixel-flow.md`](ingest-to-pixel-flow.md) | [`ingest-to-pixel-flow.svg`](ingest-to-pixel-flow.svg) | [`ingest-to-pixel-flow.notes.md`](ingest-to-pixel-flow.notes.md) |
| Planner decision loop, real `plan.considered` | [`planner-decision-loop.md`](planner-decision-loop.md) | [`planner-decision-loop.svg`](planner-decision-loop.svg) | [`planner-decision-loop.notes.md`](planner-decision-loop.notes.md) |
| Legacy virtual-reference mechanism | [`virtual-reference.md`](virtual-reference.md) | [`virtual-reference.svg`](virtual-reference.svg) | [`virtual-reference.notes.md`](virtual-reference.notes.md) |
| Standards surfaces map | [`standards-map.md`](standards-map.md) | [`standards-map.svg`](standards-map.svg) | [`standards-map.notes.md`](standards-map.notes.md) |

`planner-trace.capture.json` is the committed capture (verbatim `GET /traces` SSE frames from
the fixture stack) that the planner diagram's embedded payload is copied from.

## Screenshots

UI screenshots live in [`screenshots/`](screenshots/index.md) and are captured exclusively by
`just screenshots` (issue #112): the fixture compose stack, a pinned viewport/DPR,
deterministic filenames, and a second capture run that must reproduce every shot within a
perceptual-diff policy before the recipe passes. The index carries one-line captions and the
capture git sha; `shots.json` carries per-shot sha256 + diff policy. Never hand-edit or
hand-replace a shot — re-run the recipe.
