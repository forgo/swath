# docs/media — technical diagrams

Four diagrams (docs audit ranks 2–5, issue #113). Conventions:

- **Editable source = the committed file.** Three diagrams are Mermaid fenced blocks in
  Markdown — GitHub renders them natively, so the block is both the editable source and the
  export, with no generated artifact to drift. The virtual-reference diagram is a hand-crafted
  SVG (plan decision #3): SVG is a text format, so the same file is source and export.
- **Every figure is traceable.** Each diagram has a `*.notes.md` sidecar listing the committed
  artifact (bench file, load artifact, trace capture, test, prototype record) backing every
  number and every solid/dashed choice. No hand-typed figures.
- **Theme legibility.** Mermaid picks up GitHub's light/dark theme automatically (no hard-coded
  fills). The SVG paints its own opaque light background so it reads identically on both themes.

| Diagram | Source | Sidecar |
|---|---|---|
| Ingest-to-pixel flow, measured stage timings | [`ingest-to-pixel-flow.md`](ingest-to-pixel-flow.md) | [`ingest-to-pixel-flow.notes.md`](ingest-to-pixel-flow.notes.md) |
| Planner decision loop, real `plan.considered` | [`planner-decision-loop.md`](planner-decision-loop.md) | [`planner-decision-loop.notes.md`](planner-decision-loop.notes.md) |
| Legacy virtual-reference mechanism | [`virtual-reference.md`](virtual-reference.md) → [`virtual-reference.svg`](virtual-reference.svg) | [`virtual-reference.notes.md`](virtual-reference.notes.md) |
| Standards surfaces map | [`standards-map.md`](standards-map.md) | [`standards-map.notes.md`](standards-map.notes.md) |

`planner-trace.capture.json` is the committed capture (verbatim `GET /traces` SSE frames from
the fixture stack) that the planner diagram's embedded payload is copied from.
