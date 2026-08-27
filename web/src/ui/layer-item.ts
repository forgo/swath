// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-layer-item>` (docs/design/ui-system.md §5): one row of the layer
 * list — eye, title + meta, opacity (expanded on the active row), kebab.
 * Each control is its own tab stop. `layer-id` is mirrored as `data-layer`
 * on the host so e2e selectors keep one shape. Events, all scoped to the
 * row: `swath-layer-select / -visibility / -opacity / -action`. Parts:
 * `base row eye opacity menu`.
 */
import { SwathButton } from "./button.js";
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { type MenuItem, SwathMenu } from "./menu.js";
import { SwathSlider } from "./slider.js";
import { css } from "./styles.js";

export type LayerKind = "dataset" | "service" | "static";
export type LayerAction = "zoom" | "compare" | "info" | "delete";

export class SwathLayerItem extends SwathElement {
  static override tagName = "swath-layer-item";
  static override styles = [
    css`
      :host { display: block; }
      [part="base"] {
        display: grid;
        grid-template-columns: auto 1fr auto;
        align-items: center;
        gap: var(--swath-space-1);
        padding: var(--swath-space-1);
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-sm);
        background: none;
      }
      :host([active]) [part="base"] {
        border-color: var(--swath-color-accent-border);
        background: var(--swath-color-accent-bg);
      }
      :host(:not([visible])) [part="row"] { opacity: 0.55; }
      [part="row"] {
        display: flex;
        flex-direction: column;
        min-inline-size: 0;
        min-block-size: var(--swath-space-7);
        padding: var(--swath-space-1) var(--swath-space-2);
        border: 0;
        border-radius: var(--swath-radius-sm);
        background: none;
        color: var(--swath-color-fg);
        font: inherit;
        text-align: start;
        cursor: pointer;
      }
      [part="row"]:hover { background: var(--swath-color-accent-bg); }
      [part="title"] {
        font-size: var(--swath-text-md);
        font-weight: 600;
        line-height: var(--swath-leading-tight);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      [part="meta"] {
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      :host([active]) [part="meta"] { color: var(--swath-color-accent); }
      [part="opacity"] {
        grid-column: 1 / -1;
        padding: 0 var(--swath-space-1) var(--swath-space-1);
      }
      :host(:not([active])) [part="opacity"] { display: none; }
      [part="info"] {
        grid-column: 1 / -1;
        padding: 0 var(--swath-space-2) var(--swath-space-1);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
        word-break: break-all;
      }
      [part="info"] a { color: var(--swath-color-info); }
      :host(:not([expanded])) [part="info"] { display: none; }
      @media (pointer: coarse) {
        [part="row"] { min-block-size: var(--swath-size-target); }
      }
    `,
  ];
  static override properties = {
    layerId: { type: "string", attribute: "layer-id", reflect: true },
    title: { type: "string" },
    meta: { type: "string" },
    active: { type: "boolean", reflect: true },
    visible: { type: "boolean", reflect: true },
    opacity: { type: "number", reflect: true },
    kind: { type: "string", reflect: true },
    expanded: { type: "boolean", reflect: true },
    /** The `/tilesets/{id}` link shown under "info". */
    href: { type: "string" },
  } as const;

  declare layerId: string | undefined;
  declare title: string;
  declare meta: string | undefined;
  declare active: boolean;
  declare visible: boolean;
  declare opacity: number | undefined;
  declare kind: string | undefined;
  declare expanded: boolean;
  declare href: string | undefined;

  #row: HTMLButtonElement | undefined;
  #eye: SwathButton | undefined;
  #slider: SwathSlider | undefined;
  #menu: SwathMenu | undefined;

  constructor() {
    super();
    SwathButton.define();
    SwathSlider.define();
    SwathMenu.define();
  }

