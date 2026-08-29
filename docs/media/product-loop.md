# The product loop

Hand-crafted SVG in the house style ([`virtual-reference.md`](virtual-reference.md) set it).
The file [`product-loop.svg`](product-loop.svg) is both the editable source and the export;
every box on it is traceable via [`product-loop.notes.md`](product-loop.notes.md). It is the
README's hero: what Swath does, for someone who has never read a milestone.

![The product loop. A forward row: granules land (COG, or archival HDF5 served in place, on
S3 or disk) → the catalog fills itself in, keeping every acquisition as a time series → live
tiles over OGC API - Tiles and XYZ with datetime frames, a planner choosing cache, overview,
or live render per tile within budget → one pane of glass with layers, a time slider, a
compare swipe, a share link, QGIS over plain XYZ, phone-ready. A green return loop: from that
screen, pick bands and compose an openEO graph over the live layers — an index, a formula, a
date-vs-date change, or your own sandboxed code — publish it with one request, and the
published layer is served the same way, back into the live tiles. Below, a dashed glass box:
every tile emits a trace (the decision, the granules read, byte ranges, timings), the x-ray
in the viewer shows it (badges, why-view, bytes heatmap, trace feed, live analytics), and the
tests assert on the same data.](product-loop.svg)

The words are the product's: **immediacy** (arrive → catalog → serve with no per-scene
work), **one pane of glass**, **derive and publish**, and the **glass box** — the four
requirements the platform is measured against in [`../REQUIREMENTS.md`](../REQUIREMENTS.md)
(R1, R2, R3, R4). Nothing here is a plan; each box names shipped code and its test in the
sidecar.
