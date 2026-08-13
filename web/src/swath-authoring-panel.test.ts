// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The authoring panel's Model B contract (issue #151, design note §4):
// the pipeline is NEVER in an invalid state. Each reachable-bad-state
// from the design note's §2 matrix (B1–B11) is pinned here as either
// unconstructible or self-explaining — test names cite the B-numbers.
// Schema honesty carries over from #148: cards, chips, and fields come
// from the mocked `GET /processes` alone — remove a definition and its
// UI is gone. Composition, publish (success + inline openEO error),
// the NDVI template, the formula builder, and delete are exercised
// against a scripted fetch; no server involved. Real Custom Elements in
// a real browser, like the rest of the suite.
import { beforeAll, beforeEach, expect, test } from "vitest";
import {
  defineSwathAuthoringPanel,
  type ProcessDefinition,
  type SwathAuthoringPanel,
} from "./swath-authoring-panel.js";

/** Pinned-shape stand-ins for the served openeo-processes definitions —
 * the same structural slice (id/summary/parameters with schemas) the
 * real `GET /processes` serves. */
const DEFINITIONS: ProcessDefinition[] = [
  {
    id: "load_collection",
    summary: "Load a collection",
    parameters: [
      { name: "id", schema: { type: "string", subtype: "collection-id" } },
      {
        name: "spatial_extent",
        schema: [
          { title: "Bounding Box", type: "object", subtype: "bounding-box" },
          { title: "No filter", type: "null" },
        ],
      },
      {
        name: "temporal_extent",
        schema: [
          { type: "array", subtype: "temporal-interval" },
          { title: "No filter", type: "null" },
        ],
      },
      {
        name: "bands",
        optional: true,
        default: null,
        schema: [
          { type: "array", items: { type: "string", subtype: "band-name" } },
          { title: "No filter", type: "null" },
        ],
      },
    ],
  },
  {
    id: "ndvi",
    summary: "Normalized Difference Vegetation Index",
    parameters: [
      { name: "data", schema: { type: "object", subtype: "raster-cube" } },
      {
        name: "nir",
        optional: true,
        default: "nir",
        schema: { type: "string", subtype: "band-name" },
      },
      {
        name: "red",
        optional: true,
        default: "red",
        schema: { type: "string", subtype: "band-name" },
      },
      {
        name: "target_band",
        optional: true,
        default: null,
        schema: [{ type: "string" }, { type: "null" }],
      },
    ],
  },
  {
    id: "reduce_dimension",
    summary: "Reduce dimensions",
    parameters: [
      { name: "data", schema: { type: "object", subtype: "raster-cube" } },
      { name: "reducer", schema: { type: "object", subtype: "process-graph" } },
      { name: "dimension", schema: { type: "string" } },
      { name: "context", optional: true, default: null, schema: {} },
    ],
  },
  {
    id: "array_element",
    summary: "Get an element from an array",
    parameters: [
      { name: "data", schema: { type: "array" } },
      { name: "index", optional: true, schema: { type: "integer" } },
      { name: "label", optional: true, schema: [{ type: "number" }, { type: "string" }] },
    ],
  },
  {
    id: "add",
    summary: "Addition of two numbers",
    parameters: [
      { name: "x", schema: { type: ["number", "null"] } },
      { name: "y", schema: { type: ["number", "null"] } },
    ],
  },
  {
    id: "subtract",
    summary: "Subtraction of two numbers",
    parameters: [
      { name: "x", schema: { type: ["number", "null"] } },
      { name: "y", schema: { type: ["number", "null"] } },
    ],
  },
  {
    id: "multiply",
    summary: "Multiplication of two numbers",
    parameters: [
      { name: "x", schema: { type: ["number", "null"] } },
      { name: "y", schema: { type: ["number", "null"] } },
    ],
  },
  {
    id: "divide",
    summary: "Division of two numbers",
    parameters: [
      { name: "x", schema: { type: ["number", "null"] } },
      { name: "y", schema: { type: ["number", "null"] } },
    ],
  },
  {
    id: "linear_scale_range",
    summary: "Linear transformation between two ranges",
    parameters: [
      { name: "x", schema: { type: ["number", "null"] } },
      { name: "inputMin", schema: { type: "number" } },
      { name: "inputMax", schema: { type: "number" } },
      { name: "outputMin", optional: true, default: 0, schema: { type: "number" } },
      { name: "outputMax", optional: true, default: 1, schema: { type: "number" } },
    ],
  },
  {
    id: "save_result",
    summary: "Save processed data",
    parameters: [
      { name: "data", schema: { type: "object", subtype: "raster-cube" } },
      { name: "format", schema: { type: "string", subtype: "output-format" } },
      {
        name: "options",
        optional: true,
        default: {},
        schema: { type: "object", subtype: "output-format-options" },
      },
    ],
  },
];

/** A collections document shaped like the openEO surface serves it:
 * id plus datacube band values. */
const COLLECTIONS = [
  {
    id: "hls-s30",
    "cube:dimensions": {
      x: { type: "spatial" },
      bands: { type: "bands", values: ["b02", "b03", "b04", "b8a"] },
    },
  },
];

