// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-icon name>` (docs/design/ui-system.md §4.6). `<use href="#id">`
 * cannot reach a symbol outside its shadow root, so the sheet is parsed
 * once and the chosen symbol's content is cloned into this element's own
 * root as an inline `<svg part="svg">` drawn in `currentColor`. `label`
 * → `role="img"` + `aria-label`; otherwise decorative (`aria-hidden`).
 */
import { SwathElement } from "./element.js";
import sheet from "./icons.svg?raw";
import { css } from "./styles.js";

const SVG_NS = "http://www.w3.org/2000/svg";

let symbols: Map<string, SVGSymbolElement> | undefined;

function parseSheet(): Map<string, SVGSymbolElement> {
  const doc = new DOMParser().parseFromString(sheet, "image/svg+xml");
  const map = new Map<string, SVGSymbolElement>();
  for (const symbol of doc.querySelectorAll("symbol")) {
    map.set(symbol.id, symbol);
  }
  return map;
}

/** Every icon name the sheet defines (the vitest pins this list). */
export function iconNames(): readonly string[] {
  symbols ??= parseSheet();
  return [...symbols.keys()];
}

export class SwathIcon extends SwathElement {
  static override tagName = "swath-icon";
  static override styles = [
    css`
      :host {
        display: inline-flex;
        inline-size: var(--swath-size-icon);
        block-size: var(--swath-size-icon);
        flex: none;
        vertical-align: middle;
        color: inherit;
      }
      :host([size="sm"]) { inline-size: var(--swath-size-icon-sm); block-size: var(--swath-size-icon-sm); }
      :host([size="lg"]) { inline-size: var(--swath-size-icon-lg); block-size: var(--swath-size-icon-lg); }
      svg {
        inline-size: 100%;
        block-size: 100%;
        display: block;
      }
    `,
  ];
  static override properties = {
    name: { type: "string" },
    label: { type: "string" },
    size: { type: "string", reflect: true },
  } as const;

  declare name: string | undefined;
  declare label: string | undefined;
  declare size: string | undefined;

  protected render(): void {
    symbols ??= parseSheet();
    const symbol = this.name === undefined ? undefined : symbols.get(this.name);
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("part", "svg");
    svg.setAttribute("viewBox", symbol?.getAttribute("viewBox") ?? "0 0 16 16");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "1.5");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    if (symbol) {
      for (const child of symbol.childNodes) {
        svg.append(document.importNode(child, true));
      }
    }
    if (this.label !== undefined && this.label !== "") {
      svg.setAttribute("role", "img");
      svg.setAttribute("aria-label", this.label);
      svg.removeAttribute("aria-hidden");
    } else {
      svg.setAttribute("aria-hidden", "true");
      svg.removeAttribute("role");
    }
    this.renderRoot.replaceChildren(svg);
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-icon": SwathIcon;
  }
}
