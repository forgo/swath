# Example UDF modules (Swath UDF ABI v1)

Reference `run_udf` modules for the wire contract in
[`docs/udf-abi/v1.md`](../../docs/udf-abi/v1.md); the authoring guide is
[`docs/udf-abi/authoring.md`](../../docs/udf-abi/authoring.md). All five
are committed as conformance fixtures under
`crates/adapters/swath-udf-wasmtime/tests/fixtures/` and proven there by
`tests/abi_fixtures.rs` under the real deterministic engine. The three
Rust modules are additionally the #209 golden set: their **outputs** over
committed input tiles are pinned byte-for-byte
(`tests/fixtures/golden/`, asserted by `tests/golden_outputs.rs`;
recapture deliberately with `just udf-goldens`).

## Rust (this directory's cargo workspace — CI-rebuilt)

- **`ndvi/`** — the dual-implementation oracle: the same
  `(nir - red) / (nir + red)` the engine expresses as built-in band math,
  as a UDF, so the two paths can be pinned against each other bit-exactly.
- **`hillshade/`** — Horn hillshade: a 3x3 neighborhood op that band math
  cannot express. Its outermost pixel ring is marked invalid (no halo in
  ABI v1 — the documented tile-seam caveat; see the authoring guide).
- **`qamask/`** — Fmask-style QA cloud mask: bitwise tests over QA words
  riding in `f64` planes — logic band math cannot express — with strict
  integer-representability validity (fractions, negatives, out-of-range,
  and non-finite words are invalid, never guessed at).

This is a standalone wasm32 workspace (excluded from the root workspace).
Regenerate the committed fixtures with `just udf-fixtures`; CI rebuilds
from source and byte-compares on every rust PR (`just udf-fixtures-verify`)
— the pinned toolchain makes the build byte-reproducible across hosts
(macOS-arm64 and Linux-x86_64 verified identical).

## AssemblyScript and TinyGo (prebuilt fixtures — language neutrality)

- **`assemblyscript/index.ts`** -> `assemblyscript-double.wasm`
  (doubles every sample, validity passthrough)
- **`tinygo/negate.go`** -> `tinygo-negate.wasm`
  (negates every sample, validity passthrough)

Deliberately **not** CI-built: they exist to pin that the ABI is
language-neutral, and their committed bytes are kept honest by the
conformance suite (zero imports, export set + signatures, correct
behavior, determinism), not by rebuild. Build commands live at the top of
each source file; the pinned toolchains that produced the committed
fixtures:

| fixture | toolchain |
|---|---|
| `assemblyscript-double.wasm` | assemblyscript 0.28.20 (`asc`, via npx) |
| `tinygo-negate.wasm` | tinygo 0.41.1 (go 1.24.5, LLVM 20.1.1), binaryen version_124 (`wasm-ctor-eval`, `wasm-opt`), wabt (`wasm2wat`/`wat2wasm`) |
