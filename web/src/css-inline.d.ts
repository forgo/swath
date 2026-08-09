// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Vite's `?inline` CSS imports resolve to the stylesheet text (used to
// self-inject MapLibre's CSS once per document — consumers need zero setup).
declare module "*.css?inline" {
  const css: string;
  export default css;
}
