<!-- SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
     SPDX-License-Identifier: Apache-2.0 -->

# The self-hosted typefaces

Two faces ship inside the binary: **Space Grotesk** for the UI, **Space Mono** for readouts. ADR 0021
forbids loading fonts from a CDN — the objection is to the third-party host on every page load, not
to the typeface — so they are vendored here and served from the same origin as everything else.

Both are SIL Open Font License 1.1. See `LICENSES/OFL-1.1.txt`; the REUSE annotation is in
`REUSE.toml`.

## Provenance

| File | Upstream |
|---|---|
| `space-grotesk-latin.woff2` | `ofl/spacegrotesk/SpaceGrotesk[wght].ttf` |
| `space-mono-latin-400.woff2` | `ofl/spacemono/SpaceMono-Regular.ttf` |
| `space-mono-latin-700.woff2` | `ofl/spacemono/SpaceMono-Bold.ttf` |

All from `https://github.com/google/fonts`, `main` branch.

## Reproducing them

`just fonts` downloads the upstream files and re-runs the subsetter. The character set is Google
Fonts' own `latin` range **plus `U+2192`**: the right arrow is not in that range (only `U+2191` and
`U+2193` are), and the product writes `ingest→pixel` in mono in the status bar. Without it the arrow
alone would fall back to a system face — one mismatched glyph in the most-read readout in the
product.

Space Grotesk keeps its `wght` variable axis (300–700), so one file covers every weight the UI uses.

The output is deterministic for a given fonttools version, which the recipe pins. If a rebuild
produces different bytes, that is the toolchain moving, not the fonts — check the version before
committing the change.
