// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-layer-panel>` — the entry page's layer browser (issue #108).
 *
 * Plain Custom Element, light DOM, no framework (ADR 0005). Deliberately
 * presentational: it fetches nothing and owns no server state. The app
 * shell feeds it via [`update`] with the layer list `<swath-map>` already
 * fetched for its own apply (the `layerchange` event carries it), so the
 * two never disagree about what exists — built-in and authored layers
 * alike, since the server's `/tilesets` list is the single source for
 * both. Selection is announced as a bubbling `swath-layer-select` event;
 * the shell routes it to the map.
 *
 * Accessibility mirrors the map's built-in switcher: a listbox-free
 * pattern of real `<button>`s where `aria-pressed` marks the viewed
 * layer.
 */

import type { SwathLayer } from "./swath-map.js";

const STYLE_ELEMENT_ID = "swath-layer-panel-styles";

/** Panel chrome. Layout (width, placement) belongs to the page; this is
 * only the list's own skin, themed to match the x-ray overlay's
 * dark-telemetry look so the embedded UI reads as one instrument. */
const PANEL_CSS = `
swath-layer-panel { display: block; }
swath-layer-panel .swath-layer-panel-heading {
  margin: 0 0 8px;
  font: 700 11px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: rgb(148 163 184 / 90%);
}
swath-layer-panel .swath-layer-panel-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
swath-layer-panel .swath-layer-panel-list button {
  display: block;
  width: 100%;
  padding: 8px 10px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: none;
  text-align: left;
  cursor: pointer;
  color: inherit;
}
swath-layer-panel .swath-layer-panel-list button:hover {
  background: rgb(148 163 184 / 12%);
}
swath-layer-panel .swath-layer-panel-list button:focus-visible {
  outline: 2px solid #4ade80;
  outline-offset: 1px;
}
swath-layer-panel .swath-layer-panel-list button[aria-pressed="true"] {
  border-color: rgb(74 222 128 / 45%);
  background: rgb(74 222 128 / 10%);
}
swath-layer-panel .swath-layer-panel-title {
  display: block;
  font: 600 13px/1.35 system-ui, sans-serif;
}
swath-layer-panel .swath-layer-panel-id {
  display: block;
  font: 11px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  color: rgb(148 163 184 / 80%);
}
swath-layer-panel button[aria-pressed="true"] .swath-layer-panel-id { color: #4ade80; }
swath-layer-panel .swath-layer-panel-empty {
  margin: 0;
  font: 12px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 80%);
}
`;

function injectStyles(doc: Document): void {
  if (doc.getElementById(STYLE_ELEMENT_ID)) {
    return;
  }
  const style = doc.createElement("style");
  style.id = STYLE_ELEMENT_ID;
  style.textContent = PANEL_CSS;
  doc.head.append(style);
}

export class SwathLayerPanel extends HTMLElement {
  static readonly tagName = "swath-layer-panel";

  #layers: readonly SwathLayer[] = [];
  #active = "";

  connectedCallback(): void {
    injectStyles(this.ownerDocument);
    this.setAttribute("role", "group");
    if (!this.hasAttribute("aria-label")) {
      this.setAttribute("aria-label", "Layers");
    }
    this.#render();
  }

  /** Replaces the listed layers and the active highlight (idempotent). */
  update(layers: readonly SwathLayer[], active: string): void {
    this.#layers = layers;
    this.#active = active;
    if (this.isConnected) {
      this.#render();
    }
  }

  #render(): void {
    const heading = document.createElement("h2");
    heading.className = "swath-layer-panel-heading";
    heading.textContent = "Layers";

    if (this.#layers.length === 0) {
      const empty = document.createElement("p");
      empty.className = "swath-layer-panel-empty";
      empty.textContent = "Waiting for the server's layer list…";
      this.replaceChildren(heading, empty);
      return;
    }

    const list = document.createElement("ul");
    list.className = "swath-layer-panel-list";
    for (const layer of this.#layers) {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      button.setAttribute("aria-pressed", String(layer.id === this.#active));
      button.dataset["layer"] = layer.id;
      const title = document.createElement("span");
      title.className = "swath-layer-panel-title";
      title.textContent = layer.title;
      const id = document.createElement("span");
      id.className = "swath-layer-panel-id";
      id.textContent = layer.id;
      button.append(title, id);
      button.addEventListener("click", () => {
        this.dispatchEvent(
          new CustomEvent("swath-layer-select", { detail: { layer: layer.id }, bubbles: true }),
        );
      });
      item.append(button);
      list.append(item);
    }
    this.replaceChildren(heading, list);
  }
}

/** Registers `<swath-layer-panel>`; safe to call more than once. */
export function defineSwathLayerPanel(): void {
  if (!customElements.get(SwathLayerPanel.tagName)) {
    customElements.define(SwathLayerPanel.tagName, SwathLayerPanel);
  }
}
