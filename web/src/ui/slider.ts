// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-slider>` (docs/design/ui-system.md §5): a native `<input
 * type=range>` (arrows, Home/End, touch come free) with a value readout.
 * `swath-input` while dragging, `swath-change` on commit. `format` is a
 * property (a function) for the readout — attributes carry `value / min /
 * max / step / label / name`. Parts: `base control value`. `touch-action:
 * pan-y` so a vertical swipe still scrolls the panel the slider sits in.
 */
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

export class SwathSlider extends SwathElement {
  static override tagName = "swath-slider";
  static override shadowOptions: ShadowRootInit = { mode: "open", delegatesFocus: true };
  static override styles = [
    css`
      :host { display: block; }
      :host([disabled]) { opacity: 0.45; pointer-events: none; }
      [part="base"] {
        display: flex;
        align-items: center;
        gap: var(--swath-space-2);
        min-block-size: var(--swath-space-7);
      }
      [part="control"] {
        flex: 1;
        min-inline-size: var(--swath-space-8);
        margin: 0;
        accent-color: var(--swath-color-accent);
        touch-action: pan-y;
        cursor: pointer;
      }
      [part="value"] {
        min-inline-size: var(--swath-space-8);
        text-align: end;
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
        font-variant-numeric: tabular-nums;
      }
      @media (pointer: coarse) {
        [part="control"] { min-block-size: var(--swath-size-target); }
      }
    `,
  ];
  static override properties = {
    value: { type: "number", reflect: true },
    min: { type: "number", reflect: true },
    max: { type: "number", reflect: true },
    step: { type: "number", reflect: true },
    label: { type: "string" },
    name: { type: "string", reflect: true },
    disabled: { type: "boolean", reflect: true },
  } as const;

  declare value: number | undefined;
  declare min: number | undefined;
  declare max: number | undefined;
  declare step: number | undefined;
  declare label: string | undefined;
  declare name: string | undefined;
  declare disabled: boolean;

  /** Readout formatter; defaults to the bare number. */
  format: (value: number) => string = (value) => String(value);

  #control: HTMLInputElement | undefined;
  #readout: HTMLOutputElement | undefined;

  #ensure(): HTMLInputElement {
    if (this.#control) {
      return this.#control;
    }
    const control = el("input", { part: "control", type: "range" });
    const readout = el("output", { part: "value" });
    control.addEventListener("input", () => {
      this.value = control.valueAsNumber;
      this.#paint();
      this.emit("swath-input", { name: this.name ?? "", value: control.valueAsNumber });
    });
    control.addEventListener("change", () =>
      this.emit("swath-change", { name: this.name ?? "", value: control.valueAsNumber }),
    );
    this.#control = control;
    this.#readout = readout;
    this.renderRoot.replaceChildren(el("div", { part: "base" }, control, readout));
    return control;
  }

  #paint(): void {
    if (this.#readout && this.#control) {
      this.#readout.value = this.format(this.#control.valueAsNumber);
    }
  }

  protected render(): void {
    const control = this.#ensure();
    control.min = String(this.min ?? 0);
    control.max = String(this.max ?? 100);
    control.step = String(this.step ?? 1);
    control.disabled = this.disabled;
    if (this.value !== undefined) {
      control.value = String(this.value);
    }
    if (this.label !== undefined && this.label !== "") {
      control.setAttribute("aria-label", this.label);
    } else {
      control.removeAttribute("aria-label");
    }
    this.#paint();
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-slider": SwathSlider;
  }
}
