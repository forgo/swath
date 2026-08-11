# ADR 0013 — Extension = compile-time features + openEO process graphs (§16.6 confirm-closed)

**Status:** Accepted · **Date:** 2026-08-10 · **Resolves:** ARCHITECTURE §16.6 (and the §14 "OPEN"
marker) · **Evidence:** R9 in practice across the shipped adapter set and the openEO authoring surface

## Context

ARCHITECTURE §14 weighed three mechanisms for third-party extension — (1) compile-time Cargo
features, (2) WASM plug-ins loaded at runtime, (3) sidecar processes over a stable RPC — trading
"single-binary simplicity" against "extend without recompiling". §16.6 asked to confirm the lean:
commit to compile-time for v1 and defer WASM. Meanwhile the system shipped, and shipping answered
the question de facto; this ADR records that answer so the open marker stops contradicting the code.

The evidence is REQUIREMENTS R9 ("extensible without forking: a new source, a new product, a new
backend — without editing the core") exercised in practice:

- **New source / backend = a crate behind a port.** Six port traits in `swath-core` (§6) carry
  seven first-party adapter crates (`swath-source-cog`, `swath-source-virtual`,
  `swath-reproject-proj4rs`, `swath-catalog-pgstac`, `swath-cache-objectstore`,
  `swath-events-filedrop`, `swath-referencer`), each added without touching core logic and wired
  only in `swath-cli`. Cargo features gate optional weight (`embedded-ui`, `legacy-hdf5`), and the
  binary stays single and static (ADR 0002).
- **New product = data, not a plugin.** A custom derived product is an openEO process graph
  published at runtime through the bounded openEO API (ADR 0010) — compiled to the Render IR and
  served through the same tile path, end to end from the authoring panel (#148). No recompile, no
  plugin ABI.
- The Python `VirtualiZarr` sidecar is an ingest-time conformance reference (ADR 0006), not a
  runtime plugin seam; no other sidecar demand has appeared.

## Decision

**The extension mechanism is: compile-time Cargo features/crates for adapters, plus openEO process
graphs at runtime as the primary user-facing extension surface.** WASM plug-ins and RPC sidecars
are **deferred, not rejected** — no host ABI, no plugin loader, no RPC seam is built or promised
for v1. Third parties extend by (a) implementing a port trait in their own crate and rebuilding the
binary, or (b) authoring a process graph against the running server.

## Reopen condition

Reopen (by superseding ADR) when a concrete third party needs to add a **source or kernel and
cannot recompile** — demand for dynamic plugin loading, e.g. operator-installed extensions on
prebuilt binaries or distribution of closed-source adapters — or when a needed product cannot be
expressed in the bounded openEO profile nor reasonably added to the compiled process set. Per §14's
analysis, the first candidate mechanism on reopen is a sandboxed **WASM host ABI** for custom
sources/kernels.

## Consequences

- ARCHITECTURE §14 drops its "OPEN — needs refinement" marker and records this decision; §16.6 is
  Closed-by-ADR with a link here.
- Adding a first-party adapter remains a compile-time act: new crate, port impl, wiring in
  `swath-cli`, optional feature gate.
- The single-static-binary story (ADR 0002) and the "standards as the extension surface" principle
  (ADR 0001, ADR 0010) stand unchanged; anti-lock-in is carried by ports and standard APIs, not by
  a plugin system.
