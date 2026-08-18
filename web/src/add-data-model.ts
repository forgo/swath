// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * Semantics of the add-data panel (issue #197), DOM-free in the
 * authoring-model.ts spirit: capability detection from the server's
 * landing/capabilities document, link classification, draft derivation
 * (from a fetched STAC Item or a pasted COG link), plain-words inline
 * validation, the registration request bodies, the quick-look service
 * graph, and the mapping of server RFC 7807 problems back onto fields.
 *
 * Everything registered here renders through the engine — the panel never
 * decodes a pixel (ADR 0019 records client-side COG rendering as the
 * rejected alternative).
 */

/** What the server's capabilities document says this panel may do. */
export interface AddDataCapabilities {
  /** `POST /datasets` is mounted — the panel may exist at all. */
  register: boolean;
  /** `PUT /uploads/{filename}` is mounted — local-mode file drop. */
  upload: boolean;
}

/** Reads the `endpoints` array of `GET /` (the openEO-style capabilities
 * document, which read-only serving filters, #198): what is not advertised
 * is not mounted. A landing page without `endpoints` (static fixtures
 * mode) allows nothing. */
export function parseCapabilities(doc: unknown): AddDataCapabilities {
  const none = { register: false, upload: false };
  if (typeof doc !== "object" || doc === null) {
    return none;
  }
  const endpoints = (doc as { endpoints?: unknown }).endpoints;
  if (!Array.isArray(endpoints)) {
    return none;
  }
  const allows = (path: string, method: string): boolean =>
    endpoints.some((entry) => {
      if (typeof entry !== "object" || entry === null) {
        return false;
      }
      const { path: p, methods } = entry as { path?: unknown; methods?: unknown };
      return p === path && Array.isArray(methods) && methods.includes(method);
    });
  return {
    register: allows("/datasets", "POST"),
    upload: allows("/uploads/{filename}", "PUT"),
  };
}

/** What a pasted link points at: a STAC Item document, or a raster the
 * server reads directly. */
export type LinkKind = "stac" | "cog";

/** `.json` (before any query/hash) is a STAC Item to fetch in-browser;
 * everything else is handed to the server as a raster reference. */
