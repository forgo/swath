<!--
SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
SPDX-License-Identifier: Apache-2.0
-->

# Test fixtures: deterministic HLS COG subsets (issue #20, ADR 0004)

Tiny, committed, byte-reproducible windows of **real** HLS granules. All
integration tests pin to these — no network, no flakes. Two families: a
single-date scene-edge window (issue #20, ~1.4 MB) and a six-date fire-event
series (issue #179, ~2.3 MB).

## Provenance (single-date, issue #20)

| | |
|---|---|
| Granule | `HLS.S30.T13SDD.2024158T173909.v2.0` (HLSS30 v2.0, LP DAAC, cloud-hosted) |
| Sensing date | 2024-06-06 (day 158), Sentinel-2 overpass 17:39:09Z |
| MGRS tile | T13SDD — UTM zone 13N (EPSG:32613), southern Colorado |
| Why this scene | 46% spatial coverage: the Sentinel-2 swath edge crosses the tile, so the window contains **real** scene-edge nodata; 2% cloud; clear mountainous land |
| Pixel window | `col_off=1792, row_off=1536, width=512, height=512` into the 3660×3660 source grid (identical window for every band) |
| Window bounds | EPSG:32613 easting 453720–469080, northing 4338600–4353960 (30 m pixels) |

### Files and coverage

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

## Fire-event series (issue #179)

Six dates x four bands over the **2024 Park Fire**: pre-fire green, fresh
burn scar, early post-fire. This is the temporal series the M7 time track
demos against (NDVI collapse, dNBR burn severity).

### The fire

| | |
|---|---|
| Event | Park Fire, California — ignited 2024-07-24 in upper Bidwell Park, Chico (Butte County); burned 429,603 acres across Butte and Tehama counties; fourth-largest wildfire in recorded California history; 100% contained 2024-09-26 |
| Citations | [CAL FIRE incident page](https://www.fire.ca.gov/incidents/2024/7/24/park-fire), [Wikipedia: Park Fire](https://en.wikipedia.org/wiki/Park_Fire) |
| Granules | `HLS.S30.T10TFK.<date>.v2.0` (HLSS30 v2.0, LP DAAC, cloud-hosted) — see date table below |
| MGRS tile | T10TFK — UTM zone 10N (EPSG:32610), northern California |
| Pixel window | `col_off=256, row_off=2176, width=256, height=256` into the 3660x3660 source grid (identical window for every date and band) |
| Window bounds | EPSG:32610 easting 607680–615360, northing 4427040–4434720 (30 m pixels) — Cohasset ridge / Ishi Wilderness area, ~40.05N 121.74W, fully inside the burn perimeter |
| Why this window | Largest windowed mean-NDVI drop on the tile (0.73 → 0.30 between Jul 22 and Sep 30) with **zero** cloud, cloud shadow, and nodata on all six dates (per Fmask) |

### Dates and NDVI/NBR progression

Windowed means computed from the committed B04/B8A/B12 fixtures:

| Sensing date | Day | Phase | mean NDVI | mean NBR |
|---|---|---|---|---|
| 2024-06-07 | 159 | pre-fire green | +0.74 | +0.57 |
| 2024-07-22 | 204 | pre-fire, 2 days before ignition | +0.73 | +0.57 |
| 2024-08-16 | 229 | fresh burn scar | +0.27 | −0.11 |
| 2024-09-05 | 249 | burn scar | +0.28 | −0.12 |
| 2024-09-30 | 274 | burn scar, fire contained | +0.30 | −0.11 |
| 2024-10-15 | 289 | early post-fire | +0.30 | −0.07 |

dNBR (pre − post) ≈ 0.69 — "high severity" on the USGS/FIREMON scale.
`hlss30-t10tfk-fire-ndvi-quicklook.png` (checksummed, manifest-exempt)
renders the six NDVI panels side by side, oldest to newest:

![NDVI progression across the 2024 Park Fire](hlss30-t10tfk-fire-ndvi-quicklook.png)

### Files

Per date `<day>` in {2024159, 2024204, 2024229, 2024249, 2024274, 2024289}:

| File | Band | dtype | nodata | Covers |
|---|---|---|---|---|
| `hlss30-t10tfk-<day>-b04.tif` | B04 red | int16 | -9999 | NDVI |
| `hlss30-t10tfk-<day>-b8a.tif` | B8A narrow NIR | int16 | -9999 | NDVI + NBR |
| `hlss30-t10tfk-<day>-b12.tif` | B12 SWIR2 | int16 | -9999 | NBR / dNBR burn severity |
| `hlss30-t10tfk-<day>-fmask.tif` | Fmask QA | uint8 | 255 | QA masking (nearest overviews) |

All 24 files share one window, grid, and geotransform, so temporal tests
compose across dates exactly as multi-band tests compose across bands. Same
COG layout as the single-date family. Regenerate with:

```sh
uv run tests/fixtures/make_fire_fixtures.py   # Earthdata credentials in ~/.netrc
```

## Integrity

`manifest.json` records CRS/shape/dtype/nodata/transform per file;
`SHA256SUMS` records checksums (fixtures + manifest + quicklook). CI and
developers run:

```sh
just fixtures-verify   # shasum -c + offline rasterio load against the manifest
```

## Regeneration

```sh
uv run tests/fixtures/make_fixtures.py        # single-date family
uv run tests/fixtures/make_fire_fixtures.py   # fire-event series
```

Both need Earthdata credentials in `~/.netrc`. The scripts are deterministic
(see their docstrings): granules, bands, and windows are hard-coded; fixtures
are written as fresh datasets so no source metadata timestamps leak in;
rerunning reproduces the committed bytes exactly under the pinned dependency
versions. Each script merges only its own entries into `manifest.json` and
rewrites `SHA256SUMS` over the union, so neither clobbers the other's.

## Policy: fixtures are immutable

Once committed, a fixture's bytes never change. Any intentional change (new
window, new granule, new band) means **new files** plus PR discussion — never
an in-place edit — so downstream test expectations and recorded hashes stay
valid across history. `.gitignore` ignores `*.tif` globally; these files are
deliberately exempted via the `!tests/fixtures/*.tif` negation.
