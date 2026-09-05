<!-- SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
     SPDX-License-Identifier: Apache-2.0 -->

# ADR 0029 — A granule carries every other STAC property, opaquely

**Status:** Accepted · **Date:** 2026-09-04 · **Amends:** ADR 0023 (the catalog domain) ·
**Refs:** `crates/swath-core/src/catalog.rs`, `crates/swath-core/src/catalog/stac.rs`,
`docs/design/catalog-domain.md`, issue #407

## Context

`granule_from_stac_item` read `id`, `collection`, `bbox`, `properties.datetime`, `assets` and our
own `swath:*` keys, and **silently dropped every other property**. `eo:cloud_cover`, `platform`,
`instrument`, `proj:epsg`, `sat:orbit_state`: gone, with no warning, including on items a user pastes
into the add-data panel.

Two consequences, and the second is the one that blocks work:

1. The round-trip identity ADR 0023 implies — a STAC item in is a STAC item out — held only for
   documents Swath itself wrote. For anyone else's items it was quietly false.
2. Nothing can be filtered on what was never kept, so facet discovery (#409) and the properties half
   of search were unbuildable.

## Decision

1. **`Granule` gains `properties: BTreeMap<String, Value>`** — every STAC property the item carried,
   verbatim.

2. **It is OPAQUE.** Swath preserves and serves these keys and **interprets none of them**. No
   validation, no coercion, no schema. That is what keeps the domain honest while making the data
   lossless: the catalog's vocabulary stays exactly the fields ADR 0023 defined, and everything else
   is carried rather than modelled.

3. **Projected keys are excluded.** `datetime` and the `swath:*` namespace are not duplicated into
   the passthrough — they are already domain fields. One authority per fact, so a round-trip cannot
   resurrect a stale copy of something the domain has since changed.

4. **Order is stable.** A `BTreeMap`, so serialization is deterministic and snapshots do not churn.

## Consequences

- A foreign item survives ingest and comes back out unchanged; the round-trip identity becomes true
  in general rather than only for our own documents.
- Facets can be discovered from what items actually carry (#409), and search can filter on them.
- Existing rows read back with an empty map — the field is `#[serde(default)]`, so the migration is a
  no-op rather than a rewrite. A granule with no foreign properties serializes byte-identically to
  before, which the pinned document snapshots assert.
- **Size is unbounded by design, and that is a real risk.** A STAC item with a very large properties
  bag is stored as-is. Swath does not truncate it — silently dropping half a document is the failure
  this ADR exists to end — so the ceiling is the catalog's own row limit, and an item that exceeds it
  fails loudly at write time rather than being quietly trimmed. If that turns out to bite, the answer
  is a documented limit with a refusal, not a silent cap.

## Alternatives considered

**Model the common extensions** (`eo:`, `proj:`, `sat:`) as typed fields. Rejected: it makes the
domain grow with every extension anyone uses, and it still drops the ones we did not anticipate — the
same bug with more code.

**Keep dropping them and document it.** Rejected: the add-data panel accepts pasted items, so this is
data a user handed us and we discarded without saying so.
