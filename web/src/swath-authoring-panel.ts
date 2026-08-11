// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-authoring-panel>` — the openEO authoring panel (issue #109,
 * ADR 0010): compose a process graph as a linear pipeline of steps,
 * publish it as an XYZ secondary service, delete published services.
 *
 * Plain Custom Element, light DOM, no framework (ADR 0005). Collapsed
 * by default and lazy like the dataset browser beside it (issue #110):
 * the closed panel fetches nothing, so the entry page's request budget
 * is untouched until a user actually authors. The panel is a pure
 * client of the openEO surface — it invents nothing:
 *
 * - The **palette** (which processes can be added) and every **form
 *   field** are generated from the parameter schemas `GET /processes`
 *   serves. There are no hand-maintained per-process form definitions:
 *   delete a process from the served definitions and its palette entry
 *   and form disappear (a vitest pins exactly that).
 * - Field widgets are chosen by schema shape alone: number → number
 *   input, array-of-strings → comma-separated text, `enum` → select,
 *   everything else a text input parsed as JSON when it looks like
 *   JSON. Two subtypes get dedicated widgets: `collection-id` renders
 *   as a select fed by `GET /collections` (an unknown collection is
 *   unreachable from the UI), and `output-format-options` as the Swath
 *   colormap select (grayscale/viridis/magma/rdylgn — the render
 *   profile's vocabulary, keyed on the schema subtype, not on any
 *   process id).
 * - Data flow is a per-parameter **source** select: a parameter either
 *   holds a literal value or references an earlier step's output
 *   (`from_node`). Defaults are derived from the schemas too: raster-cube
 *   parameters — and the first required parameter of any step after the
 *   first — default to the previous step, so a straight pipeline needs
 *   no source fiddling.
 * - **Validation before submit**, from the same schemas: required
 *   parameters and numeric fields flag inline as the user types, and
 *   the publish button stays disabled — with the blocking reasons
 *   spelled out — until every field passes and some step names a
 *   collection (derived from the `collection-id` subtype, so the
 *   server's "no load_collection node names a collection" rejection is
 *   unreachable from the UI). The server stays the authority: whatever
 *   it still rejects renders inline — mapped onto the offending field
 *   when its message names a node/argument, as a general error
 *   otherwise. (The openEO `POST /validation` endpoint is not part of
 *   Swath's bounded profile; live server-side validation is a possible
 *   follow-up.)
 * - **Context** comes from the definitions themselves: process
 *   summaries under each step header and on the palette buttons,
 *   parameter descriptions as tooltips, and the chosen collection's
 *   band vocabulary (from `GET /collections`) hinted under band-name
 *   fields.
 * - **Written for non-experts** (maintainer UX round 2): every field
 *   carries a visible plain-language one-liner ([`FIELD_HELP`] — a
 *   curated glossary over the served vocabulary, not a form
 *   definition); fields that work when left alone (nullable extents,
 *   profile-pinned values like PNG/0..255) get smart defaults and
 *   collapse under a per-step "advanced" toggle; and an always-visible
 *   narrative sentence retells the pipeline in plain words, live
 *   ("Load hls-s30 (bands …) → compute NDVI (…) → … colored with
 *   rdylgn").
 * - The empty state explains the pipeline model and offers a
 *   **start-from-working-graph** template: the NDVI pipeline over the
 *   first collection, nir/red picked from its band vocabulary by
 *   common-name heuristics — something that renders, ready to modify.
 * - `POST /services` publishes. Success announces a bubbling
 *   `swath-service-created` (detail: `{id}`); the app shell switches
 *   the map, which refreshes the layer list — the authored layer
 *   appears in the browser with no reload.
 * - Published services are listed from `GET /services` with a delete
 *   button each; `DELETE /services/{id}` announces
 *   `swath-service-deleted` the same way.
 */

const STYLE_ELEMENT_ID = "swath-authoring-panel-styles";

/** The render profile's colormap vocabulary, offered for the
 * `output-format-options` schema subtype (`save_result`'s `options`).
 * A widget for a subtype — not a per-process form definition. */
const COLORMAPS = ["grayscale", "viridis", "magma", "rdylgn"] as const;

/** The NDVI template's pipeline, in order. The template is only offered
 * while every one of these is present in the served definitions — a
 * shortcut over the palette, not a parallel form source. */
const NDVI_TEMPLATE = ["load_collection", "ndvi", "linear_scale_range", "save_result"] as const;

/**
 * Plain-language one-liners, visible under every field (not tooltips):
 * what the field is and what happens when it is left alone — written
 * for a non-expert. This is a curated GLOSSARY over the served
 * vocabulary (keyed by process.parameter, `*` matching any process),
 * not a form definition: which fields exist, their widgets, and their
 * validation still come from the schemas alone.
 */
const FIELD_HELP: Record<string, string> = {
  "load_collection.id": "Which dataset to compute from.",
  "load_collection.spatial_extent":
    "The map area to compute over — leave as is to use the whole collection.",
  "load_collection.temporal_extent":
    "The time range to include — leave as is to use everything available.",
  "load_collection.bands":
    "The channels to load, comma-separated — pick from the bands listed below.",
  "load_collection.properties": "Extra metadata filters — safe to leave alone.",
  "ndvi.data": "The imagery from the step it points at.",
  "ndvi.nir": "The band that measures near-infrared light.",
  "ndvi.red": "The band that measures red light.",
  "ndvi.target_band": "Leave as is: the NDVI result replaces the bands.",
  "linear_scale_range.x": "The values to rescale — usually the previous step's result.",
  "linear_scale_range.inputMin": "The smallest value expected in the data (NDVI: -1).",
  "linear_scale_range.inputMax": "The largest value expected in the data (NDVI: 1).",
  "linear_scale_range.outputMin": "Keep at 0 — screen pixels run 0..255.",
  "linear_scale_range.outputMax": "Keep at 255 — screen pixels run 0..255.",
  "save_result.data": "The finished result from the step it points at.",
  "save_result.format": "The image format tiles are served in — png.",
  "save_result.options": "How numbers become colors on the map.",
  "reduce_dimension.data": "The imagery from the step it points at.",
  "reduce_dimension.reducer": "The formula applied across the bands.",
  "reduce_dimension.dimension": "Which dimension to collapse — bands.",
  "reduce_dimension.context": "Extra data for the reducer — safe to leave alone.",
  "array_element.data": "The band list to pick from.",
  "array_element.index": "The band's position, counting from 0.",
  "array_element.label": "The band's name.",
  "array_element.return_nodata": "Return no-data instead of failing for a missing band.",
  "*.x": "A number, or an earlier step's result.",
  "*.y": "A number, or an earlier step's result.",
};

function fieldHelp(processId: string, name: string): string {
  return FIELD_HELP[`${processId}.${name}`] ?? FIELD_HELP[`*.${name}`] ?? "";
}

/**
 * Swath-profile VALUE defaults for fields whose only right answer the
 * server's narrowing notes already pin (the render path serves PNG and
 * quantizes to 0..255) — smart defaults, mirroring the profile, so the
 * non-expert path needs no expert decisions. Values only; the fields
 * themselves still come from the schemas.
 */
const PROFILE_DEFAULTS: Record<string, string> = {
  "linear_scale_range.outputMax": "255",
  "save_result.format": "png",
};

/** Is this field tucked under the step's "advanced" toggle? Everything
 * that works when left alone (optional, nullable, defaulted, or
 * auto-wired) collapses; what stays visible is exactly the newcomer's
 * choices — the collection, bands, the colormap, and required values
 * without a default. Derived from the schemas, like the fields. */
function isAdvancedParam(processId: string, param: ProcessParameter): boolean {
  if (
    hasSubtype(param.schema, "collection-id") ||
    hasSubtype(param.schema, "output-format-options") ||
    isBandName(param.schema)
  ) {
    return false;
  }
  if (isCube(param.schema)) {
    return true; // auto-wired to the previous step
  }
  return (
    param.optional === true ||
    allowsNull(param.schema) ||
    param.default !== undefined ||
    PROFILE_DEFAULTS[`${processId}.${param.name}`] !== undefined
  );
}

/** One step's phrase of the what-is-happening narrative, in plain
 * words, from the current field values (another curated glossary over
 * the served process vocabulary). */
function narrativePhrase(processId: string, value: (name: string) => string): string {
  switch (processId) {
    case "load_collection": {
      const id = value("id");
      const bands = value("bands");
      return `load ${id === "" ? "a collection" : id}${bands === "" ? "" : ` (bands ${bands})`}`;
    }
    case "ndvi": {
      const nir = value("nir") === "" ? "nir" : value("nir");
      const red = value("red") === "" ? "red" : value("red");
      return `compute NDVI ((${nir} − ${red}) / (${nir} + ${red}))`;
    }
    case "linear_scale_range": {
      const bound = (name: string, fallback: string): string =>
        value(name) === "" ? fallback : value(name);
      return `rescale ${bound("inputMin", "?")}..${bound("inputMax", "?")} to ${bound(
        "outputMin",
        "0",
      )}..${bound("outputMax", "1")}`;
    }
    case "save_result": {
      const format = value("format") === "" ? "an image" : value("format");
      const colormap = value("options");
      return `save as ${format}${colormap === "" ? "" : `, colored with ${colormap}`}`;
    }
    case "reduce_dimension":
      return "combine the bands with a formula";
    case "array_element":
      return "pick one band";
    case "add":
    case "subtract":
    case "multiply":
    case "divide":
      return `${processId} values`;
    default:
      return processId.replaceAll("_", " ");
  }
}

/** Panel chrome, matching the layer panel's dark-telemetry look. */
const PANEL_CSS = `
swath-authoring-panel { display: block; }
swath-authoring-panel .swath-authoring-toggle {
  display: block;
  width: 100%;
  margin: 0;
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
swath-authoring-panel .swath-authoring-toggle::before { content: "▸ "; }
swath-authoring-panel .swath-authoring-toggle[aria-expanded="true"]::before { content: "▾ "; }
swath-authoring-panel .swath-authoring-toggle[aria-expanded="true"] { margin-bottom: 8px; }
swath-authoring-panel .swath-authoring-toggle:focus-visible {
  outline: 2px solid #4ade80;
  outline-offset: 1px;
}
swath-authoring-panel .swath-authoring-heading {
  margin: 0 0 8px;
  font: 700 11px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: rgb(148 163 184 / 90%);
}
swath-authoring-panel .swath-authoring-palette {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin: 0 0 10px;
}
swath-authoring-panel .swath-authoring-palette button {
  padding: 3px 7px;
  border: 1px solid rgb(148 163 184 / 35%);
  border-radius: 999px;
  background: none;
  cursor: pointer;
  color: inherit;
  font: 11px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
swath-authoring-panel .swath-authoring-palette button:hover {
  background: rgb(148 163 184 / 12%);
}
swath-authoring-panel .swath-authoring-steps {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
swath-authoring-panel .swath-authoring-step {
  border: 1px solid rgb(148 163 184 / 20%);
  border-radius: 6px;
  padding: 8px;
}
swath-authoring-panel .swath-authoring-step-header {
  display: flex;
  align-items: baseline;
  gap: 6px;
  margin: 0;
  font: 700 11px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
swath-authoring-panel .swath-authoring-step-header .swath-authoring-step-key {
  color: #4ade80;
}
swath-authoring-panel .swath-authoring-step-header button {
  margin-left: auto;
  border: none;
  background: none;
  cursor: pointer;
  color: rgb(148 163 184 / 80%);
  font: 12px/1 system-ui, sans-serif;
}
swath-authoring-panel .swath-authoring-step-header button:hover { color: #f87171; }
swath-authoring-panel .swath-authoring-step-summary {
  display: block;
  margin: 0 0 6px;
  font: italic 11px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 75%);
}
swath-authoring-panel label {
  display: block;
  margin: 0 0 6px;
  font: 11px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  color: rgb(148 163 184 / 90%);
}
swath-authoring-panel input,
swath-authoring-panel select {
  display: block;
  width: 100%;
  box-sizing: border-box;
  margin-top: 1px;
  padding: 3px 6px;
  border: 1px solid rgb(148 163 184 / 30%);
  border-radius: 4px;
  background: rgb(15 23 42 / 60%);
  color: inherit;
  font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
swath-authoring-panel input:focus-visible,
swath-authoring-panel select:focus-visible,
swath-authoring-panel button:focus-visible {
  outline: 2px solid #4ade80;
  outline-offset: 1px;
}
swath-authoring-panel input:disabled { opacity: 0.4; }
swath-authoring-panel .swath-authoring-field-help {
  display: block;
  margin: 0 0 2px;
  font: 11px/1.4 system-ui, sans-serif;
  font-weight: 400;
  letter-spacing: normal;
  text-transform: none;
  color: rgb(148 163 184 / 75%);
}
swath-authoring-panel .swath-authoring-narrative {
  margin: 0 0 10px;
  padding: 6px 8px;
  border-left: 2px solid rgb(74 222 128 / 45%);
  font: italic 12px/1.5 system-ui, sans-serif;
  color: rgb(226 232 240 / 90%);
  overflow-wrap: anywhere;
}
swath-authoring-panel .swath-authoring-narrative:empty { display: none; }
swath-authoring-panel .swath-authoring-advanced-toggle {
  display: block;
  margin: 2px 0 6px;
  padding: 0;
  border: 0;
  background: none;
  cursor: pointer;
  font: 11px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  color: rgb(148 163 184 / 70%);
}
swath-authoring-panel .swath-authoring-advanced-toggle::before { content: "▸ "; }
swath-authoring-panel .swath-authoring-advanced-toggle[aria-expanded="true"]::before {
  content: "▾ ";
}
swath-authoring-panel .swath-authoring-field-note {
  display: block;
  margin: 1px 0 0;
  font: 11px/1.5 system-ui, sans-serif;
  color: #fca5a5;
  overflow-wrap: anywhere;
}
swath-authoring-panel .swath-authoring-field-note:empty { display: none; }
swath-authoring-panel .swath-authoring-band-hint {
  display: block;
  margin: 1px 0 0;
  font: 11px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 70%);
}
swath-authoring-panel .swath-authoring-step-error {
  margin: 0 0 6px;
  font: 11px/1.5 system-ui, sans-serif;
  color: #fca5a5;
  overflow-wrap: anywhere;
}
swath-authoring-panel .swath-authoring-submit {
  margin-top: 10px;
  width: 100%;
  padding: 7px 10px;
  border: 1px solid rgb(74 222 128 / 45%);
  border-radius: 6px;
  background: rgb(74 222 128 / 10%);
  cursor: pointer;
  color: inherit;
  font: 600 12px/1.5 system-ui, sans-serif;
}
swath-authoring-panel .swath-authoring-submit:hover:not(:disabled) {
  background: rgb(74 222 128 / 20%);
}
swath-authoring-panel .swath-authoring-submit:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}
swath-authoring-panel .swath-authoring-submit-reason {
  margin: 4px 0 0;
  font: 11px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 80%);
}
swath-authoring-panel .swath-authoring-submit-reason:empty { display: none; }
swath-authoring-panel .swath-authoring-error {
  margin: 8px 0 0;
  padding: 6px 8px;
  border: 1px solid rgb(248 113 113 / 45%);
  border-radius: 6px;
  background: rgb(248 113 113 / 10%);
  color: #fca5a5;
  font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  overflow-wrap: anywhere;
}
swath-authoring-panel .swath-authoring-empty,
swath-authoring-panel .swath-authoring-hint {
  margin: 0 0 8px;
  font: 12px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 80%);
}
swath-authoring-panel .swath-authoring-template {
  display: block;
  width: 100%;
  margin: 0 0 8px;
  padding: 6px 10px;
  border: 1px dashed rgb(148 163 184 / 40%);
  border-radius: 6px;
  background: none;
  cursor: pointer;
  color: inherit;
  font: 12px/1.5 system-ui, sans-serif;
}
swath-authoring-panel .swath-authoring-template:hover {
  background: rgb(148 163 184 / 12%);
}
swath-authoring-panel .swath-authoring-services {
  margin: 10px 0 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
swath-authoring-panel .swath-authoring-services li {
  display: flex;
  align-items: center;
  gap: 6px;
}
swath-authoring-panel .swath-authoring-service-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font: 12px/1.5 system-ui, sans-serif;
}
swath-authoring-panel .swath-authoring-services button {
  margin-left: auto;
  padding: 2px 7px;
  border: 1px solid rgb(248 113 113 / 40%);
  border-radius: 4px;
  background: none;
  cursor: pointer;
  color: #fca5a5;
  font: 11px/1.5 system-ui, sans-serif;
}
swath-authoring-panel .swath-authoring-services button:hover {
  background: rgb(248 113 113 / 12%);
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

/** One parameter of a served process definition (openEO 1.2.0). */
interface ProcessParameter {
  name: string;
  description?: string;
  optional?: boolean;
  default?: unknown;
  schema?: unknown;
}

/** The slice of a `GET /processes` entry the panel reads. */
export interface ProcessDefinition {
  id: string;
  summary?: string;
  parameters?: ProcessParameter[];
}

/** The slice of a `GET /services` entry the panel reads. */
export interface ServiceItem {
  id: string;
  title?: string;
}

/** The slice of a `GET /collections` entry the panel reads: the id (the
 * collection-id picker's vocabulary) and the datacube band names (the
 * band-field hints and the template's nir/red candidates). */
export interface CollectionItem {
  id: string;
  bands: string[];
}

/** One pipeline step: a chosen process plus the user's raw field state.
 * `sources` maps a parameter to the step key it references (`""` =
 * explicit literal, absent = derive the schema default); `values` holds
 * the literal inputs as typed, parsed only at build time. */
interface Step {
  key: string;
  process: ProcessDefinition;
  sources: Map<string, string>;
  values: Map<string, string>;
  /** Whether the step's advanced (defaulted/nullable) fields are shown. */
  advanced: boolean;
}

/** A schema (or one-of list of schemas) as its list of alternatives. */
function alternatives(schema: unknown): Record<string, unknown>[] {
  const list = Array.isArray(schema) ? schema : [schema];
  return list.filter((alt): alt is Record<string, unknown> => {
    return typeof alt === "object" && alt !== null && !Array.isArray(alt);
  });
}

/** The `type` values one alternative admits (openEO allows an array). */
function typesOf(alt: Record<string, unknown>): string[] {
  const type = alt["type"];
  if (typeof type === "string") {
    return [type];
  }
  return Array.isArray(type) ? type.filter((t): t is string => typeof t === "string") : [];
}

function hasSubtype(schema: unknown, subtype: string): boolean {
  return alternatives(schema).some((alt) => alt["subtype"] === subtype);
}

/** Data-cube parameters are wired, not typed. */
function isCube(schema: unknown): boolean {
  return hasSubtype(schema, "raster-cube") || hasSubtype(schema, "vector-cube");
}

function allowsNull(schema: unknown): boolean {
  return alternatives(schema).some((alt) => typesOf(alt).includes("null"));
}

function isNumeric(schema: unknown): boolean {
  return alternatives(schema).some((alt) => {
    const types = typesOf(alt);
    return types.includes("number") || types.includes("integer");
  });
}

function isStringArray(schema: unknown): boolean {
  return alternatives(schema).some((alt) => {
    if (!typesOf(alt).includes("array")) {
      return false;
    }
    const items = alt["items"];
    return (
      typeof items === "object" && items !== null && typesOf(items as never).includes("string")
    );
  });
}

function isString(schema: unknown): boolean {
  return alternatives(schema).some((alt) => typesOf(alt).includes("string"));
}

/** Enumerated string values, when any alternative pins an `enum`. */
function enumValues(schema: unknown): string[] {
  for (const alt of alternatives(schema)) {
    const values = alt["enum"];
    if (Array.isArray(values)) {
      const strings = values.filter((value): value is string => typeof value === "string");
      if (strings.length > 0) {
        return strings;
      }
    }
  }
  return [];
}

/** Does the parameter name (or its array items) a band? Drives the
 * band-vocabulary hint under the field. */
function isBandName(schema: unknown): boolean {
  if (hasSubtype(schema, "band-name")) {
    return true;
  }
  return alternatives(schema).some((alt) => {
    const items = alt["items"];
    return typeof items === "object" && items !== null && !Array.isArray(items)
      ? (items as Record<string, unknown>)["subtype"] === "band-name"
      : false;
  });
}

/** A raw field value, parsed by what the parameter's schema admits. */
function parseLiteral(raw: string, schema: unknown): unknown {
  if (hasSubtype(schema, "output-format-options")) {
    return { colormap: raw };
  }
  if (isStringArray(schema)) {
    return raw
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry !== "");
  }
  if (isNumeric(schema)) {
    const value = Number(raw);
    return Number.isNaN(value) ? raw : value;
  }
  if (isString(schema)) {
    return raw;
  }
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return raw;
  }
}

/** The prefill for a literal input, from the definition's `default`. */
function defaultText(param: ProcessParameter): string {
  const value = param.default;
  if (value === undefined || value === null) {
    return "";
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  if (Array.isArray(value)) {
    return value.join(",");
  }
  return "";
}

/** The first band matching any pattern, else the fallback — the NDVI
 * template's nir/red picker over a collection's band vocabulary. */
function pickBand(bands: readonly string[], patterns: RegExp[], fallback: string): string {
  for (const pattern of patterns) {
    const match = bands.find((band) => pattern.test(band));
    if (match !== undefined) {
      return match;
    }
  }
  return fallback;
}

/** Where a server diagnostic points: the compiler names nodes as
 * ``node `key`(...)`` and arguments as ``argument `name```. */
function locateServerError(message: string): { node?: string; argument?: string } {
  const located: { node?: string; argument?: string } = {};
  const node = message.match(/node `([^`]+)`/);
  if (node?.[1] !== undefined) {
    located.node = node[1];
  }
  const argument = message.match(/(?:missing required argument|invalid argument) `([^`]+)`/);
  if (argument?.[1] !== undefined) {
    located.argument = argument[1];
  }
  return located;
}

export class SwathAuthoringPanel extends HTMLElement {
  static readonly tagName = "swath-authoring-panel";

  /** Collapsed by default and LAZY, like the dataset browser beside it:
   * the closed panel fetches nothing — the first open (or `reload()`)
   * loads processes, collections, and services. */
  #open = false;
  #loadStarted = false;
  #processes: readonly ProcessDefinition[] = [];
  #collections: readonly CollectionItem[] = [];
  #services: readonly ServiceItem[] = [];
  #steps: Step[] = [];
  #counter = 0;
  #title = "";
  #error = "";
  #unavailable = false;
  /** Field keys (`s1-id`) the user has interacted with — inline
   * required/type messages only show for these, so freshly added steps
   * are not a wall of red; the disabled submit still counts them. */
  #touched = new Set<string>();
  /** Server diagnostics mapped onto fields (`s3-outputMin`) or whole
   * steps (`s3`); cleared per field on edit and wholesale on publish. */
  #serverNotes = new Map<string, string>();

  /** Test seam: the fetch this panel uses for every request. Assign a
   * stub BEFORE the element connects; leave unset for the real fetch. */
  fetchImpl: typeof fetch | undefined;

  /** Base URL of a Swath API (no trailing slash); same origin when the
   * `server` attribute is absent — mirroring `<swath-map>`. */
  get server(): string {
    return (this.getAttribute("server") ?? "").replace(/\/+$/, "");
  }

  connectedCallback(): void {
    injectStyles(this.ownerDocument);
    this.setAttribute("role", "group");
    if (!this.hasAttribute("aria-label")) {
      this.setAttribute("aria-label", "Author a layer");
    }
    this.#render();
  }

  /** Opens the panel (if collapsed) and (re)fetches the process
   * definitions, collections, and the published-services list — what
   * the first user toggle does; exposed for hosts and tests. */
  async reload(): Promise<void> {
    this.#open = true;
    this.#loadStarted = true;
    await this.#load();
  }

  #toggle(): void {
    this.#open = !this.#open;
    if (this.#open && !this.#loadStarted) {
      this.#loadStarted = true;
      void this.#load(); // renders again when the responses land
    }
    this.#render();
  }

  #fetch(path: string, init?: RequestInit): Promise<Response> {
    const call = this.fetchImpl ?? fetch;
    return call(`${this.server}${path}`, init);
  }

  async #load(): Promise<void> {
    try {
      const [processes, collections, services] = await Promise.all([
        this.#fetch("/processes", { headers: { accept: "application/json" } }),
        this.#fetch("/collections", { headers: { accept: "application/json" } }),
        this.#fetch("/services", { headers: { accept: "application/json" } }),
      ]);
      if (!processes.ok || !collections.ok || !services.ok) {
        throw new Error(
          `openEO surface answered ${processes.status}/${collections.status}/${services.status}`,
        );
      }
      const processBody = (await processes.json()) as { processes?: ProcessDefinition[] };
      const collectionBody = (await collections.json()) as {
        collections?: Record<string, unknown>[];
      };
      const serviceBody = (await services.json()) as { services?: ServiceItem[] };
      this.#processes = (processBody.processes ?? []).filter((p) => typeof p.id === "string");
      this.#collections = (collectionBody.collections ?? []).flatMap((doc) => {
        if (typeof doc["id"] !== "string") {
          return [];
        }
        const cube = doc["cube:dimensions"] as
          | Record<string, { type?: string; values?: unknown[] }>
          | undefined;
        const values = Object.values(cube ?? {}).find((d) => d.type === "bands")?.values ?? [];
        return [
          {
            id: doc["id"],
            bands: values.filter((band): band is string => typeof band === "string"),
          },
        ];
      });
      this.#services = (serviceBody.services ?? []).filter((s) => typeof s.id === "string");
      this.#unavailable = false;
    } catch {
      this.#unavailable = true;
    }
    if (this.isConnected) {
      this.#render();
    }
  }

  async #refreshServices(): Promise<void> {
    try {
      const response = await this.#fetch("/services", { headers: { accept: "application/json" } });
      if (!response.ok) {
        return;
      }
      const body = (await response.json()) as { services?: ServiceItem[] };
      this.#services = (body.services ?? []).filter((s) => typeof s.id === "string");
      if (this.isConnected) {
        this.#render();
      }
    } catch {
      // The list refresh is best-effort; publishing/deleting already
      // reported its own outcome.
    }
  }

  #addStep(process: ProcessDefinition): Step {
    this.#counter += 1;
    const step: Step = {
      key: `s${this.#counter}`,
      process,
      sources: new Map(),
      values: new Map(
        (process.parameters ?? [])
          .map((param): [string, string] => [
            param.name,
            PROFILE_DEFAULTS[`${process.id}.${param.name}`] ?? defaultText(param),
          ])
          .filter(([, text]) => text !== ""),
      ),
      advanced: false,
    };
    this.#steps.push(step);
    return step;
  }

  #removeStep(key: string): void {
    this.#steps = this.#steps.filter((step) => step.key !== key);
    this.#render();
  }

  /** Builds the NDVI starter pipeline: the first collection, nir/red
   * picked from its band vocabulary, the built-in layer's -1..1 → 0..255
   * scale and rdylgn colormap — a graph that renders, ready to modify. */
  #applyTemplate(): void {
    const definition = (id: string): ProcessDefinition | undefined =>
      this.#processes.find((process) => process.id === id);
    const collection = this.#collections[0];
    if (collection === undefined) {
      return;
    }
    const nir = pickBand(collection.bands, [/nir/i, /8a$/i], "nir");
    const red = pickBand(collection.bands, [/red/i, /04$/i], "red");
    const fill: Record<string, Record<string, string>> = {
      load_collection: { id: collection.id, bands: `${nir},${red}` },
      ndvi: { nir, red },
      linear_scale_range: { inputMin: "-1", inputMax: "1", outputMin: "0", outputMax: "255" },
      save_result: { format: "png", options: "rdylgn" },
    };
    for (const id of NDVI_TEMPLATE) {
      const process = definition(id);
      if (process === undefined) {
        continue; // the button only renders when all four exist
      }
      const step = this.#addStep(process);
      for (const [name, value] of Object.entries(fill[id] ?? {})) {
        step.values.set(name, value);
      }
    }
    this.#render();
  }

  /** The step keys a parameter of `index` may reference: earlier only. */
  #priorKeys(index: number): string[] {
    return this.#steps.slice(0, index).map((step) => step.key);
  }

  /** Where a parameter draws from: the stored choice when it still names
   * an earlier step (or an explicit literal), else the schema-derived
   * default — cube parameters and the first required parameter of a
   * non-first step wire to the previous step. */
  #effectiveSource(step: Step, index: number, param: ProcessParameter): string {
    const prior = this.#priorKeys(index);
    const stored = step.sources.get(param.name);
    if (stored === "" || (stored !== undefined && prior.includes(stored))) {
      return stored;
    }
    const previous = prior[prior.length - 1];
    if (previous === undefined) {
      return "";
    }
    if (isCube(param.schema)) {
      return previous;
    }
    const firstRequired = (step.process.parameters ?? []).find((p) => p.optional !== true);
    return firstRequired?.name === param.name ? previous : "";
  }

  // --- Validation (schema-driven, before any request) ---

  /** What blocks this literal field, or `""`: required-but-empty (when
   * the schema offers no null alternative) and non-numeric input into a
   * number-typed parameter. */
  #fieldIssue(step: Step, index: number, param: ProcessParameter): string {
    if (this.#effectiveSource(step, index, param) !== "") {
      return "";
    }
    const raw = (step.values.get(param.name) ?? "").trim();
    if (raw === "") {
      if (param.optional === true || allowsNull(param.schema)) {
        return "";
      }
      return "required";
    }
    if (isNumeric(param.schema) && !isStringArray(param.schema) && Number.isNaN(Number(raw))) {
      return "must be a number";
    }
    return "";
  }

  /** Everything still blocking submit, spelled out. Structural rule:
   * some step must name a collection (a filled `collection-id`-subtype
   * parameter) — the class of rejection the server would otherwise
   * answer with is unreachable from the UI. */
  #submitIssues(): string[] {
    const issues: string[] = [];
    let blockedFields = 0;
    for (const [index, step] of this.#steps.entries()) {
      for (const param of step.process.parameters ?? []) {
        if (this.#fieldIssue(step, index, param) !== "") {
          blockedFields += 1;
        }
      }
    }
    if (blockedFields > 0) {
      issues.push(
        blockedFields === 1 ? "1 field needs a value" : `${blockedFields} fields need values`,
      );
    }
    const loadsCollection = this.#steps.some((step, index) =>
      (step.process.parameters ?? []).some(
        (param) =>
          hasSubtype(param.schema, "collection-id") &&
          this.#effectiveSource(step, index, param) === "" &&
          (step.values.get(param.name) ?? "").trim() !== "",
      ),
    );
    if (!loadsCollection) {
      issues.push("no step loads a collection yet");
    }
    return issues;
  }

  /** Refreshes every inline note, the submit button, and its reason
   * line in place — called on each keystroke, no re-render, no lost
   * focus. */
  #updateValidity(): void {
    for (const [index, step] of this.#steps.entries()) {
      const stepNote = this.querySelector(`#swath-authoring-${step.key}-error`);
      if (stepNote) {
        stepNote.textContent = this.#serverNotes.get(step.key) ?? "";
      }
      for (const param of step.process.parameters ?? []) {
        const fieldKey = `${step.key}-${param.name}`;
        const note = this.querySelector(`#swath-authoring-${fieldKey}-note`);
        if (!note) {
          continue;
        }
        const server = this.#serverNotes.get(fieldKey);
        if (server !== undefined) {
          note.textContent = server;
          continue;
        }
        const issue = this.#fieldIssue(step, index, param);
        note.textContent = this.#touched.has(fieldKey) ? issue : "";
      }
    }
    const submit = this.querySelector<HTMLButtonElement>(".swath-authoring-submit");
    const reason = this.querySelector(".swath-authoring-submit-reason");
    if (submit && reason) {
      const issues = this.#submitIssues();
      submit.disabled = issues.length > 0;
      reason.textContent = issues.length > 0 ? `To publish: ${issues.join("; ")}.` : "";
    }
    const narrative = this.querySelector("#swath-authoring-narrative");
    if (narrative) {
      narrative.textContent = this.#narrative();
    }
  }

  /** The pipeline in one plain sentence ("Load hls-s30 (bands …) →
   * compute NDVI (…) → …"), from the current field values. */
  #narrative(): string {
    const phrases = this.#steps.map((step) =>
      narrativePhrase(step.process.id, (name) => (step.values.get(name) ?? "").trim()),
    );
    const sentence = phrases.join(" → ");
    return sentence === "" ? "" : `${sentence.charAt(0).toUpperCase()}${sentence.slice(1)}.`;
  }

  /** The composed openEO process graph: one node per step, arguments
   * from the generated fields, the last step marked as the result. */
  buildGraph(): Record<string, unknown> {
    const graph: Record<string, unknown> = {};
    for (const [index, step] of this.#steps.entries()) {
      const args: Record<string, unknown> = {};
      for (const param of step.process.parameters ?? []) {
        const source = this.#effectiveSource(step, index, param);
        if (source !== "") {
          args[param.name] = { from_node: source };
          continue;
        }
        const raw = (step.values.get(param.name) ?? "").trim();
        if (raw === "") {
          if (param.optional === true) {
            continue; // optional and empty: omitted, the default applies
          }
          if (allowsNull(param.schema)) {
            args[param.name] = null; // required but nullable: explicit null
          }
          // Required without a null alternative and left empty: omitted.
          // Unreachable through submit (validation disables it), kept for
          // direct buildGraph() callers.
          continue;
        }
        args[param.name] = parseLiteral(raw, param.schema);
      }
      const node: Record<string, unknown> = { process_id: step.process.id, arguments: args };
      if (index === this.#steps.length - 1) {
        node["result"] = true;
      }
      graph[step.key] = node;
    }
    return graph;
  }

  /** Files a server diagnostic where it belongs: on the named field
   * when the message locates a node and argument, on the step when only
   * a node, else the general inline error. */
  #reportServerError(message: string): void {
    const { node, argument } = locateServerError(message);
    const step = this.#steps.find((s) => s.key === node);
    if (step !== undefined) {
      const param = (step.process.parameters ?? []).find((p) => p.name === argument);
      const key = param ? `${step.key}-${param.name}` : step.key;
      this.#serverNotes.set(key, message);
      this.#error = "";
      this.#render();
      this.querySelector(`[data-step="${step.key}"]`)?.scrollIntoView({ block: "nearest" });
    } else {
      this.#error = message;
      this.#render();
    }
  }

  async #publish(): Promise<void> {
    if (this.#submitIssues().length > 0) {
      return; // the disabled submit already says why
    }
    this.#serverNotes.clear();
    const body: Record<string, unknown> = {
      type: "xyz",
      process: { process_graph: this.buildGraph() },
    };
    if (this.#title.trim() !== "") {
      body["title"] = this.#title.trim();
    }
    try {
      const response = await this.#fetch("/services", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!response.ok) {
        this.#reportServerError(await readOpenEoError(response));
        return;
      }
      const id =
        response.headers.get("openeo-identifier") ??
        response.headers.get("location")?.split("/").filter(Boolean).pop() ??
        "";
      this.#error = "";
      this.#render();
      if (id !== "") {
        this.dispatchEvent(
          new CustomEvent("swath-service-created", { detail: { id }, bubbles: true }),
        );
      }
      await this.#refreshServices();
    } catch (error) {
      this.#error = `request failed: ${String(error)}`;
      this.#render();
    }
  }

  async #delete(id: string): Promise<void> {
    try {
      const response = await this.#fetch(`/services/${id}`, { method: "DELETE" });
      if (!response.ok) {
        this.#error = await readOpenEoError(response);
        this.#render();
        return;
      }
      this.#error = "";
      this.#render();
      this.dispatchEvent(
        new CustomEvent("swath-service-deleted", { detail: { id }, bubbles: true }),
      );
      await this.#refreshServices();
    } catch (error) {
      this.#error = `request failed: ${String(error)}`;
      this.#render();
    }
  }

  #render(): void {
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "swath-authoring-toggle";
    toggle.textContent = "Author a layer";
    toggle.setAttribute("aria-expanded", String(this.#open));
    toggle.addEventListener("click", () => {
      this.#toggle();
    });

    if (!this.#open) {
      this.replaceChildren(toggle);
      return;
    }
    if (this.#unavailable) {
      const empty = document.createElement("p");
      empty.className = "swath-authoring-empty";
      empty.textContent = "The openEO authoring surface is not reachable.";
      this.replaceChildren(toggle, empty);
      return;
    }
    if (this.#processes.length === 0) {
      const empty = document.createElement("p");
      empty.className = "swath-authoring-empty";
      empty.textContent = "Waiting for the server's process definitions…";
      this.replaceChildren(toggle, empty);
      return;
    }

    const children: Element[] = [toggle, this.#renderPalette(), this.#renderForm()];
    if (this.#error !== "") {
      const error = document.createElement("p");
      error.className = "swath-authoring-error";
      error.setAttribute("role", "alert");
      error.textContent = this.#error;
      children.push(error);
    }
    children.push(...this.#renderServices());
    this.replaceChildren(...children);
    this.#updateValidity();
  }

  /** The palette: one add-button per served process definition, the
   * process summary as its tooltip. */
  #renderPalette(): Element {
    const palette = document.createElement("div");
    palette.className = "swath-authoring-palette";
    palette.setAttribute("role", "group");
    palette.setAttribute("aria-label", "Add a process step");
    for (const process of this.#processes) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = `+ ${process.id}`;
      button.dataset["process"] = process.id;
      if (process.summary !== undefined) {
        button.title = process.summary;
      }
      button.addEventListener("click", () => {
        this.#addStep(process);
        this.#render();
      });
      palette.append(button);
    }
    return palette;
  }

  #renderForm(): Element {
    const form = document.createElement("form");
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.#publish();
    });

    if (this.#steps.length === 0) {
      const hint = document.createElement("p");
      hint.className = "swath-authoring-hint";
      hint.textContent =
        "Build a pipeline: add steps from the palette above. Later steps can use " +
        "earlier steps' outputs, and the last step is the published result.";
      form.append(hint);
      if (
        NDVI_TEMPLATE.every((id) => this.#processes.some((process) => process.id === id)) &&
        this.#collections.length > 0
      ) {
        const template = document.createElement("button");
        template.type = "button";
        template.className = "swath-authoring-template";
        template.textContent = "Start from the NDVI template";
        template.title =
          "A working pipeline to modify: load a collection, NDVI, scale to 0..255, save as PNG.";
        template.addEventListener("click", () => {
          this.#applyTemplate();
        });
        form.append(template);
      }
      return form;
    }

    // The what-is-happening narrative: the pipeline in plain words,
    // always visible, refreshed live as fields change.
    const narrative = document.createElement("p");
    narrative.className = "swath-authoring-narrative";
    narrative.id = "swath-authoring-narrative";
    form.append(narrative);

    const list = document.createElement("ol");
    list.className = "swath-authoring-steps";
    for (const [index, step] of this.#steps.entries()) {
      list.append(this.#renderStep(step, index));
    }
    form.append(list);

    const titleLabel = document.createElement("label");
    titleLabel.htmlFor = "swath-authoring-title";
    titleLabel.textContent = "title";
    titleLabel.style.marginTop = "10px";
    const titleInput = document.createElement("input");
    titleInput.id = "swath-authoring-title";
    titleInput.type = "text";
    titleInput.value = this.#title;
    titleInput.addEventListener("input", () => {
      this.#title = titleInput.value;
    });
    titleLabel.append(titleInput);

    const submit = document.createElement("button");
    submit.type = "submit";
    submit.className = "swath-authoring-submit";
    submit.textContent = "Publish layer";
    const reason = document.createElement("p");
    reason.className = "swath-authoring-submit-reason";
    reason.id = "swath-authoring-submit-reason";

    form.append(titleLabel, submit, reason);
    return form;
  }

  #renderStep(step: Step, index: number): Element {
    const item = document.createElement("li");
    item.className = "swath-authoring-step";
    item.dataset["step"] = step.key;

    const header = document.createElement("p");
    header.className = "swath-authoring-step-header";
    const key = document.createElement("span");
    key.className = "swath-authoring-step-key";
    key.textContent = step.key;
    const name = document.createElement("span");
    name.textContent = step.process.id;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "✕";
    remove.setAttribute("aria-label", `Remove step ${step.key}`);
    remove.addEventListener("click", () => {
      this.#removeStep(step.key);
    });
    header.append(key, name, remove);
    item.append(header);

    if (step.process.summary !== undefined) {
      const summary = document.createElement("small");
      summary.className = "swath-authoring-step-summary";
      summary.textContent = step.process.summary;
      item.append(summary);
    }

    const stepError = document.createElement("p");
    stepError.className = "swath-authoring-step-error";
    stepError.id = `swath-authoring-${step.key}-error`;
    stepError.setAttribute("role", "alert");
    item.append(stepError);

    // Progressive disclosure: fields that work when left alone collapse
    // under the step's "advanced" toggle; the default view is only the
    // choices a newcomer understands. A blocked or server-flagged
    // advanced field forces the section open so its message is visible.
    const parameters = step.process.parameters ?? [];
    const basic = parameters.filter((param) => !isAdvancedParam(step.process.id, param));
    const advanced = parameters.filter((param) => isAdvancedParam(step.process.id, param));
    for (const param of basic) {
      item.append(...this.#renderField(step, index, param));
    }
    if (advanced.length > 0) {
      const open =
        step.advanced ||
        advanced.some(
          (param) =>
            this.#serverNotes.has(`${step.key}-${param.name}`) ||
            this.#fieldIssue(step, index, param) !== "",
        );
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "swath-authoring-advanced-toggle";
      toggle.textContent = `advanced (${advanced.length})`;
      toggle.setAttribute("aria-expanded", String(open));
      toggle.addEventListener("click", () => {
        step.advanced = !step.advanced;
        this.#render();
      });
      item.append(toggle);
      if (open) {
        for (const param of advanced) {
          item.append(...this.#renderField(step, index, param));
        }
      }
    }
    return item;
  }

  /** The band vocabulary in play: the first chosen collection's bands. */
  #pipelineBands(): string[] {
    for (const [index, step] of this.#steps.entries()) {
      for (const param of step.process.parameters ?? []) {
        if (
          hasSubtype(param.schema, "collection-id") &&
          this.#effectiveSource(step, index, param) === ""
        ) {
          const chosen = (step.values.get(param.name) ?? "").trim();
          const collection = this.#collections.find((c) => c.id === chosen);
          if (collection !== undefined) {
            return collection.bands;
          }
        }
      }
    }
    return [];
  }

  /** One generated field: an optional source select (steps after the
   * first can wire any parameter to an earlier step's output), the
   * literal widget the parameter's schema calls for, its inline
   * validation note, and a band hint where the schema names bands. */
  #renderField(step: Step, index: number, param: ProcessParameter): Element[] {
    const fieldKey = `${step.key}-${param.name}`;
    const fieldId = `swath-authoring-${fieldKey}`;
    const source = this.#effectiveSource(step, index, param);

    const label = document.createElement("label");
    label.htmlFor = fieldId;
    label.textContent = param.optional === true ? `${param.name} (optional)` : param.name;
    if (param.description !== undefined) {
      label.title = param.description;
    }
    // The plain-language one-liner, visible by default (the served
    // description stays available as the tooltip above).
    const help = fieldHelp(step.process.id, param.name);
    if (help !== "") {
      const line = document.createElement("small");
      line.className = "swath-authoring-field-help";
      line.textContent = help;
      label.append(line);
    }

    const touch = (): void => {
      this.#touched.add(fieldKey);
      this.#serverNotes.delete(fieldKey);
      this.#serverNotes.delete(step.key);
      this.#updateValidity();
    };
    const input = this.#renderValueInput(step, param, fieldId, touch);
    input.disabled = source !== "";

    const prior = this.#priorKeys(index);
    if (prior.length > 0) {
      const select = document.createElement("select");
      select.id = `${fieldId}-source`;
      select.setAttribute("aria-label", `${step.key} ${param.name} source`);
      const literal = document.createElement("option");
      literal.value = "";
      literal.textContent = "value:";
      select.append(literal);
      for (const priorKey of prior) {
        const option = document.createElement("option");
        option.value = priorKey;
        const priorStep = this.#steps.find((s) => s.key === priorKey);
        option.textContent = `from ${priorKey} (${priorStep?.process.id ?? "?"})`;
        select.append(option);
      }
      select.value = source;
      select.addEventListener("change", () => {
        step.sources.set(param.name, select.value);
        input.disabled = select.value !== "";
        touch();
      });
      label.append(select);
    }

    label.append(input);
    const note = document.createElement("small");
    note.className = "swath-authoring-field-note";
    note.id = `${fieldId}-note`;
    label.append(note);
    if (isBandName(param.schema)) {
      const bands = this.#pipelineBands();
      if (bands.length > 0) {
        const hint = document.createElement("small");
        hint.className = "swath-authoring-band-hint";
        hint.textContent = `bands: ${bands.join(", ")}`;
        label.append(hint);
      }
    }
    return [label];
  }

  #renderValueInput(
    step: Step,
    param: ProcessParameter,
    fieldId: string,
    touch: () => void,
  ): HTMLInputElement | HTMLSelectElement {
    const stored = step.values.get(param.name) ?? "";
    const dropdown = (
      values: readonly string[],
      placeholder: string,
      rerenderOnChange: boolean,
    ): HTMLSelectElement => {
      const select = document.createElement("select");
      select.id = fieldId;
      const none = document.createElement("option");
      none.value = "";
      none.textContent = placeholder;
      select.append(none);
      for (const value of values) {
        const option = document.createElement("option");
        option.value = value;
        option.textContent = value;
        select.append(option);
      }
      select.value = values.includes(stored) ? stored : "";
      select.addEventListener("change", () => {
        step.values.set(param.name, select.value);
        touch();
        if (rerenderOnChange) {
          this.#render(); // e.g. a collection choice refreshes band hints
        }
      });
      return select;
    };

    if (hasSubtype(param.schema, "collection-id") && this.#collections.length > 0) {
      // The collection picker: served ids only — an unknown collection
      // cannot be submitted from here.
      return dropdown(
        this.#collections.map((collection) => collection.id),
        "(choose a collection)",
        true,
      );
    }
    if (hasSubtype(param.schema, "output-format-options")) {
      // The one subtype-specialized widget: the Swath colormap select.
      return dropdown(COLORMAPS, "(default colormap)", false);
    }
    const enumerated = enumValues(param.schema);
    if (enumerated.length > 0) {
      return dropdown(enumerated, "(choose)", false);
    }
    const input = document.createElement("input");
    input.id = fieldId;
    input.type = isNumeric(param.schema) && !isStringArray(param.schema) ? "number" : "text";
    if (input.type === "number") {
      input.step = "any";
    }
    input.value = stored;
    input.addEventListener("input", () => {
      step.values.set(param.name, input.value);
      touch();
    });
    return input;
  }

  #renderServices(): Element[] {
    if (this.#services.length === 0) {
      return [];
    }
    const heading = document.createElement("h2");
    heading.className = "swath-authoring-heading";
    heading.style.marginTop = "12px";
    heading.textContent = "Published";
    const list = document.createElement("ul");
    list.className = "swath-authoring-services";
    for (const service of this.#services) {
      const item = document.createElement("li");
      const title = document.createElement("span");
      title.className = "swath-authoring-service-title";
      title.textContent = service.title ?? service.id;
      title.title = service.id;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "delete";
      remove.dataset["service"] = service.id;
      remove.setAttribute("aria-label", `Delete ${service.id}`);
      remove.addEventListener("click", () => {
        void this.#delete(service.id);
      });
      item.append(title, remove);
      list.append(item);
    }
    return [heading, list];
  }
}

/** The standardized openEO error body (`{code, message}`), rendered as
 * one line; falls back to the HTTP status for non-openEO bodies. */
async function readOpenEoError(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as { code?: unknown; message?: unknown };
    if (typeof body.code === "string" && typeof body.message === "string") {
      return `${body.code}: ${body.message}`;
    }
  } catch {
    // Fall through to the status line.
  }
  return `request failed with HTTP ${response.status}`;
}

/** Registers `<swath-authoring-panel>`; safe to call more than once. */
export function defineSwathAuthoringPanel(): void {
  if (!customElements.get(SwathAuthoringPanel.tagName)) {
    customElements.define(SwathAuthoringPanel.tagName, SwathAuthoringPanel);
  }
}
