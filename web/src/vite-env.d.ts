// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Vite import-query modules the code relies on (ui-system.md §4):
//
// - `?inline`: a stylesheet's text (MapLibre's CSS self-injected once per
//   document by <swath-map>; `tokens.css` / `base.css` turned into
//   constructed `CSSStyleSheet`s by `ui/styles.ts`).
// - `?raw`: a file's text verbatim (the icon symbol sheet, #280).
// - `?url`: the emitted asset's resolved, hashed URL (the two self-hosted
//   woff2 faces, #379 — an `?inline` stylesheet's `url()` would resolve
//   against the document instead of the bundle).
declare module "*.css?inline" {
  const css: string;
  export default css;
}

declare module "*?raw" {
  const text: string;
  export default text;
}

declare module "*?url" {
  const url: string;
  export default url;
}
