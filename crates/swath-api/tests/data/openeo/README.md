# openEO API 1.2.0 spec files (pinned truth)

The openEO API specification the conformance smoke tests validate responses
against (REQUIREMENTS.md R5, the same committed-truth pattern as
`../ogc/README.md`). Committed so the suite runs offline and the validation
bar cannot drift under us; refresh deliberately, never automatically.

Provenance — openEO API repository, tag **1.2.0**
(commit `c5a45b4647b06e313a4f099e9119bfa3cca5c6a3`), retrieved 2026-08-09:

- `errors.json` — byte-identical to
  `https://raw.githubusercontent.com/Open-EO/openeo-api/1.2.0/errors.json`,
  the registry of standardized openEO error codes. The error-mapping tests
  pin the codes Swath emits against this file (code exists, HTTP status
  matches).
- `openapi.json` — a **mechanical JSON rendering** of the official
  `https://raw.githubusercontent.com/Open-EO/openeo-api/1.2.0/openapi.yaml`
  (the spec is published as YAML only; the test suite validates with a JSON
  Schema validator). Conversion, reproducible byte-for-byte:

  ```python
  import yaml, json
  class L(yaml.SafeLoader): pass
  # YAML would parse example timestamps into date objects; keep the
  # original scalars (JSON has no date type).
  L.add_constructor('tag:yaml.org,2002:timestamp',
                    lambda l, n: l.construct_scalar(n))
  doc = yaml.load(open('openapi.yaml'), Loader=L)
  json.dump(doc, open('openapi.json', 'w'), indent=2, ensure_ascii=False)
  ```

  No schema content is added, removed, or edited. The conformance tests
  compile response schemas straight out of this document (a path's
  `responses.<code>.content['application/json'].schema`, with
  `#/components/…` references resolved against the document itself).

The response schemas are OpenAPI 3.0 schema objects (a JSON Schema
draft-4-style dialect); the suite compiles them as draft 4 and OpenAPI-only
keywords (`nullable`, `discriminator`, examples) are inert annotations —
they never loosen a `required`/`type`/`enum` assertion, which is what the
tests lean on.

License: Apache-2.0 (upstream `LICENSE`); copyright the openEO consortium
contributors. REUSE annotation lives in the repository-root `REUSE.toml`.
