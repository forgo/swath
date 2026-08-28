// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-canvas-node>` (issue #290): a card on the canvas at `x`/`y`
 * (canvas units), `node-id`, `title`, `selected`. Slots: default (body),
 * `inputs` / `outputs` for `<swath-canvas-port>`s. Drag with an 8 px
 * threshold (below it a press is a click: select / activate), arrow
 * nudge (1 unit, 10 with Shift), Enter activates, Delete asks. The
 * canvas owns the viewport, so the node reports canvas-unit moves and the
 * canvas positions it. Parts: `base header body`.
 */
import { el } from "../ui/dom.js";
import { SwathElement } from "../ui/element.js";
import { css } from "../ui/styles.js";

export const DRAG_THRESHOLD_PX = 8;

export class SwathCanvasNode extends SwathElement {
  static override tagName = "swath-canvas-node";
  static override styles = [
    css`
      :host {
        position: absolute;
        display: block;
        inline-size: max-content;
        min-inline-size: calc(var(--swath-space-8) * 4);
        touch-action: none;
        user-select: none;
      }
      [part="base"] {
        display: grid;
        grid-template-columns: auto 1fr auto;
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-md);
        background: var(--swath-color-bg-raised);
        color: var(--swath-color-fg);
        outline: none;
      }
      :host([selected]) [part="base"] { border-color: var(--swath-color-accent-border); box-shadow: 0 0 0 2px var(--swath-color-accent-bg); }
      [part="base"]:focus-visible { outline: var(--swath-border-focus); }
      [part="header"] {
        grid-column: 1 / -1;
        padding: var(--swath-space-1) var(--swath-space-2);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        font-weight: 700;
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
        cursor: grab;
      }
      :host([dragging]) [part="header"] { cursor: grabbing; }
      [part="inputs"], [part="outputs"] {
        display: flex;
        flex-direction: column;
        justify-content: space-around;
        gap: var(--swath-space-1);
      }
      [part="inputs"] { margin-inline-start: calc(-1 * var(--swath-space-2)); }
      [part="outputs"] { margin-inline-end: calc(-1 * var(--swath-space-2)); }
      [part="body"] { padding: var(--swath-space-1) var(--swath-space-2) var(--swath-space-2); font-size: var(--swath-text-sm); }
    `,
  ];
  static override properties = {
    nodeId: { type: "string", attribute: "node-id", reflect: true },
    title: { type: "string" },
    x: { type: "number", reflect: true },
    y: { type: "number", reflect: true },
    selected: { type: "boolean", reflect: true },
    dragging: { type: "boolean", reflect: true },
  } as const;

  declare nodeId: string | undefined;
  declare title: string;
  declare x: number | undefined;
  declare y: number | undefined;
  declare selected: boolean;
  declare dragging: boolean;

  #base: HTMLElement | undefined;

  #ensure(): HTMLElement {
    if (this.#base) {
      return this.#base;
    }
    const base = el(
      "div",
      { part: "base", tabindex: -1, role: "group" },
      el("div", { part: "header" }),
      el("div", { part: "inputs" }, el("slot", { name: "inputs" })),
      el("div", { part: "body" }, el("slot")),
      el("div", { part: "outputs" }, el("slot", { name: "outputs" })),
    );
    base.addEventListener("keydown", (event) => this.#onKey(event));
    base.addEventListener("dblclick", () =>
      this.emit("swath-node-activate", { id: this.nodeId ?? "" }),
    );
    this.#base = base;
    this.renderRoot.replaceChildren(base);
    return base;
  }

  #onKey(event: KeyboardEvent): void {
    if (event.target !== this.#base) {
      return; // a port or a field inside handles its own keys
    }
    const step = event.shiftKey ? 10 : 1;
    const nudge: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const delta = nudge[event.key];
    if (delta) {
      event.preventDefault();
      event.stopPropagation();
      this.x = (this.x ?? 0) + delta[0];
      this.y = (this.y ?? 0) + delta[1];
      this.emit("swath-node-move", { id: this.nodeId ?? "", x: this.x, y: this.y });
    } else if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
      this.emit("swath-node-activate", { id: this.nodeId ?? "" });
    } else if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      event.stopPropagation();
      this.emit("swath-delete-request", { nodes: [this.nodeId ?? ""], edges: [] });
    }
  }

  /** Focus the node's surface (roving Tab lands here). */
  override focus(): void {
    this.#ensure().focus();
  }

  protected render(): void {
    const base = this.#ensure();
    const header = base.querySelector('[part="header"]');
    if (header) {
      header.textContent = this.title ?? "";
    }
    base.setAttribute("aria-label", this.title || (this.nodeId ?? "node"));
    base.setAttribute("aria-selected", String(this.selected));
    this.dataset["node"] = this.nodeId ?? "";
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-canvas-node": SwathCanvasNode;
  }
}