  get #id(): string {
    return this.layerId ?? "";
  }

  #ensure(): void {
    if (this.#row) {
      return;
    }
    const row = el(
      "button",
      { part: "row", type: "button" },
      el("span", { part: "title" }),
      el("span", { part: "meta" }),
    );
    row.addEventListener("click", () => this.emit("swath-layer-select", { layer: this.#id }));
    const eye = el("swath-button", { part: "eye", icon: "eye", pressed: true, size: "sm" });
    eye.addEventListener("swath-toggle", (event) => {
      event.stopPropagation();
      this.visible = event.detail.pressed;
      this.emit("swath-layer-visibility", { layer: this.#id, visible: this.visible });
    });
    const slider = el("swath-slider", { part: "opacity", min: 0, max: 1, step: 0.05, value: 1 });
    slider.format = (value) => `${Math.round(value * 100)}%`;
    const onOpacity = (event: CustomEvent<{ value: string | number | boolean }>): void => {
      event.stopPropagation();
      this.opacity = Number(event.detail.value);
      this.emit("swath-layer-opacity", { layer: this.#id, opacity: this.opacity });
    };
    slider.addEventListener("swath-input", onOpacity);
    slider.addEventListener("swath-change", onOpacity);
    const menu = el("swath-menu", { part: "menu" });
    menu.append(el("swath-button", { slot: "trigger", icon: "more", size: "sm" }));
    menu.addEventListener("swath-menu-select", (event) => {
      event.stopPropagation();
      const action = event.detail.id as LayerAction;
      if (action === "info") {
        this.expanded = !this.expanded;
      }
      this.emit("swath-layer-action", { layer: this.#id, action });
    });
    menu.addEventListener("swath-drawer-close", (event) => event.stopPropagation());
    const info = el("div", { part: "info" });
    this.#row = row;
    this.#eye = eye;
    this.#slider = slider;
    this.#menu = menu;
    this.renderRoot.replaceChildren(el("div", { part: "base" }, eye, row, menu, slider, info));
  }

  #items(): MenuItem[] {
    const items: MenuItem[] = [
      { id: "zoom", label: "Zoom to data", icon: "fit" },
      { id: "compare", label: this.active ? "Compare…" : "Compare with this", icon: "compare" },
      { id: "info", label: this.expanded ? "Hide info" : "Info", icon: "info" },
    ];
    if (this.kind === "service") {
      items.push({ id: "delete", label: "Delete service", icon: "trash", danger: true });
    }
    return items;
  }

  protected render(): void {
    this.#ensure();
    const id = this.#id;
    this.dataset["layer"] = id;
    const row = this.#row as HTMLButtonElement;
    row.setAttribute("aria-pressed", String(this.active));
    row.setAttribute("aria-label", `${this.title || id}`);
    const title = row.querySelector('[part="title"]');
    const meta = row.querySelector('[part="meta"]');
    if (title) {
      title.textContent = this.title || id;
    }
    if (meta) {
      meta.textContent = this.meta ?? id;
    }
    const eye = this.#eye as SwathButton;
    eye.pressed = this.visible;
    eye.icon = this.visible ? "eye" : "eye-off";
    eye.label = this.visible ? `Hide ${this.title || id}` : `Show ${this.title || id}`;
    const slider = this.#slider as SwathSlider;
    slider.value = this.opacity ?? 1;
    slider.label = `Opacity of ${this.title || id}`;
    slider.name = id;
    const menu = this.#menu as SwathMenu;
    menu.label = `Actions for ${this.title || id}`;
    menu.items = this.#items();
    const trigger = menu.querySelector("swath-button");
    if (trigger) {
      trigger.label = `Actions for ${this.title || id}`;
    }
    const info = this.renderRoot.querySelector('[part="info"]');
    if (info) {
      info.replaceChildren(
        el("span", {}, `${id} · ${this.kind ?? "dataset"} · `),
        this.href
          ? el("a", { href: this.href, target: "_blank", rel: "noreferrer" }, "tileset")
          : "",
      );
    }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-layer-item": SwathLayerItem;
  }
}
