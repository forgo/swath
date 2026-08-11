// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The authoring panel's contract (issue #109): schema-driven throughout.
// The palette and every form field come from the mocked `GET /processes`
// response — REMOVE a definition from the mock and its palette entry and
// form are gone (the no-hand-maintained-forms proof). Composition,
// publish (success + inline openEO error), and delete are exercised
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
      { name: "nir", optional: true, default: "nir", schema: { type: "string" } },
      { name: "red", optional: true, default: "red", schema: { type: "string" } },
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

/** One recorded request the fetch stub saw. */
interface Recorded {
  method: string;
  url: string;
  body: unknown;
}

/** A scripted same-shape fetch: GET /processes and GET /services answer
 * the given documents; POST /services and DELETE answer the scripted
 * response; everything is recorded for assertions. */
function fetchStub(options: {
  processes?: ProcessDefinition[];
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

  // load_collection: every schema parameter became a field.
  for (const name of ["id", "spatial_extent", "temporal_extent", "bands"]) {
    expect(panel.querySelector(`#swath-authoring-s1-${name}`)).not.toBeNull();
  }
  // The first step has nothing to wire from: no source selects.
  expect(panel.querySelector("#swath-authoring-s1-id-source")).toBeNull();

  // ndvi.data is a raster-cube: it defaults to the previous step's
  // output, and its literal input is disabled accordingly.
  const dataSource = field<HTMLSelectElement>(panel, "s2-data-source");
  expect(dataSource.value).toBe("s1");
  expect(field<HTMLInputElement>(panel, "s2-data").disabled).toBe(true);
  // Plain string parameters stay literal inputs, prefilled from defaults.
  expect(field<HTMLInputElement>(panel, "s2-nir").disabled).toBe(false);

  // Numbers render as number inputs (schema type, not a per-process map).
  addStep(panel, "linear_scale_range");
  expect(field<HTMLInputElement>(panel, "s3-inputMin").type).toBe("number");
  // x is typed number, but as the first required parameter of a later
  // step it defaults to wiring from the previous step.
  expect(field<HTMLSelectElement>(panel, "s3-x-source").value).toBe("s2");
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

/** Drives the full NDVI authoring flow through the generated forms. */
function authorNdvi(panel: SwathAuthoringPanel): void {
  addStep(panel, "load_collection");
  addStep(panel, "ndvi");
  addStep(panel, "linear_scale_range");
  addStep(panel, "save_result");
  fill(panel, "s1-id", "hls-s30");
  fill(panel, "s1-bands", "b8a,b04");
  fill(panel, "s2-nir", "b8a");
  fill(panel, "s2-red", "b04");
  fill(panel, "s3-inputMin", "-1");
  fill(panel, "s3-inputMax", "1");
  fill(panel, "s3-outputMin", "0");
  fill(panel, "s3-outputMax", "255");
  fill(panel, "s4-format", "png");
  const colormap = field<HTMLSelectElement>(panel, "s4-options");
  colormap.value = "rdylgn";
  colormap.dispatchEvent(new Event("change", { bubbles: true }));
}

test("publishing posts the composed graph and announces the created service", async () => {
  const stub = fetchStub({
    post: { status: 201, headers: { "openeo-identifier": "xyz-abc123def456" } },
  });
  const panel = await mount(stub);
  authorNdvi(panel);
  fill(panel, "title", "NDVI (authored)");

  const created = new Promise<string>((resolve) => {
    document.body.addEventListener(
      "swath-service-created",
      (event) => resolve((event as CustomEvent<{ id: string }>).detail.id),
      { once: true },
    );
  });
  panel.querySelector<HTMLButtonElement>(".swath-authoring-submit")?.click();
  expect(await created).toBe("xyz-abc123def456");

  const post = stub.requests.find((request) => request.method === "POST");
  expect(post?.url).toBe("/services");
  expect(post?.body).toEqual({
    type: "xyz",
    title: "NDVI (authored)",
    process: {
      process_graph: {
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
      },
    },
  });
});

test("a rejected graph renders the server's openEO error inline", async () => {
  const stub = fetchStub({
    post: {
      status: 400,
      body: {
        code: "ProcessParameterInvalid",
        message: "The value passed for parameter 'outputMax' is invalid.",
      },
    },
  });
  const panel = await mount(stub);
  addStep(panel, "load_collection");
  panel.querySelector<HTMLButtonElement>(".swath-authoring-submit")?.click();
  await expect
    .poll(() => panel.querySelector(".swath-authoring-error")?.textContent)
    .toBe("ProcessParameterInvalid: The value passed for parameter 'outputMax' is invalid.");
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
