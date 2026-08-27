// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-menu>` (docs/design/ui-system.md §5): a popover list of actions
 * behind a slotted trigger. `items` is a property; `open` reflects. Roving
 * focus with ↑↓ Home End, typeahead by first letters, Enter/Space select,
 * Esc / outside click / selection close. Presentation follows the
 * viewport: a popover under the trigger, or a bottom sheet below the
 * `narrow` breakpoint (reflected as `presentation`). Long-press on the
 * trigger opens too (touch parity). Parts: `base trigger list item`.
 */
import { BREAKPOINTS } from "./breakpoints.js";
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { SwathIcon } from "./icon.js";
import { css } from "./styles.js";

export interface MenuItem {
  readonly id: string;
  readonly label: string;
  readonly icon?: string | undefined;
  readonly disabled?: boolean | undefined;
  readonly danger?: boolean | undefined;
}

const LONG_PRESS_MS = 500;
const LONG_PRESS_DRIFT = 8;

export class SwathMenu extends SwathElement {
  static override tagName = "swath-menu";
  static override styles = [
    css`
      :host { display: inline-block; position: relative; }
      [part="trigger"] { display: inline-flex; }
      [part="list"] {
        position: absolute;
        inset-inline-end: 0;
        inset-block-start: 100%;
        z-index: var(--swath-z-drawer);
        min-inline-size: calc(var(--swath-space-8) * 4);
        margin: var(--swath-space-1) 0 0;
        padding: var(--swath-space-1);
        list-style: none;
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-md);
        background: var(--swath-color-bg-raised);
        box-shadow: var(--swath-shadow-hud);
      }
      :host([presentation="sheet"]) [part="list"] {
        position: fixed;
        inset: auto 0 0 0;
        margin: 0;
        border-radius: var(--swath-radius-md) var(--swath-radius-md) 0 0;
        padding-block-end: env(safe-area-inset-bottom, 0);
      }
      [part="item"] {
        display: flex;
        align-items: center;
        gap: var(--swath-space-2);
        inline-size: 100%;
        min-block-size: var(--swath-space-7);
        padding: var(--swath-space-1) var(--swath-space-2);
        border: 0;
        border-radius: var(--swath-radius-sm);
        background: none;
        color: var(--swath-color-fg);
        font: inherit;
        text-align: start;
        cursor: pointer;
      }
      [part="item"]:hover, [part="item"]:focus-visible { background: var(--swath-color-accent-bg); }
      [part="item"][data-danger] { color: var(--swath-color-danger); }
      [part="item"]:disabled { opacity: 0.45; cursor: default; }
      @media (pointer: coarse) {
        [part="item"] { min-block-size: var(--swath-size-target); }
      }
    `,
  ];
  static override properties = {
    open: { type: "boolean", reflect: true },
    label: { type: "string" },
    presentation: { type: "string", reflect: true },
  } as const;

  declare open: boolean;
  declare label: string | undefined;
  declare presentation: string | undefined;

  #items: readonly MenuItem[] = [];
  #list: HTMLUListElement | undefined;
  #session: AbortController | undefined;
  #typed = "";
  #typedAt = 0;

  get items(): readonly MenuItem[] {
    return this.#items;
  }

  set items(items: readonly MenuItem[]) {
    this.#items = items;
    this.requestUpdate();
  }

  constructor() {
    super();
    SwathIcon.define();
  }

  #ensure(): HTMLUListElement {
    if (this.#list) {
      return this.#list;
    }
    const trigger = el("div", { part: "trigger" }, el("slot", { name: "trigger" }));
    const list = el("ul", { part: "list", role: "menu", hidden: true });
    trigger.addEventListener("click", () => this.toggle());
    this.#wireLongPress(trigger);
    list.addEventListener("keydown", (event) => this.#onKey(event));
    this.#list = list;
    this.renderRoot.replaceChildren(el("div", { part: "base" }, trigger, list));
    return list;
  }

