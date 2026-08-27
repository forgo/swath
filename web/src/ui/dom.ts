// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `el()` — the one DOM helper (docs/design/ui-system.md §4.2). Rendering in
 * Swath elements is imperative; this only saves the createElement /
 * setAttribute / append boilerplate. No templating, no diffing.
 */
export type ElAttrs = Readonly<Record<string, string | number | boolean | null | undefined>>;
export type ElChild = Node | string | null | undefined | false;

/** Create `tag` with `attrs` (true → present, false/null/undefined → absent)
 * and `children` (strings become text nodes; falsy children are skipped). */
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: ElAttrs = {},
  ...children: readonly ElChild[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) {
    if (value === true) {
      node.setAttribute(name, "");
    } else if (value !== false && value !== null && value !== undefined) {
      node.setAttribute(name, String(value));
    }
  }
  for (const child of children) {
    if (child !== null && child !== undefined && child !== false) {
      node.append(child);
    }
  }
  return node;
}
