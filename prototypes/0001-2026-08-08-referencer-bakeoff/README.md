# Prototype 0001 — Referencer Bake-Off (Python VirtualiZarr vs pure-Rust)

**Started:** 2026-08-08 · **Status:** In progress · **Settles:** ADR 0006

## 1. Question

For turning legacy file archives (NetCDF4/HDF5, GRIB2) into **virtual references** (so we can serve them as
cloud-native cubes without rewriting them), should Swath generate the reference manifest with the **Python
VirtualiZarr** sidecar, a **pure-Rust** generator, or **both behind one port** — and per format, is the
pure-Rust path correct, faster, and worth owning?

## 2. Why now

Legacy virtual-reference generation is the single highest-uncertainty bet in the ingest design. Settling it
first prevents us from committing the rest of the ingest architecture around an assumption that might not
hold. Generation latency is also a direct input to the north-star metric (ingest-to-pixel), so this is not a
side quest.

## 3. The port contract being validated

Both paths implement one interface and must emit the **same manifest**:

```
IngestReferencer: granule (file/URI) -> VirtualManifest
```

`VirtualManifest` (prototype JSON; the production form is Icechunk virtual chunk references):

```json
{
  "generator": "virtualizarr | referencer-rs",
  "source": "path-or-uri",
  "arrays": [
    {
      "name": "SurfReflect_I1",
      "shape": [3232, 3200],
      "chunks": [1616, 1600],
      "dtype": "int16",
      "codecs": ["shuffle", "zlib"],
      "refs": [
        { "key": "0.0", "path": "granule.nc", "offset": 40381, "length": 812345 }
      ]
    }
  ]
}
```

The manifest is the contract (ADR 0001, ADR 0006): whoever generates it, the serve path reads it identically.

## 4. Hypotheses (stated before running)

- **H1.** Both generators produce **equivalent** manifests — identical arrays, chunk grids, and per-chunk
  (offset, length) — for the same VIIRS VNP09 (HDF5) granule.
- **H2.** For **GRIB2**, pure-Rust (`gribberish`) is straightforward and low-risk.
- **H3.** For **HDF5/NetCDF4**, pure-Rust via `hdf5-metno` (`H5Dget_chunk_info`) is correct, and its
  generation latency is **≤** the Python sidecar's (no interpreter/warm-up, no cross-process hop).
- **H4.** The Rust path yields a materially smaller deployment footprint (single binary, no Python env).

## 5. Method (reproducible)

1. Acquire one **VIIRS VNP09** granule (HDF5) and one **GRIB2** sample — see `scripts/fetch_sample.sh`.
2. Generate a manifest three ways: `virtualizarr` (Python sidecar), `referencer-rs --features hdf5` (VIIRS),
   `referencer-rs --features grib` (GRIB2).
3. **Compare** manifests for equivalence with the harness (`swath-referencer-bakeoff compare a.json b.json`).
4. Optionally **resolve-and-render**: read each manifest and render one tile; perceptual-diff the tiles
   (reuses the planned GDAL-oracle harness — out of scope for the first pass, noted for completeness).
5. Record metrics (below) into §7.

## 6. Metrics

| Metric | Why |
|---|---|
| **Manifest equivalence** (arrays / chunk grid / per-chunk offset+length / codecs) | Correctness gate (H1). The equivalence check *is* the conformance test that lets us swap generators safely. |
| **Generation latency** (cold + warm) | Direct ingest-to-pixel input (H3). |
| **Manifest size** | Storage/transfer cost. |
| **Deployment footprint** (binary vs Python env; deps) | Operational simplicity, R8 (H4). |
| **Format robustness** (chunk-index variants, filters: shuffle/deflate) | Where pure-Rust correctness risk lives. |

## 7. Results

*(To be filled in when run, dated. Do not edit prior entries — append.)*

- 2026-08-08 — **GRIB2 (H2): pure-Rust path implemented and equivalent to kerchunk.**
  - **Sample:** `data/gfs_sample.grib2` (2,706,908 bytes, 3 messages: TMP@850mb, UGRD@10m, PRMSL),
    assembled by `scripts/fetch_sample.sh` from `gfs.20260801/00/atmos/gfs.t00z.pgrb2.0p25.f000`
    on the public AWS Open Data bucket (`noaa-gfs-bdp-pds.s3.amazonaws.com`, no auth) — each field
    byte-ranged out via the `.idx` sidecar, so each range is a complete GRIB2 message.
  - **Generators:** Rust `gribberish` 1.6.0 (metadata only — no field decode needed for
    referencing) vs Python sidecar using `kerchunk.grib2.scan_grib` (kerchunk 0.2.10, cfgrib
    0.9.15.1, eccodes 2.48.0). Grouping model per kerchunk: one single-chunk array per message
    (key `0.0`), whole-message byte range, cfgrib variable names (`t`, `u10`, `prmsl` — the Rust
    side carries a small NCEP-abbrev→cfgrib-name table, prototype scope). Codec recorded from
    section 5 independently on both sides (Rust: template number; Python: eccodes `packingType`);
    all three messages are `grib2:complex-spatial-diff` (template 3).
  - **Equivalence (H1-for-GRIB2):** `just compare` → `arrays: A=3 B=3 matched=3`, 0 grid/dtype
    mismatches, 0 chunk mismatches ⇒ **EQUIVALENT** (per-message offset/length identical).
  - **Latency** (Apple M2 Max, local file): referencer-rs <1 ms cold and warm; virtualizarr sidecar
    ~1150 ms cold, ~890–910 ms warm (dominated by Python import + eccodes scan).
  - **Manifest size:** rs 935 B vs vz 934 B (same schema; trailing-newline difference).
  - **Verdict on H2:** supported — gribberish exposes message offset/length/grid/packing directly
    (`read_messages`, `Message::{byte_offset,len,grid_dimensions,data_template_number}`); no custom
    section walker was needed. Residual risk: variable-name vocabulary (NCEP abbrev vs eccodes
    shortName) needs a real table for production parity.

