// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-drawer>` (docs/design/ui-system.md §5, §6): a panel docked to
 * an `edge` (`right` | `bottom`) of its positioned container. Below the
 * `narrow` breakpoint of the CONTAINER's width it always presents as a
 * bottom sheet (reflected `presentation`), dragged by its handle between
 * `snap` points (percent heights, e.g. `snap="40,90"`) and swiped down to
 * close. `modal` adds a scrim and a focus trap; Esc asks to close in every
 * mode — the drawer never closes itself: it emits `swath-drawer-close`
 * and the host flips `open` (that is what lets the shell keep the map
 * `inert` under a full-height sheet). `swath-change` carries the snap
 * index (`name: "snap"`). Parts: `base header body footer handle scrim`.
 */
import { BREAKPOINTS } from "./breakpoints.js";
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

const SWIPE_CLOSE_PX = 48;

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export class SwathDrawer extends SwathElement {
  static override tagName = "swath-drawer";
  static override styles = [
    css`
      :host {
        display: block;
        position: absolute;
        inset: 0;
        z-index: var(--swath-z-drawer);
        pointer-events: none;
      }
      :host(:not([open])) { display: none; }
      [part="scrim"] {
        position: absolute;
        inset: 0;
        background: var(--swath-color-bg-hud);
        opacity: 0.6;
        pointer-events: auto;
      }
      :host(:not([modal])) [part="scrim"] { display: none; }
      [part="base"] {
        position: absolute;
        display: flex;
        flex-direction: column;
        background: var(--swath-color-bg-raised);
        color: var(--swath-color-fg);
        border: var(--swath-border-hairline);
        box-shadow: var(--swath-shadow-hud);
        pointer-events: auto;
        overflow: hidden;
        transition: transform var(--swath-motion-normal) var(--swath-motion-ease);
      }
      :host([presentation="right"]) [part="base"] {
        inset-block: 0;
        inset-inline-end: 0;
        inline-size: var(--swath-drawer-size, var(--swath-size-inspector));
        max-inline-size: 100%;
      }
      :host([presentation="bottom"]) [part="base"] {
        inset-inline: 0;
        inset-block-end: 0;
        block-size: var(--swath-drawer-size, 50%);
        max-block-size: 100%;
        border-radius: var(--swath-radius-md) var(--swath-radius-md) 0 0;
        padding-block-end: env(safe-area-inset-bottom, 0);
        touch-action: none;
      }
      [part="handle"] {
        display: none;
        flex: none;
        align-self: center;
        inline-size: var(--swath-space-8);
        block-size: var(--swath-space-1);
        margin: var(--swath-space-2) 0;
        border-radius: var(--swath-radius-pill);
        background: var(--swath-color-line);
        cursor: grab;
      }
      :host([presentation="bottom"]) [part="handle"] { display: block; }
      :host([presentation="right"][resizable]) [part="handle"] {
        display: block;
        position: absolute;
        inset-block: 0;
        inset-inline-start: 0;
        inline-size: var(--swath-space-2);
        block-size: auto;
        margin: 0;
        border-radius: 0;
        background: transparent;
        cursor: col-resize;
        touch-action: none;
      }
      [part="header"] { flex: none; padding: var(--swath-space-2) var(--swath-space-3); }
      [part="body"] { flex: 1; min-block-size: 0; overflow: auto; padding: var(--swath-space-3); }
      [part="footer"] { flex: none; padding: var(--swath-space-2) var(--swath-space-3); border-block-start: var(--swath-border-hairline); }
      [part="header"]:not(:has(*)):empty, [part="footer"]:not(:has(*)):empty { display: none; }
      @media (pointer: coarse) {
        [part="handle"] { min-block-size: var(--swath-space-6); background-clip: content-box; padding-block: var(--swath-space-2); }
      }
    `,
  ];
  static override properties = {
    edge: { type: "string", reflect: true },
    open: { type: "boolean", reflect: true },
    size: { type: "string" },
    resizable: { type: "boolean", reflect: true },
    modal: { type: "boolean", reflect: true },
    snap: { type: "string" },
    label: { type: "string" },
    presentation: { type: "string", reflect: true },
  } as const;

  declare edge: string | undefined;
  declare open: boolean;
  declare size: string | undefined;
  declare resizable: boolean;
  declare modal: boolean;
  declare snap: string | undefined;
  declare label: string | undefined;
  declare presentation: string | undefined;

  #base: HTMLDivElement | undefined;
  #observer: ResizeObserver | undefined;
  #containerWidth = Number.POSITIVE_INFINITY;
  #snapIndex = 0;
  #previousFocus: Element | null = null;

  /** Snap heights in percent of the container, ascending. */
  get snapPoints(): readonly number[] {
    return (this.snap ?? "")
      .split(",")
      .map((s) => Number(s.trim()))
      .filter((n) => Number.isFinite(n) && n > 0)
      .sort((a, b) => a - b);
  }

  get snapIndex(): number {
    return this.#snapIndex;
  }

  set snapIndex(index: number) {
    const points = this.snapPoints;
    const next = Math.max(0, Math.min(points.length - 1, index));
    if (next !== this.#snapIndex) {
      this.#snapIndex = next;
      this.emit("swath-change", { name: "snap", value: next });
    }
    this.requestUpdate();
  }

  override connectedCallback(): void {
    super.connectedCallback();
    const container = this.parentElement;
    if (container) {
      this.#observer = new ResizeObserver((entries) => {
        const width = entries[0]?.contentRect.width;
        if (width !== undefined && width !== this.#containerWidth) {
          this.#containerWidth = width;
          this.requestUpdate();
        }
      });
      this.#observer.observe(container);
      this.#containerWidth = container.getBoundingClientRect().width;
    }
    this.addEventListener("keydown", (event) => this.#onKey(event), { signal: this.disconnected });
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    this.#observer?.disconnect();
    this.#observer = undefined;
  }

  #ensure(): HTMLDivElement {
    if (this.#base) {
      return this.#base;
    }
    const scrim = el("div", { part: "scrim" });
    scrim.addEventListener("click", () => this.#requestClose("scrim"));
    const handle = el("div", { part: "handle", "aria-hidden": "true" });
    this.#wireDrag(handle);
    const base = el(
      "div",
      { part: "base", role: "dialog", tabindex: -1 },
      handle,
      el("div", { part: "header" }, el("slot", { name: "header" })),
      el("div", { part: "body" }, el("slot")),
      el("div", { part: "footer" }, el("slot", { name: "footer" })),
    );
    this.#base = base;
    this.renderRoot.replaceChildren(scrim, base);
    return base;
  }

  #requestClose(reason: "esc" | "scrim" | "swipe"): void {
    this.emit("swath-drawer-close", { reason });
  }

  #onKey(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.stopPropagation();
      this.#requestClose("esc");
      return;
    }
    if (event.key !== "Tab" || !this.modal) {
      return;
    }
    const focusables = [...this.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
      (node) => node.offsetParent !== null || node === document.activeElement,
    );
    if (focusables.length === 0) {
      event.preventDefault();
      this.#base?.focus();
      return;
    }
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    const active = this.#activeElement();
    if (event.shiftKey && (active === first || active === this.#base)) {
      event.preventDefault();
      last?.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first?.focus();
    }
  }

  #activeElement(): Element | null {
    let active: Element | null = document.activeElement;
    while (active?.shadowRoot?.activeElement) {
      active = active.shadowRoot.activeElement;
    }
    return active;
  }

  #wireDrag(handle: HTMLElement): void {
    let start: { x: number; y: number; height: number; width: number } | undefined;
    handle.addEventListener("pointerdown", (event) => {
      const rect = this.#base?.getBoundingClientRect();
      if (!rect) {
        return;
      }
      start = { x: event.clientX, y: event.clientY, height: rect.height, width: rect.width };
      handle.setPointerCapture(event.pointerId);
      event.preventDefault();
    });
    handle.addEventListener("pointermove", (event) => {
      if (!start || !this.#base) {
        return;
      }
      if (this.presentation === "bottom") {
        const height = Math.max(0, start.height - (event.clientY - start.y));
        this.#base.style.setProperty("--swath-drawer-size", `${height}px`);
      } else if (this.resizable) {
        const width = Math.max(0, start.width - (event.clientX - start.x));
        this.style.setProperty("--swath-drawer-size", `${width}px`);
      }
    });
    const finish = (event: PointerEvent): void => {
      if (!start || !this.#base) {
        return;
      }
      const from = start;
      start = undefined;
      if (this.presentation !== "bottom") {
        return;
      }
      this.#base.style.removeProperty("--swath-drawer-size");
      const dy = event.clientY - from.y;
      const container = this.getBoundingClientRect().height || 1;
      const points = this.snapPoints;
      if (points.length === 0) {
        if (dy > SWIPE_CLOSE_PX) {
          this.#requestClose("swipe");
        }
        return;
      }
      const targetPct = ((from.height - dy) / container) * 100;
      const lowest = points[0] ?? 0;
      if (targetPct < lowest / 2 && dy > SWIPE_CLOSE_PX) {
        this.#requestClose("swipe");
        return;
      }
      let nearest = 0;
      for (const [index, point] of points.entries()) {
        if (Math.abs(point - targetPct) < Math.abs((points[nearest] ?? 0) - targetPct)) {
          nearest = index;
        }
      }
      this.snapIndex = nearest;
    };
    handle.addEventListener("pointerup", finish);
    handle.addEventListener("pointercancel", finish);
  }

  #presentation(): "right" | "bottom" {
    if (this.#containerWidth < BREAKPOINTS.narrow) {
      return "bottom";
    }
    return this.edge === "bottom" ? "bottom" : "right";
  }

  protected render(): void {
    const base = this.#ensure();
    const presentation = this.#presentation();
    if (this.presentation !== presentation) {
      this.presentation = presentation;
    }
    base.setAttribute("aria-modal", String(this.modal));
    if (this.label !== undefined && this.label !== "") {
      base.setAttribute("aria-label", this.label);
    }
    const points = this.snapPoints;
    if (presentation === "bottom" && points.length > 0) {
      this.style.setProperty("--swath-drawer-size", `${points[this.#snapIndex] ?? points[0]}%`);
    } else if (this.size !== undefined && this.size !== "") {
      this.style.setProperty("--swath-drawer-size", this.size);
    } else {
      this.style.removeProperty("--swath-drawer-size");
    }
    if (this.open && this.modal && !this.contains(this.#activeElement())) {
      this.#previousFocus = document.activeElement;
      const first = this.querySelector<HTMLElement>(FOCUSABLE);
      (first ?? base).focus();
    } else if (!this.open && this.#previousFocus instanceof HTMLElement) {
      this.#previousFocus.focus();
      this.#previousFocus = null;
    }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-drawer": SwathDrawer;
  }
}