/** One recorded request the fetch stub saw. */
interface Recorded {
  method: string;
  url: string;
  body: unknown;
}

/** A 1×1 PNG (the preview stub's default body — the panel only needs
 * real image bytes to object-URL). */
const PNG_BYTES = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
  ),
  (char) => char.codePointAt(0) ?? 0,
);

/** A scripted same-shape fetch: the GET surfaces answer the given
 * documents; POST /services, POST /result (the ADR 0014 preview), and
 * DELETE answer the scripted response; everything is recorded for
 * assertions. */
function fetchStub(options: {
  processes?: ProcessDefinition[];
  collections?: unknown[];
  services?: { id: string; title?: string }[];
  post?: { status: number; body?: unknown; headers?: Record<string, string> };
  result?: { status: number; body?: unknown };
  delete?: { status: number; body?: unknown };
}): { impl: typeof fetch; requests: Recorded[] } {
  const requests: Recorded[] = [];
  const impl = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    requests.push({
      method,
      url,
      body: typeof init?.body === "string" ? JSON.parse(init.body) : undefined,
    });
    const json = (body: unknown, status = 200, headers?: Record<string, string>) =>
      new Response(JSON.stringify(body ?? {}), {
        status,
        headers: { "content-type": "application/json", ...headers },
      });
    if (method === "GET" && url.endsWith("/processes")) {
      return json({ processes: options.processes ?? DEFINITIONS });
    }
    if (method === "GET" && url.endsWith("/collections")) {
      return json({ collections: options.collections ?? COLLECTIONS });
    }
    if (method === "GET" && url.endsWith("/services")) {
      return json({ services: options.services ?? [] });
    }
    if (method === "POST" && url.endsWith("/services")) {
      const post = options.post ?? { status: 201 };
      return json(post.body, post.status, post.headers);
    }
    if (method === "POST" && url.endsWith("/result")) {
      const result = options.result ?? { status: 200 };
      if (result.status === 200) {
        return new Response(new Blob([PNG_BYTES], { type: "image/png" }), {
          status: 200,
          headers: { "content-type": "image/png" },
        });
      }
      return json(result.body, result.status);
    }
    if (method === "DELETE") {
      const del = options.delete ?? { status: 204 };
      return new Response(null, { status: del.status });
    }
    return new Response("not scripted", { status: 500 });
  }) as typeof fetch;
  return { impl, requests };
}

beforeAll(() => {
  defineSwathAuthoringPanel();
});

beforeEach(() => {
  document.body.replaceChildren();
});

async function mount(stub: { impl: typeof fetch }): Promise<SwathAuthoringPanel> {
  const panel = document.createElement("swath-authoring-panel") as SwathAuthoringPanel;
  panel.fetchImpl = stub.impl;
  document.body.append(panel);
  await panel.reload();
  return panel;
}

function field<T extends HTMLElement>(panel: SwathAuthoringPanel, id: string): T {
  const element = panel.querySelector<T>(`#swath-authoring-${id}`);
  if (!element) {
    throw new Error(`no field ${id}`);
  }
  return element;
}

