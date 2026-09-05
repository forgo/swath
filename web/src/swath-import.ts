// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-import>` (#420): the guided import, and the register it starts
 * from.
 *
 * One input detects the method (`import-model.ts`); when it cannot, it
 * says what it tried and offers the explicit choice. The steps are named
 * and the current one is announced as `swath-import-step`, which the host
 * puts in the URL — so a half-finished import is a link.
 *
 * Nothing here fetches a remote endpoint. The register says which entries
 * this deployment may reach, and the fetch itself is an operator action
 * behind the egress allowlist (ADR 0030 §5); the flow's job is to get an
 * operator to a correct, named, resumable state.
 *
 * Parts: `register entry step input detected undetected choice note`.
 */
import { SwathApi } from "./api.js";
import {
  type Detection,
  detect,
  IMPORT_STEPS,
  type ImportMethod,
  type ImportStep,
  METHOD_LABELS,
  nextStep,
  STEP_LABELS,
  stepFrom,
  undetectedNote,
} from "./import-model.js";
import {
  billingWarning,
  blockedNote,
  EMPTY_REGISTER,
  parseRegister,
  type Register,
  type RegisterRow,
} from "./register-model.js";
import { SwathButton } from "./ui/button.js";
import { el } from "./ui/dom.js";
import { SwathElement } from "./ui/element.js";
import { SwathField } from "./ui/field.js";
import { css } from "./ui/styles.js";

export class SwathImport extends SwathElement {
  static override tagName = "swath-import";
  static override styles = [
    css`
      :host { display: block; }
      [part="steps"] {
        display: flex;
        gap: var(--swath-space-2);
        margin-block-end: var(--swath-space-3);
        font-size: var(--swath-text-xs);
        font-family: var(--swath-font-mono);
        color: var(--swath-color-fg-muted);
      }
      [part="step"][aria-current="step"] { color: var(--swath-color-accent); }
      [part="register"] {
        display: grid;
        gap: var(--swath-space-2);
        margin: 0 0 var(--swath-space-3);
        padding: 0;
        list-style: none;
      }
      [part="entry"] {
        display: grid;
        gap: var(--swath-space-1);
        padding: var(--swath-space-2);
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-md);
        font-size: var(--swath-text-sm);
      }
      [part="entry"][data-allowed="false"] { color: var(--swath-color-fg-muted); }
      [part="note"], [part="undetected"] {
        margin: 0;
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
      }
      [part="undetected"] { color: var(--swath-color-danger); }
      [part="choice"] {
        display: flex;
        flex-wrap: wrap;
        gap: var(--swath-space-2);
        margin-block-start: var(--swath-space-2);
      }
      [part="detected"] { margin: 0; font-size: var(--swath-text-sm); }
    `,
  ];
  static override properties = {
    active: { type: "boolean", reflect: true },
    step: { type: "string", reflect: true },
    server: { type: "string" },
  } as const;

  declare active: boolean;
  declare step: string | undefined;
  declare server: string | undefined;

  #api: SwathApi | undefined;
  #register: Register = EMPTY_REGISTER;
  #loaded = false;
  #input = "";
  #detection: Detection | undefined;
  /** The method an operator picked after detection failed. */
  #chosen: ImportMethod | undefined;
  #field: SwathField | undefined;

  set api(value: SwathApi) {
    this.#api = value;
  }

  get api(): SwathApi {
    this.#api ??= new SwathApi({ base: this.server ?? "" });
    return this.#api;
  }

  /** The register as served (test seam). */
  get register(): Register {
    return this.#register;
  }

  /** What the current input was detected as (test seam). */
  get detection(): Detection | undefined {
    return this.#detection;
  }

  /** The current step, always one of the named ones. */
  get current(): ImportStep {
    return stepFrom(this.step);
  }

  async refresh(): Promise<void> {
    this.#loaded = true;
    try {
      this.#register = parseRegister(await this.api.json("/sources/register"));
    } catch {
      // A register we could not read is not a register that is empty: the
      // rows are none either way, but nothing is claimed reachable.
      this.#register = EMPTY_REGISTER;
    }
    this.requestUpdate();
  }

  /** Moves to `step` and announces it, so the host can put it in the URL
   * and the flow becomes resumable. */
  goTo(step: ImportStep): void {
    this.step = step;
    this.emit("swath-import-step", { step });
    this.requestUpdate();
  }

  /** Applies pasted or dropped text as the one detecting input. */
  applyInput(text: string): void {
    this.#input = text;
    this.#detection = detect(text);
    this.#chosen = undefined;
    this.requestUpdate();
  }

  /** The method an operator chose after detection failed. */
  chooseMethod(method: ImportMethod): void {
    this.#chosen = method;
    this.requestUpdate();
  }

