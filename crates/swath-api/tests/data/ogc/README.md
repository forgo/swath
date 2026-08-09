# OGC schemas (committed test data)

Official OGC JSON schemas the conformance smoke tests validate responses
against (REQUIREMENTS.md R5). Committed so the suite runs offline and the
validation bar cannot drift under us; refresh deliberately, never
automatically.

Provenance — fetched 2026-08-08 from `schemas.opengis.net`:

- `tms/*.json` — OGC Two Dimensional Tile Matrix Set and Tile Set Metadata
  2.0 (OGC 17-083r4) JSON schemas, one file per schema, byte-identical to
  `https://schemas.opengis.net/tms/2.0/json/<name>.json`. `tileSet.json` is
  the tileset-metadata schema OGC API - Tiles 1.0 (OGC 20-057)
  requirement `/req/tileset/description` points at; the rest are its
  transitive `$ref` closure (relative refs, resolved by the test suite's
  local retriever).
- `common/*.json` — OGC API - Common Part 1: Core (OGC 19-072) response
  schemas, byte-identical to
  `https://schemas.opengis.net/ogcapi/common/part1/1.0/openapi/schemas/<name>.json`
  (`landingPage.json`, `confClasses.json`, `link.json`, `exception.json`).

Known upstream quirk, kept as-is (files are committed unmodified):
`common/landingPage.json` spells its link item reference `"$href"` instead
of `"$ref"`, so link objects inside a landing page are not transitively
validated by that schema — the suite validates landing-page links against
`common/link.json` separately.

License: OGC Software License 1.0 (`LICENSES/OGC-1.0.txt`), per the REUSE
annotation in `REUSE.toml`.
