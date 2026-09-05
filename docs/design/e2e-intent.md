<!-- SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
     SPDX-License-Identifier: Apache-2.0 -->

# The e2e suite, by intent

_What every browser spec is protecting — the **behaviour**, not the markup. Written for #426, the
debt record of the refactor-era CI lane: a rewrite that drops one of these without saying so is the
failure mode that issue exists to prevent._

This list is closed in both directions and `e2e_intent.rs` enforces it: every `test("…")` in
`web/e2e/` has a row here, and every row names a spec that exists. Renaming a spec without moving
its row fails the gate; deleting one fails it too. The gate cannot tell you the row is still
*true* — that is review's job — but it can guarantee nobody drops a protection silently.

**Nothing was lost in the refactor.** Every spec title that existed when the fast PR lane landed
(767e83c^, 63 of them) still exists verbatim; the suite grew to 81. That is checkable with
`git show 767e83c^:web/e2e/…` and was checked when this list was written.

## Reading a row

The intent is the sentence you would use to argue the spec should exist at all. If a redesign makes
a spec's *steps* wrong, the intent is what the replacement has to keep true.

## `swath-map` — the engine reaches the screen

| Spec | Protects |
|---|---|
| map loads, fetches real tiles, renders pixels, and switches layers | The whole stack in one assertion: a real tile request answers `200 image/png` and the canvas shows non-blank pixels. If only one spec survived, this is the one. |

## `landing` — the paramless first minute

| Spec | Protects |
|---|---|
| paramless / is the cinematic landing: the fire season loops, URL untouched | A first visit shows motion without a click, and does not dirty the URL before anyone has navigated. |
| hover pauses the landing loop; a scrub takes it over and the URL follows | The loop yields to the person the moment they touch it, and their frame becomes shareable. |
| the invitation turns x-ray on, keeps the loop going, and joins the link | The one-click invitation is additive: it never stops what you were watching. |
| the landing holds on its latest frame with a play affordance | `prefers-reduced-motion` is honoured: no loop, and a way to start it deliberately. |
| Share copies the explicit landing link; that link is a still, byte-stable view | The share control produces a link that reproduces what is on screen, and does not drift. |
| after an interaction, Share and the address bar agree byte-for-byte | One URL, one truth — the share button never offers a different view than the one you are in. |
| selecting a layer updates the URL; the URL alone reproduces the view | The URL is the state, not a decoration on it. |
| URL params beat storage, and the deep link stays byte-stable | A pasted link wins over what this browser remembers, and does not get rewritten under the reader. |
| localStorage restores the last layer and viewport on a paramless visit | A return visit resumes where you were, but only when the URL says nothing. |
| the x-ray toggle joins the share link from the entry page | X-ray is part of the view, so it travels with it. |

## `modes` — the shell, its modes, and history

