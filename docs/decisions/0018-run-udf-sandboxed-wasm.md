# ADR 0018 — `run_udf` as sandboxed WASM in the tile path

**Status:** Proposed (maintainer approval pending — issue #199's box) · **Date:** 2026-08-18 ·
**Supersedes:** [ADR 0013](0013-extension-features-plus-openeo-graphs.md) **for the pixel stage
only** · **Extends:** [ADR 0010](0010-openeo-bounded-profile.md) (the way
[ADR 0014](0014-preview-bounded-sync-result.md) did) · **Source:** issue #199, ADR 0013's own
reopen clause, the planning round 3 UDF design · **Normative companion:** `docs/udf-abi/v1.md`

## Context

ADR 0013 committed extension to compile-time crates plus openEO process graphs, and named its
own reopen condition: a needed product that cannot be expressed in the bounded profile, with a
sandboxed **WASM host ABI** as the first candidate mechanism. That condition has tripped where
the wedge analysis said it would (the open-frontier corner of `wedge-a-quadrants.svg`): pixel
math beyond the profile's arithmetic — custom indices, QA-bit logic, per-scene calibration —
has no home today except forking or waiting on the process vocabulary. openEO already has the
standard spelling for exactly this: **`run_udf`**.

The danger is equally well known: user code in the live tile path attacks the three properties
the engine is built on — determinism (byte-identical tiles, golden-testable), latency
(ADR 0012: render runs inline on the async runtime, protected by measured budgets), and the
single static binary (ADR 0002). This ADR admits user code only in the shape that provably
preserves all three.

## Decision

**Adopt openEO `run_udf` executed as sandboxed WASM, per plane, in the live tile path — for
the PIXEL STAGE ONLY.** Sources and kernels remain compile-time crates behind ports: ADR 0013
stands for everything except the pixel stage, and a UDF can never do I/O, name a file, or
touch the network — it maps input planes to output planes and nothing else.

The commitments (each one structural, not policy):

- **Zero-import modules.** A module importing *anything* — WASI, host functions, another
  module — is rejected at registration. Determinism is not a sandbox setting that could drift;
  with no imports there is nothing nondeterministic to call. No clock, no randomness, no I/O
  exists in the module's world.
- **NaN canonicalization on.** The one WASM-spec nondeterminism (NaN payload bits) is
  canonicalized by the runtime, so identical inputs give byte-identical outputs across
  platforms and runtime versions — UDF outputs stay golden-testable.
- **Fuel primary, 250 ms epoch backstop.** Deterministic fuel metering is the budget that
  reproduces (same inputs, same fuel consumed); the wall-clock epoch deadline is the backstop
  that guarantees the ADR 0012 inline-render posture survives a pathological module. Both trip
  into a per-tile UDF error, never a hung worker.
- **64 MiB memory cap** per instance, declared and enforced by the runtime; instantiation
  fails loudly over it.
- **Per-plane granularity.** One guest call per plane (`f64` samples + validity, the
  `WarpedBuffer` shape): boundary-crossing arithmetic rules out per-pixel host calls (256×256
  = 65 536 crossings/plane/tile at ~µs-scale overhead each versus one bulk copy), and
  per-plane is exactly the Render IR's own evaluation granularity. The host **ANDs** the
  module's output validity with the input validity — a UDF can mark pixels invalid, never
  resurrect them.
- **Runtime `"wasm"`, version `"1"`, only.** `run_udf`'s `runtime` argument accepts exactly
  this pair; no Python claim is made or implied (openEO Python UDFs are a different contract —
  a deferred decision, not a degraded one).

The ABI is normative in `docs/udf-abi/v1.md` (manifest-v1 discipline: versioned, deny-unknown,
snapshot-pinned, superseded never edited): four exports —
`swath_udf_abi` (= 1), `swath_udf_output_planes`, `swath_udf_alloc`, `swath_udf_run` — over a
length-prefixed JSON header + `f64` planes + `u8` validity masks.

## Rollback condition

If deployment-representative load shows the ADR 0012 signals tripping under UDF traffic
(p99 inflation / runtime starvation per that ADR's reopen trigger) **and** the executor-seam
mitigation (moving UDF execution behind the existing evaluator seam onto a bounded worker
pool) fails to restore them, `run_udf` is withdrawn from the live tile path (preview-only
until superseded). The seam is kept explicitly so rollback is a wiring change, not a redesign.

## v2 reopen conditions (deferred, recorded — not designed now)

- **Halo/neighborhood UDFs** (kernels need surrounding pixels: convolution, focal stats).
- **`f32` plane transfer** (halves copy cost; only with evidence the copy is the bottleneck).
- **Component model / WASI preview, or a Python UDF story** (changes the zero-import posture —
  a new ADR, not a v1.x).
- Operational deferrals tracked with the roadmap's demand-triggered rows: module-store GC,
  planner fuel-cost feedback (the M9 cost axis learning loop), `Module::serialize` caching.

## Consequences

- ADR 0013's compile-time answer is superseded **only** for the pixel stage; its port/adapter
  and openEO-graph surfaces stand. ADR 0010's bounded profile gains `run_udf` the way ADR 0014
  added the preview: an extension with its own hard bounds, not a loosening.
- The supply chain grows by the WASM runtime (wasmtime) — the written-justification checkpoint
  the #190 zarrs adoption rehearsed, executed at adoption time (#200).
- The determinism story extends to user code: goldens can pin UDF-rendered tiles exactly like
  built-in products.

## Execution (M9, gated on this ADR's approval)

#200 (wasmtime supply-chain review) → #201 (IR op + executor port) → #202 (guest kit) →
#203 (wasmtime adapter: pooled, fueled, deterministic) → #204 (compiler + module store) →
#205 (serve wiring: fuel budget axis, trace fields, cache identity) → #206 (preview) →
#207 (bench + load evidence under the ADR 0012 guard) → #208 (authoring UX) → #209 (SDK +
reference UDFs).