- 2026-08-08 — **HDF5/NetCDF4 (H1, H3): pure-Rust path implemented and equivalent to VirtualiZarr.**
  - **Sample:** `data/VNP09GA.A2012019.h33v12.002.2023122182434.h5` (8,202,915 bytes, HDF-EOS5),
    fetched via earthaccess from LP DAAC. **Substitution:** the plan named VNP09, but VNP09
    collection-002 swath granules are distributed as **HDF4** (MODIS heritage container — verified
    by download: magic `0e 03 13 01`), unreadable by libhdf5/h5py entirely. VNP09GA (daily gridded
    surface reflectance, same instrument/science) is real HDF5 with netCDF-4-style nested groups,
    so the bake-off runs on it; `scripts/fetch_sample.sh` updated accordingly.
  - **Generators:** Rust `hdf5-metno` 0.14.0 with the `static` feature (bundled libhdf5 2.2.0
    built from source — no system HDF5; builds clean on macOS arm64), walking each dataset's chunk
    index with `Dataset::chunks_visit` (H5Dchunk_iter) for per-chunk (logical offset → dotted key,
    file address, stored size); vs Python sidecar using VirtualiZarr 2.7.3's `HDFParser`
    (h5py 3.16.0 backend). Sidecar note: `open_virtual_dataset` cannot represent this granule
    (datasets across nested groups + phony dims with conflicting sizes in one group break the
    xarray merge), so the sidecar walks the parser's `ManifestStore` group tree directly.
    Grouping model: one array per dataset, named by HDF5 path without leading slash
    (`HDFEOS/GRIDS/VIIRS_Grid_1km_2D/Data Fields/SurfReflect_M1_1`); contiguous datasets are one
    whole-storage ref with chunk shape = shape (key `0` per rank, `""` for the scalar
    `StructMetadata.0`, dtype `|S32000`); unallocated datasets (HDF-EOS `Projection` stubs) keep
    empty refs. Codec vocabulary derived independently on both sides (Rust: H5Pget_filter
    pipeline; Python: VirtualiZarr's zarr codecs): `zlib:<level>`, `shuffle`, `fletcher32`, …
  - **Equivalence (H1):** `just compare` → `arrays: A=67 B=67 matched=67`, 0 grid/dtype
    mismatches, 0 chunk mismatches ⇒ **EQUIVALENT** (1,551 chunk refs each, per-chunk
    offset+length identical; codec strings verified identical for all 67 arrays).
  - **Robustness:** granule uses deflate level 8 only (`zlib:8` on 58 arrays; no shuffle in this
    product); 9 arrays uncompressed. All 1,544 chunked-dataset chunk `filter_mask`s are 0
    (verified via h5py `chunk_iter`), so the per-chunk filter-skip case remains unexercised —
    both sides currently record pipeline-level codecs only. Granule quirk survived: QF datasets
    carry `_FillValue=b"N/A"` on uint8, which crashes VirtualiZarr's CF fill-value encoding; the
    sidecar degrades that to no-fill (fill values are outside the manifest contract).
  - **Latency** (Apple M2 Max, local file, dev-profile harness as with the GRIB entry):
    referencer-rs 29 ms cold / 14 ms warm; virtualizarr sidecar 876 ms cold / ~556-574 ms warm
    (Python import + h5py scan). **H3 supported:** Rust ≤ sidecar by ~40x warm.
  - **Manifest size:** rs 201,585 B vs vz 201,584 B (same schema; trailing-newline difference).
  - **Verdict on H1/H3:** supported on this granule — `hdf5-metno` exposes exactly what
    referencing needs (`chunks_visit`, `offset`, `storage_size`, `filters`, type descriptors)
    with no raw FFI required. Residual risks: per-chunk nonzero filter_mask handling untested;
    big-endian/exotic dtypes unmapped (deliberate hard error); the sidecar reaches into
    VirtualiZarr's private `ManifestStore._group` because no public API exposes the raw manifest
    tree.

## 8. Decision

*(To be recorded here and promoted to / reconciled with ADR 0006 when concluded.)*

- Provisional (pre-run): stage Python-first, build Rust behind the same port, sunset per-format as Rust
  reaches parity. This prototype produces the per-format evidence for that plan.

## 9. How to run

```bash
# from this directory
just setup        # create Python venv for the sidecar
just fetch        # download sample granules (needs NASA Earthdata login for VIIRS)
just bakeoff data/VNP09.nc      # run both generators + compare, print report
# or step by step:
just vz-gen   data/VNP09.nc  out/vz.json
just rust-gen data/VNP09.nc  out/rs.json     # requires: cargo build --features hdf5
just compare  out/vz.json    out/rs.json
```

## 10. Scope / non-scope

In scope: manifest generation + equivalence for VIIRS HDF5 and one GRIB2 sample; latency + footprint.
Out of scope (noted for later): full tile render/perceptual-diff, MODIS HDF-EOS, S3-hosted byte-range reads,
production Icechunk manifest emission. This prototype answers "is pure-Rust generation correct and worth
owning, per format" — nothing more.
