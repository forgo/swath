// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-button>` (docs/design/ui-system.md §5): a real `<button>` (or
 * `<a href>`) inside the shadow root so roles and keyboard come free, the
 * host delegating focus. Variants `ghost | solid | accent | danger`, sizes
 * `sm | md`, an optional leading `icon`, `label` for the accessible name
 * (required when the button is icon-only), `pressed` for a toggle button
 * (`aria-pressed`, `swath-toggle` on activation), `disabled`, `href`.
 * Parts: `base label icon`. ≥ 44 px hit target on coarse pointers.
 */
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { SwathIcon } from "./icon.js";
import { css } from "./styles.js";

export class SwathButton extends SwathElement {
  static override tagName = "swath-button";
  static override shadowOptions: ShadowRootInit = { mode: "open", delegatesFocus: true };
  static override styles = [
    css`
      :host { display: inline-flex; vertical-align: middle; }
      :host([disabled]) { pointer-events: none; }
      [part="base"] {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: var(--swath-space-2);
        min-block-size: var(--swath-space-7);
        padding: var(--swath-space-1) var(--swath-space-3);
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-sm);
        background: none;
        color: var(--swath-color-fg-muted);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        font-weight: 700;
        line-height: var(--swath-leading-normal);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        text-decoration: none;
        cursor: pointer;
        transition: background var(--swath-motion-fast) var(--swath-motion-ease),
          color var(--swath-motion-fast) var(--swath-motion-ease);
      }
      :host([size="sm"]) [part="base"] {
        min-block-size: var(--swath-space-6);
        padding: 0 var(--swath-space-2);
      }
      :host(:not([icon])) [part="icon"], :host([icon]) [part="label"]:empty { display: none; }
      [part="base"]:hover { background: var(--swath-color-accent-bg); color: var(--swath-color-fg); }
      :host([variant="solid"]) [part="base"] {
        background: var(--swath-color-bg-raised);
        color: var(--swath-color-fg);
      }
      :host([variant="accent"]) [part="base"],
      [part="base"][aria-pressed="true"] {
        border-color: var(--swath-color-accent-border);
        background: var(--swath-color-accent-bg);
        color: var(--swath-color-accent);
      }
      :host([variant="danger"]) [part="base"] {
        border-color: var(--swath-color-danger);
        color: var(--swath-color-danger);
      }
      [part="base"]:disabled { opacity: 0.45; cursor: default; }
      @media (pointer: coarse) {
        [part="base"] {
          min-block-size: var(--swath-size-target);
          min-inline-size: var(--swath-size-target);
        }
      }
    `,
  ];
  static override properties = {
    variant: { type: "string", reflect: true },
    size: { type: "string", reflect: true },
    icon: { type: "string", reflect: true },
    label: { type: "string" },
    pressed: { type: "boolean", reflect: true },
    disabled: { type: "boolean", reflect: true },
    href: { type: "string" },
  } as const;

  declare variant: string | undefined;
  declare size: string | undefined;
  declare icon: string | undefined;
  declare label: string | undefined;
  declare pressed: boolean;
  declare disabled: boolean;
  declare href: string | undefined;

  #base: HTMLButtonElement | HTMLAnchorElement | undefined;
  /** Once `pressed` has been used the button is a toggle for life: it
   * keeps `aria-pressed="false"` after un-pressing (the attribute is gone
   * by then — a boolean reflects false as absent). */
  #toggle = false;

  constructor() {
    super();
    SwathIcon.define();
  }

  /** Built once, so re-renders never drop focus from the control. */
  #ensureBase(): HTMLButtonElement | HTMLAnchorElement {
    const wantAnchor = this.href !== undefined && this.href !== "";
    if (this.#base && this.#base instanceof HTMLAnchorElement === wantAnchor) {
      return this.#base;
    }
    const icon = el("swath-icon", { part: "icon" });
    const label = el("span", { part: "label" }, el("slot"));
    const base = wantAnchor
      ? el("a", { part: "base" }, icon, label)
      : el("button", { part: "base", type: "button" }, icon, label);
    base.addEventListener("click", () => this.#activate(), { signal: this.disconnected });
    this.#base = base;
    this.renderRoot.replaceChildren(base);
    return base;
  }

  override attributeChangedCallback(attr: string, old: string | null, value: string | null): void {
    super.attributeChangedCallback(attr, old, value);
    if (attr === "pressed" && value !== null) {
      this.#toggle = true;
    }
  }

  #activate(): void {
    if (this.disabled) {
      return;
    }
    if (this.#toggle) {
      this.pressed = !this.pressed;
      this.emit("swath-toggle", { pressed: this.pressed });
    }
  }

  protected render(): void {
    const base = this.#ensureBase();
    const icon = base.querySelector<SwathIcon>('[part="icon"]');
    if (icon) {
      icon.name = this.icon;
    }
    const textual = this.textContent?.trim() !== "";
    if (this.icon !== undefined && !textual && (this.label ?? "") === "") {
      throw new Error(`<swath-button icon="${this.icon}"> is icon-only and needs a label`);
    }
    if (this.label !== undefined && this.label !== "") {
      base.setAttribute("aria-label", this.label);
      base.setAttribute("title", this.label);
    } else {
      base.removeAttribute("aria-label");
      base.removeAttribute("title");
    }
    this.#toggle ||= this.pressed;
    if (this.#toggle) {
      base.setAttribute("aria-pressed", String(this.pressed));
    } else {
      base.removeAttribute("aria-pressed");
    }
    if (base instanceof HTMLAnchorElement) {
      base.href = this.href ?? "";
      base.setAttribute("aria-disabled", String(this.disabled));
      base.tabIndex = this.disabled ? -1 : 0;
    } else {
      base.disabled = this.disabled;
    }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-button": SwathButton;
  }
}
