# swath-udf-guest

Guest-side kit for the Swath UDF ABI v1 (`docs/udf-abi/v1.md`, ADR 0018):
the ABI structs, strict header parse/emit (deny-unknown), and the
`swath_udf!` macro producing the four exports a `run_udf` module needs —
plus the `no_std` `wasm32-unknown-unknown` runtime (bump allocator,
aborting panic handler). Zero dependencies.

Write a `#![no_std]` cdylib crate, decode a `Request`, return a
`Response`, and the macro does the rest. Worked examples (NDVI,
hillshade) live in `examples/udf/`; the authoring guide — including
AssemblyScript/TinyGo and the v1 tile-seam caveat for neighborhood ops —
is `docs/udf-abi/authoring.md`.

The encode/decode halves also run host-side: they build the request
buffers and read the responses in `swath-udf-wasmtime`'s fixture
conformance tests, so host and guest can never drift apart silently.
