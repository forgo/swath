<!--
SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
SPDX-License-Identifier: Apache-2.0
-->

# Virtual-reference manifest, version 1

This document is the normative description of manifest schema v1 — the
versioned JSON contract by which a *generator* describes a legacy granule
(HDF5/NetCDF4, GRIB2) as a set of arrays whose chunks are **byte ranges into
the original file**, so the granule can be served as a cloud-native cube
without rewriting it. The `swath-manifest` crate is this document made
executable: its serde types accept exactly the documents described here, and
its schema snapshot test pins the serialized shape. Where prose and code
could ever disagree, the crate's pinned snapshot is the tie-breaker and the
mismatch is a bug in one of them.

The contract's purpose is generator interchangeability: any two conforming
generators handed the same granule must produce manifests that are
*reference-equivalent* (§6), and a reader serves them identically.

## 1. Document shape

A manifest is a single JSON object:

```json
{
  "manifest_version": 1,
  "generator": "swath-referencer",
  "source": "VNP09GA.A2012019.h33v12.002.2023122182434.h5",
  "arrays": [ ... ]
}
```

| Field | Type | Meaning |
|---|---|---|
| `manifest_version` | integer | Always `1` for this schema. Readers MUST reject any other value loudly; a version they do not understand is never half-parsed. |
| `generator` | string | Which generator produced the manifest (e.g. `swath-referencer`, `virtualizarr`). Informational: excluded from equivalence (§6). |
| `source` | string | The granule this manifest references — path or URI, as given to the generator. |
| `arrays` | array of objects (§2) | The granule's arrays, in the generator's traversal order: depth-first datasets then subgroups for HDF5; message order for GRIB2. |

Unknown fields are rejected at every object level of the document
(`deny_unknown_fields`): a manifest carrying fields this version does not
define is version skew, not noise. (The one deliberate exception is the
nested `transform` object (§4), whose unknown members are ignored.)

## 2. Arrays

Each element of `arrays` is an object:

| Field | Type | Meaning |
|---|---|---|
| `name` | string | The HDF5 path without the leading slash (`HDFEOS/GRIDS/…/SurfReflect_M1_1`), or the cfgrib-style variable name for GRIB2 messages. Names are the join key for equivalence (§6). |
| `shape` | array of non-negative integers | Dimension sizes. Empty for scalars. |
| `chunks` | array of non-negative integers | Chunk shape; equals `shape` for contiguous storage. |
| `dtype` | string | Numpy-style dtype (`int16`, `float64`, `|S32000`, …) — the vocabulary both reference generators derive independently. |
| `codecs` | array of strings | The codec chain, §3. |
| `georef` | object (§4), optional | Spatial identity, when the generator could derive one. Omitted (never `null`) for non-spatial arrays. |
| `refs` | array of objects (§5) | The chunk byte ranges. Unallocated chunks are absent; an array with no allocated storage has an empty list. |

## 3. Codec chain

`codecs` lists the stored encoding in **filter-pipeline (encode) order** —
the order the HDF5 pipeline lists its filters (e.g. `["shuffle", "zlib:4"]`).
A reader decoding a chunk applies the chain in *reverse* (inflate, then
unshuffle). The vocabulary in use:

- `zlib:N` — deflate at level `N` (HDF5 deflate filter);
- `shuffle` — HDF5 byte shuffle;
- `grib2:simple`, `grib2:complex`, `grib2:complex-spatial-diff`,
  `grib2:ieee-float`, `grib2:jpeg2000`, `grib2:png`, `grib2:aec` — GRIB2
  section-5 packing, by data-representation template (0, 2, 3, 4, 40, 41,
  42); `grib2:templateN` for any other template number `N`.

An empty list means the bytes are the raw array data.

## 4. Georeferencing (`georef`)

Everything a server needs to place the array's pixel grid on the planet:

```json
{
  "crs": {"proj4": "+proj=sinu +R=6371007.181 +units=m +no_defs"},
  "transform": {
    "origin_x": 16679257.796, "pixel_width": 926.6254330558333, "row_rotation": 0.0,
    "origin_y": -3335851.559, "col_rotation": 0.0, "pixel_height": -926.6254330558333
  },
  "nodata": -28672.0,
  "band": "SurfReflect_M1_1"
}
```

- **`crs`** — the manifest's own CRS vocabulary, an object with exactly one
  member: `{"epsg": N}` for an EPSG-registered CRS, or
  `{"proj4": "<proj string>"}` for grids with no EPSG registration (VIIRS/
  MODIS-heritage sinusoidal). The identity is recorded losslessly; resolving
  it into projection math is the consumer's concern.
- **`transform`** — the pixel↔CRS affine mapping in GDAL's six-parameter
  convention, top-left anchored: `x = origin_x + col*pixel_width +
  row*row_rotation`, `y = origin_y + col*col_rotation + row*pixel_height`.
  North-up rasters carry a **negative** `pixel_height` and zero rotations.
- **`nodata`** (optional) — the nodata sentinel widened to a float (GDAL
  convention), when the source declares a numeric one (HDF5 `_FillValue`).
- **`band`** (optional) — band semantics: the science name of what the
  samples mean (e.g. `SurfReflect_M1_1`).

Optional fields are omitted when absent, never serialized as `null`.
`georef` is excluded from equivalence (§6): its correctness is asserted by a
generator's own known-answer tests, not cross-generator agreement.

## 5. Chunk references (`refs`)

Each element addresses one stored chunk:

| Field | Type | Meaning |
|---|---|---|
| `key` | string | Dotted chunk-grid position: `"0.0"`, `"1.2"`; `"0"` per rank for whole-array refs; `""` for scalars. |
| `path` | string | The file holding the bytes (usually the manifest's `source`). |
| `offset` | non-negative integer | Byte offset of the chunk within `path`. |
| `length` | non-negative integer | Stored (compressed) length in bytes. |

## 6. Reference equivalence

Two manifests are *reference-equivalent* when they name the same arrays and,
per array (joined by `name`): `shape`, `chunks`, `dtype`, and `codecs` agree,
and the refs agree per `key` on `(offset, length)` with equal ref counts.
`generator`, `source`, ref `path`s, and `georef` are deliberately outside the
comparison — the first three legitimately differ between generators naming
the same granule. The crate's `compare` function is this definition's
executable form; its report lists every observed mismatch.

## 7. Versioning and evolution

The schema is frozen per version. Any change to the fields, their meaning,
or the vocabulary above is a new `manifest_version`, and v1 readers reject
it. Additive-looking changes are not exempt: unknown fields fail parsing by
design, so there is no silent skew between generator and reader.
