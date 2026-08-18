<!--
SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
SPDX-License-Identifier: Apache-2.0
-->

# swath-referencer

A pure-Rust virtual-reference generator: point it at a legacy granule —
HDF5/NetCDF4 (`.h5`/`.hdf5`/`.nc`/`.nc4`) or GRIB2 (`.grib2`/`.grb2`/`.grib`)
— and it emits a versioned manifest of byte-range references
(kerchunk-style) into the original file. A metadata walk only: chunk
indexes, filter pipelines, and grid metadata are read; pixel data never is.

Extracted from [Swath](https://github.com/forgo/swath) per
[ADR 0016](https://github.com/forgo/swath/blob/main/docs/decisions/0016-extraction-boundary-published-crates.md).
Status: `0.x` alpha — built from tagged commits through the full CI gate,
with no API-stability promise between alphas.

## Measured

On the conformance granule (an 8 MB VNP09GA VIIRS HDF-EOS5 granule: 67
arrays, 1,551 chunk refs), generation takes
<!-- number:ref-warm-ms -->13.8 ms<!-- /number:ref-warm-ms --> warm
(median) against the Python VirtualiZarr sidecar's
<!-- number:ref-sidecar-warm-ms -->545.6 ms<!-- /number:ref-sidecar-warm-ms --> —
<!-- number:ref-ratio -->39.5×<!-- /number:ref-ratio --> faster, with
byte-identical output. Method, environment, and raw numbers live in the
committed artifact
([`docs/perf/referencer-baseline.json`](https://github.com/forgo/swath/blob/main/docs/perf/referencer-baseline.json));
these figures are generated from it and CI-checked against it — never
hand-typed.

## Library

```rust
use swath_referencer::{ReferencerError, SwathReferencer};

fn main() -> Result<(), ReferencerError> {
    let manifest = SwathReferencer::new().generate("granule.h5".as_ref())?;
    println!("{}", manifest.to_json_string());
    Ok(())
}
```

Errors follow a three-way taxonomy: `Unsupported` (this generator
deliberately does not map that — never a guessed manifest), `Malformed`
(the granule is broken), `Backend` (the machinery failed).

## Cargo features

- `legacy-hdf5` (default): HDF5/NetCDF4 via `hdf5-metno`'s statically
  bundled libhdf5 — no system HDF5 required, at the cost of a bundled C
  build. Without it the crate is pure Rust end to end, `handles()`
  declines `.h5`/`.nc`, and `generate` fails loudly naming the feature.
  GRIB2 (`gribberish`, pure Rust) is always on.
- `cli`: the `swath-referencer` binary — granule in, manifest JSON to
  stdout or `--output`. Off by default so library consumers never pull
  clap.

```sh
cargo install swath-referencer --features cli
swath-referencer granule.h5 --output granule.vmanifest.json
```

## Conformance

The claim is byte-range **equivalence with VirtualiZarr**, and it is
executable, not aspirational:

- **Known-answer suite** (runs in every CI build): the committed
  [`tests/data/tiny.h5`](https://github.com/forgo/swath/tree/main/crates/swath-referencer/tests/data)
  fixture must match `tiny.expected.json`, whose offsets and codecs were
  derived independently via h5py's chunk index.
- **Real-granule gate**: [`just test-referencer`](https://github.com/forgo/swath/blob/main/justfile)
  runs this crate and the
  [VirtualiZarr sidecar](https://github.com/forgo/swath/tree/main/python)
  on a real VNP09GA granule and asserts equivalence with
  `vmanifest-compare` (ships with this crate): `vmanifest-compare a.json
  b.json` prints per-array/per-chunk mismatches and exits non-zero on any.
- **Bring your own generator**: emit the v1 manifest schema and run
  `vmanifest-compare` against this crate's output — the same harness, no
  Swath required.

This crate is **wasmtime-free forever**: a guard test fails the build if a
WebAssembly runtime ever enters its dependency tree.

The manifest vocabulary lives in the sibling
[`swath-manifest`](https://github.com/forgo/swath/tree/main/crates/swath-manifest)
crate (its normative spec included) and is re-exported here as
`swath_referencer::manifest`.

## License

Apache-2.0.
