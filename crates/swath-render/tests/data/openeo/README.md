# openEO process definitions (pinned truth)

The `*.json` files in this directory (not `graphs/`) are the official openEO
process definitions for the subset the process compiler supports, committed
byte-identical from the openEO processes repository:

- Source: <https://github.com/Open-EO/openeo-processes>
- Version: **1.2.0** (the latest stable release)
- Commit: `d0ce91fcd347360b907ea2d9589d7564a2c1e1e3`
- Retrieved: 2026-08-09 from
  `https://raw.githubusercontent.com/Open-EO/openeo-processes/d0ce91fcd347360b907ea2d9589d7564a2c1e1e3/<name>.json`
- License: Apache-2.0 (upstream `LICENSE`); copyright the openEO consortium
  contributors. REUSE annotation lives in the repository-root `REUSE.toml`.

They are the compiler's conformance oracle: `process_compiler.rs` pins the
parameter names, defaults, and semantics the compiler assumes (e.g. `ndvi`'s
`nir`/`red` defaults, `linear_scale_range`'s clip-to-input-range contract) so
a future re-pin that changes the spec fails loudly instead of silently
drifting.

`graphs/` contains process graphs authored for this repository (Swath
copyright, Apache-2.0): the NDVI-two-ways and true-color round-trip inputs.
