// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * Layout breakpoints (docs/design/ui-system.md §4.1). `@media` cannot read
 * custom properties, so these are TS constants mirrored in the comment at
 * the top of tokens.css; the shell uses container queries where a value
 * must be shared with CSS. Pixel widths are the viewport's.
 */
export const BREAKPOINTS = {
  /** Below: phone layout — bottom tab bar, sheets instead of drawers. */
  narrow: 640,
  /** Below: rail collapses to icons, inspector becomes a drawer. */
  medium: 1024,
  /** At or above: rail, map and inspector all open at once. */
  wide: 1280,
} as const;

export type Breakpoint = keyof typeof BREAKPOINTS;

/** A `matchMedia` query string for "viewport at least this breakpoint". */
export function minWidth(bp: Breakpoint): string {
  return `(min-width: ${BREAKPOINTS[bp]}px)`;
}

/** The layout tier the viewport is in (ui-system.md §6's table). */
export type LayoutTier = "wide" | "medium" | "narrow" | "phone";

export function layoutTier(width: number): LayoutTier {
  if (width >= BREAKPOINTS.wide) {
    return "wide";
  }
  if (width >= BREAKPOINTS.medium) {
    return "medium";
  }
  if (width >= BREAKPOINTS.narrow) {
    return "narrow";
  }
  return "phone";
}
