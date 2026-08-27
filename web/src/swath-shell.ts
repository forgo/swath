// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-shell>` — the one shell (docs/design/ui-system.md §6): five
 * regions, the map always present. Slots: `rail` (a `<swath-rail>`),
 * `topbar`, `map` (the `<swath-map>`, LIGHT DOM — MapLibre keeps document
 * scope and `document.querySelector("swath-map")` keeps working), `hud`
 * (a `<swath-hud-dock>` over the map), `inspector` (the right column,
 * `author` only), `statusbar`. `view` reflects the mode; `inspector`
 * reflects whether the column is open. One `role="status"` live region,
 * fed by `announce(text)`, replaces ad-hoc status nodes. Desktop layout
 * only in #284; responsive reflow is its own issue.
 */
import { el } from "./ui/dom.js";
import { SwathElement } from "./ui/element.js";
import { css } from "./ui/styles.js";

export class SwathShell extends SwathElement {
  static override tagName = "swath-shell";
  static override styles = [
    css`
      :host {
        display: grid;
        grid-template-columns: auto 1fr auto;
        grid-template-rows: var(--swath-size-topbar) 1fr var(--swath-size-statusbar);
        grid-template-areas:
          "rail topbar topbar"
          "rail main inspector"
          "rail status status";
        inline-size: 100%;
        block-size: 100%;
        background: var(--swath-color-bg);
        color: var(--swath-color-fg);
        overflow: hidden;
      }
      [part="rail"] { grid-area: rail; min-block-size: 0; }
      [part="topbar"] {
        grid-area: topbar;
        display: flex;
        align-items: center;
        gap: var(--swath-space-3);
        padding: 0 var(--swath-space-3);
        border-block-end: var(--swath-border-hairline);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        font-weight: 700;
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
      }
      [part="main"] { grid-area: main; position: relative; min-inline-size: 0; min-block-size: 0; }
      ::slotted([slot="map"]) { position: absolute; inset: 0; block-size: 100%; }
      [part="inspector"] {
        grid-area: inspector;
        inline-size: var(--swath-size-inspector);
        border-inline-start: var(--swath-border-hairline);
        overflow: auto;
      }
      :host(:not([inspector])) [part="inspector"] { display: none; }
      [part="status"] { grid-area: status; min-inline-size: 0; }
      [part="live"] {
        position: absolute;
        inline-size: 1px;
        block-size: 1px;
        overflow: hidden;
        clip-path: inset(50%);
        white-space: nowrap;
      }
    `,
  ];
  static override properties = {
    view: { type: "string", reflect: true },
    inspector: { type: "boolean", reflect: true },
  } as const;

  declare view: string | undefined;
  declare inspector: boolean;

  #live: HTMLElement | undefined;

  /** Layout containers render synchronously: a slotted `<swath-map>` must
   * have its slot (and so its size) the moment it upgrades — MapLibre
   * reads the container size at construction and a batched first render
   * would hand it 0 × 0 (the #284 mis-framed landing). */
  override connectedCallback(): void {
    super.connectedCallback();
    this.render();
  }

  /** Say `text` to assistive tech through the shell's one live region. */
  announce(text: string): void {
    const live = this.#ensureLive();
    live.textContent = "";
    // A re-announcement of identical text needs a DOM change to be spoken.
    requestAnimationFrame(() => {
      live.textContent = text;
    });
  }

  #ensureLive(): HTMLElement {
    if (!this.#live) {
      this.#live = el("div", { part: "live", role: "status", "aria-live": "polite" });
    }
    return this.#live;
  }

  protected render(): void {
    if (this.renderRoot.childElementCount > 0) {
      return;
    }
    this.renderRoot.replaceChildren(
      el("div", { part: "rail" }, el("slot", { name: "rail" })),
      el("header", { part: "topbar" }, el("slot", { name: "topbar" })),
      el(
        "main",
        { part: "main" },
        el("slot", { name: "map" }),
        el("slot", { name: "hud" }),
        el("slot", { name: "main" }),
      ),
      el("aside", { part: "inspector" }, el("slot", { name: "inspector" })),
      el("footer", { part: "status" }, el("slot", { name: "statusbar" })),
      this.#ensureLive(),
    );
  }
}

/** Registers `<swath-shell>`; safe to call more than once. */
export function defineSwathShell(): void {
  SwathShell.define();
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-shell": SwathShell;
  }
}
