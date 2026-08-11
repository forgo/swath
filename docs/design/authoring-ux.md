# Authoring UX rethink — guided flow and preview-before-publish

_Design note for issue #151, satisfying its first acceptance criterion: compare ≥2 interaction
models concretely, with the maintainer choosing before any implementation issue is filed.
Companion code studied: `web/src/swath-authoring-panel.ts` (the M5 panel, #109/PR #148) and
`crates/swath-render/src/process.rs` (the #32 compiler, whose diagnostics define the bad-state
family). August 2026._

## 1. Where #148 landed, and where it ceilings

The current panel already does a lot of the right things: palette and forms generated from
`GET /processes` schemas alone, pre-submit validation with spelled-out reasons, a collection
picker that makes unknown collections unreachable, plain-language one-liners under every field,
smart defaults, per-step advanced collapse, a live narrative sentence, and a one-click NDVI
template. Its shape, though, is still **graph-first**: the user assembles openEO process nodes
in order, and the panel checks *fields*, not *pipelines*. Two failure cases from the
maintainer's non-expert walkthroughs mark the ceiling:

- **Unknown `spatial_extent`** (PR #148 review, round 3): faced with a field named
  `spatial_extent`, a non-expert has no idea what value shape it wants (a bbox object? which
  CRS?) or whether leaving it alone is safe. Round 3 answered with a visible one-liner ("The
  map area to compute over — leave as is to use the whole collection") and advanced-collapse —
  a good patch, but the field still *exists* as an openEO concept the user must be talked past.
- **Dangling `divide`** (issue #151 comment): add a `divide` step, publish, and the server
  answers `ProcessGraphInvalid: result node `s6` is `divide`: the graph must end in
  save_result (format "png")` — accurate, mapped inline, and still meaningless without openEO
  context. Per-field validation cannot catch it: every field of that graph is individually
  fine; the *pipeline shape* is wrong.

The second case is one member of a family (§2). The models in §3–§4 are judged by how much of
the family they make unconstructible versus merely explained.

## 2. The reachable-bad-states family

Every state below passes the panel's current field-level validation (or produces no error at
all) yet publishes something rejected or wrong. Sources: the compiler's typed diagnostics in
`crates/swath-render/src/process.rs` and the render profile's narrowing.

| # | State (how a user reaches it) | What happens today |
| --- | --- | --- |
| B1 | **Wrong tail** — last step is not `save_result` (append `divide`, or delete the save step, or stop after `ndvi`) | server: `UnsavedResult` — the recorded `divide` case |
| B2 | **Scalar op at top level** — `add`/`subtract`/`multiply`/`divide` added as a pipeline step; the compiler admits them only inside a reducer's child graph, and their auto-wired `x` receives a cube | server: type mismatch "expected a number, got a … data cube" |
| B3 | **`array_element` at top level** — its `data` must be the reducer's band array (`from_parameter`), which no top-level step can supply | server: type mismatch "expected the reducer's band array" |
| B4 | **Scale before reduce** — `linear_scale_range` placed before `ndvi`/`reduce_dimension` | server: "expected an unscaled data cube (apply linear_scale_range after reducing)" |
| B5 | **Wrong shape at save** — a multi-band cube with ≠3 bands reaches `save_result` (e.g. load 2 bands, forget the `ndvi` step) | server: "exactly 3 bands for an RGB composite (or reduce to gray first)" |
| B6 | **Colormap on a composite** — colormap chosen while the result is still multi-band | server: "a colormap maps one gray value per pixel …" |
| B7 | **Unknown band names** — `bands`/`nir`/`red` typed free-form; the band *hint* lists the vocabulary but nothing enforces it | server: `UnknownBand` with the available list |
| B8 | **Degenerate range** — `inputMin >= inputMax` | server: "degenerate input range" |
| B9 | **Non-png format** — `format` is a free text field (defaulted "png" under advanced) | server: "only \"png\" is supported in v0" |
| B10 | **Silently dead steps** — the compiler evaluates lazily from the result node, so steps nothing references compile away without any diagnostic; the user believes their step contributed | **publishes fine, does the wrong thing** |
| B11 | **Valid but visually wrong** — plausible-but-off input range (0..10000 for NDVI → washed-out tiles), swapped `nir`/`red` (inverted index), wrong colormap for the data | **publishes fine, looks wrong**; no validator can catch this — only seeing it can (§6) |

B1–B9 are *rejected-by-server* states a structurally-aware UI can make unconstructible.
B10–B11 are worse — they publish. B10 needs structural awareness (dead steps visible or
impossible); B11 needs preview.

## 3. Model A — outcome-first wizard

**Shape.** Invert the entry point: instead of "here are openEO processes, assemble them," ask
**"What do you want to see?"** and walk forward:

1. **Outcome** — a small curated set of cards: *vegetation health (NDVI)*, *single band*,
   *RGB composite*, *custom band formula*. Each card is a generalized template (the existing
   `NDVI_TEMPLATE` mechanism grown into a family): a pipeline skeleton that always carries the
   correct reduce → scale → save tail.
2. **Collection** — the existing `/collections`-fed picker, with the outcome's band roles
   (nir/red; r/g/b; formula operands) prefilled by the existing common-name heuristics
   (`pickBand`) and shown as *choices among the collection's bands*, never free text.
3. **Where/when** — plain-worded: "Everywhere the collection covers" (default) or a
   map-drawn area; "All available dates" or a range. `spatial_extent`/`temporal_extent` as
   *names* never appear.
4. **Look** — input range (prefilled per outcome, e.g. −1..1 for NDVI) and colormap swatches.
5. **Publish** — narrative sentence + (once §6 lands) the preview tile, then the existing
   `POST /services` path.

An **expert-mode escape hatch** keeps the current panel reachable (a "switch to advanced
editor" link carrying the wizard's state over as steps), preserving full palette generality.

**Walkthrough — unknown `spatial_extent`.** The field does not exist in the wizard; step 3
asks the actual question in the user's vocabulary and the default is the safe one. Eliminated
by construction (inside the wizard).

**Walkthrough — dangling `divide`.** There is no way to append a bare `divide`: arithmetic is
reachable only inside the *custom band formula* outcome, whose skeleton owns the reduce
context, the scale step, and the `save_result` tail permanently. B1/B2 unconstructible —
inside the wizard. Via the escape hatch, the whole family returns.

**Implementation scope.** A new `<swath-authoring-wizard>` beside (not replacing) the panel;
the heavy plumbing is reused: schema fetch/parse, `buildGraph()`, submit gating, server-error
mapping, the template mechanism (generalized from one hardcoded pipeline to a small recipe
table, still gated on every recipe process existing in the served definitions — the
`NDVI_TEMPLATE` availability rule, so schema-derivation stays honest). New work: the step
flow/chrome, the recipe table, the map-drawn extent control, state handoff to expert mode.
Moderate: mostly additive UI over existing machinery.

**Risks.**
- **Curated ceiling.** Outcomes are hand-curated; anything off the list drops the user into
  expert mode where B1–B11 all still exist. The wizard *narrows* the bad-state family's
  audience rather than eliminating it.
- **Vocabulary drift.** Recipes are a second curated layer (after `FIELD_HELP` and
  `narrativePhrase`) over the served definitions; each new server process wants a recipe
  decision.
- **Two surfaces to maintain and test** (wizard + panel), including the state handoff.

## 4. Model B — always-valid canvas

**Shape.** Keep one editor, but change its invariant: **the pipeline is never in an invalid
state**. The generalization of the pattern #148 already proved with the collection picker
("an unknown collection is unreachable from the UI") — apply it to pipeline structure:

- **Permanent output tail.** The canvas always ends in an *Output* card — `save_result`
  rendered as "format png · colormap ▾", not removable, not reorderable. B1 is
  unconstructible; the maintainer's suggested "the UI always keeps a save_result tail
  attached" made structural.
- **Stage-typed insertion.** The compiler's value discipline (load → cube ops → reduce/ndvi →
  scale → output; arithmetic and `array_element` only inside a reducer's child graph) becomes
  the client's insertion rule: a step can only be added where its input type exists, and the
  palette shows only what fits the selected insertion point. `divide` never appears as a
  top-level candidate — it lives inside a *formula builder* sub-editor that owns the
  `reduce_dimension` child-graph context (B2/B3 unconstructible). `linear_scale_range` offers
  itself only after a gray/composite result exists (B4).
- **Vocabulary-only values.** Band fields become selects over the chosen collection's bands
  (the hint in `#pipelineBands()` promoted to the widget itself) — B7 gone. `format` becomes
  the served enum (B9). Colormap greys out with an inline "reduce to one value per pixel
  first" note while the result is multi-band (B6 explained *before* submit, made
  unconstructible once the tail knows the result kind). Range inputs swap on inversion or
  flag inline (B8).
- **No dangling steps.** Linear pipeline + permanent tail + insertion-only-where-typed means
  every step is on the result path; B10 unconstructible. B5 becomes a pre-submit narrative
  warning in the user's words — "this pipeline produces 2 channels; a picture needs 1 (a
  formula) or 3 (red/green/blue)" — with submit gated, since "which fix" is the user's call.
- `spatial_extent`/`temporal_extent` get the same treatment as Model A step 3: a
  plain-worded "area/time" control on the load card (subtype-keyed widget, like
  `collection-id` — schema-derived in mechanism, curated in wording).

Templates stay as the entry point (the NDVI chip, growable into more), so an outcome-first
*flavor* is cheap here later without a second surface.

**Walkthrough — unknown `spatial_extent`.** Same elimination as Model A (the widget asks the
real question), but on the one and only editor — no escape hatch where the raw field returns.

**Walkthrough — dangling `divide`.** `divide` is not offered at top level at all; the user
finds "combine bands with a formula" (the reducer builder), inside which divide is valid and
the tail is untouched. The recorded error class is unconstructible for every user, not only
wizard users.

**Implementation scope.** A substantial rework of `swath-authoring-panel.ts` in place: a
client-side stage model (a small pinned table mirroring the compiler's `Value`/`Cube` states),
palette filtering by insertion point, the permanent tail card, the formula-builder sub-editor
(new — the panel has no child-graph support today; the compiler and `buildGraph()` need it
expressed as `reduce_dimension`'s `reducer` child graph), widget upgrades (band selects, area
control). Larger than Model A, concentrated in one component; the e2e suite's byte-identical
NDVI check carries over as the safety net.

**Risks.**
- **Client re-encodes server semantics.** The stage table duplicates compiler type rules;
  drift means the UI forbids something the server allows (annoying) or allows something it
  rejects (back to mapped server errors — the current, survivable behavior). Mitigations:
  keep the table tiny and pinned by tests on both sides; e2e-prove every palette-offered
  insertion actually publishes; longer-term, a server `POST /validation` (openEO-standard)
  could replace the client table — noted as a possible follow-up, same ADR-shaped question
  as §6.
- **Schema-purity bend.** Stage rules are not in the parameter schemas; they come from
  compiler semantics. The #148 principle ("delete a served definition and its form
  disappears") survives — the table only *orders* processes that exist — but the README-level
  claim needs honest restatement.
- **Formula builder is real new UI** with its own UX pitfalls; scoping it to "binary ops over
  band selects" for v1 keeps it small.

## 5. Bad states × models

| # | Today | Model A (in wizard / via escape hatch) | Model B |
| --- | --- | --- | --- |
| B1 wrong tail | server error, mapped | unconstructible / reachable | unconstructible |
| B2 top-level arithmetic | server error | unconstructible / reachable | unconstructible (formula builder) |
| B3 top-level array_element | server error | unconstructible / reachable | unconstructible (formula builder) |
| B4 scale before reduce | server error | unconstructible / reachable | unconstructible (insertion typing) |
| B5 ≠3-band composite | server error | unconstructible / reachable | explained pre-submit, gated |
| B6 colormap on composite | server error | unconstructible / reachable | explained, then unconstructible |
| B7 unknown band | server error | unconstructible / reachable | unconstructible (band selects) |
| B8 degenerate range | server error | prefilled per outcome / reachable | inline flag pre-submit |
| B9 non-png format | server error | not asked / reachable | enum select |
| B10 dead steps | **silent** | unconstructible / reachable | unconstructible (linear + tail) |
| B11 valid but wrong | **silent** | preview (§6) | preview (§6) |

Model A eliminates the family for wizard users and reintroduces all of it behind the escape
hatch. Model B eliminates or pre-explains all of B1–B10 for every user; B11 is unreachable by
any validator and belongs to preview in either model.

## 6. Preview-before-publish vs ADR 0010's bounded profile

B11 — and honestly the *delight* half of #151 — needs the user to **see** the draft before
publishing. Feasibility against the bounded profile:

- **What exists.** ADR 0010's profile has no execution endpoint: no jobs, no batch, no
  `POST /result` (openEO synchronous processing), no `POST /validation`. The capabilities
  document lists only what exists, honestly. The compiler and the render path, however,
  already do all the work a preview needs — `POST /services` compiles the same graph, and the
  tile path renders it; only the *combination* (render a draft graph without persisting a
  service) has no route.
- **Zero-server bridge (rejected).** The UI could publish, fetch one tile, and delete.
  Works today, but pollutes the services list and the catalog (`swath:layers` churn on every
  keystroke-debounce), races with the layer-list refresh events, and makes "preview" a
  side-effectful lie. Not acceptable beyond a prototype.
- **Option P1 — openEO `POST /result`, preview-bounded subset.** The standard sync-execute
  endpoint, admitted to the profile the same way everything else was: bounded, with the
  narrowing declared honestly. Body as in the spec (`{"process": {"process_graph": …}}`);
  the server compiles via the #32 compiler (same diagnostics, standardized error format) and
  answers `image/png` — not the spec's "full extent at native resolution" (unbounded work,
  and Swath's engine is a tiler), but **one small overview-backed tile** covering the graph's
  `spatial_extent` (or the collection extent when null), sized so the planner's byte ceiling
  (`max_estimated_live_bytes`, materialization-planner design §1) admits it from overviews —
  a preview is exactly the workload overviews exist for. Real openEO clients gain a lawful,
  clearly-narrowed sync endpoint; the UI gains its preview with no bespoke vocabulary.
- **Option P2 — bespoke `POST /preview` route.** Smaller to specify (no spec semantics to
  narrow), but it is precisely the "minimal control-plane wrapper, no discoverability" shape
  ADR 0010 already weighed and declined for the authoring surface. A second, nonstandard
  verb beside a standard one that fits is hard to justify.
- **Server scope → needs an ADR.** Either option grows the public surface, which ADR 0010
  fixed by decision; this is not a UI-side call. **What the ADR would say** (to be written
  when this design is accepted, not here): extend the bounded openEO profile with
  `POST /result` as a preview-grade synchronous subset — same compiler, same error registry,
  response bounded to a single small render admitted by the planner's cost model, the
  narrowing declared in the capabilities document exactly as the profile's other honest
  omissions are; general synchronous-processing conformance is *not* claimed. It would also
  record why the bespoke route was declined (consistency with 0010's own reasoning) and note
  ADR 0012's evidence that inline rendering tolerates this load class (a preview is one
  bounded tile — no new runtime shape). Rate/debounce behavior stays a UI concern.
- **In the UI**, either model hosts it the same way: a debounced preview tile beside the
  narrative sentence, rendered whenever the draft compiles, replacing the narrative's
  guesswork with ground truth — swapped nir/red *shows* wrong, a washed-out range *shows*
  washed out (B11's only countermeasure).

## 7. Recommendation

**Model B — always-valid canvas — as the core editor, with §6's preview via the
`POST /result` bounded subset (P1) proposed in a follow-up ADR.**

Reasoning: the project has already proven the pattern twice — the collection picker made
`CollectionNotFound` unreachable, and submit gating made "no collection named" unreachable;
Model B is that same move applied to pipeline structure, and it covers **every** user rather
than only those who stay inside a wizard. Model A's outcome-first *entry* is genuinely the
more delightful first minute, but its escape hatch preserves the entire bad-state family, and
its best ideas (outcome cards, plain-worded where/when, prefilled ranges) all fit *inside*
Model B as template chips and card widgets — the reverse is not true. Sequencing if B is
chosen: invariant canvas first (kills B1–B10), preview ADR + endpoint second (kills B11),
outcome-card templates third (the delight layer), each its own issue.

## 8. Maintainer: interaction model selected

_Check one; this design note is inert until a box is checked, and the implementation issues
get filed against the choice._

- [ ] **Model A — outcome-first wizard** (expert-mode escape hatch keeps the current panel)
- [x] **Model B — always-valid canvas** ← recommended (§7)
- [ ] **Other / hybrid** (describe in a comment on #151)

_Selected by the maintainer 2026-08-11: Model B as the core editor, with §6's
preview-before-publish via the `POST /result` bounded subset (P1) proposed in a follow-up ADR._
