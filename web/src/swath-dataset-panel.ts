// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-dataset-panel>` — the entry page's dataset browser (issue #110):
 * "what have I ingested?" answered visually. Lists the catalog's datasets
 * (`GET /collections` — the openEO surface is the one dataset listing the
 * server exposes); expanding a dataset fetches its granules
 * (`GET /datasets/{id}/granules`, issue #107) and announces them so the
 * page shell can paint footprint outlines on the map
 * (see granule-footprints.ts); clicking a granule announces a zoom.
 *
 * Plain Custom Element, light DOM, no framework (ADR 0005). Unlike the
 * deliberately presentational `<swath-layer-panel>`, this panel owns its
 * fetching — but LAZILY, and that laziness is contractual (asserted by a
 * network-count test): the panel mounts collapsed and issues NO requests
 * until the user opens it, so the entry page's cost is untouched for
 * everyone who never browses datasets. Granules are fetched only when
 * their dataset is expanded, and re-fetched on every expand — granules
 * arrive live via ingest, so a stale cache would lie.
 *
 * Events (both bubbling):
 * - `swath-dataset-granules` `{ dataset, granules }` — the expanded
 *   dataset's granules (empty on collapse); the shell routes it to the
 *   footprint layer.
 * - `swath-granule-zoom` `{ dataset, id, bbox }` — a granule was clicked.
 *
 * Attributes: `server` — base URL of the Swath API (default same origin),
 * mirroring `<swath-map>`.
 */

import { SwathApi } from "./api.js";
import { type FootprintGranule, parseBbox } from "./granule-footprints.js";
import { createSwathEvent } from "./ui/events.js";

/** One dataset of the listing. */
export interface DatasetItem {
  id: string;
  title: string;
}

/** One granule row: the footprint slice plus what the list displays. */
export interface GranuleListItem extends FootprintGranule {
  datetime: string;
}

/** The empty state: an existing dataset with zero granules is guidance,
 * not a blank hole — point at how granules arrive (the filedrop ingest
 * path; `swath ingest reference` prepares legacy manifests). Exported so
 * tests assert the exact contract string. */
export const GRANULES_EMPTY_GUIDANCE =
  "No granules ingested yet. Drop a granule into the server's watched ingest " +
  "directory to register it (legacy formats: `swath ingest reference <granule>` " +
  "writes the manifest), then re-open this dataset.";

const STYLE_ELEMENT_ID = "swath-dataset-panel-styles";

/** Panel skin, matching the layer panel's dark-telemetry look so the
 * rail reads as one instrument. Layout belongs to the page. */
const PANEL_CSS = `
swath-dataset-panel { display: block; }
swath-dataset-panel .swath-dataset-panel-toggle {
  display: block;
  width: 100%;
  margin: 0 0 8px;
  padding: 0;
  border: 0;
  background: none;
  text-align: left;
  cursor: pointer;
  font: 700 11px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: rgb(148 163 184 / 90%);
}
swath-dataset-panel .swath-dataset-panel-toggle::before {
  content: "▸ ";
}
swath-dataset-panel .swath-dataset-panel-toggle[aria-expanded="true"]::before {
  content: "▾ ";
}
swath-dataset-panel .swath-dataset-panel-toggle:focus-visible {
  outline: 2px solid #4ade80;
  outline-offset: 1px;
}
swath-dataset-panel ul,
swath-dataset-panel ol {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
swath-dataset-panel button[data-dataset],
swath-dataset-panel button[data-granule] {
  display: block;
  width: 100%;
  padding: 6px 10px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: none;
  text-align: left;
  cursor: pointer;
  color: inherit;
}
swath-dataset-panel button[data-dataset]:hover,
swath-dataset-panel button[data-granule]:hover {
  background: rgb(148 163 184 / 12%);
}
swath-dataset-panel button[data-dataset]:focus-visible,
swath-dataset-panel button[data-granule]:focus-visible {
  outline: 2px solid #4ade80;
  outline-offset: 1px;
}
swath-dataset-panel button[data-dataset][aria-expanded="true"] {
  border-color: rgb(74 222 128 / 45%);
  background: rgb(74 222 128 / 10%);
}
swath-dataset-panel .swath-dataset-panel-title {
  display: block;
  font: 600 13px/1.35 system-ui, sans-serif;
}
swath-dataset-panel .swath-dataset-panel-id,
swath-dataset-panel .swath-dataset-panel-datetime {
  display: block;
  font: 11px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  color: rgb(148 163 184 / 80%);
}
swath-dataset-panel ol {
  margin: 4px 0 4px 10px;
}
swath-dataset-panel .swath-dataset-panel-granule-id {
  display: block;
  font: 600 12px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  overflow-wrap: anywhere;
}
swath-dataset-panel .swath-dataset-panel-empty,
swath-dataset-panel .swath-dataset-panel-error,
swath-dataset-panel .swath-dataset-panel-loading {
  margin: 4px 0 4px 10px;
  font: 12px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 80%);
}
swath-dataset-panel .swath-dataset-panel-error {
  color: #fca5a5;
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

export class SwathDatasetPanel extends HTMLElement {
  static readonly tagName = "swath-dataset-panel";

  #open = false;
  /** undefined until the first (lazy) fetch has settled. */
  #datasets: DatasetItem[] | undefined;
  #datasetsError: string | undefined;
  #datasetsLoading = false;
  /** The one expanded dataset ("" = none) — an accordion: the footprints
   * on the map are always exactly the expanded dataset's. */
  #expanded = "";
  #granules: GranuleListItem[] = [];
  #granulesError: string | undefined;
  #granulesLoading = false;
  /** Settles when the last user-triggered load has rendered (never
   * rejects — failures render as error states). Test seam. */
  #ready: Promise<void> = Promise.resolve();

  /** Base URL of the Swath API (no trailing slash); same origin when the
   * `server` attribute is absent. */
  get server(): string {
    return (this.getAttribute("server") ?? "").replace(/\/+$/, "");
  }

  #api: SwathApi | undefined;
  #ownApi: SwathApi | undefined;

  /** The API client (ui-system.md §4.4): injected by a host or test, else
   * built from `server` — same origin when the attribute is absent. */
  get api(): SwathApi {
    if (this.#api !== undefined) {
      return this.#api;
    }
    if (this.#ownApi === undefined || this.#ownApi.base !== this.server) {
      this.#ownApi = new SwathApi({ base: this.server });
    }
    return this.#ownApi;
  }

  set api(api: SwathApi) {
    this.#api = api;
  }

  /** `await el.ready` before inspecting the DOM after an interaction. */
  get ready(): Promise<void> {
    return this.#ready;
  }

  connectedCallback(): void {
    injectStyles(this.ownerDocument);
    this.setAttribute("role", "group");
    if (!this.hasAttribute("aria-label")) {
      this.setAttribute("aria-label", "Datasets");
    }
    this.#render();
  }

  #togglePanel(): void {
    this.#open = !this.#open;
    if (!this.#open) {
      // Closing collapses everything and clears the map's footprints.
      if (this.#expanded !== "") {
        this.#collapseDataset();
      }
      this.#render();
      return;
    }
    if (this.#datasets === undefined && !this.#datasetsLoading) {
      this.#ready = this.#loadDatasets();
    }
    this.#render();
  }

  async #loadDatasets(): Promise<void> {
    this.#datasetsLoading = true;
    this.#datasetsError = undefined;
    this.#render();
    try {
      const response = await this.api.fetch("/collections", {
        headers: { accept: "application/json" },
      });
      if (!response.ok) {
        throw new Error(`GET ${this.server}/collections failed: ${response.status}`);
      }
      const body = (await response.json()) as {
        collections?: { id?: string; title?: string }[];
      };
      const datasets: DatasetItem[] = [];
      for (const item of body.collections ?? []) {
        if (typeof item.id === "string" && item.id !== "") {
          datasets.push({ id: item.id, title: item.title ?? item.id });
        }
      }
      this.#datasets = datasets;
    } catch (error) {
      this.#datasetsError = error instanceof Error ? error.message : String(error);
    } finally {
      this.#datasetsLoading = false;
    }
    this.#render();
  }

  #toggleDataset(id: string): void {
    if (this.#expanded === id) {
      this.#collapseDataset();
      this.#render();
      return;
    }
    this.#expanded = id;
    this.#granules = [];
    this.#granulesError = undefined;
    this.#ready = this.#loadGranules(id);
    this.#render();
  }

  #collapseDataset(): void {
    this.#expanded = "";
    this.#granules = [];
    this.#granulesError = undefined;
    this.#granulesLoading = false;
    this.#announceGranules("", []);
  }

  async #loadGranules(id: string): Promise<void> {
    this.#granulesLoading = true;
    this.#render();
    let granules: GranuleListItem[] = [];
    let failure: string | undefined;
    try {
      const url = `/datasets/${encodeURIComponent(id)}/granules`;
      const response = await this.api.fetch(url, { headers: { accept: "application/json" } });
      if (!response.ok) {
        throw new Error(`GET ${this.api.url(url)} failed: ${response.status}`);
      }
      const body = (await response.json()) as {
        granules?: { id?: string; bbox?: unknown; datetime?: string }[];
      };
      for (const item of body.granules ?? []) {
        const bbox = parseBbox(item.bbox);
        if (typeof item.id === "string" && bbox) {
          granules.push({ id: item.id, bbox, datetime: item.datetime ?? "" });
        }
      }
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
      granules = [];
    }
    if (this.#expanded !== id) {
      return; // stale: the user moved on while this fetch was in flight
    }
    this.#granulesLoading = false;
    this.#granulesError = failure;
    this.#granules = granules;
    if (failure === undefined) {
      this.#announceGranules(id, granules);
    }
    this.#render();
  }

  #announceGranules(dataset: string, granules: readonly GranuleListItem[]): void {
    this.dispatchEvent(
      createSwathEvent("swath-dataset-granules", { dataset, granules: [...granules] }),
    );
  }

  #render(): void {
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "swath-dataset-panel-toggle";
    toggle.textContent = "Datasets";
    toggle.setAttribute("aria-expanded", String(this.#open));
    toggle.addEventListener("click", () => {
      this.#togglePanel();
    });
    if (!this.#open) {
      this.replaceChildren(toggle);
      return;
    }
    this.replaceChildren(toggle, ...this.#body());
  }

  #body(): HTMLElement[] {
    if (this.#datasetsLoading) {
      return [this.#note("swath-dataset-panel-loading", "Loading datasets…")];
    }
    if (this.#datasetsError !== undefined) {
      return [
        this.#note(
          "swath-dataset-panel-error",
          `Dataset list unavailable: ${this.#datasetsError}. Close and re-open to retry.`,
        ),
      ];
    }
    if (this.#datasets === undefined) {
      return [];
    }
    if (this.#datasets.length === 0) {
      return [this.#note("swath-dataset-panel-empty", "The catalog has no datasets.")];
    }
    const list = document.createElement("ul");
    for (const dataset of this.#datasets) {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      button.dataset["dataset"] = dataset.id;
      button.setAttribute("aria-expanded", String(dataset.id === this.#expanded));
      const title = document.createElement("span");
      title.className = "swath-dataset-panel-title";
      title.textContent = dataset.title;
      const id = document.createElement("span");
      id.className = "swath-dataset-panel-id";
      id.textContent = dataset.id;
      button.append(title, id);
      button.addEventListener("click", () => {
        this.#toggleDataset(dataset.id);
      });
      item.append(button);
      if (dataset.id === this.#expanded) {
        item.append(...this.#granuleSection(dataset.id));
      }
      list.append(item);
    }
    return [list];
  }

  #granuleSection(dataset: string): HTMLElement[] {
    if (this.#granulesLoading) {
      return [this.#note("swath-dataset-panel-loading", "Loading granules…")];
    }
    if (this.#granulesError !== undefined) {
      return [
        this.#note(
          "swath-dataset-panel-error",
          `Granules unavailable: ${this.#granulesError}. Re-open the dataset to retry.`,
        ),
      ];
    }
    if (this.#granules.length === 0) {
      return [this.#note("swath-dataset-panel-empty", GRANULES_EMPTY_GUIDANCE)];
    }
    const list = document.createElement("ol");
    list.setAttribute("aria-label", `Granules of ${dataset}, newest first`);
    for (const granule of this.#granules) {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      button.dataset["granule"] = granule.id;
      button.setAttribute("aria-label", `Zoom to granule ${granule.id}`);
      const id = document.createElement("span");
      id.className = "swath-dataset-panel-granule-id";
      id.textContent = granule.id;
      const datetime = document.createElement("span");
      datetime.className = "swath-dataset-panel-datetime";
      datetime.textContent = granule.datetime;
      button.append(id, datetime);
      button.addEventListener("click", () => {
        this.dispatchEvent(
          createSwathEvent("swath-granule-zoom", { dataset, id: granule.id, bbox: granule.bbox }),
        );
      });
      item.append(button);
      list.append(item);
    }
    return [list];
  }

  #note(className: string, text: string): HTMLParagraphElement {
    const note = document.createElement("p");
    note.className = className;
    note.textContent = text;
    return note;
  }
}

/** Registers `<swath-dataset-panel>`; safe to call more than once. */
export function defineSwathDatasetPanel(): void {
  if (!customElements.get(SwathDatasetPanel.tagName)) {
    customElements.define(SwathDatasetPanel.tagName, SwathDatasetPanel);
  }
}