function fill(panel: SwathAuthoringPanel, id: string, value: string): void {
  const input = field<HTMLInputElement>(panel, id);
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function choose(panel: SwathAuthoringPanel, id: string, value: string): void {
  const select = field<HTMLSelectElement>(panel, id);
  select.value = value;
  select.dispatchEvent(new Event("change", { bubbles: true }));
}

/** Ticks a band checkbox on the Load card (tick order = loaded order). */
function tickBand(panel: SwathAuthoringPanel, band: string): void {
  field<HTMLInputElement>(panel, `s1-bands-${band}`).click();
}

/** The insert chips offered at `gap` (their process ids). */
function chipsAt(panel: SwathAuthoringPanel, gap: number): string[] {
  return [
    ...panel.querySelectorAll<HTMLButtonElement>(
      `.swath-authoring-insert[data-gap="${gap}"] button`,
    ),
  ].map((button) => button.dataset["process"] ?? "");
}

/** Every insert chip currently offered anywhere on the canvas. */
function allChips(panel: SwathAuthoringPanel): string[] {
  return [...panel.querySelectorAll<HTMLButtonElement>(".swath-authoring-insert button")].map(
    (button) => button.dataset["process"] ?? "",
  );
}

function insertAt(panel: SwathAuthoringPanel, gap: number, processId: string): void {
  const button = panel.querySelector<HTMLButtonElement>(
    `.swath-authoring-insert[data-gap="${gap}"] button[data-process="${processId}"]`,
  );
  if (!button) {
    throw new Error(`no chip for ${processId} at gap ${gap}`);
  }
  button.click();
}

function stepProcesses(panel: SwathAuthoringPanel): string[] {
  return [...panel.querySelectorAll<HTMLElement>(".swath-authoring-step")].map(
    (item) => item.dataset["process"] ?? "",
  );
}

function submitButton(panel: SwathAuthoringPanel): HTMLButtonElement {
  const button = panel.querySelector<HTMLButtonElement>(".swath-authoring-submit");
  if (!button) {
    throw new Error("no submit button");
  }
  return button;
}

function submitReason(panel: SwathAuthoringPanel): string {
  return panel.querySelector("#swath-authoring-submit-reason")?.textContent ?? "";
}

function openAdvanced(panel: SwathAuthoringPanel, stepKey: string): void {
  const toggle = panel.querySelector<HTMLButtonElement>(
    `[data-step="${stepKey}"] .swath-authoring-advanced-toggle`,
  );
  if (!toggle) {
    throw new Error(`no advanced toggle on ${stepKey}`);
  }
  toggle.click();
}

function helpText(panel: SwathAuthoringPanel, id: string): string {
  return (
    panel.querySelector(`label[for="swath-authoring-${id}"] .swath-authoring-field-help`)
      ?.textContent ?? ""
  );
}

test("collapsed by default and lazy: no requests until opened", async () => {
  const stub = fetchStub({});
  const panel = document.createElement("swath-authoring-panel") as SwathAuthoringPanel;
  panel.fetchImpl = stub.impl;
  document.body.append(panel);
  // Connected but closed: nothing fetched, only the toggle rendered.
  expect(stub.requests).toEqual([]);
  const toggle = panel.querySelector<HTMLButtonElement>(".swath-authoring-toggle");
  expect(toggle?.getAttribute("aria-expanded")).toBe("false");
  // The first open loads the definitions (and renders the canvas).
  toggle?.click();
  await expect.poll(() => panel.querySelector('[data-step="s1"]')).not.toBeNull();
  expect(stub.requests.map((request) => request.url).sort()).toEqual([
    "/collections",
    "/processes",
    "/services",
  ]);
});

test("B1: the canvas is a permanent Load → Output pipeline — the save_result tail cannot be removed", async () => {
  const panel = await mount(fetchStub({}));
  // The empty canvas is already the full frame: Load then Output.
  expect(stepProcesses(panel)).toEqual(["load_collection", "save_result"]);
  // Neither permanent card offers a remove control…
  expect(panel.querySelector('[data-step="s1"] [aria-label^="Remove step"]')).toBeNull();
  expect(panel.querySelector('[data-step="s2"] [aria-label^="Remove step"]')).toBeNull();
  // …and the graph always ends in save_result with result: true, even
  // after inserting and removing middle steps.
  insertAt(panel, 0, "ndvi");
  panel.querySelector<HTMLButtonElement>('[aria-label="Remove step s2"]')?.click();
  const graph = panel.buildGraph() as Record<string, { process_id: string; result?: boolean }>;
  const keys = Object.keys(graph);
  const last = graph[keys[keys.length - 1] ?? ""];
  expect(last?.process_id).toBe("save_result");
  expect(last?.result).toBe(true);
});

test("B2/B3: arithmetic and array_element are never offered as pipeline steps — the formula builder owns them", async () => {
  const panel = await mount(fetchStub({}));
  const forbidden = ["add", "subtract", "multiply", "divide", "array_element"];
  const offered = new Set(allChips(panel));
  for (const id of forbidden) {
    expect(offered.has(id), id).toBe(false);
  }
  // Still true at every gap of a longer pipeline (and load/save are
  // never chips either — they are the permanent frame).
  insertAt(panel, 0, "ndvi");
  const later = new Set(allChips(panel));
  for (const id of [...forbidden, "load_collection", "save_result"]) {
    expect(later.has(id), id).toBe(false);
  }
});

test("B4: scale-before-reduce is unconstructible — chips only appear where the whole pipeline still types", async () => {
  const panel = await mount(fetchStub({}));
  // Fresh canvas: everything fits right after Load.
  expect(chipsAt(panel, 0)).toEqual(["ndvi", "reduce_dimension", "linear_scale_range"]);
  // With a scale step in place, the gap BEFORE it still offers the
  // reduce steps (reduce-then-scale types)…
  insertAt(panel, 0, "linear_scale_range");
  expect(chipsAt(panel, 0)).toEqual(["ndvi", "reduce_dimension"]);
  // …but the gap AFTER it offers nothing: nothing reduces or re-scales
  // a scaled cube, so the row is not rendered at all.
  expect(panel.querySelector('.swath-authoring-insert[data-gap="1"]')).toBeNull();
  // A complete NDVI pipeline is saturated: no insert rows anywhere.
  panel.querySelector<HTMLButtonElement>('[aria-label="Remove step s2"]')?.click();
  insertAt(panel, 0, "ndvi");
  insertAt(panel, 1, "linear_scale_range");
  expect(allChips(panel)).toEqual([]);
});

test("B5: a multi-band pipeline that is not an RGB composite explains itself and gates submit", async () => {
  const panel = await mount(fetchStub({}));
  choose(panel, "s1-id", "hls-s30");
  tickBand(panel, "b02");
  tickBand(panel, "b03");
  expect(submitButton(panel).disabled).toBe(true);
  expect(submitReason(panel)).toContain(
    "produces 2 channels; a picture needs 1 (add NDVI or a formula) or 3 (red, green, blue)",
  );
  // Ticking a third band makes it a lawful RGB composite: reason gone,
  // submit enabled with zero expert decisions (extents null, png).
  tickBand(panel, "b04");
  expect(submitReason(panel)).toBe("");
  expect(submitButton(panel).disabled).toBe(false);
});

test("B6: the colormap greys out on a multi-band result, says why, and never publishes", async () => {
  const panel = await mount(fetchStub({}));
  choose(panel, "s1-id", "hls-s30");
  tickBand(panel, "b02");
  tickBand(panel, "b03");
  tickBand(panel, "b04");
  // Multi-band result: the select is disabled and the card explains in
  // plain words.
  expect(field<HTMLSelectElement>(panel, "s2-options").disabled).toBe(true);
  expect(field(panel, "s2-composite-note").textContent).toContain(
    "A colormap maps one gray value per pixel",
  );
  // Adding NDVI reduces to gray: the colormap becomes available.
  insertAt(panel, 0, "ndvi");
  expect(field<HTMLSelectElement>(panel, "s3-options").disabled).toBe(false);
  choose(panel, "s3-options", "viridis");
  // Removing the reduce step again: the stored colormap cannot ride the
  // composite — buildGraph omits it (unconstructible, not just noted).
  panel.querySelector<HTMLButtonElement>('[aria-label="Remove step s2"]')?.click();
  expect(field<HTMLSelectElement>(panel, "s2-options").disabled).toBe(true);
  const graph = panel.buildGraph() as Record<string, { arguments: Record<string, unknown> }>;
  expect(graph["s2"]?.arguments["options"]).toBeUndefined();
});

test("B7: bands are vocabulary widgets — checkboxes on Load, selects for NDVI — never free text", async () => {
  const panel = await mount(fetchStub({}));
  // Before a collection is chosen the bands widget says what to do.
  expect(field(panel, "s1-bands").textContent).toContain("Choose a dataset first");
  choose(panel, "s1-id", "hls-s30");
  // One checkbox per band of the CHOSEN collection; no text input.
  const boxes = [
    ...panel.querySelectorAll<HTMLInputElement>('#swath-authoring-s1-bands input[type="checkbox"]'),
  ];
  expect(boxes.map((box) => box.dataset["band"])).toEqual(["b02", "b03", "b04", "b8a"]);
  // NDVI's nir/red are selects over the LOADED bands only (tick order).
  tickBand(panel, "b8a");
  tickBand(panel, "b04");
  insertAt(panel, 0, "ndvi");
  const nir = field<HTMLSelectElement>(panel, "s2-nir");
  expect(nir.tagName).toBe("SELECT");
  expect([...nir.options].map((option) => option.value)).toEqual(["", "b8a", "b04"]);
  // The prefill heuristics picked the right bands already.
  expect(nir.value).toBe("b8a");
  expect(field<HTMLSelectElement>(panel, "s2-red").value).toBe("b04");
  // Unticking a band a step still uses flags it in plain words.
  tickBand(panel, "b04");
  expect(field(panel, "s2-red-note").textContent).toContain("b04 is not loaded any more");
  expect(submitButton(panel).disabled).toBe(true);
});

test("B8: a degenerate stretch range flags inline before any request", async () => {
  const panel = await mount(fetchStub({}));
  panel.querySelector<HTMLButtonElement>(".swath-authoring-template")?.click();
  fill(panel, "s3-inputMin", "2");
  fill(panel, "s3-inputMax", "1");
  expect(field(panel, "s3-inputMin-note").textContent).toBe(
    "the smallest value must be below the largest",
  );
  expect(submitButton(panel).disabled).toBe(true);
  fill(panel, "s3-inputMax", "3");
  expect(field(panel, "s3-inputMin-note").textContent).toBe("");
  expect(submitButton(panel).disabled).toBe(false);
});

test("B9: the output format is a select over the profile vocabulary — png, no free text", async () => {
  const panel = await mount(fetchStub({}));
  openAdvanced(panel, "s2");
  const format = field<HTMLSelectElement>(panel, "s2-format");
  expect(format.tagName).toBe("SELECT");
  expect([...format.options].map((option) => option.value)).toEqual(["png"]);
  expect(format.value).toBe("png");
});

test("B10: the canvas is a linear chain — every step feeds the next, nothing dangles", async () => {
  const panel = await mount(fetchStub({}));
  choose(panel, "s1-id", "hls-s30");
  tickBand(panel, "b8a");
  tickBand(panel, "b04");
  insertAt(panel, 0, "ndvi");
  insertAt(panel, 1, "linear_scale_range");
  const wired = (graph: Record<string, { arguments: Record<string, unknown> }>): string[] =>
    Object.keys(graph).map((key) => {
      const args = graph[key]?.arguments ?? {};
      const reference = Object.values(args).find(
        (value) => typeof value === "object" && value !== null && "from_node" in value,
      ) as { from_node?: string } | undefined;
      return reference?.from_node ?? "";
    });
  // s1 ← nothing, s2 ← s1, s3 ← s2, s4 ← s3: a chain by construction.
  expect(wired(panel.buildGraph() as never)).toEqual(["", "s1", "s2", "s3"]);
  // Removing a middle step REWIRES rather than dangles: the neighbors
  // join up and the keys stay positional.
  panel.querySelector<HTMLButtonElement>('[aria-label="Remove step s2"]')?.click();
  const after = panel.buildGraph() as Record<string, { process_id: string }>;
  expect(Object.keys(after)).toEqual(["s1", "s2", "s3"]);
  expect(after["s2"]?.process_id).toBe("linear_scale_range");
  expect(wired(after as never)).toEqual(["", "s1", "s2"]);
});

test("B11: the narrative retells the exact pipeline live — the words show a swap the schema cannot catch", async () => {
  // Swapped nir/red publishes fine and renders wrong; no validator can
  // catch it (the design note sends B11 to preview, a follow-up ADR).
  // The narrative is the honest countermeasure available today: it
  // spells the formula out with the user's actual choices.
  const panel = await mount(fetchStub({}));
  panel.querySelector<HTMLButtonElement>(".swath-authoring-template")?.click();
  const narrative = () => panel.querySelector("#swath-authoring-narrative")?.textContent ?? "";
  expect(narrative()).toBe(
    "Load hls-s30 (bands b8a,b04) → compute NDVI ((b8a − b04) / (b8a + b04)) → " +
      "rescale -1..1 to 0..255 → save as png, colored with rdylgn.",
  );
  choose(panel, "s2-nir", "b04");
  choose(panel, "s2-red", "b8a");
  expect(narrative()).toContain("compute NDVI ((b04 − b8a) / (b04 + b8a))");
  // Live: changing the colormap re-narrates without a re-render cycle.
  choose(panel, "s4-options", "viridis");
  expect(narrative()).toContain("colored with viridis");
});

test("schema honesty: chips and cards exist only for served definitions", async () => {
  const without = (...ids: string[]): ProcessDefinition[] =>
    DEFINITIONS.filter((process) => !ids.includes(process.id));
  // Delete ndvi: its chip is gone (and the template with it).
  const noNdvi = await mount(fetchStub({ processes: without("ndvi") }));
  expect(chipsAt(noNdvi, 0)).toEqual(["reduce_dimension", "linear_scale_range"]);
  expect(noNdvi.querySelector(".swath-authoring-template")).toBeNull();
  // The formula chip needs its whole toolkit: reduce_dimension,
  // array_element, and at least one arithmetic op.
  document.body.replaceChildren();
  const noElement = await mount(fetchStub({ processes: without("array_element") }));
  expect(chipsAt(noElement, 0)).toEqual(["ndvi", "linear_scale_range"]);
  document.body.replaceChildren();
  const noOps = await mount(
    fetchStub({ processes: without("add", "subtract", "multiply", "divide") }),
  );
  expect(chipsAt(noOps, 0)).toEqual(["ndvi", "linear_scale_range"]);
  // Without save_result there is no lawful pipeline at all: the canvas
  // says so instead of offering one.
  document.body.replaceChildren();
  const noSave = await mount(fetchStub({ processes: without("save_result") }));
  expect(noSave.querySelector(".swath-authoring-step")).toBeNull();
  expect(noSave.querySelector(".swath-authoring-empty")?.textContent).toContain(
    "load_collection and save_result",
  );
});

test("every field carries a visible plain-language explainer, and the Load card speaks area/time plainly", async () => {
  const panel = await mount(fetchStub({}));
  expect(helpText(panel, "s1-id")).toBe("Which dataset to compute from.");
  // The area extent stays an advanced expert field; the card carries
  // the plain-worded summary of what the current choices mean.
  expect(field(panel, "s1-extent-summary").textContent).toBe(
    "Area: everywhere the collection covers · Time: everything available",
  );
  // Time is the card's own plain-words control (ADR 0015), visible
  // without the advanced toggle, with its explainer beside it.
  const whenHelp = field(panel, "s1-temporal_extent")
    .closest("label")
    ?.querySelector(".swath-authoring-field-help")?.textContent;
  expect(whenHelp).toBe(
    "When: the dates to show — leave both empty to use everything available. " +
      "The map shows the newest image inside the range (the end date itself is not included).",
  );
  openAdvanced(panel, "s1");
  expect(helpText(panel, "s1-spatial_extent")).toBe(
    "The map area to compute over — leave as is to use the whole collection.",
  );
});

test("the Load card's when control is two date pickers that window the graph", async () => {
  const panel = await mount(fetchStub({}));
  choose(panel, "s1-id", "hls-s30");
  tickBand(panel, "b8a");
  tickBand(panel, "b04");
  // The control is on the card itself — no advanced toggle needed.
  const setDate = (slot: "from" | "until", value: string): void => {
    const input = field<HTMLInputElement>(panel, `s1-temporal_extent-${slot}`);
    input.value = value;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  };
  setDate("from", "2024-06-01");
  setDate("until", "2024-09-01");
  // The when line and the narrative speak the choice plainly...
  expect(field(panel, "s1-extent-summary").textContent).toBe(
    "Area: everywhere the collection covers · Time: 2024-06-01 until 2024-09-01",
  );
  // ...and the graph carries the interval (dates are valid openEO
  // temporal-interval bounds; the compiler treats the end as excluded).
  const graph = panel.buildGraph() as Record<string, { arguments: Record<string, unknown> }>;
  expect(graph["s1"]?.arguments["temporal_extent"]).toEqual(["2024-06-01", "2024-09-01"]);

  // One side open: null on that side, per the spec.
  setDate("until", "");
  expect(field(panel, "s1-extent-summary").textContent).toBe(
    "Area: everywhere the collection covers · Time: from 2024-06-01",
  );
  const open = panel.buildGraph() as Record<string, { arguments: Record<string, unknown> }>;
  expect(open["s1"]?.arguments["temporal_extent"]).toEqual(["2024-06-01", null]);

  // Both cleared: back to "everything available" — explicit null, the
  // exact pre-#181 graph (see NDVI_GRAPH).
  setDate("from", "");
  const cleared = panel.buildGraph() as Record<string, { arguments: Record<string, unknown> }>;
  expect(cleared["s1"]?.arguments["temporal_extent"]).toBeNull();
});

test("submit stays disabled with spelled-out reasons until the pipeline is complete", async () => {
  const panel = await mount(fetchStub({}));
  // The permanent frame alone: no collection yet, and the reasons say
  // so in order (the server's CollectionNotFound/UnknownBand family is
  // unreachable from here).
  expect(submitButton(panel).disabled).toBe(true);
  expect(submitReason(panel)).toContain("no collection chosen yet");
  choose(panel, "s1-id", "hls-s30");
  expect(submitReason(panel)).toContain("no bands ticked yet");
  tickBand(panel, "b8a");
  // One band, no reduce: B5's explanation takes over.
  expect(submitReason(panel)).toContain("produces 1 channel");
  insertAt(panel, 0, "ndvi");
  // nir/red prefill from the single loaded band cannot pick two: the
  // fields flag and count.
  expect(submitReason(panel)).toContain("fields need values");
  tickBand(panel, "b04");
  choose(panel, "s2-nir", "b8a");
  choose(panel, "s2-red", "b04");
  expect(submitButton(panel).disabled).toBe(false);
  expect(submitReason(panel)).toBe("");
});

/** Drives the full NDVI authoring flow through the Model B canvas —
 * vocabulary widgets only: collection select, band checkboxes (tick
 * order = loaded order), prefilled nir/red selects, range numbers, the
 * colormap select. Extents and format ride their smart defaults. */
function authorNdvi(panel: SwathAuthoringPanel): void {
  choose(panel, "s1-id", "hls-s30");
  tickBand(panel, "b8a");
  tickBand(panel, "b04");
  insertAt(panel, 0, "ndvi");
  insertAt(panel, 1, "linear_scale_range");
  fill(panel, "s3-inputMin", "-1");
  fill(panel, "s3-inputMax", "1");
  choose(panel, "s4-options", "rdylgn");
}

/** The graph [`authorNdvi`] composes — also what the NDVI template must
 * produce over the mocked collection (b8a/b04 via the band heuristics).
 * Identical to the #148 panel's graph: Model B changed how the pipeline
 * is assembled, not what it publishes. */
const NDVI_GRAPH = {
  s1: {
    process_id: "load_collection",
    arguments: {
      id: "hls-s30",
      spatial_extent: null,
      temporal_extent: null,
      bands: ["b8a", "b04"],
    },
  },
  s2: {
    process_id: "ndvi",
    arguments: { data: { from_node: "s1" }, nir: "b8a", red: "b04" },
  },
  s3: {
    process_id: "linear_scale_range",
    arguments: {
      x: { from_node: "s2" },
      inputMin: -1,
      inputMax: 1,
      outputMin: 0,
      outputMax: 255,
    },
  },
  s4: {
    process_id: "save_result",
    arguments: {
      data: { from_node: "s3" },
      format: "png",
      options: { colormap: "rdylgn" },
    },
    result: true,
  },
};

test("publishing posts the composed graph and announces the created service", async () => {
  const stub = fetchStub({
    post: { status: 201, headers: { "openeo-identifier": "xyz-abc123def456" } },
  });
  const panel = await mount(stub);
  authorNdvi(panel);
  fill(panel, "title", "NDVI (authored)");
  expect(submitButton(panel).disabled).toBe(false);

  const created = new Promise<string>((resolve) => {
    document.body.addEventListener(
      "swath-service-created",
      (event) => resolve((event as CustomEvent<{ id: string }>).detail.id),
      { once: true },
    );
  });
  submitButton(panel).click();
  expect(await created).toBe("xyz-abc123def456");

  const post = stub.requests.find(
    (request) => request.method === "POST" && request.url === "/services",
  );
  expect(post?.body).toEqual({
    type: "xyz",
    title: "NDVI (authored)",
    process: { process_graph: NDVI_GRAPH },
  });
});

test("the NDVI template composes a valid, submittable pipeline", async () => {
  const panel = await mount(fetchStub({}));
  const template = panel.querySelector<HTMLButtonElement>(".swath-authoring-template");
  expect(template).not.toBeNull();
  template?.click();
  // The template fills a pipeline that renders: the first collection,
  // nir/red picked from its band vocabulary, the built-in NDVI scale
  // and colormap — identical to the hand-authored pipeline.
  expect(field<HTMLSelectElement>(panel, "s1-id").value).toBe("hls-s30");
  expect(field<HTMLSelectElement>(panel, "s4-options").value).toBe("rdylgn");
  expect(submitButton(panel).disabled).toBe(false);
  expect(submitReason(panel)).toBe("");
  expect(panel.buildGraph()).toEqual(NDVI_GRAPH);
});

test("the formula builder composes the reduce_dimension reducer child graph", async () => {
  const panel = await mount(fetchStub({}));
  choose(panel, "s1-id", "hls-s30");
  tickBand(panel, "b8a");
  tickBand(panel, "b04");
  insertAt(panel, 0, "reduce_dimension");
  // The card starts with one incomplete line and explains what is
  // missing (self-explaining, never silently wrong).
  expect(field(panel, "s2-formula-issues").textContent).toContain("line 1: pick the left value");
  expect(submitButton(panel).disabled).toBe(true);
  // NDVI by hand: line1 = b8a − b04; line2 = b8a + b04; line3 = l1 ÷ l2.
  choose(panel, "s2-row1-left", "band:b8a");
  choose(panel, "s2-row1-op", "subtract");
  choose(panel, "s2-row1-right", "band:b04");
  panel.querySelector<HTMLButtonElement>(".swath-authoring-formula-add")?.click();
  choose(panel, "s2-row2-left", "band:b8a");
  choose(panel, "s2-row2-op", "add");
  choose(panel, "s2-row2-right", "band:b04");
  panel.querySelector<HTMLButtonElement>(".swath-authoring-formula-add")?.click();
  choose(panel, "s2-row3-left", "row:0");
  choose(panel, "s2-row3-op", "divide");
  choose(panel, "s2-row3-right", "row:1");
  expect(field(panel, "s2-formula-issues").textContent).toBe("");
  // A complete formula unblocks submit: the card's pinned dimension and
  // composed reducer never count as missing fields.
  expect(submitButton(panel).disabled).toBe(false);
  // The narrative reads the formula back as plain math.
  expect(panel.querySelector("#swath-authoring-narrative")?.textContent).toContain(
    "combine the bands with a formula ((b8a − b04) ÷ (b8a + b04))",
  );
  const graph = panel.buildGraph() as Record<string, { arguments: Record<string, unknown> }>;
  expect(graph["s2"]).toEqual({
    process_id: "reduce_dimension",
    arguments: {
      data: { from_node: "s1" },
      dimension: "bands",
      reducer: {
        process_graph: {
          b1: {
            process_id: "array_element",
            arguments: { data: { from_parameter: "data" }, label: "b8a" },
          },
          b2: {
            process_id: "array_element",
            arguments: { data: { from_parameter: "data" }, label: "b04" },
          },
          r1: {
            process_id: "subtract",
            arguments: { x: { from_node: "b1" }, y: { from_node: "b2" } },
          },
          r2: {
            process_id: "add",
            arguments: { x: { from_node: "b1" }, y: { from_node: "b2" } },
          },
          r3: {
            process_id: "divide",
            arguments: { x: { from_node: "r1" }, y: { from_node: "r2" } },
            result: true,
          },
        },
      },
    },
  });
  // Gray result: the colormap select is live again (B6's flip side).
  expect(field<HTMLSelectElement>(panel, "s3-options").disabled).toBe(false);
});

test("a server error naming a node and argument lands on that field (the safety net)", async () => {
  const message =
    "node `s3` (linear_scale_range): invalid argument `outputMin`: the Render IR quantizes " +
    "to 8-bit; the output range must be exactly 0..255, got 0..1";
  const stub = fetchStub({
    post: { status: 400, body: { code: "ProcessParameterInvalid", message } },
  });
  const panel = await mount(stub);
  authorNdvi(panel);
  // Force the semantically wrong output range through the advanced fold.
  openAdvanced(panel, "s3");
  fill(panel, "s3-outputMax", "1");
  submitButton(panel).click();
  await expect
    .poll(() => panel.querySelector("#swath-authoring-s3-outputMin-note")?.textContent)
    .toContain("the output range must be exactly 0..255");
  // Mapped errors do not double up as the general inline error.
  expect(panel.querySelector(".swath-authoring-error")).toBeNull();
  // Editing the field clears the stale server note.
  fill(panel, "s3-outputMin", "0");
  expect(panel.querySelector("#swath-authoring-s3-outputMin-note")?.textContent).toBe("");
});

test("a rejected graph the panel cannot locate renders the general error inline", async () => {
  const stub = fetchStub({
    post: {
      status: 400,
      body: { code: "ServiceUnsupported", message: "Service type 'xyz' is not supported." },
    },
  });
  const panel = await mount(stub);
  choose(panel, "s1-id", "hls-s30");
  tickBand(panel, "b02");
  tickBand(panel, "b03");
  tickBand(panel, "b04");
  submitButton(panel).click();
  await expect
    .poll(() => panel.querySelector(".swath-authoring-error")?.textContent)
    .toBe("ServiceUnsupported: Service type 'xyz' is not supported.");
});

test("published services list with a delete control that announces deletion", async () => {
  const stub = fetchStub({
    services: [{ id: "xyz-abc123def456", title: "NDVI (authored)" }],
  });
  const panel = await mount(stub);
  const remove = panel.querySelector<HTMLButtonElement>(
    '.swath-authoring-services button[data-service="xyz-abc123def456"]',
  );
  expect(remove).not.toBeNull();

  const deleted = new Promise<string>((resolve) => {
    document.body.addEventListener(
      "swath-service-deleted",
      (event) => resolve((event as CustomEvent<{ id: string }>).detail.id),
      { once: true },
    );
  });
  remove?.click();
  expect(await deleted).toBe("xyz-abc123def456");
  const request = stub.requests.find((entry) => entry.method === "DELETE");
  expect(request?.url).toBe("/services/xyz-abc123def456");
});

// --- Preview before publish (issue #169, ADR 0014 — B11) -----------------

/** The `POST /result` preview requests the stub saw. */
function previewPosts(stub: { requests: Recorded[] }): Recorded[] {
  return stub.requests.filter((request) => request.method === "POST" && request.url === "/result");
}

function previewImage(panel: SwathAuthoringPanel): HTMLImageElement | null {
  return panel.querySelector<HTMLImageElement>("#swath-authoring-preview-image");
}

function previewNote(panel: SwathAuthoringPanel): string {
  return panel.querySelector("#swath-authoring-preview-note")?.textContent ?? "";
}

test("B11 preview: a complete draft renders the POST /result image inline, debounced and keyed on the graph", async () => {
  const stub = fetchStub({});
  const panel = await mount(stub);
  panel.querySelector<HTMLButtonElement>(".swath-authoring-template")?.click();
  // The debounced preview lands: the exact composed graph, spec-shaped.
  await expect.poll(() => previewImage(panel)?.getAttribute("src") ?? "").toMatch(/^blob:/);
  expect(previewImage(panel)?.hidden).toBe(false);
  expect(previewNote(panel)).toContain("Preview");
  expect(previewPosts(stub)).toHaveLength(1);
  expect(previewPosts(stub)[0]?.body).toEqual({ process: { process_graph: NDVI_GRAPH } });
  // An unchanged draft never refetches (keystroke churn re-runs
  // validation constantly; the preview is keyed on the composed graph)…
  fill(panel, "title", "only metadata"); // the title is not part of the graph
  await new Promise((resolve) => setTimeout(resolve, 450));
  expect(previewPosts(stub)).toHaveLength(1);
  // …while a real edit re-previews: the swapped-band draft (B11's
  // canonical valid-and-wrong graph) gets its own ground-truth image.
  choose(panel, "s2-nir", "b04");
  choose(panel, "s2-red", "b8a");
  await expect.poll(() => previewPosts(stub).length).toBe(2);
  const swapped = previewPosts(stub)[1]?.body as {
    process: { process_graph: Record<string, { arguments: Record<string, unknown> }> };
  };
  expect(swapped.process.process_graph["s2"]?.arguments["nir"]).toBe("b04");
});

test("B11 preview: the budget refusal explains itself in plain words and never gates publish", async () => {
  const stub = fetchStub({
    result: {
      status: 400,
      body: {
        code: "ProcessGraphComplexity",
        message: "The process is too complex for synchronous processing.",
      },
    },
  });
  const panel = await mount(stub);
  panel.querySelector<HTMLButtonElement>(".swath-authoring-template")?.click();
  await expect.poll(() => previewNote(panel)).toContain("too much data to preview");
  expect(previewNote(panel)).toContain("narrow the area");
  expect(previewImage(panel)?.hidden).toBe(true);
  // The budget bounds the preview, not the layer: publish stays enabled
  // and no general inline error appears.
  expect(submitButton(panel).disabled).toBe(false);
  expect(panel.querySelector(".swath-authoring-error")).toBeNull();
});

test("B11 preview: incomplete drafts show no preview and make no request", async () => {
  const stub = fetchStub({});
  const panel = await mount(stub);
  choose(panel, "s1-id", "hls-s30");
  tickBand(panel, "b8a"); // one channel, submit gated — nothing to show
  await new Promise((resolve) => setTimeout(resolve, 450));
  expect(previewPosts(stub)).toHaveLength(0);
  expect(panel.querySelector<HTMLElement>("#swath-authoring-preview")?.hidden).toBe(true);
  // Completing the pipeline flips the preview on; breaking it again
  // clears the image rather than showing a stale draft.
  panel.querySelector<HTMLButtonElement>(".swath-authoring-template")?.click();
  await expect.poll(() => previewImage(panel)?.getAttribute("src") ?? "").toMatch(/^blob:/);
  tickBand(panel, "b04"); // untick a band NDVI still uses: draft breaks
  expect(panel.querySelector<HTMLElement>("#swath-authoring-preview")?.hidden).toBe(true);
  expect(previewImage(panel)?.getAttribute("src")).toBeNull();
});