| Spec | Protects |
|---|---|
| /?view=data lands in Data mode with the URL untouched | A mode is addressable, and honouring a link does not rewrite it. |
| a mode switch writes view=; back to layers removes it; storage remembers | The default mode is absent from the URL rather than spelled out. |
| a bare visit restores the last mode from storage; the x-ray mode turns the overlay on | Entering x-ray is a user act that turns the overlay on; leaving leaves it. |
| rail collapse is a device preference: storage remembers it, a rail=collapsed link is honoured without a rewrite | A device preference is not an artifact: read from a link, never written back. |
| the back button walks artifacts (#392) | ADR 0027: artifacts push history, so `back` returns to what you were looking at. |
| panning replaces rather than pushes: the camera adds no history (#392) | The other half of ADR 0027 — forty pans must not bury the view you came from. |
| the chip row is the URL made visible; removing a chip drops its param (#393) | The chips *are* the URL: removing one is a state change through the same write path. |
| the icon strip reaches every mode, and new layer stands in the footer (#398) | Every mode is reachable from the collapsed rail; the standing action never scrolls away. |
| composing puts the map in a preview column, painted, beside the canvas (#400/#463) | ADR 0028: the map is never smaller than a live preview, and it really paints. |
| the map's toggles keep a surface of their own over bright imagery (#472) | HUD controls stay legible over any basemap. |

## `layer-list` — the rail's layer rows

| Spec | Protects |
|---|---|
| switching layers through the list updates the URL and the pressed row | One selection model: the list and the URL never disagree. |
| the eye hides the viewed raster and shows it again; the URL never learns | Visibility is a local toggle, deliberately not an artifact. |
| the opacity slider on the active row drives raster-opacity | The control moves the actual MapLibre paint property, not a shadow value. |

## `palette` — the command surface

| Spec | Protects |
|---|---|
| Ctrl-K opens; 'ndvi' + Enter switches the layer and writes layer= | The palette is a real path to every action, and its actions go through the same write path. |
| the top-bar button opens it; Esc closes and returns focus to the button | Focus returns where it came from — the keyboard contract. |

## `status-bar` — the glass-box numbers

| Spec | Protects |
|---|---|
| ingest→pixel reads a number after the first traced tile, x-ray off; CRS names the scheme | The headline metric is measured from a real trace and needs no x-ray to appear. |
| the cursor cell follows the mouse and copies on click | Coordinates are readable and copyable, which is what a person actually does with them. |

## `swath-xray` — the overlay agrees with the stream

| Spec | Protects |
|---|---|
| overlay paints decisions matching the traces the test received over SSE | The overlay is a view of the trace stream, not an independent story. |
| v1: heatmap buckets, feed lines, and why-view match the SSE stream | Every panel of the overlay reads the same events. |
| analytics panel counters agree with the test's own SSE stream | The counters are derived, not accumulated separately. |
| analytics panel survives a kill-and-resume of the SSE stream | A dropped connection degrades honestly and recovers. |

## `time-slider` — the time dimension

| Spec | Protects |
|---|---|
| slider domain is the granules API's datetimes; single-date layers hide it | The control's domain is server truth; with nothing to scrub it does not appear. |
| scrubbing re-points tiles with datetime=; t joins the deep link, byte-stably | ADR 0015's frame selection, and the frame is shareable. |
| the signature loop: first pass renders live, second pass replays from cache | The product's signature claim: the second pass is served from the tile cache. |
| play advances frames and prefetches the next frame before showing it | Playback is smooth because it prefetches, not because it hides latency. |
| switching to the off-screen fire layer auto-frames it; deep links are honored | Choosing a layer you cannot see moves the camera to it — unless the URL said otherwise. |
| date-vs-date: t and ct split one layer across the handle, byte-stably | Compare is two frames of one layer, and it is a link. |
| per-side x-ray badges: the trace stream's requested= splits the sides | Under compare the x-ray attributes each tile to the correct side. |
| the compare control starts before-vs-after and dismisses it | Compare can be entered and left without stranding state. |

## `compare` — layer against layer

| Spec | Protects |
|---|---|
| layer-vs-layer: cl puts the second layer's tiles on the right side | The other compare mode: two layers, one handle, addressable. |

## `dataset-browser` — finding data

| Spec | Protects |
|---|---|
| lazy by contract: zero browse requests until Data mode is entered; one listing, live granules | Entering the app costs nothing; the panel fetches when it is opened and not before. |
| cards carry engine-rendered thumbnails (POST /result), never a client decode | ADR 0019: every pixel is the engine's, including the thumbnails. |
| choosing a dataset renders footprint outlines as a MapLibre layer | The catalog and the map are one view. |
| activating a card zooms the map to its footprint (fixtures); filters narrow the count | A result is a place, and the filters really filter. |
| a dataset with no granules shows the ingest guidance (fixtures) | An empty state tells you what to do, in words. |
| the timeline's bands come from the counts endpoint, and a drag becomes a shareable date chip | M16: both bands are server counts, and narrowing dates is navigation. |
| the search field states its scope, and a pasted shape is reduced to its box and says so | The scope tag is always visible, and a reduction is admitted rather than hidden. |
| hovering a result draws its footprint, and the keyboard reaches the same thing | Hover is informative and not mouse-only. |

## `add-data` — registering something new

| Spec | Protects |
|---|---|
| paste a fixture COG: registered, in the rail, serving, traced | The whole add-data loop ends in a served, traced tile. |
| a file the server cannot read renders its refusal under the link | The server's own words reach the person, at the field that caused them. |
| the /?stac= deep link pre-fills the flow and registers nothing | A deep link prepares work; it never performs a write nobody asked for. |
| read-only capabilities hide the form (capabilities-driven, not hardcoded) | The UI reads what the server offers instead of assuming a build. |

## `authoring` — composing pipelines

| Spec | Protects |
|---|---|
| UI-authored NDVI serves tiles byte-identical to the built-in layer, no reload | The authoring path and the config path produce the same bytes. |
| the formula builder's reducer child graph compiles to the same NDVI bytes | Two ways of expressing one computation compile identically. |
| an RGB composite (3 ticked bands, stretch, no reduce) publishes and serves | The non-reducing shape of the graph works end to end. |
| the canvas keeps the pipeline valid: permanent frame, typed chips, plain reasons | Invalid states are unreachable, and refusals are in plain words. |
| a graph the server rejects renders its diagnostic on the offending field | Server diagnostics land on the field that caused them. |
| a complete draft shows its live preview image before anything is published | You see the result before you commit to it (ADR 0014's bounded preview). |
| the NDVI template publishes a working layer from one click | The template is a real shortcut, not a form pre-fill. |
| change detection (ADR 0022): template → preview → publish → tile matches the oracle golden → delete 404s | The two-cube join's whole lifecycle, pinned against the GDAL oracle. |
| deleting the join orphans both branches: the canvas greys them, the gate says so, nothing is dropped | Destructive edits are visible and reversible; nothing is silently discarded. |
| deleting a published service 404s its tile URL and drops it from the browser | Deletion is real on both the serving and the browsing surface. |
| the layer row's kebab deletes a published service (issue #282) | The same deletion is reachable from where the layer is listed. |
| UDF stage: upload → preview → publish → x-ray shows fuel → delete → 404 | ADR 0018's whole loop, including the fuel number in the x-ray. |
| UDF stage: a fuel bomb's refusal reads in plain words on the module and never gates a different valid draft | A refusal is scoped to the thing that caused it. |
| publishing shows a receipt whose numbers come from the server (#395) | M14's receipt is built from a real trace, never from client arithmetic. |
| a drag off an output port opens the same typed menu as the sentence (#402) | M15: two routes to the same typed menu, filtered by what the server serves. |
| selecting two steps offers the join, served-driven, and says why not (#403) | The join offer is driven by `GET /processes`, and refusals explain themselves. |
| a join computing backwards says so, and the warning can be accepted (#404) | Order is a semantic warning, not a block — the person decides. |
| after publish the join's label is the server's, not the client's guess (#405) | Labels come from `swath:window`/`swath:sources` and the trace, never inference. |

## `canvas` — the DAG surface itself

| Spec | Protects |
|---|---|
| desktop: drag a node, wheel-zoom, connect by drag, delete by key | The direct-manipulation basics work with a mouse. |
| keyboard: Tab roves nodes, Enter on ports connects, Esc cancels | Everything the mouse can do, the keyboard can do. |
| touch (mobile project): one finger pans, tap-to-connect completes | Touch is a first-class input, not a degraded mouse. |

## `sources` — where data comes from

| Spec | Protects |
|---|---|
| the sources screen is lazy, then shows a measured state per source | M17: every state on the screen came from `GET /sources`, and nothing is fetched until the mode opens. |
| the mode is a shareable artifact, like every other | Sources is addressable like every other mode. |
| the import's register comes from the server, and a half-finished import is a link | The register is data the server holds; the flow's step is in the URL and resumes. |
| detection failure says what it tried and offers the explicit choice | A failed detection names what it ruled out rather than saying "invalid". |

## `mobile` — the small screen is not a leftover

| Spec | Protects |
|---|---|
| landing: the tab bar sits at the bottom, the status chip in the dock, the map paints | The mobile shell is its own layout and it renders. |
| layers: the Layers tab opens the sheet; a row switches the layer; the 90% snap makes the map inert | The sheet's snap points behave, including making the map inert when covered. |
| data + x-ray + author entry from the tab bar and the dock | Every mode is reachable on a phone. |
| author: the pipeline is a canvas — a tap on a node's chip selects its step (#299) | Authoring is usable by touch, not desktop-only. |
