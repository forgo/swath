// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The breadcrumb chip row (issue #393) — the URL, made visible.
 *
 * Each chip is one thing the current view is *of*: the dataset, the layer,
 * the frame, the selected tile. Removing one drops its parameter, which is
 * a `pushState` (ADR 0027), so the row and the back button are two views of
 * the same history.
 *
 * Why chips rather than a mode tab strip: a mode is what the app is doing;
 * a chip is what you are looking at. The second is the thing a person wants
 * to send to a colleague, and it was already in the URL — just invisible.
 *
 * It renders nothing when there is nothing to show. An empty bar that is
 * always present is chrome charging rent for a view it does not have.
 */
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

/** One breadcrumb: a labelled value the view is scoped to. */
export interface Chip {
  /** Stable identity, and what `swath-chip-remove` reports. */
  id: string;
  /** The dimension, in the instrument register (`layer`, `date`, `tile`). */
  label: string;
  /** The value, verbatim — a layer id, an instant, a tile address. */
  value: string;
  /** Whether this chip can be dropped. A dataset chip usually cannot: with
   * no dataset there is no view to show. */
  removable?: boolean;
}

export class SwathChipRow extends SwathElement {
  static override tagName = "swath-chip-row";
  static override styles = [
    css`
      :host {
        display: contents;
      }
      [part="base"] {
        display: flex;
        align-items: center;
        gap: var(--swath-space-1);
        min-inline-size: 0;
        overflow-x: auto;
        scrollbar-width: none;
      }
      [part="base"]:empty { display: none; }
      [part="chip"] {
        display: inline-flex;
        align-items: center;
        gap: var(--swath-space-1);
        flex: none;
        padding: 0 var(--swath-space-2);
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-pill);
        background: var(--swath-color-bg-raised);
        color: var(--swath-color-fg-muted);
        font: inherit;
        line-height: calc(var(--swath-space-5) + var(--swath-space-1));
      }
      [part="value"] {
        color: var(--swath-color-fg);
        text-transform: none;
        max-inline-size: 22ch;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      [part="remove"] {
        display: inline-flex;
        align-items: center;
        padding: 0;
        border: 0;
        background: none;
        color: inherit;
        cursor: pointer;
        font: inherit;
      }
      [part="remove"]:hover { color: var(--swath-color-fg); }
      [part="separator"] {
        flex: none;
        color: var(--swath-color-fg-muted);
      }
    `,
  ];

  #chips: readonly Chip[] = [];

  /** The row's content. Assigning re-renders. */
  get chips(): readonly Chip[] {
    return this.#chips;
  }

  set chips(next: readonly Chip[]) {
    this.#chips = [...next];
    this.requestUpdate();
  }

  protected render(): void {
    const base = el("div", { part: "base", role: "list" });
    this.#chips.forEach((chip, index) => {
      if (index > 0) {
        base.append(el("span", { part: "separator", "aria-hidden": "true" }, "/"));
      }
      const item = el("span", { part: "chip", role: "listitem", "data-chip": chip.id });
      item.append(
        el("span", { part: "label" }, chip.label),
        el("span", { part: "value" }, chip.value),
      );
      if (chip.removable === true) {
        const remove = el("button", {
          part: "remove",
          type: "button",
          "aria-label": `remove the ${chip.label} ${chip.value}`,
        });
        const icon = document.createElement("swath-icon");
        icon.setAttribute("name", "close");
        icon.setAttribute("size", "sm");
        remove.append(icon);
        remove.addEventListener("click", () => {
          this.emit("swath-chip-remove", { chip: chip.id });
        });
        item.append(remove);
      }
      base.append(item);
    });
    this.renderRoot.replaceChildren(base);
  }
}
