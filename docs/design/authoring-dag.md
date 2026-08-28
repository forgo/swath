# Authoring as a DAG — earning the graph with one join

_Design note for issue #294 (M11 — Earn the DAG). Companion to
[`authoring-ux.md`](authoring-ux.md) (Model B, the always-valid canvas) and
[`ui-system.md`](ui-system.md) (the canvas primitives, #290). The decision it prepares is
[ADR 0022](../decisions/0022-two-cube-join-merge-cubes.md)._

## 1. Where Model B ceilings

Model B (authoring-ux.md §4) made the pipeline **never invalid**: a permanent `save_result` tail,
stage-typed insertion, vocabulary-only values, no dangling steps. It did so by being a **chain** —
one `load_collection`, a line of cube ops, one output. Every bad state B1–B11 is unconstructible
or explained because a chain has exactly one path and the compiler
(`crates/swath-render/src/process.rs`) evaluates exactly one source.

The chain ceilings at the first product that needs two inputs. "How did this field change
between June and October?" is not a step in a chain: it is two frames of one collection,
combined per pixel. The M10 compare swipe shows two frames *side by side*; it cannot subtract
them. The x-ray traces one granule per render (ADR 0015: every frame is backed by exactly one
granule). Today's answer to "change detection" is "publish two layers and look".

A DAG editor is the obvious UI answer, and it is the one the Stitch exploration drew — a free
node graph. `ui-system.md` §2 fenced that out for M10 on purpose: **a graph with nothing to join
is a worse chain**. It invites every reachable-bad-state the chain eliminated (a dangling node is
just B10 with more room) and buys no capability, because the compiler still admits one source.
The DAG must be *earned* by a server-side join: the first process that takes two cubes.

## 2. The invariant, restated for a graph

Model B's invariant was "the pipeline is never in an invalid state". For a graph it becomes:

> **Every node is on a typed path from a source to the one output, and every edge carries the
> type its input port declares.**

Three consequences, each a rule the editor enforces rather than a diagnostic it shows:

1. **Ports are typed.** A port is `cube` (a data cube: bands × time × pixels), `gray` (a cube
   reduced to one value per pixel), `rgb` (three bands), or `number`. An output port declares
   what it produces; an input port declares what it accepts. `edgeAllowed(from, to)` is the one
   pure function that says whether a drag may connect — the client-side twin of the compiler's
   value discipline (the "stage table" of authoring-ux.md §4, now keyed by port rather than by
   position).
2. **One output, permanent.** `save_result` is a node that cannot be deleted and has no output
   port. The graph's *result* is whatever feeds it. B1 stays unconstructible.
3. **Orphans are explained and gated, not hidden.** A node with no path to the output is an
   orphan. The chain made orphans impossible; a graph cannot — you place a node before you wire
   it. So B10 moves honestly from *unconstructible* to *explained + gated*: the orphan is drawn
   dimmed with a plain-words badge ("not connected — nothing reaches the output through this
   step"), the narrative omits it, the preview ignores it, and **publish is gated** while one
   exists. The compiler's lazy evaluation is unchanged; the editor refuses to hand it a graph the
   user has not finished.

## 3. Why `merge_cubes`, not cube arithmetic or `mask`

Three ways to give the graph its first join were weighed.

| Option | What it would be | Why not / why |
| --- | --- | --- |
| **Top-level cube arithmetic** — `subtract(x: cube, y: cube)` | The intuitive "A − B" node | **Contradicts the pinned process definitions.** `subtract.json` (and `add`/`multiply`/`divide`, served verbatim from `crates/swath-api/data/openeo-processes/`) declares `x` and `y` as `number \| null` and returns a number — the openEO definition of scalar arithmetic. Widening them to cubes would serve a definition that says one thing and does another, the exact dishonesty ADR 0010's narrowing pattern exists to avoid. B2 ("scalar op at top level") stays a bad state for a *reason*. |
| **`mask`** — `mask(data: cube, mask: cube)` | A second cube gates the first | A join in shape, but its semantics are a special case (replace where the mask is truthy) that does not express change detection, and it drags in a nodata/replacement vocabulary the IR does not have. Declined for v1; a reopen condition. |
| **`merge_cubes`** — `merge_cubes(cube1, cube2, overlap_resolver)` | The openEO join: two cubes with the same dimensions become one; where both carry a value, the resolver (a reducer child graph over the pair) decides | **Selected.** It is the standard's own two-cube process; its resolver is exactly the `reduce_dimension` child-graph mechanism the compiler already has (`subtract` *inside* a reducer is where scalar arithmetic is admitted today — B2's other half). Narrowed to what the engine honestly supports (ADR 0022 §Decision), it composes with everything Model B built: gray × gray, same collection, resolver required. |

The join is deliberately tiny. Its purpose is not "general algebra over cubes"; it is to make one
product real — **change detection: `(frame at t₂) − (frame at t₁)` of one gray value, colormapped**
— on the same bounded profile, with the same trace, cache and budget story every other pixel has.

## 4. The graph model

Pure, in `web/src/authoring-graph.ts` (issue #298); the canvas primitives (#290) only draw it.

- **Node**: `{ id, process, params, ports: { inputs: Port[], outputs: Port[] } }`. Ports come from
  the served process definition plus the narrowing table: `load_collection` → out `cube`;
  `filter_temporal` → in `cube`, out `cube`; `ndvi` / `reduce_dimension` → in `cube`, out `gray`
  (or `rgb` when the reducer keeps three); `linear_scale_range` → in `gray|rgb`, out the same;
  `run_udf` → in `gray`, out `gray`; **`merge_cubes` → in `gray` × 2 (`cube1`, `cube2`), out `gray`**;
  `save_result` → in `gray|rgb`, no output.
- **Edge**: `{ from: {node, port}, to: {node, port} }`. `edgeAllowed` returns a plain-words reason
  when it refuses: type mismatch, an input port already fed, a cycle, or the two branches of a
  `merge_cubes` not from the same collection (§5's narrowing surfaces here, before the server).
- **Orphans**: `orphans(graph)` — nodes with no path to `save_result`. Gate publish; dim on the
  canvas; omit from the narrative.
- **Lowering** `lower(graph) → process_graph`: the openEO JSON the chain's `buildGraph()` produced,
  generalised — node ids become graph keys, edges become `from_node` references, the resolver
  becomes `merge_cubes.arguments.overlap_resolver.process_graph` with `x`/`y` bound to the pair
  (openEO's own convention). A lowered chain is byte-identical to today's `buildGraph()` output —
  the e2e's byte-identical NDVI check is the safety net for the rewrite (#299).
- **Templates**: the NDVI chain (as today) and the first DAG template, **change detection** —
  `load(A) → filter_temporal(t₁) → ndvi → ┐`, `load(A) → filter_temporal(t₂) → ndvi → ┘ → merge_cubes(subtract) → linear_scale_range → save_result`. A template is a graph literal; applying it is `lower` in reverse (#300).
- **Layout**: node positions are editor state, persisted with the draft (localStorage, the same
  key discipline as `view-state`), never in the process graph.

## 5. The narrowing, in the editor's words

ADR 0022 fixes these server-side; the editor makes them unconstructible or explained:

| Rule | Server | Editor |
| --- | --- | --- |
| Both inputs gray (one value per pixel) | type mismatch otherwise | `edgeAllowed`: only `gray` ports may feed `cube1`/`cube2` |
| Same collection on both branches | `MergeCubesMismatch` | `edgeAllowed` refuses the second edge with "both branches must load the same collection" |
| Resolver required, over `x`/`y` | `MissingResolver` | the `merge_cubes` node is created *with* a resolver builder (the formula builder scoped to two operands) — never without |
| `context` absent | rejected | not offered |
| Both branches frame-selected | every pixel pair resolves to one granule per branch (ADR 0015) | each branch's `load_collection` carries a `temporal_extent`; the template pre-fills t₁/t₂ from the collection's extent |
| `datetime=` intersects every branch | the served layer's frames = the intersection of both windows | the time slider's domain for a two-source layer is that intersection; a frame outside it is not offered |

## 6. Bad states × the DAG

The chain's table (authoring-ux.md §2) extended. "Unconstructible" means the editor cannot express
it; "explained + gated" means it can be reached but is named in plain words and blocks publish.

| # | State | Chain (Model B) | DAG |
| --- | --- | --- | --- |
| B1 | Wrong tail | unconstructible | unconstructible (permanent output node) |
| B2 | Scalar op at top level | unconstructible | unconstructible (arithmetic only inside a resolver / reducer builder) |
| B3 | `array_element` at top level | unconstructible | unconstructible |
| B4 | Scale before reduce | unconstructible | unconstructible (`linear_scale_range` accepts `gray\|rgb` only) |
| B5 | Wrong shape at save | explained + gated | explained + gated (the output port's type shows it) |
| B6 | Colormap on a composite | explained | explained |
| B7 | Unknown band | unconstructible | unconstructible |
| B8 | Degenerate range | explained | explained |
| B9 | Non-png format | unconstructible | unconstructible |
| **B10** | **Dead / orphan step** | **unconstructible** | **explained + gated** — the honest regression of going from a line to a graph (§2.3) |
| B11 | Valid but visually wrong | preview | preview (the join's preview renders the resolved pair) |
| B12 | **Type mismatch on an edge** (a cube into a gray port) | n/a | unconstructible (`edgeAllowed`) |
| B13 | **Two collections into one join** | n/a | unconstructible (`edgeAllowed`) |
| B14 | **Join without a resolver** | n/a | unconstructible (created with one) |
| B15 | **Empty frame intersection** (t₁/t₂ windows that share no frame with a request) | n/a | explained: the slider shows the intersection; a `datetime=` outside it is the tile route's 404, as for one source |

## 7. Three walkthroughs

**Change detection from the template.** Author mode → "Start from change detection". The canvas
shows the two-branch graph with t₁/t₂ pre-filled to the collection's first and last frames. The
preview renders the difference colormapped. Publish serves a layer whose time slider spans the
intersection of both windows; scrubbing moves *both* frames (ADR 0022's `datetime=` rule). The
x-ray shows, per tile, the two granules the join resolved (#296).

**A dangling node.** The user drags a second `ndvi` onto the canvas and forgets to wire it. It
dims immediately with "not connected"; the narrative reads the connected path only; publish is
gated with "1 step is not connected — wire or delete it". No server round trip, no silent
compile-away (B10, explained + gated).

**A refused edge.** The user tries to feed a `cube` (an unreduced `load`) into `merge_cubes`. The
port refuses the drop and says "combine one value per pixel — add an NDVI or a formula step
first". The same wording the server would use, one interaction earlier (B12).

## 8. Risks

- **Two definitions of the join.** `edgeAllowed` re-encodes ADR 0022's narrowing. Mitigation as
  in authoring-ux.md §4: keep the table tiny, pin it on both sides (`process.rs` tests and the
  graph-model vitest share the fixture graphs), e2e-prove every template publishes.
- **Orphans are a new class of "the user believes it worked".** The gate is the mitigation; the
  narrative omitting them is the tell. If the dimming is not enough in the device pass, the
  next step is refusing to *place* a node except by dragging from a port — not hiding orphans.
- **Layout state is a third store.** Positions in localStorage beside view/app state. Bounded
  by keeping positions out of the process graph and out of the URL.
- **The join's preview cost doubles reads.** ADR 0014's budget applies per graph; a two-branch
  preview may refuse where a chain would not. The refusal reads in plain words, as today.

## 9. Maintainer: shape and narrowing selected

_Check to unblock #295–#301; the ADR carries the decision, this note the reasoning._

- [ ] **`merge_cubes` at the bounded profile** as narrowed in ADR 0022 (gray × gray, same
  collection, resolver required, both branches frame-selected, `datetime=` intersects) ← recommended
- [ ] **Widen top-level arithmetic to cubes** (re-serve `subtract.json` widened — rejected in §3)
- [ ] **`mask` first** (a gated special case — declined in §3, reopen condition in the ADR)
- [ ] **Other** (describe on #294)
