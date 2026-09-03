// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The two self-hosted faces (issue #379).
 *
 * ADR 0021 §4 forbids web fonts *from a CDN* — the objection is the
 * third-party host on every page load, not the typeface. These ship inside
 * the binary like every other asset, same origin, no network beyond the one
 * the page already made.
 *
 * Built in TypeScript rather than as a `fonts.css` imported `?inline`: an
 * inline CSS import hands back the text untouched, so a relative `url()` in
 * it would resolve against the document rather than against the hashed file
 * the bundler emits. Importing each file `?url` puts the resolved path in the
 * one place that knows it.
 *
 * Provenance, the character set, and how to rebuild these: `fonts/README.md`.
 *
 * Exported as text, not as a constructed sheet: `styles.ts` owns sheet
 * construction and caching, and importing `css` from here would close a
 * module cycle whose only symptom would be a temporal-dead-zone error at
 * first paint.
 */
import grotesk from "./fonts/space-grotesk-latin.woff2?url";
import mono400 from "./fonts/space-mono-latin-400.woff2?url";
import mono700 from "./fonts/space-mono-latin-700.woff2?url";

/** Google Fonts' `latin` range plus `U+2192` — see `fonts/README.md`. */
const LATIN =
  "U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191-2193, U+2212, U+2215, U+FEFF, U+FFFD";

/** The `@font-face` rules; `styles.ts` adopts them at document level ahead
 * of the tokens.
 *
 * `font-display: swap` on purpose: the fallback stacks in
 * `--swath-font-ui`/`--swath-font-mono` are real faces, so text is readable
 * from the first paint and reflows once. A blocking face would trade a
 * legible page for a consistent one. */
export const facesCss = `
  @font-face {
    font-family: "Space Grotesk";
    font-style: normal;
    /* One variable file covers every weight the UI uses. */
    font-weight: 300 700;
    font-display: swap;
    src: url(${grotesk}) format("woff2");
    unicode-range: ${LATIN};
  }
  @font-face {
    font-family: "Space Mono";
    font-style: normal;
    font-weight: 400;
    font-display: swap;
    src: url(${mono400}) format("woff2");
    unicode-range: ${LATIN};
  }
  @font-face {
    font-family: "Space Mono";
    font-style: normal;
    font-weight: 700;
    font-display: swap;
    src: url(${mono700}) format("woff2");
    unicode-range: ${LATIN};
  }
`;
