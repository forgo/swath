# Catalog domain model — Dataset / Granule / Layer, and the lossless STAC mapping

_Mini-spec resolving ARCHITECTURE.md §16.5 ("exact `Dataset`/`Layer` schema that cleanly hides
STAC yet round-trips to it losslessly"). Companion code: `swath_core::catalog` (domain types +
`Catalog` port + STAC converters) and `swath-catalog-pgstac` (the adapter). August 2026._

## 1. The contract

Two requirements pull against each other:

- **R2 (single pane of glass):** users manage *datasets* and *layers*; the word "STAC" never
  appears in the public surface.
- **R5 (standards as the contract):** what Swath persists **is** a valid STAC catalog — any
  third-party STAC client pointed at the same pgstac database sees well-formed Collections and
  Items.

The resolution: the domain model is the API; STAC is the *storage document format*. Every
swath-owned field lives under a namespaced `swath:` prefix inside the STAC document (STAC
explicitly permits additional fields; extension prefixes are the conventional carrier), so the
document stays valid for plain STAC tooling **and** the round trip back to the domain loses
nothing.

**Round-trip property (normative):** for every valid `Dataset` `d`,
`dataset_from_stac_collection(dataset_to_stac_collection(d)) == d`, and likewise
`granule_from_stac_item(granule_to_stac_item(g)) == g`. The property is directional —
domain → STAC → domain is the identity. The reverse (STAC → domain → STAC) is *not* promised
for arbitrary foreign documents: a Collection some other tool wrote may carry fields Swath does
not model; converting it either fails loudly (missing swath-required fields) or normalizes it.
Swath owns the collections it writes; it does not promise to round-trip anyone else's.
A proptest suite in `swath-core` enforces the property over arbitrary in-bounds values.

This also sharpens the ARCHITECTURE.md §6 port sketch: the `Catalog` port there was drafted
STAC-shaped (`upsert_collection(&Collection)`). It is now **domain-shaped**
(`upsert_dataset(&Dataset)`): STAC appears only inside adapters, produced/consumed by the pure
converter functions in `swath_core::catalog::stac`. R2 is then enforced by construction — no
STAC type exists in any port signature.

## 2. The three nouns

| Noun | Is | Maps to |
| --- | --- | --- |
| `Dataset` | A logical collection of granules sharing a band vocabulary, CRS family, and cadence (e.g. "HLS S30") | STAC **Collection** |
| `Granule` | One acquisition's assets: a band → asset-URI map plus footprint and timestamp | STAC **Item** |
| `Layer` | A *serving* definition over a Dataset: the `TileRequest` template (plan kind, rescale, colormap, resampling, tile size) | `swath:layers` entry **on the Collection** (see §4) |

### Dataset

| Field | Type | Notes |
| --- | --- | --- |
| `id` | `DatasetId` (string newtype) | pgstac collection id |
| `title` | `String` | required in the domain (STAC makes it optional; Swath always writes it) |
| `description` | `String` | |
| `license` | `String` | SPDX id or `other`, passed through |
| `extent` | `Extent` = `Bbox` + optional open/closed time interval | overall spatial + temporal extent |
| `bands` | `BTreeSet<String>` | the band names granules of this dataset provide (sorted, deduped — canonical order makes round-trip structural) |
| `layers` | `Vec<Layer>` | serving definitions; order preserved (it is the presentation order) |

### Granule

