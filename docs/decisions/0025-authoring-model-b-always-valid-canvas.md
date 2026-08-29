# ADR 0025 — Authoring model B: the always-valid canvas

**Status:** Accepted · **Date:** 2026-08-29 (records the maintainer's selection of 2026-08-11 in
`docs/design/authoring-ux.md` §8; #340) · **Refs:** `docs/design/authoring-ux.md` (the two
models, the bad-state family B1–B11), ADR 0014 (preview via the bounded `POST /result`),
ADR 0021 (the UI system), ADR 0022 (the DAG extends this invariant to a join), issue #151

## Context

The first authoring panel (#148) generated its palette and forms from `GET /processes` alone
and validated before submit, but a user could still reach eleven bad states the compiler would
reject — a missing `save_result` tail, a dangling `divide`, a multi-band colormap, an unknown
band. Issue #151 required comparing at least two interaction models before any implementation
issue was filed: an outcome-first wizard with an expert escape hatch (Model A), or one editor
whose invariant is that the pipeline is never invalid (Model B). The maintainer chose B on
2026-08-11; M10 and M11 built on it.

## Decision

1. **One editor, never invalid.** The canvas always ends in a non-removable Output card
   (`save_result`); a step can be inserted only where its input type exists, so the palette
   shows only what fits the selected point; values are vocabulary-only (band selects over the
   chosen collection, served enums for format, colormap greyed until the result is one value
   per pixel); there are no dangling steps.
2. **Arithmetic lives inside a formula builder**, the sub-editor that owns a reducer's child
   graph; `divide` and friends never appear at the top level.
3. **The client mirrors the compiler's stage discipline with a small pinned table**, tested on
   both sides and e2e-proven (every insertion the palette offers must publish). The server's
   diagnostics remain the backstop; a server-side `POST /validation` may replace the table later
   without changing this decision.
4. **Templates are the entry point** (NDVI, then more), so an outcome-first first minute is a
   layer on this editor, not a second surface.
5. **Preview before publish** goes through the bounded synchronous `POST /result` subset
   (ADR 0014), never a second rendering path.

Rejected: Model A — its escape hatch preserves the whole bad-state family for anyone who leaves
the wizard, and its best ideas fit inside B as template chips and card widgets; the reverse is
not true.

## Consequences

- Bad states B1–B10 are unconstructible; B5 (a two-channel picture) is a pre-submit narrative
  with submit gated, because which fix is the user's call.
- The stage table is a deliberate duplication of compiler semantics; drift is caught by the
  pinned tests on both sides, and the cost is accepted for the user-facing guarantee.
- ADR 0022 restates the invariant for a graph (one join, orphans shown never dropped) rather
  than reopening this decision.
