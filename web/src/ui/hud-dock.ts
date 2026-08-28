// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-hud-dock>` (docs/design/ui-system.md §5/§6): the overlay grid
 * over the map with eight named slots — `top-left top-center top-right
 * left right bottom-left bottom-center bottom-right`. The dock itself is
 * `pointer-events: none` (the map underneath stays interactive); slotted
 * cards get `auto`. It sits OUTSIDE the map element so it never joins the
 * map's internal stacking order. Parts: `base corner`.
 */
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

export const DOCK_SLOTS = [
  "top-left",
  "top-center",
  "top-right",
  "left",
  "right",
  "bottom-left",
  "bottom-center",
  "bottom-right",
] as const;
export type DockSlot = (typeof DOCK_SLOTS)[number];

export class SwathHudDock extends SwathElement {
  static override tagName = "swath-hud-dock";
  static override styles = [
    css`
      :host {
        display: block;
        position: absolute;
        inset: 0;
        z-index: var(--swath-z-hud);
        pointer-events: none;
      }
      :host([collapsed]) [part="corner"]:not([data-slot="top-right"]) { display: none; }
      [part="base"] {
        display: grid;
        grid-template-columns: auto 1fr auto;
        grid-template-rows: auto 1fr auto;
        gap: var(--swath-space-2);
        block-size: 100%;
        padding: var(--swath-space-2);
      }
      /* Explicit cells: eight corners auto-placed into a 3 × 3 grid skip the
       * centre and slide the second row over (left → centre, …). */
      [data-slot="top-left"] { grid-area: 1 / 1; }
      [data-slot="top-center"] { grid-area: 1 / 2; }
      [data-slot="top-right"] { grid-area: 1 / 3; }
      [data-slot="left"] { grid-area: 2 / 1; }
      [data-slot="right"] { grid-area: 2 / 3; }
      [data-slot="bottom-left"] { grid-area: 3 / 1; }
      [data-slot="bottom-center"] { grid-area: 3 / 2; }
      [data-slot="bottom-right"] { grid-area: 3 / 3; }
      [part="corner"] {
        display: flex;
        flex-direction: column;
        gap: var(--swath-space-2);
        min-inline-size: 0;
        min-block-size: 0;
      }
      [data-slot="top-center"], [data-slot="bottom-center"] { align-items: center; justify-content: flex-start; }
      [data-slot="bottom-center"] { justify-content: flex-end; }
      [data-slot="top-right"], [data-slot="bottom-right"], [data-slot="right"] { align-items: flex-end; }
      [data-slot="bottom-left"], [data-slot="bottom-right"] { justify-content: flex-end; }
      [data-slot="left"], [data-slot="right"] { justify-content: center; }
      ::slotted(*) { pointer-events: auto; }
    `,
  ];
  static override properties = {
    collapsed: { type: "boolean", reflect: true },
  } as const;

  declare collapsed: boolean;

  /** Synchronous first render, like the shell: slotted chrome is laid out
   * the moment the dock upgrades. */
  override connectedCallback(): void {
    super.connectedCallback();
    this.render();
  }

  protected render(): void {
    if (this.renderRoot.childElementCount > 0) {
      return;
    }
    this.renderRoot.replaceChildren(
      el(
        "div",
        { part: "base" },
        ...DOCK_SLOTS.map((name) =>
          el("div", { part: "corner", "data-slot": name }, el("slot", { name })),
        ),
      ),
    );
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-hud-dock": SwathHudDock;
  }
}
