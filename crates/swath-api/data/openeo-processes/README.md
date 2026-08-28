# openEO process definitions served by `GET /processes`

Runtime copies of the official openEO process definitions for the subset the
process compiler supports (`swath_render::process` module docs) — the
documents the openEO surface serves from `GET /processes`, embedded via
`include_str!`.

Byte-identical to the compiler's pinned oracle copies in
`crates/swath-render/tests/data/openeo/` (provenance in its README:
openeo-processes **1.2.0**, commit
`d0ce91fcd347360b907ea2d9589d7564a2c1e1e3`); the test suite asserts the two
sets stay identical, so a deliberate re-pin must update both.
(`merge_cubes.json` is pinned ahead of its serving — the compiler admits the
join per ADR 0022; `GET /processes` lists it once issue #297 lands.)

Definitions are served **verbatim** except that, where Swath's v0 profile
narrows a parameter's accepted range, the served document's `description`
gains an appended `**Swath profile:**` note (e.g. `linear_scale_range`'s
output range must be exactly 0..255, `save_result`'s format must be PNG).
The files themselves are never edited — the note is applied at serve time,
and the honesty tests pin it.

License: Apache-2.0 (upstream `LICENSE`); copyright the openEO consortium
contributors. REUSE annotation lives in the repository-root `REUSE.toml`.
