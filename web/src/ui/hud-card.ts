// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-hud-card>` (docs/design/ui-system.md §5): a translucent card for
 * the HUD dock — readouts, the trace feed, analytics. `title` and an
 * `actions` slot in the header; `collapsible` makes the header a button
 * that folds the body (`collapsed` reflects, `swath-toggle` fires);
 * `dense` tightens padding. Parts: `base header body`.
 */
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { SwathIcon } from "./icon.js";
import { css } from "./styles.js";

export class SwathHudCard extends SwathElement {
  static override tagName = "swath-hud-card";
  static override styles = [
    css`
      :host { display: block; max-inline-size: 100%; }
      [part="base"] {
        display: flex;
        flex-direction: column;
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-md);
        background: var(--swath-color-bg-hud);
        backdrop-filter: var(--swath-blur-hud);
        color: var(--swath-color-fg);
        box-shadow: var(--swath-shadow-hud);
        overflow: hidden;
      }
      [part="header"] {
        display: flex;
        align-items: center;
        gap: var(--swath-space-2);
        min-block-size: var(--swath-space-7);
        padding: var(--swath-space-1) var(--swath-space-2);
        border: 0;
        background: none;
        color: var(--swath-color-fg-muted);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        font-weight: 700;
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        text-align: start;
      }
      [part="header"]:empty { display: none; }
      button[part="header"] { cursor: pointer; }
      button[part="header"]:hover { color: var(--swath-color-fg); }
      [part="title"] { flex: 1; }
      [part="body"] { padding: var(--swath-space-2); font-size: var(--swath-text-sm); }
      :host([dense]) [part="body"] { padding: var(--swath-space-1) var(--swath-space-2); }
      :host([collapsed]) [part="body"] { display: none; }
      :host([collapsed]) swath-icon { transform: rotate(-90deg); }
    `,
  ];
  static override properties = {
    /** Hidden while the default slot is empty (an inspector card that
     * only has content once a badge is clicked). */
    autoHide: { type: "boolean", attribute: "auto-hide", reflect: true },
    title: { type: "string" },
    collapsible: { type: "boolean", reflect: true },
    collapsed: { type: "boolean", reflect: true },
    dense: { type: "boolean", reflect: true },
  } as const;

  declare autoHide: boolean;
  declare title: string;
  declare collapsible: boolean;
  declare collapsed: boolean;
  declare dense: boolean;

  #header: HTMLElement | undefined;

  constructor() {
    super();
    SwathIcon.define();
  }

  #ensure(): HTMLElement {
    const wantButton = this.collapsible;
    if (this.#header && this.#header instanceof HTMLButtonElement === wantButton) {
      return this.#header;
    }
    const children = [
      el("span", { part: "title" }),
      el("slot", { name: "actions" }),
      wantButton ? el("swath-icon", { name: "chevron-down" }) : null,
    ];
    const header = wantButton
      ? el("button", { part: "header", type: "button" }, ...children)
      : el("div", { part: "header" }, ...children);
    if (wantButton) {
      header.addEventListener("click", () => {
        this.collapsed = !this.collapsed;
        this.emit("swath-toggle", { pressed: !this.collapsed });
      });
    }
    this.#header = header;
    const body = el("slot");
    body.addEventListener("slotchange", () => this.#syncAutoHide());
    this.#header = header;
    this.renderRoot.replaceChildren(
      el("div", { part: "base" }, header, el("div", { part: "body" }, body)),
    );
    return header;
  }

  #syncAutoHide(): void {
    if (!this.autoHide) {
      return;
    }
    const slot = this.renderRoot.querySelector<HTMLSlotElement>("slot:not([name])");
    this.hidden = (slot?.assignedNodes({ flatten: true }).length ?? 0) === 0;
  }

  protected render(): void {
    const header = this.#ensure();
    this.#syncAutoHide();
    const title = header.querySelector('[part="title"]');
    if (title) {
      title.textContent = this.title ?? "";
    }
    if (header instanceof HTMLButtonElement) {
      header.setAttribute("aria-expanded", String(!this.collapsed));
    }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-hud-card": SwathHudCard;
  }
}
