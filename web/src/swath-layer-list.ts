// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-layer-list>` — the rail's layer control (ui-system.md §6), one
 * `<swath-layer-item>` per server layer, on `SwathElement`. It replaces
 * `<swath-layer-panel>` (M5's list of buttons). The host feeds it with
 * `update(layers, active, view)` from the map's `swath-layer-change` and
 * `services` (ids that are authored services — they get a delete action);
 * every user act leaves as a row-scoped `swath-layer-*` event. Scope fence
 * (#282): eye and opacity act on the viewed layer only; a multi-layer
 * stack is a later issue.
 */
import type { SwathLayer } from "./swath-map.js";
import { el } from "./ui/dom.js";
import { SwathElement } from "./ui/element.js";
import { SwathLayerItem } from "./ui/layer-item.js";
import { css } from "./ui/styles.js";

export interface LayerView {
  visible: boolean;
  opacity: number;
}

export class SwathLayerList extends SwathElement {
  static override tagName = "swath-layer-list";
  static override styles = [
    css`
      :host { display: block; }
      [part="heading"] {
        margin: 0 0 var(--swath-space-2);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        font-weight: 700;
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
      }
      [part="list"] {
        display: flex;
        flex-direction: column;
        gap: var(--swath-space-1);
        margin: 0;
        padding: 0;
        list-style: none;
      }
      [part="empty"] {
        margin: 0;
        font-size: var(--swath-text-sm);
        color: var(--swath-color-fg-muted);
      }
    `,
  ];
  static override properties = {
    /** Base URL for the "info" tileset links (same origin by default). */
    server: { type: "string" },
  } as const;

  declare server: string | undefined;

  #layers: readonly SwathLayer[] = [];
  #active = "";
  #view: LayerView = { visible: true, opacity: 1 };
  #services: ReadonlySet<string> = new Set();
  #items = new Map<string, SwathLayerItem>();

  constructor() {
    super();
    SwathLayerItem.define();
  }

  override connectedCallback(): void {
    super.connectedCallback();
    this.setAttribute("role", "group");
    if (!this.hasAttribute("aria-label")) {
      this.setAttribute("aria-label", "Layers");
    }
  }

  /** The server's layers, the viewed one, and how it is shown. */
  update(layers: readonly SwathLayer[], active: string, view?: LayerView): void {
    this.#layers = layers;
    this.#active = active;
    if (view) {
      this.#view = view;
    }
    this.requestUpdate();
  }

  /** Ids of layers that are authored services (deletable). */
  get services(): readonly string[] {
    return [...this.#services];
  }

  set services(ids: readonly string[]) {
    this.#services = new Set(ids);
    this.requestUpdate();
  }

  get layers(): readonly SwathLayer[] {
    return this.#layers;
  }

  protected render(): void {
    const heading = el("h2", { part: "heading" }, "Layers");
    if (this.#layers.length === 0) {
      this.renderRoot.replaceChildren(
        heading,
        el("p", { part: "empty" }, "Waiting for the server's layer list…"),
      );
      return;
    }
    // Rows are keyed by id and kept in place across updates: moving a
    // custom element (a fresh <li> per render) fires disconnect/connect,
    // which is what a real DOM change means — so only the order changes
    // touch the tree, and a row in the same slot keeps its focus and state.
    let list = this.renderRoot.querySelector<HTMLUListElement>('[part="list"]');
    if (!list) {
      list = el("ul", { part: "list" });
      this.renderRoot.replaceChildren(heading, list);
    } else {
      this.renderRoot.firstElementChild?.replaceWith(heading);
    }
    const seen = new Set<string>();
    const wanted: HTMLLIElement[] = [];
    for (const layer of this.#layers) {
      seen.add(layer.id);
      let item = this.#items.get(layer.id);
      if (!item) {
        item = el("swath-layer-item");
        this.#items.set(layer.id, item);
      }
      item.layerId = layer.id;
      item.title = layer.title;
      item.kind = this.#services.has(layer.id) ? "service" : "dataset";
      item.href = `${(this.server ?? "").replace(/\/+$/, "")}/tilesets/${encodeURIComponent(layer.id)}`;
      const active = layer.id === this.#active;
      item.active = active;
      item.visible = active ? this.#view.visible : true;
      item.opacity = active ? this.#view.opacity : 1;
      wanted.push(
        item.parentElement instanceof HTMLLIElement ? item.parentElement : el("li", {}, item),
      );
    }
    for (const id of this.#items.keys()) {
      if (!seen.has(id)) {
        this.#items.delete(id);
      }
    }
    const current = [...list.children];
    if (current.length !== wanted.length || current.some((node, i) => node !== wanted[i])) {
      list.replaceChildren(...wanted);
    }
  }
}

/** Registers `<swath-layer-list>`; safe to call more than once. */
export function defineSwathLayerList(): void {
  SwathLayerList.define();
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-layer-list": SwathLayerList;
  }
}
