// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-catalog>` — Data mode's answer to "what have I ingested?"
 * (issue #288): a filter rail (dataset · date range · in current view),
 * grid/list, count + sort, and one `<swath-granule-card>` per granule
 * with a thumbnail the ENGINE rendered (`POST /result`, ADR 0014's bounded
 * preview; ADR 0019: never a client decode). Lazy by contract: nothing is
 * fetched until `active` (the mode is entered). Keeps the dataset panel's
 * events — `swath-dataset-granules` for footprints, `swath-granule-zoom`.
 */
import { ApiProblem, SwathApi } from "./api.js";
import {
  type CatalogDataset,
  type CatalogGranule,
  filterGranules,
  GRANULES_EMPTY_GUIDANCE,
  type GranuleFilter,
  type GranuleSort,
  parseCollections,
  parseGranules,
  previewGraph,
  previewKind,
  sortGranules,
} from "./catalog-model.js";
import type { FacetSummary } from "./facets-model.js";
import { coverageNote, facetSummary, parseFacets } from "./facets-model.js";
import type { LonLatBounds } from "./swath-map.js";
import { SwathTimeline } from "./swath-timeline.js";
import { buildTimeline, EMPTY_TIMELINE, type Timeline } from "./timeline-model.js";
import { SwathButton } from "./ui/button.js";
import { el } from "./ui/dom.js";
import { SwathElement } from "./ui/element.js";
import { SwathField } from "./ui/field.js";
import { SwathGranuleCard } from "./ui/granule-card.js";
import { css } from "./ui/styles.js";
import { SwathToggle } from "./ui/toggle.js";

/** Thumbnails in flight at once: previews are bounded but not free. */
const PREVIEW_CONCURRENCY = 3;

/** Object URLs by granule id, shared across catalog instances and mode
 * switches: the engine rendered it once; the URL lives for the page. */
const thumbnails = new Map<string, Promise<string>>();

/** Drop every cached preview (tests; a host that re-registers a dataset). */
export function clearThumbnailCache(): void {
  thumbnails.clear();
}

export class SwathCatalog extends SwathElement {
  static override tagName = "swath-catalog";
  static override styles = [
    css`
      :host { display: block; }
      [part="filters"] {
        display: grid;
        gap: var(--swath-space-2);
        margin-block-end: var(--swath-space-3);
      }
      [part="dates"] { display: grid; grid-template-columns: 1fr 1fr; gap: var(--swath-space-2); }
      [part="toolbar"] {
        display: flex;
        align-items: center;
        gap: var(--swath-space-2);
        margin-block-end: var(--swath-space-2);
      }
      [part="count"] {
        flex: 1;
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
      }
      [part="grid"] {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: var(--swath-space-2);
        margin: 0;
        padding: 0;
        list-style: none;
      }
      :host([layout="list"]) [part="grid"] { grid-template-columns: 1fr; }
      [part="empty"], [part="error"], [part="loading"] {
        margin: 0;
        font-size: var(--swath-text-sm);
        color: var(--swath-color-fg-muted);
      }
      [part="error"] { color: var(--swath-color-danger); }
      [part="facets"] {
        margin: 0 0 var(--swath-space-3);
        display: grid;
        gap: var(--swath-space-1);
        font-size: var(--swath-text-xs);
      }
      [part="facets"] > div {
        display: grid;
        grid-template-columns: minmax(0, auto) minmax(0, 1fr);
        gap: var(--swath-space-2);
        align-items: baseline;
      }
      [part="facet-key"] {
        font-family: var(--swath-font-mono);
        color: var(--swath-color-fg-muted);
      }
      [part="facet-coverage"] { color: var(--swath-color-fg-muted); }
    `,
  ];
  static override properties = {
    /** Fetches begin when true (the host sets it on entering Data mode). */
    active: { type: "boolean", reflect: true },
    layout: { type: "string", reflect: true },
    server: { type: "string" },
  } as const;

  declare active: boolean;
  declare layout: string | undefined;
  declare server: string | undefined;

  #api: SwathApi | undefined;
  #datasets: CatalogDataset[] = [];
  #datasetsError: string | undefined;
  #loading = false;
  #selected = "";
  #granules: CatalogGranule[] = [];
  #granulesError: string | undefined;
  #facets: FacetSummary = { total: 0, facets: [] };
  #timeline: Timeline = EMPTY_TIMELINE;
  #timelineElement: SwathTimeline | undefined;
  #sort: GranuleSort = "newest";
  #filter: GranuleFilter = {};
  #inView = false;
  #viewBounds: LonLatBounds | undefined;
  #cards = new Map<string, SwathGranuleCard>();
  #inFlight = 0;
  #queue: (() => void)[] = [];
  #loaded = false;

  constructor() {
    super();
    SwathField.define();
    SwathButton.define();
    SwathToggle.define();
    SwathGranuleCard.define();
  }

  get api(): SwathApi {
    this.#api ??= new SwathApi({ base: this.server ?? "" });
    return this.#api;
  }

  set api(api: SwathApi) {
    this.#api = api;
  }

  /** The map's current bounds, for the "in current view" filter. */
  set viewBounds(bounds: LonLatBounds | undefined) {
    this.#viewBounds = bounds;
    if (this.#inView) {
      this.requestUpdate();
    }
  }

  get datasets(): readonly CatalogDataset[] {
    return this.#datasets;
  }

  /** The open dataset's id ("" when none). */
  get selected(): string {
    return this.#selected;
  }

  get granules(): readonly CatalogGranule[] {
    return this.#granules;
  }

  override connectedCallback(): void {
    super.connectedCallback();
    this.setAttribute("role", "region");
    if (!this.hasAttribute("aria-label")) {
      this.setAttribute("aria-label", "Catalog");
    }
  }

  /** Re-read the listing (and the open dataset): granules arrive live,
   * a cache would lie — the dataset panel's rule. */
  async reload(): Promise<void> {
    this.#loaded = true;
    this.#loading = true;
    this.#datasetsError = undefined;
    this.requestUpdate();
    try {
      this.#datasets = parseCollections(await this.api.json("/collections"));
    } catch (error) {
      this.#datasetsError = error instanceof Error ? error.message : String(error);
      this.#datasets = [];
    }
    this.#loading = false;
    if (this.#selected !== "" && !this.#datasets.some((d) => d.id === this.#selected)) {
      this.#selected = "";
    }
    this.requestUpdate();
    if (this.#selected !== "") {
      await this.#loadGranules(this.#selected);
    }
  }

  /** Open a dataset: fetch its granules (every time — live), announce the
   * footprints. `""` closes and announces an empty set. */
  async select(id: string): Promise<void> {
    this.#selected = id;
    this.#granules = [];
    this.#granulesError = undefined;
    this.#facets = { total: 0, facets: [] };
    this.#timeline = EMPTY_TIMELINE;
    this.requestUpdate();
    if (id === "") {
      this.#announce("", []);
      return;
    }
    await this.#loadGranules(id);
  }

  async #loadGranules(id: string): Promise<void> {
    this.#loading = true;
    this.requestUpdate();
    let granules: CatalogGranule[] = [];
    let failure: string | undefined;
    try {
      granules = parseGranules(await this.api.json(`/datasets/${encodeURIComponent(id)}/granules`));
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    }
    // What the items actually carry (#409). A facet exists only because
    // the server found the key on a granule in scope, so nothing rendered
    // from this can be a control over data that is not there. A failure
    // here is not a granule failure: the list still loads, and the facet
    // block simply says nothing.
    let facets: FacetSummary = { total: 0, facets: [] };
    if (failure === undefined) {
      try {
        facets = parseFacets(await this.api.json(`/datasets/${encodeURIComponent(id)}/facets`));
      } catch {
        facets = { total: 0, facets: [] };
      }
    }
    // The two bands (#411), both from the counts endpoint: what the
    // collection holds, and what survives the dates in force. Neither is
    // inferred from the page above — a count we did not get from the
    // server is not a count we draw.
    let timeline: Timeline = EMPTY_TIMELINE;
    if (failure === undefined) {
      timeline = await this.#loadTimeline(id);
    }
    if (this.#selected !== id) {
      return; // stale: the user moved on while this fetch was in flight
    }
    this.#timeline = timeline;
    this.#loading = false;
    this.#granulesError = failure;
    this.#granules = granules;
    this.#facets = facets;
    if (failure === undefined) {
      this.#announce(id, granules);
    }
    this.requestUpdate();
  }

  /** The date bounds in force, as the URL carries them (#411). Setting
   * them applies the filter and re-asks for the surviving band; it does
   * not emit, so a restore from the URL cannot write history back. */
  get dates(): { from: string | undefined; to: string | undefined } {
    return { from: this.#filter.from, to: this.#filter.to };
  }

  set dates(value: { from: string | undefined; to: string | undefined }) {
    if (this.#filter.from === value.from && this.#filter.to === value.to) {
      return;
    }
    this.#filter = { ...this.#filter, from: value.from, to: value.to };
    this.requestUpdate();
    void this.#refreshTimeline();
  }

  /** The counts behind the two bands. The scoped call carries the dates
   * in force; with none, the same answer serves both bands and the
   * control says the filters remove nothing. */
  async #loadTimeline(id: string): Promise<Timeline> {
    const counts = (datetime?: string): Promise<unknown> =>
      this.api.json(
        this.api.url(`/datasets/${encodeURIComponent(id)}/counts`, { step: "month", datetime }),
      );
    try {
      const scope = this.#datetimeScope();
      const exists = await counts();
      const survives = scope === undefined ? exists : await counts(scope);
      return buildTimeline(exists, survives);
    } catch {
      // A timeline failure costs the timeline, never the granule list.
      return EMPTY_TIMELINE;
    }
  }

  /** The `datetime=` the current date filter means, or nothing when
   * neither bound is set. */
  #datetimeScope(): string | undefined {
    const { from, to } = this.#filter;
    if (from === undefined && to === undefined) {
      return undefined;
    }
    return `${from ?? ".."}/${to === undefined ? ".." : `${to}T23:59:59Z`}`;
  }

  /** The two bands, plus the control that narrows the dates. Nothing
   * when the axis is empty: a timeline with no buckets is not a control,
   * it is a blank. */
  #timelineBlock(): HTMLElement | undefined {
    if (this.#timeline.buckets.length === 0) {
      return undefined;
    }
    // Built once so a drag does not lose the element mid-gesture.
    const element = this.#timelineElement ?? new SwathTimeline();
    if (this.#timelineElement === undefined) {
      this.#timelineElement = element;
      element.setAttribute("part", "timeline");
      element.addEventListener("swath-dates", (event) => {
        event.stopPropagation();
        const { from, to } = event.detail;
        this.#filter = {
          ...this.#filter,
          from: from ?? undefined,
          to: to ?? undefined,
        };
        this.emit("swath-dates", { from, to });
        void this.#refreshTimeline();
      });
    }
    // Only on a real change: assigning re-anchors the control, and a
    // re-render mid-drag must not drop the range the user is drawing.
    if (element.timeline !== this.#timeline) {
      element.timeline = this.#timeline;
    }
    return element;
  }

  /** Re-ask for the surviving band after the dates changed. The held band
   * is unscoped and cannot have moved, but asking for both keeps one code
   * path and one source of truth. */
  async #refreshTimeline(): Promise<void> {
    const id = this.#selected;
    if (id === "") {
      return;
    }
    const timeline = await this.#loadTimeline(id);
    if (this.#selected !== id) {
      return;
    }
    this.#timeline = timeline;
    this.requestUpdate();
  }

  /** The discovered facets, one line each: the key, what its values are,
   * and how much of the scope carries it. Nothing when the items carry
   * nothing — an empty block, not an empty control. */
  #facetBlock(): HTMLElement | undefined {
    if (this.#facets.facets.length === 0) {
      return undefined;
    }
    const rows = this.#facets.facets.map((facet) =>
      el(
        "div",
        {},
        el("span", { part: "facet-key" }, facet.key),
        el(
          "span",
          {},
          facetSummary(facet),
          el("span", { part: "facet-coverage" }, ` · ${coverageNote(facet, this.#facets.total)}`),
        ),
      ),
    );
    return el("section", { part: "facets", "aria-label": "What these granules carry" }, ...rows);
  }

  #announce(dataset: string, granules: readonly CatalogGranule[]): void {
    this.emit("swath-dataset-granules", {
      dataset,
      granules: granules.map((g) => ({ id: g.id, bbox: g.bbox, datetime: g.datetime })),
    });
  }

  /** The engine's preview for one granule, at most N at a time, cached
   * for the page. A refusal resolves the card to its plain-words note. */
  #thumbnail(dataset: CatalogDataset, granule: CatalogGranule, card: SwathGranuleCard): void {
    const key = `${dataset.id}/${granule.id}`;
    let promise = thumbnails.get(key);
    if (!promise) {
      promise = new Promise<string>((resolve, reject) => {
        const run = (): void => {
          this.#inFlight += 1;
          this.api
            .blob("/result", {
              method: "POST",
              headers: { accept: "image/png", "content-type": "application/json" },
              body: JSON.stringify({ process: { process_graph: previewGraph(dataset, granule) } }),
            })
            .then((blob) => resolve(URL.createObjectURL(blob)), reject)
            .finally(() => {
              this.#inFlight -= 1;
              this.#queue.shift()?.();
            });
        };
        if (this.#inFlight < PREVIEW_CONCURRENCY) {
          run();
        } else {
          this.#queue.push(run);
        }
      });
      thumbnails.set(key, promise);
      promise.catch(() => thumbnails.delete(key)); // a refusal is retried on the next open
    }
    promise.then(
      (url) => {
        card.thumbnail = url;
        card.note = undefined;
      },
      (error: unknown) => {
        card.thumbnail = undefined;
        card.note =
          error instanceof ApiProblem && error.detail !== ""
            ? `no preview — ${error.detail}`
            : `no preview — ${error instanceof Error ? error.message : String(error)}`;
      },
    );
  }

  protected render(): void {
    if (this.active && !this.#loaded) {
      void this.reload();
    }
    const datasetField = el("swath-field", {
      type: "select",
      name: "dataset",
      label: "Dataset",
      part: "dataset",
    });
    datasetField.options = [
      { value: "", label: this.#datasets.length === 0 ? "—" : "Choose a dataset" },
      ...this.#datasets.map((d) => ({ value: d.id, label: `${d.title} (${d.id})` })),
    ];
    datasetField.value = this.#selected;
    datasetField.addEventListener("swath-change", (event) => {
      event.stopPropagation();
      void this.select(String(event.detail.value));
    });
    const from = el("swath-field", {
      type: "date",
      name: "from",
      label: "From",
      value: this.#filter.from ?? "",
    });
    const to = el("swath-field", {
      type: "date",
      name: "to",
      label: "To",
      value: this.#filter.to ?? "",
    });
    for (const [field, key] of [
      [from, "from"],
      [to, "to"],
    ] as const) {
      field.addEventListener("swath-change", (event) => {
        event.stopPropagation();
        const value = String(event.detail.value);
        this.#filter = { ...this.#filter, [key]: value === "" ? undefined : value };
        // The date fields and the timeline narrow the same scope, so they
        // announce the same way: one path to the URL's date chip.
        this.emit("swath-dates", {
          from: this.#filter.from ?? null,
          to: this.#filter.to ?? null,
        });
        this.requestUpdate();
        void this.#refreshTimeline();
      });
    }
    const inView = el("swath-toggle", {
      name: "in-view",
      label: "Only granules in the current view",
      checked: this.#inView,
    });
    inView.append(el("span", { slot: "label" }, "in current view"));
    inView.addEventListener("swath-change", (event) => {
      event.stopPropagation();
      this.#inView = event.detail.value === true;
      this.requestUpdate();
    });
    const filters = el(
      "div",
      { part: "filters" },
      datasetField,
      el("div", { part: "dates" }, from, to),
      inView,
    );

    const visible = sortGranules(
      filterGranules(this.#granules, {
        ...this.#filter,
        view: this.#inView ? this.#viewBounds : undefined,
      }),
      this.#sort,
    );
    const sort = el("swath-field", {
      type: "select",
      name: "sort",
      label: "Sort",
      value: this.#sort,
    });
    sort.options = [
      { value: "newest", label: "newest first" },
      { value: "oldest", label: "oldest first" },
      { value: "id", label: "by id" },
    ];
    sort.addEventListener("swath-change", (event) => {
      event.stopPropagation();
      this.#sort = String(event.detail.value) as GranuleSort;
      this.requestUpdate();
    });
    const layout = el("swath-button", {
      icon: this.layout === "list" ? "menu" : "layers",
      size: "sm",
      label: this.layout === "list" ? "Show as a grid" : "Show as a list",
      part: "layout",
    });
    layout.addEventListener("click", () => {
      this.layout = this.layout === "list" ? "grid" : "list";
    });
    const count = el(
      "p",
      { part: "count", role: "status" },
      this.#selected === ""
        ? ""
        : `${visible.length} of ${this.#granules.length} granule${this.#granules.length === 1 ? "" : "s"}`,
    );
    const toolbar = el("div", { part: "toolbar" }, count, sort, layout);

    let body: HTMLElement;
    const dataset = this.#datasets.find((d) => d.id === this.#selected);
    if (this.#datasetsError !== undefined) {
      body = el(
        "p",
        { part: "error", role: "alert" },
        `Could not list datasets: ${this.#datasetsError}`,
      );
    } else if (this.#loading) {
      body = el("p", { part: "loading", "aria-busy": "true" }, "Loading…");
    } else if (this.#selected === "" || !dataset) {
      body = el(
        "p",
        { part: "empty" },
        this.#datasets.length === 0 && this.#loaded
          ? "No datasets registered."
          : "Choose a dataset to see its granules.",
      );
    } else if (this.#granulesError !== undefined) {
      body = el(
        "p",
        { part: "error", role: "alert" },
        `Could not list granules: ${this.#granulesError}`,
      );
    } else if (this.#granules.length === 0) {
      body = el("p", { part: "empty" }, GRANULES_EMPTY_GUIDANCE);
    } else if (visible.length === 0) {
      body = el("p", { part: "empty" }, "No granules match the filters.");
    } else {
      const seen = new Set<string>();
      const kind = previewKind(dataset);
      const items = visible.map((granule) => {
        seen.add(granule.id);
        let card = this.#cards.get(granule.id);
        if (!card) {
          card = el("swath-granule-card", { "granule-id": granule.id });
          card.addEventListener("swath-activate", (event) => {
            event.stopPropagation();
            this.emit("swath-granule-zoom", {
              dataset: dataset.id,
              id: granule.id,
              bbox: granule.bbox,
            });
          });
          this.#cards.set(granule.id, card);
          this.#thumbnail(dataset, granule, card);
        }
        card.datasetId = dataset.id;
        card.datetime = granule.datetime;
        card.kind = kind;
        card.layout = this.layout ?? "grid";
        return el("li", {}, card);
      });
      for (const id of this.#cards.keys()) {
        if (!seen.has(id)) {
          this.#cards.delete(id);
        }
      }
      body = el("ul", { part: "grid" }, ...items);
    }
    const facets = this.#facetBlock();
    const timeline = this.#timelineBlock();
    this.renderRoot.replaceChildren(
      filters,
      ...(timeline === undefined ? [] : [timeline]),
      ...(facets === undefined ? [] : [facets]),
      toolbar,
      body,
    );
  }
}

/** Registers `<swath-catalog>`; safe to call more than once. */
export function defineSwathCatalog(): void {
  SwathTimeline.define();
  SwathCatalog.define();
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-catalog": SwathCatalog;
  }
}
