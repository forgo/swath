# ADR 0020 — Publish the UDF guest kit: `swath-udf-guest` is the fifth crate

**Status:** Accepted (maintainer, 2026-08-19 — name approved in issue #209's execution) ·
**Date:** 2026-08-19 · **Extends:** [ADR 0016](0016-extraction-boundary-published-crates.md)
(the way [ADR 0014](0014-preview-bounded-sync-result.md) extends
[ADR 0010](0010-openeo-authoring-surface.md)) · **Source:** issue #209, ADR 0018's guest kit
(#202)

## Context

ADR 0016 drew the extraction boundary — four crates ship because nothing on crates.io does
what they do; everything else stays home — and its consequences fixed the rule for what comes
after: future extraction questions "are answered by the same test, or they supersede this
ADR", and widening the set silently is expressly a reopen condition, not a default. The #202
guest kit (`crates/swath-udf-guest`) landed unpublished, its manifest deferring the SDK
question to #209. M9 ("Run their code, live") now needs UDF authors outside this repository
to `cargo add` the kit rather than vendor it.

## Decision

**Apply ADR 0016's own boundary test to the guest kit, on the record: it passes, and
`swath-udf-guest` publishes as the fifth crate.** Nothing on crates.io implements the
guest side of Swath UDF ABI v1 (`docs/udf-abi/v1.md`, ADR 0018) — the ABI structs, the
strict deny-unknown header codec, and the `swath_udf!` export macro with the `no_std`
`wasm32-unknown-unknown` runtime; issue #209 is the demand record (authoring without the kit
means hand-rolling the wire contract). Every ADR 0016 discipline holds unchanged:

- **Standalone rule** — the kit has zero dependencies and depends on no workspace crate,
  published or not; `swath-udf-wasmtime` (unpublished) consumes it, never the reverse.
- **Zero new supply-chain surface** — publishing adds no dependency to the vetted graph.
- **Namespace posture** — the in-repo name is adopted as-is (maintainer-approved, #209) and
  claimed only with real content, by its real publishing PR's release, never as a reservation.
- **0.x-alpha semver, MSRV, REUSE** — the workspace lockstep version, inherited MSRV policy,
  and standing-alone REUSE compliance (packaged `LICENSE`, per-crate `REUSE.toml`) apply
  exactly as ADR 0016 defines them; graduation stays per-crate via RELEASING.md's checklist.
- **Operational meaning of "publish"** — the existing crates tier, unchanged: `publish-dry`
  in `just check`, the tag-triggered dry-run + semver-report workflow, maintainer-executed
  `cargo publish`, no credentials in CI.

## Consequences

- The published set is now **five**: swath-manifest, swath-planner, swath-referencer,
  swath-warp, swath-udf-guest. Prose stating the count (ENGINEERING.md §7, RELEASING.md,
  `justfile`, `release-plz.toml`, workflow and manifest comments) reads five and cites this
  ADR's issue.
- ADR 0016's boundary sentence and its test survive intact — this is the "same test" path its
  consequences prescribed, exercised and recorded rather than a quiet widening. Future
  additions still answer that test on the record, or supersede.
- The reference UDFs (`examples/udf/`) double as the kit's published proof: their outputs are
  pinned byte-for-byte as the #209 golden set under the deterministic engine.
