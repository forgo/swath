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
import { layoutTier } from "./ui/breakpoints.js";
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
        font-weight: var(--swath-weight-label);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
      }
      [part="main"] { grid-area: main; position: relative; min-inline-size: 0; min-block-size: 0; }
      ::slotted([slot="map"]) { position: absolute; inset: 0; block-size: 100%; }
      /* Composing (#400): the canvas takes the screen and the map becomes a
       * live preview column beside it — never under it. ADR 0028 amends ADR
       * 0021 §1 to "the map is always present AND never smaller than a live
       * preview", which is what this column is. Below the medium tier there
       * is no room for two columns, so the map keeps the region and the
       * canvas returns to a sheet over it. */
      :host([compose]) ::slotted([slot="map"]) {
        inset-inline-start: auto;
        inline-size: var(--swath-size-preview);
        /* Slotted content is light DOM, so the shadow sheet's box-sizing
         * reset does not reach it: without this the hairline would make the
         * preview 321px and quietly steal a pixel from the canvas. */
        box-sizing: border-box;
        border-inline-start: var(--swath-border-hairline);
      }
      :host([compose]) ::slotted([slot="main"]) { inset-inline-end: var(--swath-size-preview); }
      @media (max-width: 1023px) {
        :host([compose]) ::slotted([slot="map"]) {
          inset-inline-start: 0;
          inline-size: auto;
          border-inline-start: 0;
        }
        :host([compose]) ::slotted([slot="main"]) { inset-inline-end: 0; }
      }
      [part="inspector"] {
        grid-area: inspector;
        inline-size: var(--swath-size-inspector);
        border-inline-start: var(--swath-border-hairline);
        overflow: auto;
      }
      :host(:not([inspector])) [part="inspector"] { display: none; }
      [part="status"] { grid-area: status; min-inline-size: 0; }
      /* Reflow (ui-system.md §6): below the wide tier the inspector is a
       * drawer (the host moves it); under 640 px the rail is a bottom tab
       * bar and the status bar folds into the dock as one chip. Viewport
       * media queries, not container queries: the shell IS the viewport. */
      @media (max-width: 639px) {
        :host {
          grid-template-columns: 1fr;
          grid-template-rows: var(--swath-size-topbar) 1fr auto;
          grid-template-areas:
            "topbar"
            "main"
            "rail";
        }
        [part="status"] { display: none; }
        [part="inspector"] { display: none; }
      }
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
    /** Composing: the canvas takes the main region and the map becomes the
     * live preview column beside it (#400, ADR 0028). */
    compose: { type: "boolean", reflect: true },
    /** The layout tier (`wide | medium | narrow | phone`), reflected for
     * hosts and tests; the shell computes it from its own width. */
    tier: { type: "string", reflect: true },
  } as const;

  declare view: string | undefined;
  declare inspector: boolean;
  declare compose: boolean;
  declare tier: string | undefined;

  #observer: ResizeObserver | undefined;

  #syncTier(): void {
    const next = layoutTier(this.clientWidth || window.innerWidth);
    if (this.tier !== next) {
      this.tier = next;
      this.emit("swath-change", { name: "tier", value: next });
    }
  }

  #live: HTMLElement | undefined;

  /** Layout containers render synchronously: a slotted `<swath-map>` must
   * have its slot (and so its size) the moment it upgrades — MapLibre
   * reads the container size at construction and a batched first render
   * would hand it 0 × 0 (the #284 mis-framed landing). */
  override connectedCallback(): void {
    super.connectedCallback();
    this.render();
    this.#syncTier();
    this.#observer = new ResizeObserver(() => this.#syncTier());
    this.#observer.observe(this);
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    this.#observer?.disconnect();
    this.#observer = undefined;
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