| Field | Type | Notes |
| --- | --- | --- |
| `id` | `GranuleId` (string newtype) | pgstac item id, unique within the dataset |
| `dataset` | `DatasetId` | the owning collection |
| `bbox` | `Bbox` (WGS84 lon/lat) | footprint; geometry is *derived* (see §3) |
| `datetime` | `Datetime` (RFC 3339 UTC, `Z`-suffixed, validated newtype) | acquisition time |
| `assets` | `BTreeMap<String, AssetRef>` | band name → asset URI; the map the tiler's `bands` template resolves against |
| `ingested_at` | `Option<Datetime>` | when Swath ingested the granule — the ingest-to-pixel zero point (#31); stamped by the ingest orchestrator, `None` for granules registered outside the event path |

### Layer

| Field | Type | Notes |
| --- | --- | --- |
| `id`, `title`, `description` | strings | the identity `swath-api`'s registry exposes today |
| `plan` | `PlanKind`: `Composite { r, g, b }` \| `BandMath { expression }` | band names refer to the dataset's band vocabulary; `expression` is the infix band-math string the process compiler (#34) parses — the catalog stores it opaquely |
| `rescale` | `{ min, max }` | linear map to 0..255 |
| `colormap` | `Option<Colormap>` (`grayscale`) | absent = none |
| `resampling` | `Nearest` \| `Bilinear` | nodata policy is a serving-time default, not catalog state |
| `tile_size` | `u32` | 256 today |

`Layer` deliberately does **not** embed `swath-render`'s `RenderPlan`/`Expr` AST: the catalog is
below the render crate in the dependency graph, and persisted schemas should not be coupled to an
executable IR that refactors freely. `PlanKind` is the small, stable, storage-facing vocabulary;
lowering `PlanKind` → `RenderPlan` happens at serving wire-up (the `swath-api` registry seam —
`LayerRegistry` is replaced by catalog-resolved layers behind the same `get`/`iter` surface;
deliberately untouched by #30).

## 3. The STAC mapping, field by field

STAC version pinned: **1.1.0**.

### Dataset ⇄ Collection

| Dataset field | STAC Collection location | Inverse |
| --- | --- | --- |
| — | `type: "Collection"`, `stac_version: "1.1.0"` | checked, then dropped |
| `id` | `id` | read back |
| `title` | `title` | read back; **missing ⇒ error** (a Collection without `title` is not a swath Dataset) |
| `description` | `description` | read back |
| `license` | `license` | read back |
| `extent.bbox` | `extent.spatial.bbox[0]` | read back (first bbox = overall, per STAC) |
| `extent.start`/`end` | `extent.temporal.interval[0]` (`null` = open end) | read back |
| `bands` (sorted) | `swath:bands` (JSON array, sorted) | read into `BTreeSet`; **missing ⇒ error** |
| `layers` | `swath:layers` (JSON array, order preserved) | read back; **missing ⇒ error** |
| — | `links: []` | ignored on read (links are catalog-plumbing owned by pgstac/API layers, not domain state) |

The `swath:bands`/`swath:layers`-missing errors are how foreign Collections in a shared pgstac
database are detected and rejected loudly instead of half-converted.

### Granule ⇄ Item

| Granule field | STAC Item location | Inverse |
| --- | --- | --- |
| — | `type: "Feature"`, `stac_version: "1.1.0"` | checked, then dropped |
| `id` | `id` | read back |
| `dataset` | `collection` | read back |
| `bbox` | `bbox` (`[w, s, e, n]`) | read back |
| `bbox` (derived) | `geometry`: the bbox's closed CCW polygon ring | **ignored on read** — `bbox` is the source of truth; the derivation is deterministic, so the round trip is exact |
| `datetime` | `properties.datetime` | read back |
| `assets` | `assets` — `{ "<band>": { "href": "<uri>" } }` | `href` read back; other asset keys ignored (Swath writes only `href`, so its own documents round-trip exactly) |
| `ingested_at` | `properties."swath:ingested_at"` (omitted when `None`) | read back when present; **present-but-invalid ⇒ error** (missing is fine — plain STAC Items are valid swath granules without an ingest stamp) |

The band → URI map *is* the STAC assets map, key for key. Granule-level swath-owned metadata goes
under `properties."swath:…"`, as reserved here from the start — `swath:ingested_at` (#31, the
ingest-to-pixel zero point) is the first such field.

### Layers on the Collection, not separate documents (decision)

`swath:layers` lives **inside the Collection document**. The alternative — Layers as separate
documents (Items in a side collection, or a swath-private table) — was rejected:

1. **Atomicity.** A Dataset and its serving definitions change together; one
   `pgstac.upsert_collection` call updates both. Separate documents mean two writes and a
   torn-state window.
2. **Plain-STAC visibility stays honest.** A Layer is not an acquisition; modeling it as an Item
   would make third-party clients see phantom "features" in search results. As a namespaced
   Collection field it is exactly what STAC says unknown fields are: ignorable extension data.
   The catalog a plain client sees contains only real Collections and real Items.
3. **No orphan lifecycle.** Layers cannot dangle when their Dataset is deleted, and
   `get_dataset` needs no join.

Cost, accepted knowingly: layers are not independently searchable in pgstac, and very large layer
lists bloat the Collection document. Neither matters at the current scale (layers per dataset:
single digits); if it ever does, the escape hatch is a superseding design doc plus a migration —
the domain API (`Dataset.layers`) would not change, which is the point of hiding the storage
shape.

## 4. The `Catalog` port

Domain-shaped, async-in-trait exactly like `RasterSource` (native AFIT, `Send` futures, no
runtime dependency in core, deliberately not dyn-compatible):

```rust
pub trait Catalog: Send + Sync {
    async fn upsert_dataset(&self, dataset: &Dataset) -> Result<(), CatalogError>;
    async fn upsert_granules(&self, granules: &[Granule]) -> Result<(), CatalogError>;
    async fn get_dataset(&self, id: &DatasetId) -> Result<Option<Dataset>, CatalogError>;
    async fn list_datasets(&self) -> Result<Vec<Dataset>, CatalogError>;
    async fn find_granules(&self, dataset: &DatasetId, query: &GranuleQuery)
        -> Result<Vec<Granule>, CatalogError>;
}
```

`GranuleQuery` is deliberately minimal — optional `bbox` intersection and optional
half-open/closed `datetime` range — because that is what serving and ingest (#31) consume. It is
**not** a STAC search façade; breadth (sortby, arbitrary property filters, paging surfaced to
callers) is added when a consumer exists, not speculatively. The pgstac adapter internally pages
through `pgstac.search` and returns the full result set.

Error taxonomy (`CatalogError`): `DatasetNotFound` (writes/reads against an absent dataset),
`Stac(StacError)` (a stored document failed to map back — the "someone else wrote to our
database" signal), `Backend` (connection/SQL failure, source preserved). `StacError` itself is
the converter taxonomy: `MissingField`, `WrongType`, `InvalidValue`, each naming the JSON path.

## 5. Validation strategy

- **Property test (normative):** proptest generates arbitrary in-bounds `Dataset`s/`Granule`s
  (finite bboxes, valid RFC 3339 UTC datetimes, arbitrary band/layer content) and asserts the §1
  identity.
- **Structural STAC assertions:** unit tests assert every STAC-required field is present in
  emitted documents. Full JSON-Schema validation is deliberately **not** wired in-repo: the STAC
  1.1 schemas are a multi-file network of remote `$ref`s (a committed-schema approach like the
  OGC conformance suite would mean vendoring that whole graph), and the live gate is stronger
  anyway —
- **The pgstac gate (integration):** pgstac itself validates document structure on ingest; the
  integration suite (`just test-catalog`) upserts through the adapter, reads back through plain
  `pgstac.search`, and asserts the returned features carry the STAC-required fields — the R2/R5
  bridge test: swath hides STAC and still emits a catalog plain STAC clients can read.
- **Snapshot:** one representative Collection + Item pinned with insta, so any change to the
  persisted shape is a reviewed diff, not an accident.
