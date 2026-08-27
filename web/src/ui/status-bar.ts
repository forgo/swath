// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-status-bar>` + `<swath-status-cell>` (docs/design/ui-system.md
 * §5/§6): the 24 px strip under the map — lat/lon · zoom · CRS ·
 * ingest→pixel. Cells take `label` + `value` (`mono` for figures). The bar
 * is presentational; the shell feeds the values. Parts: `base label value`.
 */
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

export class SwathStatusBar extends SwathElement {
  static override tagName = "swath-status-bar";
  static override styles = [
    css`
      :host {
        box-sizing: border-box;
        display: flex;
        align-items: center;
        gap: var(--swath-space-4);
        block-size: var(--swath-size-statusbar);
        padding: 0 var(--swath-space-3);
        border-block-start: var(--swath-border-hairline);
        background: var(--swath-color-bg);
        color: var(--swath-color-fg-muted);
        font-size: var(--swath-text-xs);
        overflow: hidden;
        white-space: nowrap;
      }
    `,
  ];

  protected render(): void {
    if (this.renderRoot.childElementCount === 0) {
      this.renderRoot.replaceChildren(el("slot"));
    }
  }
}

export class SwathStatusCell extends SwathElement {
  static override tagName = "swath-status-cell";
  static override styles = [
    css`
      :host { display: inline-flex; }
      [part="base"] { display: inline-flex; align-items: baseline; gap: var(--swath-space-1); }
      [part="label"] {
        font-family: var(--swath-font-mono);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
      }
      [part="label"]:empty { display: none; }
      [part="value"] { color: var(--swath-color-fg); font-variant-numeric: tabular-nums; }
      :host([mono]) [part="value"] { font-family: var(--swath-font-mono); }
      [part="value"]:empty::before { content: "—"; color: var(--swath-color-fg-muted); }
    `,
  ];
  static override properties = {
    label: { type: "string" },
    value: { type: "string" },
    mono: { type: "boolean", reflect: true },
  } as const;

  declare label: string | undefined;
  declare value: string | undefined;
  declare mono: boolean;

  protected render(): void {
    if (this.renderRoot.childElementCount === 0) {
      this.renderRoot.replaceChildren(
        el("span", { part: "base" }, el("span", { part: "label" }), el("span", { part: "value" })),
      );
    }
    const label = this.renderRoot.querySelector('[part="label"]');
    const value = this.renderRoot.querySelector('[part="value"]');
    if (label) {
      label.textContent = this.label ?? "";
    }
    if (value) {
      value.textContent = this.value ?? "";
    }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-status-bar": SwathStatusBar;
    "swath-status-cell": SwathStatusCell;
  }
}
