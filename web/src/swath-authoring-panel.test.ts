// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The authoring panel's contract (issue #109): schema-driven throughout.
// The palette and every form field come from the mocked `GET /processes`
// response — REMOVE a definition from the mock and its palette entry and
// form are gone (the no-hand-maintained-forms proof). The collection
// picker is fed by `GET /collections`; validation blocks submit (with
// reasons) until the graph is structurally valid; required fields flag
// inline as the user types; server diagnostics map onto the offending
// field. Composition, publish (success + inline openEO error), the NDVI
// template, and delete are exercised against a scripted fetch; no
// server involved. Real Custom Elements in a real browser, like the
// rest of the suite.
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

/** A scripted same-shape fetch: the GET surfaces answer the given
 * documents; POST /services and DELETE answer the scripted response;
 * everything is recorded for assertions. */
function fetchStub(options: {
  processes?: ProcessDefinition[];
  collections?: unknown[];
  services?: { id: string; title?: string }[];
  post?: { status: number; body?: unknown; headers?: Record<string, string> };
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

function paletteIds(panel: SwathAuthoringPanel): string[] {
  return [...panel.querySelectorAll<HTMLButtonElement>(".swath-authoring-palette button")].map(
    (button) => button.dataset["process"] ?? "",
  );
}

function addStep(panel: SwathAuthoringPanel, processId: string): void {
  const button = panel.querySelector<HTMLButtonElement>(
    `.swath-authoring-palette button[data-process="${processId}"]`,
  );
  if (!button) {
    throw new Error(`no palette entry for ${processId}`);
  }
  button.click();
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
  // The first open loads the definitions (and renders the palette).
  toggle?.click();
  await expect.poll(() => paletteIds(panel).length).toBeGreaterThan(0);
  expect(stub.requests.map((request) => request.url).sort()).toEqual([
    "/collections",
    "/processes",
    "/services",
  ]);
});

test("the palette lists exactly the served process definitions", async () => {
  const panel = await mount(fetchStub({}));
  expect(paletteIds(panel)).toEqual([
    "load_collection",
    "ndvi",
    "linear_scale_range",
    "save_result",
  ]);
});

test("deleting a definition from /processes removes its palette entry and form", async () => {
  const withNdvi = await mount(fetchStub({}));
  addStep(withNdvi, "ndvi");
  // The form exists, generated from the schema: nir/red prefilled with
  // the definition's defaults.
  expect(field<HTMLInputElement>(withNdvi, "s1-nir").value).toBe("nir");
  expect(field<HTMLInputElement>(withNdvi, "s1-red").value).toBe("red");

  document.body.replaceChildren();
  const without = await mount(
    fetchStub({ processes: DEFINITIONS.filter((process) => process.id !== "ndvi") }),
  );
  expect(paletteIds(without)).toEqual(["load_collection", "linear_scale_range", "save_result"]);
  expect(without.querySelector('.swath-authoring-palette button[data-process="ndvi"]')).toBeNull();
  expect(() => addStep(without, "ndvi")).toThrow();
  expect(without.querySelector("#swath-authoring-s1-nir")).toBeNull();
});

test("fields are generated from the parameter schemas, data flow included", async () => {
  const panel = await mount(fetchStub({}));
  addStep(panel, "load_collection");
  addStep(panel, "ndvi");

  // load_collection: every schema parameter became a field (the
  // defaulted/nullable ones under the advanced toggle).
  openAdvanced(panel, "s1");
  for (const name of ["id", "spatial_extent", "temporal_extent", "bands"]) {
    expect(panel.querySelector(`#swath-authoring-s1-${name}`)).not.toBeNull();
  }
  // The first step has nothing to wire from: no source selects.
  expect(panel.querySelector("#swath-authoring-s1-id-source")).toBeNull();

  // ndvi.data is a raster-cube: it defaults to the previous step's
  // output, and its literal input is disabled accordingly.
  openAdvanced(panel, "s2");
  const dataSource = field<HTMLSelectElement>(panel, "s2-data-source");
  expect(dataSource.value).toBe("s1");
  expect(field<HTMLInputElement>(panel, "s2-data").disabled).toBe(true);
  // Band-name parameters stay literal inputs, prefilled from defaults.
  expect(field<HTMLInputElement>(panel, "s2-nir").disabled).toBe(false);

  // Numbers render as number inputs (schema type, not a per-process map).
  addStep(panel, "linear_scale_range");
  expect(field<HTMLInputElement>(panel, "s3-inputMin").type).toBe("number");
  // x is typed number, but as the first required parameter of a later
  // step it defaults to wiring from the previous step.
  openAdvanced(panel, "s3");
  expect(field<HTMLSelectElement>(panel, "s3-x-source").value).toBe("s2");
});

test("every field carries a visible plain-language explainer", async () => {
  const panel = await mount(fetchStub({}));
  addStep(panel, "load_collection");
  expect(helpText(panel, "s1-id")).toBe("Which dataset to compute from.");
  openAdvanced(panel, "s1");
  expect(helpText(panel, "s1-spatial_extent")).toBe(
    "The map area to compute over — leave as is to use the whole collection.",
  );
  expect(helpText(panel, "s1-temporal_extent")).toBe(
    "The time range to include — leave as is to use everything available.",
  );
});

test("defaulted fields collapse under advanced and still publish correctly", async () => {
  const panel = await mount(fetchStub({}));
  addStep(panel, "load_collection");
  addStep(panel, "linear_scale_range");
  addStep(panel, "save_result");

  // Collapsed by default: the nullable extents and the profile-pinned
  // outputs/format are not in the default view...
  for (const id of ["s1-spatial_extent", "s2-outputMax", "s3-format"]) {
    expect(panel.querySelector(`#swath-authoring-${id}`)).toBeNull();
  }
  const toggle = panel.querySelector('[data-step="s1"] .swath-authoring-advanced-toggle');
  expect(toggle?.getAttribute("aria-expanded")).toBe("false");
  // ...while the newcomer's choices stay visible.
  for (const id of ["s1-id", "s1-bands", "s2-inputMin", "s3-options"]) {
    expect(panel.querySelector(`#swath-authoring-${id}`)).not.toBeNull();
  }

  // Smart defaults make the hidden fields publishable with zero expert
  // decisions: null extents, 0..255 output, png.
  choose(panel, "s1-id", "hls-s30");
  fill(panel, "s2-inputMin", "-1");
  fill(panel, "s2-inputMax", "1");
  expect(submitButton(panel).disabled).toBe(false);
  const graph = panel.buildGraph() as Record<string, { arguments: Record<string, unknown> }>;
  expect(graph["s1"]?.arguments["spatial_extent"]).toBeNull();
  expect(graph["s2"]?.arguments["outputMin"]).toBe(0);
  expect(graph["s2"]?.arguments["outputMax"]).toBe(255);
  expect(graph["s3"]?.arguments["format"]).toBe("png");
});

test("the collection is a dropdown fed by GET /collections", async () => {
  const panel = await mount(fetchStub({}));
  addStep(panel, "load_collection");
  const picker = field<HTMLSelectElement>(panel, "s1-id");
  expect(picker.tagName).toBe("SELECT");
  expect([...picker.options].map((option) => option.value)).toEqual(["", "hls-s30"]);
  // Choosing the collection surfaces its band vocabulary as a hint on
  // band-name fields (context from /collections, not hand-maintained).
  choose(panel, "s1-id", "hls-s30");
  expect(panel.querySelector('[data-step="s1"] .swath-authoring-band-hint')?.textContent).toContain(
    "b02, b03, b04, b8a",
  );
});

test("the colormap is selectable on save_result's options", async () => {
  const panel = await mount(fetchStub({}));
  addStep(panel, "save_result");
  const options = field<HTMLSelectElement>(panel, "s1-options");
  expect(options.tagName).toBe("SELECT");
  expect([...options.options].map((option) => option.value)).toEqual([
    "",
    "grayscale",
    "viridis",
    "magma",
    "rdylgn",
  ]);
});

test("submit stays disabled with spelled-out reasons until the graph is valid", async () => {
  const panel = await mount(fetchStub({}));
  addStep(panel, "load_collection");
  // Required collection missing: blocked, and the reasons say so.
  expect(submitButton(panel).disabled).toBe(true);
  expect(submitReason(panel)).toContain("1 field needs a value");
  expect(submitReason(panel)).toContain("no step loads a collection");
  // Choosing the collection clears both reasons (spatial/temporal are
  // nullable, bands optional): the graph is structurally valid.
  choose(panel, "s1-id", "hls-s30");
  expect(submitButton(panel).disabled).toBe(false);
  expect(submitReason(panel)).toBe("");
  // A disabled submit never POSTs — clicking earlier sent nothing.
  expect(panel.buildGraph()["s1"]).toMatchObject({ process_id: "load_collection" });
});

test("required fields flag inline as the user types", async () => {
  const panel = await mount(fetchStub({}));
  addStep(panel, "load_collection");
  addStep(panel, "save_result");
  // format arrives with the profile default (png) under advanced —
  // no issue, and only load_collection's id blocks submit.
  expect(submitReason(panel)).toContain("1 field needs a value");
  openAdvanced(panel, "s2");
  const note = () => panel.querySelector("#swath-authoring-s2-format-note")?.textContent;
  expect(field<HTMLInputElement>(panel, "s2-format").value).toBe("png");
  expect(note()).toBe("");
  // Clearing it flags the field the moment it empties.
  fill(panel, "s2-format", "");
  expect(note()).toBe("required");
  expect(submitReason(panel)).toContain("2 fields need values");
  fill(panel, "s2-format", "png");
  expect(note()).toBe("");
});

/** Drives the full NDVI authoring flow through the generated forms —
 * only the newcomer-visible fields: outputs, extents, and format ride
 * their smart defaults (null/0..255/png) untouched. */
function authorNdvi(panel: SwathAuthoringPanel): void {
  addStep(panel, "load_collection");
  addStep(panel, "ndvi");
  addStep(panel, "linear_scale_range");
  addStep(panel, "save_result");
  choose(panel, "s1-id", "hls-s30");
  fill(panel, "s1-bands", "b8a,b04");
  fill(panel, "s2-nir", "b8a");
  fill(panel, "s2-red", "b04");
  fill(panel, "s3-inputMin", "-1");
  fill(panel, "s3-inputMax", "1");
  choose(panel, "s4-options", "rdylgn");
}

/** The graph [`authorNdvi`] composes — also what the NDVI template must
 * produce over the mocked collection (b8a/b04 via the band heuristics). */
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

  const post = stub.requests.find((request) => request.method === "POST");
  expect(post?.url).toBe("/services");
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
  // The template fills a graph that renders: the first collection,
  // nir/red picked from its band vocabulary, the built-in NDVI scale
  // and colormap — identical to the hand-authored pipeline.
  expect(field<HTMLSelectElement>(panel, "s1-id").value).toBe("hls-s30");
  expect(field<HTMLSelectElement>(panel, "s4-options").value).toBe("rdylgn");
  expect(submitButton(panel).disabled).toBe(false);
  expect(submitReason(panel)).toBe("");
  expect(panel.buildGraph()).toEqual(NDVI_GRAPH);
});

