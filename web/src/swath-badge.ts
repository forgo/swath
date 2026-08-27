// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-badge>` — the conventions exemplar, now on `SwathElement`
 * (docs/design/ui-system.md §4.2): a static property table instead of
 * `observedAttributes` by hand, a shadow root with the shared sheets, an
 * imperative `render()`, styling only through tokens. Deliberately
 * trivial; `<swath-map>` is the first real component.
 */
import { SwathElement } from "./ui/element.js";
import { css } from "./ui/styles.js";

export class SwathBadge extends SwathElement {
  static override tagName = "swath-badge";
  static override styles = [
    css`
      :host {
        display: inline-block;
        padding: var(--swath-space-0) var(--swath-space-2);
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-pill);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
      }
    `,
  ];
  static override properties = {
    label: { type: "string" },
  } as const;

  declare label: string | undefined;

  override connectedCallback(): void {
    super.connectedCallback();
    this.setAttribute("role", "status");
  }

  protected render(): void {
    this.renderRoot.textContent = this.label ?? "swath";
  }
}

/** Registers `<swath-badge>`; safe to call more than once. */
export function defineSwathBadge(): void {
  SwathBadge.define();
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-badge": SwathBadge;
  }
}