  #wireLongPress(trigger: HTMLElement): void {
    let timer: number | undefined;
    let origin: { x: number; y: number } | undefined;
    const cancel = (): void => {
      window.clearTimeout(timer);
      timer = undefined;
      origin = undefined;
    };
    trigger.addEventListener("pointerdown", (event) => {
      if (event.pointerType === "mouse") {
        return;
      }
      origin = { x: event.clientX, y: event.clientY };
      timer = window.setTimeout(() => {
        cancel();
        this.show();
      }, LONG_PRESS_MS);
    });
    trigger.addEventListener("pointermove", (event) => {
      if (
        origin &&
        Math.hypot(event.clientX - origin.x, event.clientY - origin.y) > LONG_PRESS_DRIFT
      ) {
        cancel();
      }
    });
    for (const type of ["pointerup", "pointercancel"]) {
      trigger.addEventListener(type, cancel);
    }
  }

  toggle(): void {
    if (this.open) {
      this.close("select");
    } else {
      this.show();
    }
  }

  show(): void {
    this.presentation = window.matchMedia(`(min-width: ${BREAKPOINTS.narrow}px)`).matches
      ? "popover"
      : "sheet";
    this.open = true;
  }

  close(reason: "esc" | "outside" | "select"): void {
    if (!this.open) {
      return;
    }
    this.open = false;
    this.emit("swath-drawer-close", { reason });
  }

  #buttons(): HTMLButtonElement[] {
    return [...(this.#list?.querySelectorAll<HTMLButtonElement>('[part="item"]:enabled') ?? [])];
  }

  #focusAt(index: number): void {
    const buttons = this.#buttons();
    const target = buttons.at(((index % buttons.length) + buttons.length) % buttons.length);
    for (const button of buttons) {
      button.tabIndex = button === target ? 0 : -1;
    }
    target?.focus();
  }

  #onKey(event: KeyboardEvent): void {
    const buttons = this.#buttons();
    const current = buttons.indexOf(this.renderRoot.activeElement as HTMLButtonElement);
    switch (event.key) {
      case "ArrowDown":
        this.#focusAt(current + 1);
        break;
      case "ArrowUp":
        this.#focusAt(current - 1);
        break;
      case "Home":
        this.#focusAt(0);
        break;
      case "End":
        this.#focusAt(-1);
        break;
      case "Escape":
        this.close("esc");
        break;
      default: {
        if (event.key.length !== 1 || event.altKey || event.ctrlKey || event.metaKey) {
          return;
        }
        const now = performance.now();
        this.#typed = now - this.#typedAt < LONG_PRESS_MS ? this.#typed + event.key : event.key;
        this.#typedAt = now;
        const prefix = this.#typed.toLowerCase();
        const start = this.#typed.length === 1 ? current + 1 : current;
        const hit = [...buttons.slice(start), ...buttons.slice(0, start)].find((button) =>
          (button.textContent ?? "").trim().toLowerCase().startsWith(prefix),
        );
        if (hit) {
          this.#focusAt(buttons.indexOf(hit));
        }
        break;
      }
    }
    event.preventDefault();
  }

  #startSession(): void {
    this.#session?.abort();
    const session = new AbortController();
    this.#session = session;
    const signal = AbortSignal.any([session.signal, this.disconnected]);
    document.addEventListener(
      "pointerdown",
      (event) => {
        if (!event.composedPath().includes(this)) {
          this.close("outside");
        }
      },
      { signal, capture: true },
    );
    document.addEventListener(
      "keydown",
      (event) => {
        if (event.key === "Escape") {
          this.close("esc");
        }
      },
      { signal },
    );
  }

  protected render(): void {
    const list = this.#ensure();
    if (this.label !== undefined && this.label !== "") {
      list.setAttribute("aria-label", this.label);
    }
    list.replaceChildren(
      ...this.#items.map((item) => {
        const button = el(
          "button",
          {
            part: "item",
            type: "button",
            role: "menuitem",
            "data-id": item.id,
            "data-danger": item.danger === true,
            disabled: item.disabled === true,
            tabindex: -1,
          },
          item.icon ? el("swath-icon", { name: item.icon }) : null,
          item.label,
        );
        button.addEventListener("click", () => {
          this.emit("swath-menu-select", { id: item.id });
          this.close("select");
        });
        return el("li", { role: "none" }, button);
      }),
    );
    list.hidden = !this.open;
    if (this.open) {
      if (!this.#session) {
        this.#startSession();
      }
      this.#focusAt(0);
    } else {
      this.#session?.abort();
      this.#session = undefined;
    }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-menu": SwathMenu;
  }
}