test("the narrative retells the pipeline in plain words, live", async () => {
  const panel = await mount(fetchStub({}));
  panel.querySelector<HTMLButtonElement>(".swath-authoring-template")?.click();
  const narrative = () => panel.querySelector("#swath-authoring-narrative")?.textContent;
  expect(narrative()).toBe(
    "Load hls-s30 (bands b8a,b04) → compute NDVI ((b8a − b04) / (b8a + b04)) → " +
      "rescale -1..1 to 0..255 → save as png, colored with rdylgn.",
  );
  // Live: changing the colormap re-narrates without a re-render cycle.
  choose(panel, "s4-options", "viridis");
  expect(narrative()).toContain("colored with viridis");
});

test("the template is not offered when its processes are not all served", async () => {
  const panel = await mount(
    fetchStub({ processes: DEFINITIONS.filter((process) => process.id !== "ndvi") }),
  );
  expect(panel.querySelector(".swath-authoring-template")).toBeNull();
});

test("a server error naming a node and argument lands on that field", async () => {
  const message =
    "node `s3` (linear_scale_range): invalid argument `outputMin`: the Render IR quantizes " +
    "to 8-bit; the output range must be exactly 0..255, got 0..1";
  const stub = fetchStub({
    post: { status: 400, body: { code: "ProcessParameterInvalid", message } },
  });
  const panel = await mount(stub);
  authorNdvi(panel);
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
  addStep(panel, "load_collection");
  choose(panel, "s1-id", "hls-s30");
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
