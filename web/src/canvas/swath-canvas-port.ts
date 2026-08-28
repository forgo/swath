// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-canvas-port>` (issue #290): one connection point on a node.
 * `side` (`input` | `output`), `name`, `label`. Two ways to connect, both
 * ending in `swath-port-connect-end` that the CONSUMER answers (no port
 * typing here): drag-to-connect (pointer down on a port, up on another)
 * and tap-to-connect (first tap / Enter arms, the second completes, Esc
 * cancels). 44 px hit box on coarse pointers. Parts: `base control`.
 */
import { el } from "../ui/dom.js";
import { SwathElement } from "../ui/element.js";
import { css } from "../ui/styles.js";

export class SwathCanvasPort extends SwathElement {
  static override tagName = "swath-canvas-port";
  static override shadowOptions: ShadowRootInit = { mode: "open", delegatesFocus: true };
  static override styles = [
    css`
      :host { display: inline-flex; touch-action: none; }
      [part="control"] {
        display: inline-grid;
        place-items: center;
        inline-size: var(--swath-space-5);
        block-size: var(--swath-space-5);
        padding: 0;
        border: 0;
        background: none;
        color: inherit;
        cursor: crosshair;
      }
      [part="base"] {
        inline-size: var(--swath-space-3);
        block-size: var(--swath-space-3);
        border: 2px solid var(--swath-color-fg-muted);
        border-radius: var(--swath-radius-pill);
        background: var(--swath-color-bg-raised);
      }
      :host([armed]) [part="base"], [part="control"]:hover [part="base"] {
        border-color: var(--swath-color-accent);
        background: var(--swath-color-accent-bg);
      }
      :host([armed]) [part="base"] { box-shadow: 0 0 0 4px var(--swath-color-accent-bg); }
      @media (pointer: coarse) {
        [part="control"] { inline-size: var(--swath-size-target); block-size: var(--swath-size-target); }
      }
    `,
  ];
  static override properties = {
    side: { type: "string", reflect: true },
    name: { type: "string", reflect: true },
    label: { type: "string" },
    /** The tap-to-connect state: this port is the pending source. */
    armed: { type: "boolean", reflect: true },
  } as const;

  declare side: string | undefined;
  declare name: string | undefined;
  declare label: string | undefined;
  declare armed: boolean;

  #control: HTMLButtonElement | undefined;

  get ref(): { node: string; port: string; side: "input" | "output" } {
    const node = this.closest("swath-canvas-node");
    return {
      node: node?.getAttribute("node-id") ?? "",
      port: this.name ?? "",
      side: this.side === "input" ? "input" : "output",
    };
  }

  #ensure(): HTMLButtonElement {
    if (this.#control) {
      return this.#control;
    }
    const control = el("button", { part: "control", type: "button" }, el("span", { part: "base" }));
    // Drag-to-connect starts here; the canvas tracks the pointer and ends
    // it on whatever port is under the release point.
    control.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) {
        return;
      }
      event.stopPropagation();
      event.preventDefault();
      this.emit("swath-port-connect-start", this.ref);
    });
    // Enter / a synthetic click (detail 0): arm or complete (the canvas
    // holds the armed port). A pointer's click (detail ≥ 1) is not a tap
    // here — the canvas already handled its pointerup, and Chromium does
    // not always deliver a click after a touch pan anyway.
    control.addEventListener("click", (event) => {
      event.stopPropagation();
      if (event.detail === 0) {
        this.emit("swath-port-tap", this.ref);
      }
    });
    this.#control = control;
    this.renderRoot.replaceChildren(control);
    return control;
  }

  protected render(): void {
    const control = this.#ensure();
    control.setAttribute(
      "aria-label",
      `${this.label ?? this.name ?? "port"} (${this.side ?? "output"})`,
    );
    control.setAttribute("aria-pressed", String(this.armed));
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-canvas-port": SwathCanvasPort;
  }
}