  /** What the flow will import as: what was detected, or what was
   * chosen when detection failed. */
  get method(): ImportMethod | undefined {
    if (this.#detection?.ok === true) {
      return this.#detection.method;
    }
    return this.#chosen;
  }

  #steps(): HTMLElement {
    const current = this.current;
    return el(
      "nav",
      { part: "steps", "aria-label": "Import steps" },
      ...IMPORT_STEPS.map((step) =>
        el(
          "span",
          {
            part: "step",
            // `data-import-step`, not `data-step`: the authoring canvas
            // owns `data-step` for its own nodes, and a shared name
            // means a global query picks up whichever is in the DOM.
            "data-import-step": step,
            ...(step === current ? { "aria-current": "step" } : {}),
          },
          STEP_LABELS[step],
        ),
      ),
    );
  }

  #registerList(): HTMLElement {
    const rows = this.#register.rows;
    if (rows.length === 0) {
      return el(
        "p",
        { part: "note" },
        this.#loaded
          ? "This deployment offers no endpoints yet — an operator adds them to the config."
          : "—",
      );
    }
    return el("ul", { part: "register" }, ...rows.map((row) => this.#entry(row)));
  }

  #entry(row: RegisterRow): HTMLElement {
    const blocked = blockedNote(row, this.#register);
    const item = el(
      "li",
      { part: "entry", "data-entry": row.id, "data-allowed": String(row.allowed) },
      el("span", {}, row.title),
    );
    const billing = billingWarning(row);
    if (billing !== undefined) {
      item.append(el("p", { part: "note" }, billing));
    }
    if (blocked !== undefined) {
      item.append(el("p", { part: "note" }, blocked));
    } else {
      const use = el("swath-button", { size: "sm", label: `Use ${row.title}` });
      use.textContent = "Use this";
      use.addEventListener("click", () => {
        // The register fills the same one input the paste does: one
        // path through the flow, whichever way you started.
        this.applyInput(row.url);
        this.goTo("review");
      });
      item.append(use);
    }
    return item;
  }

  #inputField(): SwathField {
    if (this.#field !== undefined) {
      this.#field.value = this.#input;
      return this.#field;
    }
    const field = el("swath-field", {
      type: "text",
      name: "import",
      label: "Paste a link or a STAC document",
      placeholder: 'https://… or {"type":"Catalog",…}',
      part: "input",
      value: this.#input,
    });
    field.addEventListener("swath-change", (event) => {
      event.stopPropagation();
      this.applyInput(String(event.detail.value));
    });
    this.#field = field;
    return field;
  }

  #detectionBlock(): HTMLElement | undefined {
    const detection = this.#detection;
    if (detection === undefined) {
      return undefined;
    }
    if (detection.ok) {
      return el(
        "p",
        { part: "detected", role: "status" },
        `That looks like ${METHOD_LABELS[detection.method]} — ${detection.title}.`,
      );
    }
    // Graceful: the reason, what was ruled out, and the explicit choice.
    const block = el(
      "div",
      {},
      el("p", { part: "undetected", role: "alert" }, undetectedNote(detection)),
    );
    const choice = el("div", { part: "choice" });
    for (const method of Object.keys(METHOD_LABELS) as ImportMethod[]) {
      const button = el("swath-button", {
        size: "sm",
        label: `It is ${METHOD_LABELS[method]}`,
        "data-method": method,
        ...(this.#chosen === method ? { pressed: true } : {}),
      });
      button.textContent = METHOD_LABELS[method];
      button.addEventListener("click", () => this.chooseMethod(method));
      choice.append(button);
    }
    block.append(choice);
    return block;
  }

  protected render(): void {
    if (this.active && !this.#loaded) {
      void this.refresh();
    }
    const children: HTMLElement[] = [this.#steps()];
    if (this.current === "source") {
      children.push(this.#registerList(), this.#inputField());
      const detection = this.#detectionBlock();
      if (detection !== undefined) {
        children.push(detection);
      }
      if (this.method !== undefined) {
        const next = el("swath-button", { variant: "primary", label: "Continue" });
        next.textContent = "Continue";
        next.addEventListener("click", () => {
          const step = nextStep("source");
          if (step !== undefined) {
            this.goTo(step);
          }
        });
        children.push(next);
      }
    } else {
      const method = this.method;
      children.push(
        el(
          "p",
          { part: "detected" },
          method === undefined
            ? "Nothing chosen yet — go back and paste a link or a document."
            : `Importing ${METHOD_LABELS[method]}.`,
        ),
      );
      const back = el("swath-button", { size: "sm", label: "Back" });
      back.textContent = "Back";
      back.addEventListener("click", () => this.goTo("source"));
      children.push(back);
    }
    this.renderRoot.replaceChildren(...children);
  }
}

export function defineSwathImport(): void {
  SwathButton.define();
  SwathField.define();
  SwathImport.define();
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-import": SwathImport;
  }
}
