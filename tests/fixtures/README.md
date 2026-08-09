<!--
SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
SPDX-License-Identifier: Apache-2.0
-->

# Test fixtures: deterministic HLS COG subsets (issue #20, ADR 0004)

Tiny, committed, byte-reproducible windows of one **real** HLS granule. All
integration tests pin to these — no network, no flakes. Total ~1.4 MB.

## Provenance

| | |
|---|---|
| Granule | `HLS.S30.T13SDD.2024158T173909.v2.0` (HLSS30 v2.0, LP DAAC, cloud-hosted) |
| Sensing date | 2024-06-06 (day 158), Sentinel-2 overpass 17:39:09Z |
| MGRS tile | T13SDD — UTM zone 13N (EPSG:32613), southern Colorado |
| Why this scene | 46% spatial coverage: the Sentinel-2 swath edge crosses the tile, so the window contains **real** scene-edge nodata; 2% cloud; clear mountainous land |
| Pixel window | `col_off=1792, row_off=1536, width=512, height=512` into the 3660×3660 source grid (identical window for every band) |
| Window bounds | EPSG:32613 easting 453720–469080, northing 4338600–4353960 (30 m pixels) |

## Files and coverage

| File | Band | dtype | nodata | Covers |
|---|---|---|---|---|
| `hlss30-t13sdd-2024158-b02.tif` | B02 blue | int16 | -9999 | true-color composition |
| `hlss30-t13sdd-2024158-b03.tif` | B03 green | int16 | -9999 | true-color composition |
| `hlss30-t13sdd-2024158-b04.tif` | B04 red | int16 | -9999 | true-color + NDVI |
| `hlss30-t13sdd-2024158-b8a.tif` | B8A narrow NIR | int16 | -9999 | NDVI (the band HLS's own NDVI uses) |
| `hlss30-t13sdd-2024158-fmask.tif` | Fmask QA | uint8 | 255 | categorical/QA masking (nearest overviews) |

Every file additionally covers, by construction:

- **multi-band composition** — all bands share the same window, grid, and
  geotransform, so RGB / NDVI tests compose across files;
- **nodata handling** — ~31% of the window is real swath-edge nodata;
- **UTM → WebMercator reprojection** — native EPSG:32613 CRS and correct
  windowed geotransform are preserved (no resampling, no reprojection).

Each fixture is a proper COG: 256-px internal tiles, deflate, one overview
level (average resampling; nearest for the categorical Fmask).

## Integrity

`manifest.json` records CRS/shape/dtype/nodata/transform per file;
`SHA256SUMS` records checksums (fixtures + manifest). CI and developers run:

```sh
just fixtures-verify   # shasum -c + offline rasterio load against the manifest
```

## Regeneration

```sh
uv run tests/fixtures/make_fixtures.py   # Earthdata credentials in ~/.netrc
```

The script is deterministic (see its docstring): granule, bands, and window
are hard-coded; fixtures are written as fresh datasets so no source metadata
timestamps leak in; rerunning reproduces the committed bytes exactly under
the pinned dependency versions.

## Policy: fixtures are immutable

Once committed, a fixture's bytes never change. Any intentional change (new
window, new granule, new band) means **new files** plus PR discussion — never
an in-place edit — so downstream test expectations and recorded hashes stay
valid across history. `.gitignore` ignores `*.tif` globally; these files are
deliberately exempted via the `!tests/fixtures/*.tif` negation.
