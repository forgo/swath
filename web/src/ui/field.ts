// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-field>` (docs/design/ui-system.md §5): one labelled native
 * control — `text | number | date | select | checkbox | textarea | file` —
 * form-associated through `ElementInternals`: the control's own validity
 * and the `error` attribute (a server diagnostic routed by `fieldFor`)
 * both surface as the host's validity. `help` (or the `help` slot) is
 * described-by; `options` is a property for selects. `swath-input` live,
 * `swath-change` on commit. Parts: `base label control help error`.
 */
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

export interface FieldOption {
  readonly value: string;
  readonly label: string;
}

export type FieldType = "text" | "number" | "date" | "select" | "checkbox" | "textarea" | "file";

export class SwathField extends SwathElement {
  static override tagName = "swath-field";
  static formAssociated = true;
  static override shadowOptions: ShadowRootInit = { mode: "open", delegatesFocus: true };
  static override styles = [
    css`
      :host { display: block; }
      :host([disabled]) { opacity: 0.45; }
      [part="base"] {
        display: flex;
        flex-direction: column;
        gap: var(--swath-space-1);
      }
      :host([type="checkbox"]) [part="base"] {
        flex-direction: row;
        align-items: center;
        gap: var(--swath-space-2);
        min-block-size: var(--swath-space-7);
      }
      [part="label"] {
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
      }
      [part="control"] {
        box-sizing: border-box;
        inline-size: 100%;
        min-block-size: var(--swath-space-7);
        padding: var(--swath-space-1) var(--swath-space-2);
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-sm);
        background: var(--swath-color-bg);
        color: var(--swath-color-fg);
        font: inherit;
        accent-color: var(--swath-color-accent);
      }
      :host([type="checkbox"]) [part="control"] {
        inline-size: var(--swath-space-4);
        block-size: var(--swath-space-4);
        min-block-size: 0;
        padding: 0;
      }
      textarea[part="control"] { min-block-size: calc(var(--swath-space-8) * 2); resize: vertical; }
      :host([error]) [part="control"] { border-color: var(--swath-color-danger); }
      [part="help"], [part="error"] { font-size: var(--swath-text-xs); color: var(--swath-color-fg-muted); }
      [part="error"] { color: var(--swath-color-danger); }
      [part="help"]:empty, [part="error"]:empty { display: none; }
    `,
  ];
  static override properties = {
    label: { type: "string" },
    help: { type: "string" },
    error: { type: "string", reflect: true },
    type: { type: "string", reflect: true },
    name: { type: "string", reflect: true },
    value: { type: "string" },
    placeholder: { type: "string" },
    required: { type: "boolean", reflect: true },
    disabled: { type: "boolean", reflect: true },
    readonly: { type: "boolean", reflect: true },
  } as const;

  declare label: string | undefined;
  declare help: string | undefined;
  declare error: string | undefined;
  declare type: string | undefined;
  declare name: string | undefined;
  declare value: string | undefined;
  declare placeholder: string | undefined;
  declare required: boolean;
  declare disabled: boolean;
  declare readonly: boolean;

  readonly #internals: ElementInternals;
  #options: readonly FieldOption[] = [];
  #control: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement | undefined;
  #controlType: string | undefined;
  #checked = false;

  constructor() {
    super();
    this.#internals = this.attachInternals();
  }

  get options(): readonly FieldOption[] {
    return this.#options;
  }

  set options(options: readonly FieldOption[]) {
    this.#options = options;
    this.requestUpdate();
  }

  /** Checkbox state (the form value is "on" / absent). */
  get checked(): boolean {
    return this.#checked;
  }

  set checked(checked: boolean) {
    this.#checked = checked;
    this.requestUpdate();
  }

  /** The chosen files of a `file` field. */
  get files(): FileList | null {
    return this.#control instanceof HTMLInputElement ? this.#control.files : null;
  }

  get validity(): ValidityState {
    return this.#internals.validity;
  }

  get validationMessage(): string {
    return this.#internals.validationMessage;
  }

  checkValidity(): boolean {
    return this.#internals.checkValidity();
  }

  reportValidity(): boolean {
    return this.#internals.reportValidity();
  }

