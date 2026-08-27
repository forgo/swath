// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-rail>` (docs/design/ui-system.md §5/§6): the left column —
 * `brand` slot (title, Share), the mode switcher (`items` prop, `mode`
 * reflects the active one, `swath-mode-change` on a pick), the default
 * slot for mode content, a `footer` slot. `collapsed` narrows it from
 * `--swath-size-rail` to `--swath-size-rail-icon` (icons only; the host
 * persists the preference). ↑↓ move between modes, Enter/Space pick.
 * Parts: `base nav item content`.
 */
import { SwathButton } from "./button.js";
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { SwathIcon } from "./icon.js";
import { css } from "./styles.js";

export interface RailItem {
  readonly id: string;
  readonly label: string;
  readonly icon: string;
}

export class SwathRail extends SwathElement {
  static override tagName = "swath-rail";
  static override styles = [
    css`
      :host {
        box-sizing: border-box;
        display: flex;
        flex-direction: column;
        inline-size: var(--swath-size-rail);
        min-inline-size: var(--swath-size-rail);
        block-size: 100%;
        background: var(--swath-color-bg);
        color: var(--swath-color-fg);
        border-inline-end: var(--swath-border-hairline);
        overflow: hidden;
        transition: inline-size var(--swath-motion-normal) var(--swath-motion-ease),
          min-inline-size var(--swath-motion-normal) var(--swath-motion-ease);
      }
      :host([collapsed]) {
        inline-size: var(--swath-size-rail-icon);
        min-inline-size: var(--swath-size-rail-icon);
      }
      [part="base"] { display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; }
      [part="brand"] { padding: var(--swath-space-3) var(--swath-space-3) var(--swath-space-2); }
      :host([collapsed]) [part="brand"] { display: none; }
      [part="nav"] {
        display: flex;
        flex-direction: column;
        gap: var(--swath-space-1);
        margin: 0;
        padding: var(--swath-space-1) var(--swath-space-2);
        list-style: none;
      }
      [part="item"] {
        display: flex;
        align-items: center;
        gap: var(--swath-space-2);
        inline-size: 100%;
        min-block-size: var(--swath-space-8);
        padding: 0 var(--swath-space-2);
        border: var(--swath-border-hairline);
        border-color: transparent;
        border-radius: var(--swath-radius-sm);
        background: none;
        color: var(--swath-color-fg-muted);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        font-weight: 700;
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        text-align: start;
        cursor: pointer;
      }
      [part="item"]:hover { background: var(--swath-color-accent-bg); color: var(--swath-color-fg); }
      [part="item"][aria-current="page"] {
        border-color: var(--swath-color-accent-border);
        background: var(--swath-color-accent-bg);
        color: var(--swath-color-accent);
      }
      :host([collapsed]) [part="item"] { justify-content: center; padding: 0; }
      :host([collapsed]) [part="item"] span { display: none; }
      [part="content"] {
        flex: 1;
        min-block-size: 0;
        overflow-y: auto;
        padding: var(--swath-space-2) var(--swath-space-3);
      }
      :host([collapsed]) [part="content"] { display: none; }
      [part="footer"] { padding: var(--swath-space-2) var(--swath-space-3); }
      [part="collapse"] { align-self: flex-end; margin: 0 var(--swath-space-2) var(--swath-space-2); }
      :host([collapsed]) [part="collapse"] { align-self: center; margin-inline: 0; }
      :host([collapsed]) [part="footer"] { display: none; }
    `,
  ];
  static override properties = {
    collapsed: { type: "boolean", reflect: true },
    mode: { type: "string", reflect: true },
  } as const;

  declare collapsed: boolean;
  declare mode: string | undefined;

  #items: readonly RailItem[] = [];
  #nav: HTMLUListElement | undefined;
  #collapse: SwathButton | undefined;

  constructor() {
    super();
    SwathIcon.define();
    SwathButton.define();
  }

  get items(): readonly RailItem[] {
    return this.#items;
  }

  set items(items: readonly RailItem[]) {
    this.#items = items;
    this.requestUpdate();
  }

  #ensure(): HTMLUListElement {
    if (this.#nav) {
      return this.#nav;
    }
    const nav = el("ul", { part: "nav", role: "list" });
    nav.addEventListener("keydown", (event) => this.#onKey(event));
    const collapse = el("swath-button", { part: "collapse", icon: "chevron-left", size: "sm" });
    collapse.addEventListener("click", () => {
      this.collapsed = !this.collapsed;
      this.emit("swath-toggle", { pressed: this.collapsed });
    });
    this.#nav = nav;
    this.#collapse = collapse;
    this.renderRoot.replaceChildren(
      el(
        "div",
        { part: "base" },
        el("div", { part: "brand" }, el("slot", { name: "brand" })),
        el("nav", { "aria-label": "Mode" }, nav),
        el("div", { part: "content" }, el("slot")),
        el("div", { part: "footer" }, el("slot", { name: "footer" })),
        collapse,
      ),
    );
    return nav;
  }

  #buttons(): HTMLButtonElement[] {
    return [...(this.#nav?.querySelectorAll<HTMLButtonElement>('[part="item"]') ?? [])];
  }

  #onKey(event: KeyboardEvent): void {
    const buttons = this.#buttons();
    const current = buttons.indexOf(this.renderRoot.activeElement as HTMLButtonElement);
    let next: number | undefined;
    if (event.key === "ArrowDown") {
      next = (current + 1) % buttons.length;
    } else if (event.key === "ArrowUp") {
      next = (current - 1 + buttons.length) % buttons.length;
    } else if (event.key === "Home") {
      next = 0;
    } else if (event.key === "End") {
      next = buttons.length - 1;
    }
    if (next === undefined) {
      return;
    }
    event.preventDefault();
    for (const [index, button] of buttons.entries()) {
      button.tabIndex = index === next ? 0 : -1;
    }
    buttons[next]?.focus();
  }

  protected render(): void {
    const nav = this.#ensure();
    nav.replaceChildren(
      ...this.#items.map((item, index) => {
        const active = item.id === this.mode;
        const button = el(
          "button",
          {
            part: "item",
            type: "button",
            "data-mode": item.id,
            "aria-current": active ? "page" : false,
            "aria-label": item.label,
            title: item.label,
            tabindex: active || (this.mode === undefined && index === 0) ? 0 : -1,
          },
          el("swath-icon", { name: item.icon }),
          el("span", {}, item.label),
        );
        button.addEventListener("click", () => {
          if (this.mode !== item.id) {
            this.mode = item.id;
            this.emit("swath-mode-change", { mode: item.id });
          }
        });
        return el("li", {}, button);
      }),
    );
    if (this.#collapse) {
      this.#collapse.icon = this.collapsed ? "chevron-right" : "chevron-left";
      this.#collapse.label = this.collapsed ? "Expand the rail" : "Collapse the rail";
      this.#collapse.pressed = this.collapsed;
    }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-rail": SwathRail;
  }
}
