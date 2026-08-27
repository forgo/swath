// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-toggle>` (docs/design/ui-system.md §5): a switch. Form-associated
 * through `ElementInternals` (`name` + `checked` → the form value "on"),
 * a real `<button role="switch">` inside for Space and focus, `label` as
 * the accessible name (or the `label` slot for visible text). Parts:
 * `base control track thumb`. Emits `swath-change` on user toggles only.
 */
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

export class SwathToggle extends SwathElement {
  static override tagName = "swath-toggle";
  static formAssociated = true;
  static override shadowOptions: ShadowRootInit = { mode: "open", delegatesFocus: true };
  static override styles = [
    css`
      :host { display: inline-flex; vertical-align: middle; }
      :host([disabled]) { pointer-events: none; opacity: 0.45; }
      [part="base"] {
        display: inline-flex;
        align-items: center;
        gap: var(--swath-space-2);
        min-block-size: var(--swath-space-7);
        cursor: pointer;
        color: var(--swath-color-fg-muted);
        font-size: var(--swath-text-sm);
      }
      [part="control"] {
        display: inline-flex;
        align-items: center;
        padding: 0;
        border: 0;
        background: none;
        color: inherit;
        cursor: pointer;
      }
      [part="track"] {
        display: block;
        inline-size: var(--swath-space-7);
        block-size: var(--swath-space-4);
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-pill);
        background: var(--swath-color-bg-raised);
        position: relative;
        transition: background var(--swath-motion-fast) var(--swath-motion-ease);
      }
      [part="thumb"] {
        position: absolute;
        inset-block-start: var(--swath-space-0);
        inset-inline-start: var(--swath-space-0);
        inline-size: calc(var(--swath-space-4) - 2px);
        block-size: calc(var(--swath-space-4) - 2px);
        border-radius: var(--swath-radius-pill);
        background: var(--swath-color-fg-muted);
        transition: transform var(--swath-motion-fast) var(--swath-motion-ease);
      }
      [aria-checked="true"] [part="track"] {
        background: var(--swath-color-accent-bg);
        border-color: var(--swath-color-accent-border);
      }
      [aria-checked="true"] [part="thumb"] {
        background: var(--swath-color-accent);
        transform: translateX(var(--swath-space-4));
      }
      @media (pointer: coarse) {
        [part="control"] {
          min-block-size: var(--swath-size-target);
          min-inline-size: var(--swath-size-target);
        }
      }
    `,
  ];
  static override properties = {
    checked: { type: "boolean", reflect: true },
    disabled: { type: "boolean", reflect: true },
    label: { type: "string" },
    name: { type: "string", reflect: true },
  } as const;

  declare checked: boolean;
  declare disabled: boolean;
  declare label: string | undefined;
  declare name: string | undefined;

  readonly #internals: ElementInternals;
  #control: HTMLButtonElement | undefined;

  constructor() {
    super();
    this.#internals = this.attachInternals();
  }

  /** Built once so toggling never drops focus. */
  #ensureControl(): HTMLButtonElement {
    if (this.#control) {
      return this.#control;
    }
    const control = el(
      "button",
      { part: "control", type: "button", role: "switch" },
      el("span", { part: "track" }, el("span", { part: "thumb" })),
    );
    control.addEventListener("click", () => {
      if (this.disabled) {
        return;
      }
      this.checked = !this.checked;
      this.emit("swath-change", { name: this.name ?? "", value: this.checked });
    });
    const base = el("label", { part: "base" }, control, el("slot", { name: "label" }));
    this.#control = control;
    this.renderRoot.replaceChildren(base);
    return control;
  }

  protected render(): void {
    const control = this.#ensureControl();
    control.setAttribute("aria-checked", String(this.checked));
    control.disabled = this.disabled;
    if (this.label !== undefined && this.label !== "") {
      control.setAttribute("aria-label", this.label);
    } else {
      control.removeAttribute("aria-label");
    }
    this.#internals.setFormValue(this.checked ? "on" : null);
  }

  /** Form reset returns the switch to its attribute default. */
  formResetCallback(): void {
    this.checked = false;
  }

  formDisabledCallback(disabled: boolean): void {
    this.disabled = disabled;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-toggle": SwathToggle;
  }
}