  #ensure(): HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement {
    const type = this.type ?? "text";
    if (this.#control && this.#controlType === type) {
      return this.#control;
    }
    const control =
      type === "select"
        ? el("select", { part: "control" })
        : type === "textarea"
          ? el("textarea", { part: "control" })
          : el("input", { part: "control", type });
    control.id = "control";
    control.addEventListener("input", () => this.#onInput("swath-input"));
    control.addEventListener("change", () => this.#onInput("swath-change"));
    this.#control = control;
    this.#controlType = type;
    const base = el(
      "div",
      { part: "base" },
      el("label", { part: "label", for: "control" }),
      control,
      el("div", { part: "help", id: "help" }, el("slot", { name: "help" })),
      el("div", { part: "error", id: "error", role: "alert" }),
    );
    if (type === "checkbox") {
      base.prepend(control);
    }
    this.renderRoot.replaceChildren(base);
    return control;
  }

  #onInput(type: "swath-input" | "swath-change"): void {
    const control = this.#control;
    if (!control) {
      return;
    }
    if (control instanceof HTMLInputElement && control.type === "checkbox") {
      this.#checked = control.checked;
    } else if (!(control instanceof HTMLInputElement && control.type === "file")) {
      this.value = control.value;
    }
    if (type === "swath-input" && this.error !== undefined) {
      this.error = undefined; // the user is fixing it: a stale server note lies
    }
    this.#sync();
    this.emit(type, { name: this.name ?? "", value: this.#eventValue() });
  }

  #eventValue(): string | number | boolean {
    const control = this.#control;
    if (control instanceof HTMLInputElement && control.type === "checkbox") {
      return control.checked;
    }
    if (control instanceof HTMLInputElement && control.type === "number") {
      return control.valueAsNumber;
    }
    return control?.value ?? "";
  }

  /** Form value + validity, from the native control and `error`. */
  #sync(): void {
    const control = this.#control;
    if (!control) {
      return;
    }
    if (control instanceof HTMLInputElement && control.type === "checkbox") {
      this.#internals.setFormValue(control.checked ? "on" : null);
    } else if (control instanceof HTMLInputElement && control.type === "file") {
      this.#internals.setFormValue(control.files?.[0] ?? null);
    } else {
      this.#internals.setFormValue(control.value);
    }
    if (this.error !== undefined && this.error !== "") {
      this.#internals.setValidity({ customError: true }, this.error, control);
    } else if (!control.validity.valid) {
      const flags: ValidityStateFlags = {};
      for (const key of [
        "valueMissing",
        "typeMismatch",
        "rangeUnderflow",
        "rangeOverflow",
        "stepMismatch",
        "badInput",
        "tooLong",
        "tooShort",
        "patternMismatch",
      ] as const) {
        if (control.validity[key]) {
          flags[key] = true;
        }
      }
      this.#internals.setValidity(flags, control.validationMessage, control);
    } else {
      this.#internals.setValidity({});
    }
  }

  protected render(): void {
    const control = this.#ensure();
    const labelEl = this.renderRoot.querySelector('[part="label"]');
    if (labelEl) {
      labelEl.textContent = this.label ?? "";
    }
    const helpEl = this.renderRoot.querySelector('[part="help"]');
    if (helpEl && this.help !== undefined) {
      helpEl.textContent = this.help;
    }
    const errorEl = this.renderRoot.querySelector('[part="error"]');
    if (errorEl) {
      errorEl.textContent = this.error ?? "";
    }
    control.setAttribute("aria-describedby", this.error ? "error help" : "help");
    control.disabled = this.disabled;
    control.required = this.required;
    if (!(control instanceof HTMLSelectElement)) {
      control.readOnly = this.readonly;
    }
    if (control instanceof HTMLSelectElement) {
      control.replaceChildren(
        ...this.#options.map((option) => el("option", { value: option.value }, option.label)),
      );
      control.value = this.value ?? "";
    } else if (control instanceof HTMLInputElement && control.type === "checkbox") {
      control.checked = this.#checked;
    } else if (control instanceof HTMLInputElement && control.type === "file") {
      // A file input's value is the user's alone.
    } else {
      if (this.placeholder !== undefined && !(control instanceof HTMLSelectElement)) {
        control.placeholder = this.placeholder;
      }
      if (control.value !== (this.value ?? "")) {
        control.value = this.value ?? "";
      }
    }
    this.#sync();
  }

  formResetCallback(): void {
    this.value = undefined;
    this.#checked = false;
    this.error = undefined;
    if (this.#control instanceof HTMLInputElement && this.#control.type === "file") {
      this.#control.value = "";
    }
    this.requestUpdate();
  }

  formDisabledCallback(disabled: boolean): void {
    this.disabled = disabled;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-field": SwathField;
  }
}
