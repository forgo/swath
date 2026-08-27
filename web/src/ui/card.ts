// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-card>` (docs/design/ui-system.md §5): a bordered surface with
 * `header` / `media` / `footer` slots around the default body. `selected`
 * and `dense` reflect for styling; `interactive` makes the base a
 * focusable `button`-role surface: Enter / Space / tap emit
 * `swath-activate`, a long-press (500 ms, < 8 px drift) emits it with
 * `long: true` (the touch stand-in for a context action). Parts mirror
 * the slots: `base header media body footer`.
 */
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

const LONG_PRESS_MS = 500;
const LONG_PRESS_DRIFT = 8;

export class SwathCard extends SwathElement {
  static override tagName = "swath-card";
  static override shadowOptions: ShadowRootInit = { mode: "open", delegatesFocus: true };
  static override styles = [
    css`
      :host { display: block; }
      [part="base"] {
        display: flex;
        flex-direction: column;
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-md);
        background: var(--swath-color-bg-raised);
        color: var(--swath-color-fg);
        overflow: hidden;
      }
      :host([interactive]) [part="base"] { cursor: pointer; }
      :host([interactive]) [part="base"]:hover { border-color: var(--swath-color-fg-muted); }
      :host([selected]) [part="base"] {
        border-color: var(--swath-color-accent-border);
        background: var(--swath-color-accent-bg);
      }
      [part="header"], [part="body"], [part="footer"] { padding: var(--swath-space-3); }
      :host([dense]) [part="header"], :host([dense]) [part="body"], :host([dense]) [part="footer"] {
        padding: var(--swath-space-2);
      }
      [part="header"] {
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
      }
      [part="media"] { display: block; }
      [part="media"] ::slotted(*) { display: block; inline-size: 100%; }
      [part="footer"] { border-block-start: var(--swath-border-hairline); }
      [part="header"]:not(:has(*)):empty, [part="footer"]:not(:has(*)):empty { display: none; }
    `,
  ];
  static override properties = {
    title: { type: "string" },
    dense: { type: "boolean", reflect: true },
    selected: { type: "boolean", reflect: true },
    interactive: { type: "boolean", reflect: true },
  } as const;

  declare title: string;
  declare dense: boolean;
  declare selected: boolean;
  declare interactive: boolean;

  #base: HTMLDivElement | undefined;
  #heading: HTMLSlotElement | undefined;

  #ensure(): HTMLDivElement {
    if (this.#base) {
      return this.#base;
    }
    const heading = el("slot", { name: "header" });
    const base = el(
      "div",
      { part: "base" },
      el("div", { part: "media" }, el("slot", { name: "media" })),
      el("div", { part: "header" }, heading),
      el("div", { part: "body" }, el("slot")),
      el("div", { part: "footer" }, el("slot", { name: "footer" })),
    );
    let pressed = false;
    base.addEventListener(
      "click",
      () => {
        if (this.interactive && !pressed) {
          this.emit("swath-activate", { id: this.id, long: false });
        }
        pressed = false;
      },
      { signal: this.disconnected },
    );
    base.addEventListener(
      "keydown",
      (event) => {
        if (this.interactive && (event.key === "Enter" || event.key === " ")) {
          event.preventDefault();
          this.emit("swath-activate", { id: this.id, long: false });
        }
      },
      { signal: this.disconnected },
    );
    let timer: number | undefined;
    let origin: { x: number; y: number } | undefined;
    const cancel = (): void => {
      window.clearTimeout(timer);
      timer = undefined;
      origin = undefined;
    };
    base.addEventListener(
      "pointerdown",
      (event) => {
        if (!this.interactive || event.pointerType === "mouse") {
          return;
        }
        origin = { x: event.clientX, y: event.clientY };
        timer = window.setTimeout(() => {
          cancel();
          pressed = true; // the click that follows is the long-press, not a tap
          this.emit("swath-activate", { id: this.id, long: true });
        }, LONG_PRESS_MS);
      },
      { signal: this.disconnected },
    );
    base.addEventListener(
      "pointermove",
      (event) => {
        if (
          origin &&
          Math.hypot(event.clientX - origin.x, event.clientY - origin.y) > LONG_PRESS_DRIFT
        ) {
          cancel();
        }
      },
      { signal: this.disconnected },
    );
    for (const type of ["pointerup", "pointercancel"]) {
      base.addEventListener(type, cancel, { signal: this.disconnected });
    }
    this.#base = base;
    this.#heading = heading;
    this.renderRoot.replaceChildren(base);
    return base;
  }

  protected render(): void {
    const base = this.#ensure();
    if (this.#heading) {
      this.#heading.textContent = this.title ?? "";
    }
    if (this.interactive) {
      base.setAttribute("role", "button");
      base.tabIndex = 0;
      base.setAttribute("aria-pressed", String(this.selected));
    } else {
      base.removeAttribute("role");
      base.removeAttribute("tabindex");
      base.removeAttribute("aria-pressed");
    }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-card": SwathCard;
  }
}
