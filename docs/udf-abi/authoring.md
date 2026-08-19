# Authoring Swath UDFs (ABI v1)

How to write a `run_udf` module against the normative wire contract in
[`v1.md`](v1.md) (ADR 0018). Any toolchain that can emit a **zero-import
`wasm32-unknown-unknown` module with the four exports** can author UDFs;
this guide works through Rust (the supported kit), AssemblyScript, and
TinyGo, and states the v1 caveats a UDF author must know.

Reference implementations live in [`examples/udf/`](../../examples/udf/)
and are proven as committed fixtures by `swath-udf-wasmtime`'s conformance
suite (`tests/abi_fixtures.rs`) under the real deterministic engine.

## The rules every module lives under (v1)

- **Zero imports.** No WASI, no host functions, no clock, no randomness —
  a module importing anything is rejected at registration.
- **Four exports** (`swath_udf_abi` = 1, `swath_udf_output_planes`,
  `swath_udf_alloc`, `swath_udf_run`), signatures per `v1.md` — plus the
  exported linear **`memory`** the host reads and writes buffers through
  (every toolchain exports it by default; a module without it is unusable).
  Avoid exporting anything else.
- **≤ 64 MiB linear memory**, fuel + a 250 ms epoch deadline around every
  run: allocate proportionally to the tile, and keep per-pixel work flat.
- **Dimension-preserving:** output planes have the request's
  `width x height`. Output validity is ANDed with input validity host-side;
  non-finite "valid" outputs are canonicalized to invalid.
- **No parameters, no georeferencing.** The v1 header carries dimensions
  only: sun angles, thresholds, calibration tables are compile-time
  constants of the module. Register variants, not parameterized modules.

## Rust (the supported kit)

`crates/swath-udf-guest` is the authoring floor: strict header
parse/emit, the `swath_udf!` macro producing the four exports, and the
`no_std` runtime (bump allocator, aborting panic handler). A module is
one `#![no_std]` cdylib crate — see
[`examples/udf/ndvi`](../../examples/udf/ndvi/src/lib.rs) (the
dual-implementation oracle against built-in band math) and
[`examples/udf/hillshade`](../../examples/udf/hillshade/src/lib.rs) (a
neighborhood op band math cannot express):

```rust
#![no_std]
extern crate alloc;
use swath_udf_guest::{swath_udf, Plane, Request, Response};

swath_udf! {
    output_planes: |input_planes| i32::from(input_planes == 2),
    run: my_udf,
}

fn my_udf(request: &Request) -> Option<Response> {
    let mut out = Plane::invalid(request.pixels());
    // ... read request.planes[i].values / .validity, fill out ...
    Some(Response { planes: alloc::vec![out] })
}
```

Build: `cargo build --release --target wasm32-unknown-unknown` with the
`examples/udf` profile (size-optimized, `panic = "abort"`, symbols
stripped). The build is byte-reproducible under the pinned toolchain —
CI rebuilds the examples and byte-compares them against the committed
fixtures (`just udf-fixtures-verify`).

`no_std` means no `std` float math (`sqrt`, `sin`, …): precompute
constants at compile time, bring `libm`, or carry a small deterministic
implementation (the hillshade example ships a Newton-iteration `sqrt`).

## Tile seams: neighborhood ops in v1

A UDF sees **one tile and nothing beyond it** — there is no halo of
neighboring pixels in v1 (halo/neighborhood UDFs are a recorded v2 reopen
condition, ADR 0018). Per-pixel math is unaffected. A neighborhood op
(convolution, focal statistics, hillshade) cannot honestly compute its
outermost ring, and must **mark that ring invalid rather than invent
values** — rendered output shows thin invalid seams at tile borders,
matching the hillshade example and its conformance test. That visible,
honest artifact is the documented v1 behavior; do not fake the ring by
clamping or mirroring, which produces silently wrong pixels that vary
with tile alignment.

## AssemblyScript (worked)

Worked source: [`examples/udf/assemblyscript/index.ts`](../../examples/udf/assemblyscript/index.ts)
(the committed `assemblyscript-double.wasm` conformance fixture — prebuilt,
pinned by the conformance suite, deliberately not CI-built). The whole
recipe is one file and one command:

```sh
npx --yes --package assemblyscript@0.28.20 asc index.ts \
  -o module.wasm -O3 --runtime stub --use abort= --noAssert
```

The two flags that make it conform: `--runtime stub` (arena allocator, no
GC machinery — `heap.alloc` backs `swath_udf_alloc`) and `--use abort=`
(replaces the default `env.abort` import with a trap, keeping the module
zero-import). Read and write buffers with `load<T>`/`store<T>`.

## TinyGo (worked)

Worked source: [`examples/udf/tinygo/negate.go`](../../examples/udf/tinygo/negate.go)
(the committed `tinygo-negate.wasm` fixture — same prebuilt discipline).
Exports are `//go:wasmexport` functions; the freestanding target keeps
imports at zero, but two post-processing steps (binaryen/wabt) are needed:

```sh
tinygo build -o raw.wasm -target=wasm-unknown -no-debug -panic=trap .
wasm-ctor-eval raw.wasm --ctors=_initialize -o ctor.wasm
wasm2wat ctor.wasm | grep -vE '\(export "f(min|max)imumf?"' > stripped.wat
wat2wasm stripped.wat -o stripped.wasm
wasm-opt -O2 --remove-unused-module-elements stripped.wasm -o module.wasm
```

TinyGo emits an `_initialize` export that must run before any other —
but the ABI has no init hook, deliberately (a per-instance init step
would be host-observable state). `wasm-ctor-eval` evaluates it at build
time and snapshots the initialized memory into the module, after which
the export is gone. The remaining LLVM helper exports (`fminimum` etc.)
are stripped to keep the export set at the ABI's four plus `memory`.
Toolchain pins for the committed fixture: `examples/udf/README.md`.

## Python: unsupported in v1, deliberately

There is no Python UDF path in v1 and none is implied: `run_udf` accepts
runtime `"wasm"` version `"1"` only. openEO Python UDFs are a different
contract (an interpreter, an environment, a package universe — everything
the zero-import posture exists to exclude); supporting them is a new ADR,
not a v1 extension (recorded as a reopen condition in ADR 0018).

The working stance is **prototype-then-port**: prototype pixel math in
Python against real arrays (xarray/NumPy over the same COG/virtual
sources Swath serves — see `docs/RECIPES.md` for getting identical pixels
client-side), then port the final expression to Rust/AssemblyScript/
TinyGo. The NDVI example is that port pattern in miniature; the
conformance suite is where the ported module gets pinned against the
prototype's numbers.