export function classifyLink(link: string): LinkKind {
  const path = link.split(/[?#]/, 1)[0] ?? "";
  return path.toLowerCase().endsWith(".json") ? "stac" : "cog";
}

/** The pre-filled registration form: everything the flow needs, editable
 * before anything is sent. */
export interface AddDataDraft {
  kind: LinkKind;
  /** The raster reference registered as the asset (`cog` kind). */
  href: string;
  /** The fetched STAC Item document, registered inline (`stac` kind). */
  stacItem?: Record<string, unknown>;
  datasetId: string;
  title: string;
  /** Declared dataset bands; for `cog` also the single asset's band. */
  bands: string[];
  granuleId: string;
  /** Acquisition instant, RFC 3339 UTC (`Z`). */
  datetime: string;
}

/** RFC 3339 UTC instant — the same alphabet the server's `Datetime` and
 * the view-state `t` param accept. */
const DATETIME_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/;

/** The last path segment of a link, extension dropped — the seed for ids
 * derived from a pasted file name. */
function stem(link: string): string {
  const path = link.split(/[?#]/, 1)[0] ?? "";
  const name = path.split("/").filter(Boolean).at(-1) ?? "";
  return name.replace(/\.[A-Za-z0-9]+$/, "");
}

/** A URL-safe id from arbitrary text: the server's alphabet (ascii
 * alphanumerics, `-`, `_`), everything else collapsed to `-`. */
export function slug(text: string): string {
  return text
    .replace(/[^A-Za-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .toLowerCase();
}

/** A draft from a pasted raster link: ids seeded from the file name, the
 * band left for the user to confirm. */
export function cogDraft(link: string): AddDataDraft {
  const seed = slug(stem(link));
  return {
    kind: "cog",
    href: link,
    datasetId: seed,
    title: stem(link) === "" ? link : stem(link),
    bands: ["data"],
    granuleId: seed === "" ? "granule-1" : seed,
    datetime: "",
  };
}

/** A draft from a fetched STAC Item, or a plain-words reason the document
 * is not one the server accepts (the panel shows it under the link
 * field). The server re-validates the inline item; this mirrors its
 * required fields so problems surface before anything is sent. */
export function stacDraft(doc: unknown): AddDataDraft | string {
  if (typeof doc !== "object" || doc === null || Array.isArray(doc)) {
    return "that link is JSON, but not a STAC Item document";
  }
  const item = doc as Record<string, unknown>;
  if (item["type"] !== "Feature") {
    return 'a STAC Item has "type": "Feature" — this document does not';
  }
  const id = item["id"];
  if (typeof id !== "string" || id === "") {
    return "the item has no id";
  }
  const collection = item["collection"];
  if (typeof collection !== "string" || collection === "") {
    return "the item names no collection — Swath needs one as the dataset id";
  }
  const properties = item["properties"];
  const datetime =
    typeof properties === "object" && properties !== null
      ? (properties as Record<string, unknown>)["datetime"]
      : undefined;
  if (typeof datetime !== "string" || !DATETIME_PATTERN.test(datetime)) {
    return "the item has no properties.datetime (RFC 3339 UTC, ending in Z)";
  }
  const assets = item["assets"];
  const bands: string[] = [];
  if (typeof assets === "object" && assets !== null && !Array.isArray(assets)) {
    for (const [band, asset] of Object.entries(assets)) {
      if (
        typeof asset === "object" &&
        asset !== null &&
        typeof (asset as Record<string, unknown>)["href"] === "string"
      ) {
        bands.push(band);
      }
    }
  }
  if (bands.length === 0) {
    return "the item has no assets with an href — nothing to serve";
  }
  return {
    kind: "stac",
    href: "",
    stacItem: item,
    datasetId: collection,
    title: typeof item["title"] === "string" ? (item["title"] as string) : collection,
    bands,
    granuleId: id,
    datetime,
  };
}

// --- Inline validation: lowercase plain-words phrases, "" = fine ---

/** Dataset id issue, mirroring the server's URL-safety rule. */
export function datasetIdIssue(id: string): string {
  if (id.trim() === "") {
    return "required";
  }
  return /^[A-Za-z0-9_-]+$/.test(id) ? "" : "use only letters, digits, - and _";
}

/** Granule id issue. */
export function granuleIdIssue(id: string): string {
  return id.trim() === "" ? "required" : "";
}

/** Band name issue (the direct form's one asset band). */
export function bandIssue(band: string): string {
  return band.trim() === "" ? "name the band this file carries" : "";
}

/** Acquisition datetime issue. */
export function datetimeIssue(datetime: string): string {
  if (datetime.trim() === "") {
    return "required";
  }
  return DATETIME_PATTERN.test(datetime)
    ? ""
    : "an RFC 3339 UTC instant, like 2024-06-06T17:54:00Z";
}

/** Brightest-value issue (the quick look's display range). */
export function brightestIssue(text: string): string {
  const value = Number(text.trim());
  return Number.isFinite(value) && value > 0 ? "" : "a positive number";
}

// --- Request bodies (the #248 contracts) ---

/** The `POST /datasets` body. */
export function datasetBody(draft: AddDataDraft): Record<string, unknown> {
  return {
    id: draft.datasetId,
    title: draft.title === "" ? draft.datasetId : draft.title,
    description: "Registered from the add-data panel.",
    bands: draft.bands,
  };
}

/** The `POST /datasets/{id}/granules` body: the inline STAC Item when the
 * link was one (the server never fetches URLs — this panel does), else
 * the direct form. */
export function granuleBody(draft: AddDataDraft): Record<string, unknown> {
  if (draft.kind === "stac" && draft.stacItem !== undefined) {
    return { stac_item: draft.stacItem };
  }
  const band = draft.bands[0] ?? "data";
  return {
    id: draft.granuleId,
    datetime: draft.datetime,
    assets: { [band]: draft.href },
  };
}

/** The bands a quick look composites: red/green/blue by name when the
 * vocabulary has them (HLS spells them b04/b03/b02), else the first
 * three; fewer than three bands render gray from the first. */
export function quicklookBands(bands: readonly string[]): string[] {
  if (bands.length < 3) {
    return bands.length === 0 ? [] : [bands[0] as string];
  }
  const find = (names: string[]): string | undefined =>
    bands.find((band) => names.includes(band.toLowerCase()));
  const r = find(["red", "b04"]);
  const g = find(["green", "b03"]);
  const b = find(["blue", "b02"]);
  if (r !== undefined && g !== undefined && b !== undefined) {
    return [r, g, b];
  }
  return bands.slice(0, 3);
}

/** The `POST /services` body of the dataset's quick look: through the
 * engine like every layer — load, scale to the display range, save. Three
 * or more bands compose RGB; fewer reduce the first band to gray. */
export function quicklookService(
  datasetId: string,
  title: string,
  bands: readonly string[],
  brightest: number,
): Record<string, unknown> {
  const picked = quicklookBands(bands);
  const scale = (from: string): Record<string, unknown> => ({
    process_id: "linear_scale_range",
    arguments: {
      x: { from_node: from },
      inputMin: 0,
      inputMax: brightest,
      outputMin: 0,
      outputMax: 255,
    },
  });
  const save = {
    process_id: "save_result",
    arguments: { data: { from_node: "scale" }, format: "png" },
    result: true,
  };
  const load = {
    process_id: "load_collection",
    arguments: {
      id: datasetId,
      spatial_extent: null,
      temporal_extent: null,
      bands: picked,
    },
  };
  const graph: Record<string, unknown> =
    picked.length === 3
      ? { load, scale: scale("load"), save }
      : {
          load,
          gray: {
            process_id: "reduce_dimension",
            arguments: {
              data: { from_node: "load" },
              dimension: "bands",
              reducer: {
                process_graph: {
                  pick: {
                    process_id: "array_element",
                    arguments: { data: { from_parameter: "data" }, label: picked[0] },
                    result: true,
                  },
                },
              },
            },
          },
          scale: scale("gray"),
          save,
        };
  return {
    type: "xyz",
    title: `${title} (quick look)`,
    description: `Quick look of ${datasetId}, added through the add-data panel.`,
    process: { process_graph: graph },
  };
}

// --- Server RFC 7807 problems, mapped back onto fields ---

/** Which form field a server refusal belongs under; "" renders as the
 * flow-level error line. */
export type ProblemField = "link" | "dataset" | "band" | "granule" | "datetime" | "";

/** A server problem translated for the form. */
export interface ProblemNote {
  field: ProblemField;
  note: string;
}

/** Reads an RFC 7807 body (`{type,title,status,detail}` — every non-2xx
 * of the dataset surface) into a field-routed plain-words note. Unknown
 * shapes fall back to the HTTP status. */
export function mapProblem(status: number, body: unknown): ProblemNote {
  const detail =
    typeof body === "object" && body !== null
      ? (body as Record<string, unknown>)["detail"]
      : undefined;
  if (typeof detail !== "string" || detail === "") {
    return { field: "", note: `the server refused with HTTP ${status}` };
  }
  if (detail.includes("failed header validation") || detail.includes("bbox derivation")) {
    return {
      field: "link",
      note: `the server could not read that file as a cloud-optimized GeoTIFF — ${detail}`,
    };
  }
  if (detail.includes("stac_item")) {
    return { field: "link", note: detail };
  }
  if (detail.includes("is not URL-safe")) {
    return { field: "dataset", note: "use only letters, digits, - and _" };
  }
  if (detail.includes("declared bands")) {
    return { field: "band", note: detail };
  }
  if (detail.includes("`datetime`")) {
    return { field: "datetime", note: detail };
  }
  return { field: "", note: detail };
}
