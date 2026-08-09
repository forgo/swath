// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-badge>` — the scaffold's proof-of-life component.
 *
 * Deliberately trivial, but it exercises the real conventions every Swath
 * component follows (ADR 0005, forgo-auth lineage): plain Custom Element,
 * light DOM, `observedAttributes` reactivity, no framework. `<swath-map>`
 * replaces this as the first real component (issue #33).
 */
export class SwathBadge extends HTMLElement {
  static readonly tagName = "swath-badge";

  static get observedAttributes(): readonly string[] {
    return ["label"];
  }

  connectedCallback(): void {
    this.render();
  }

  attributeChangedCallback(): void {
    if (this.isConnected) {
      this.render();
    }
  }

  private render(): void {
    const label = this.getAttribute("label") ?? "swath";
    this.textContent = label;
    this.setAttribute("role", "status");
  }
}

export function defineSwathBadge(): void {
  if (!customElements.get(SwathBadge.tagName)) {
    customElements.define(SwathBadge.tagName, SwathBadge);
  }
}
