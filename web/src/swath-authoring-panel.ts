// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-authoring-panel>` — the openEO authoring panel (issues #109
 * and #151, ADR 0010): compose a process graph as an ALWAYS-VALID
 * pipeline, publish it as an XYZ secondary service, delete published
 * services.
 *
 * Plain Custom Element, light DOM, no framework (ADR 0005). Collapsed
 * by default and lazy like the dataset browser beside it (issue #110):
 * the closed panel fetches nothing until a user actually authors.
 *
 * # Model B — the always-valid canvas (issue #151, design note §4/§8)
 *
 * The #148 panel validated FIELDS; the server still rejected pipeline
 * SHAPES (the recorded dangling-`divide` case). This panel generalizes
 * the pattern the collection picker proved — "an invalid value is
 * unreachable from the UI" — to pipeline structure. The invariant: the
 * pipeline is never in a state the server's process compiler rejects.
 *
 * - **Permanent head and tail.** The canvas always reads Load →
 *   [steps…] → Output. The Load and Output cards cannot be removed or
 *   reordered, so "the graph must end in save_result" (B1) and "no step
 *   loads a collection" are unconstructible; every step sits on the
 *   result path, so silently dead steps (B10) are too.
 * - **Stage-typed insertion.** Steps are insertable only where the
 *   whole pipeline still types under the compiler's cube discipline
 *   (the pinned stage table in `authoring-model.ts`): NDVI and the
 *   formula need a multi-band unscaled cube, stretching needs an
 *   unscaled cube — so scale-before-reduce (B4) and its mirror are
 *   unconstructible, and each gap's palette shows only what fits.
 * - **The formula builder.** Arithmetic (`add`…`divide`) and
 *   `array_element` never appear as pipeline steps (B2/B3); they live
 *   inside the "combine bands with a formula" card, which owns the
 *   `reduce_dimension` reducer child graph: lines of `left op right`
 *   over band selects, numbers, and earlier lines.
 * - **Vocabulary-only values.** Bands are checkboxes over the chosen
 *   collection's band vocabulary and NDVI's nir/red are selects over
 *   the LOADED bands (B7); the output format is a select over the
 *   profile vocabulary (B9); the colormap select greys out with a
 *   plain-words note while the result is multi-band (B6). Free text
 *   remains only where genuinely free (the title, numbers, expert
 *   extents under advanced).
 * - **Explained, not blocked silently.** What no structural rule can
 *   decide for the user is explained pre-submit in their words and
 *   gates the publish button: a 2-band pipeline "produces 2 channels; a
 *   picture needs 1 (NDVI or a formula) or 3 (red, green, blue)" (B5);
 *   a degenerate stretch range flags inline (B8).
 *
 * The #148 wins carry over: plain-language one-liners under every
 * field, smart defaults with per-card advanced collapse, the live
 * plain-words narrative, the one-click NDVI template, and server
 * diagnostics mapped onto the offending field — now nearly unreachable,
 * kept as the safety net (the server stays the authority; the client's
 * stage table only mirrors it, pinned by tests on both sides).
 *
 * Schema honesty, restated for Model B: the cards, their fields, their
 * widgets, and their validation still come from the served
 * `GET /processes` definitions — delete a definition and its card or
 * chip disappears (vitests pin exactly that). What the stage table adds
 * is ORDER, not vocabulary: it only sequences processes that exist,
 * mirroring compiler semantics the parameter schemas cannot express.
 *
 * # Preview before publish (issue #169, ADR 0014 — B11's countermeasure)
 *
 * A graph that is *valid and wrong* (swapped nir/red, a washed-out
 * range, a mismatched colormap) is unreachable by any validator; the
 * only countermeasure is SEEING the draft. Whenever the pipeline is
 * complete (the same gate as publish), the panel debounces a
 * `POST /result` — the server's preview-bounded synchronous subset,
 * which compiles the draft through the exact publish path — and renders
 * the returned PNG inline beside the narrative: swapped bands *show*
 * wrong, a washed-out range *shows* washed out, before anything is
 * published. A draft the server's preview budget refuses
 * (`ProcessGraphComplexity`) explains itself in plain words and never
 * blocks publishing — the budget bounds the preview, not the layer.
 *
 * # The UDF stage (issue #208, ADR 0018)
 *
 * `run_udf` joins the canvas as one more stage-typed step — offered
 * exactly where the server serves it (a stack without `--udf-store`
 * lists no `run_udf` in `GET /processes`, so no chip exists: the same
 * capabilities-driven rule as every other card) and only where the
 * stage table admits it (over the loaded cube, once per graph; its
 * result accepts scaling and a colormap-less output). The module is a
 * `.wasm` picked or dropped onto the card, base64-encoded client-side
 * into the node's `udf` argument as a `data:` URL (refused past the
 * server's 8 MiB bound, in plain words, before encoding); runtime
 * `"wasm"` / version `"1"` are vocabulary selects, `context` is a small
 * JSON field. The output arity (gray or RGB) is the module's answer,
 * pinned server-side at compile time — the canvas says so rather than
 * guessing, and the preview shows which. The preview's fuel/trap
 * diagnostics (#206's `ProcessGraphComplexity` / `ProcessParameterInvalid`)
 * land on the module field in plain words, and never gate publishing.
 * Writing the module IN the browser (a Rust playground) is a recorded
 * deferral — `docs/ROADMAP.md` §2 row 18.
 *
 * `POST /services` publishes; success announces a bubbling
 * `swath-service-created` (detail: `{id}`). Published services list
 * from `GET /services` with per-item delete announcing
 * `swath-service-deleted`.
 */

import { ApiProblem, SwathApi } from "./api.js";
import {
  buildReducerGraph,
  CUBE_PARAM,
  contextIssue,
  EMPTY_OPERAND,
  FORMULA_OPS,
  type FormulaOp,
  type FormulaRow,
  finalStage,
  formatMib,
  formulaIssues,
  formulaPhrase,
  insertableAt,
  locateServerError,
  type Operand,
  type ProcessDefinition,
  type ProcessParameter,
  pickBand,
  type Stage,
  UDF_MAX_BYTES,
  udfDiagnostic,
  wasmDataUrl,
} from "./authoring-model.js";
import { createSwathEvent } from "./ui/events.js";

export type { ProcessDefinition, ProcessParameter } from "./authoring-model.js";

const STYLE_ELEMENT_ID = "swath-authoring-panel-styles";

/** The render profile's colormap vocabulary, offered for the
 * `output-format-options` schema subtype (`save_result`'s `options`).
 * A widget for a subtype — not a per-process form definition. */
const COLORMAPS = ["grayscale", "viridis", "magma", "rdylgn"] as const;

/** The render profile's output-format vocabulary: the profile serves
 * PNG tiles, so the format select offers exactly that — a wrong format
 * (B9) is unconstructible, mirroring the profile note on the served
 * `save_result` definition. */
const FORMATS = ["png"] as const;

/** How long the canvas stays quiet after an edit before previewing the
 * draft (ADR 0014 keeps rate/debounce a UI concern: the endpoint is
 * stateless, the canvas owns its own pacing). */
const PREVIEW_DEBOUNCE_MS = 300;

/** The NDVI template's pipeline, in order. The template is only offered
 * while every one of these is present in the served definitions — a
 * shortcut over the canvas, not a parallel form source. */
const NDVI_TEMPLATE = ["load_collection", "ndvi", "linear_scale_range", "save_result"] as const;

/** Plain step titles over the served vocabulary (curated wording, like
 * [`FIELD_HELP`]); unknown processes fall back to their id. */
const STEP_TITLES: Record<string, string> = {
  load_collection: "Load imagery",
  ndvi: "NDVI (vegetation health)",
  reduce_dimension: "Combine bands with a formula",
  run_udf: "Run your own code (WASM module)",
  linear_scale_range: "Stretch values for display",
  save_result: "Output",
};

/** The insert chips' plain wording, same curation. */
const INSERT_LABELS: Record<string, string> = {
  ndvi: "NDVI (vegetation health)",
  reduce_dimension: "combine bands with a formula",
  run_udf: "run your own code (.wasm)",
  linear_scale_range: "stretch values for display",
};

/** The `run_udf` runtime vocabulary the profile admits (ADR 0018:
 * runtime "wasm", version "1", only) — selects, never free text, like
 * the format select (B9). */
const UDF_RUNTIMES = ["wasm"] as const;
const UDF_VERSIONS = ["1"] as const;

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
    "When: the dates to show — leave both empty to use everything available. " +
    "The map shows the newest image inside the range (the end date itself is not included).",
  "load_collection.bands":
    "The channels to load — tick them in the order you want them (for a 3-band picture, " +
    "that order is red, green, blue).",
  "load_collection.properties": "Extra metadata filters — safe to leave alone.",
  "ndvi.nir": "The band that measures near-infrared light.",
  "ndvi.red": "The band that measures red light.",
  "ndvi.target_band": "Leave as is: the NDVI result replaces the bands.",
  "linear_scale_range.inputMin": "The smallest value expected in the data (NDVI: -1).",
  "linear_scale_range.inputMax": "The largest value expected in the data (NDVI: 1).",
  "linear_scale_range.outputMin": "Keep at 0 — screen pixels run 0..255.",
  "linear_scale_range.outputMax": "Keep at 255 — screen pixels run 0..255.",
  "save_result.format": "The image format tiles are served in — png.",
  "save_result.options": "How numbers become colors on the map.",
  "run_udf.udf":
    "A compiled .wasm module (up to 8 MiB) that turns the loaded bands into 1 value per " +
    "pixel (gray) or 3 (red, green, blue) — the module decides which; the preview shows it.",
  "run_udf.context":
    'Settings handed to the module as-is, as a JSON object like {"threshold": 0.3} — ' +
    "leave empty if it needs none.",
  "run_udf.runtime": "Leave as is: modules run as sandboxed WebAssembly.",
  "run_udf.version": "Leave as is: the module speaks Swath UDF ABI version 1.",
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
  "run_udf.runtime": "wasm",
  "run_udf.version": "1",
};

/** Is this field tucked under the step's "advanced" toggle? Everything
 * that works when left alone (optional, nullable, or defaulted)
 * collapses; what stays visible is exactly the newcomer's choices —
 * the collection, bands, ranges, and the colormap. Derived from the
 * schemas, like the fields. */
function isAdvancedParam(processId: string, param: ProcessParameter): boolean {
  if (
    hasSubtype(param.schema, "collection-id") ||
    hasSubtype(param.schema, "output-format-options") ||
    // The load card's plain-words "when" control (ADR 0015): time is a
    // newcomer's choice now that windows select which granule serves.
    (processId === "load_collection" && hasSubtype(param.schema, "temporal-interval")) ||
    // The UDF card's module settings (#208): optional, but the one thing
    // a module author actually tunes — a newcomer's choice, on the card.
    (processId === "run_udf" && param.name === "context") ||
    isBandName(param.schema)
  ) {
    return false;
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
function narrativePhrase(
  processId: string,
  value: (name: string) => string,
  formula: string,
): string {
  switch (processId) {
    case "load_collection": {
      const id = value("id");
      const bands = value("bands");
      const when = temporalPhrase(value("temporal_extent"));
      return `load ${id === "" ? "a collection" : id}${bands === "" ? "" : ` (bands ${bands})`}${
        when === "everything available" ? "" : `, ${when}`
      }`;
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
      return formula === ""
        ? "combine the bands with a formula"
        : `combine the bands with a formula (${formula})`;
    case "run_udf": {
      // `udf` reads as the module's file name here (the narrative never
      // retells 8 MiB of base64); the arity is the module's answer.
      const module = value("udf");
      return `run ${module === "" ? "your module" : module} on the bands (1 or 3 channels — the module decides)`;
    }
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
swath-authoring-panel .swath-authoring-steps {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
swath-authoring-panel .swath-authoring-step {
  border: 1px solid rgb(148 163 184 / 20%);
  border-radius: 6px;
  padding: 8px;
}
swath-authoring-panel .swath-authoring-step[data-permanent] {
  border-color: rgb(74 222 128 / 25%);
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
swath-authoring-panel .swath-authoring-insert {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
  margin: 0;
  padding: 0 8px;
}
swath-authoring-panel .swath-authoring-insert button {
  padding: 2px 8px;
  border: 1px dashed rgb(148 163 184 / 40%);
  border-radius: 999px;
  background: none;
  cursor: pointer;
  color: rgb(148 163 184 / 90%);
  font: 11px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
swath-authoring-panel .swath-authoring-insert button:hover {
  background: rgb(148 163 184 / 12%);
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
swath-authoring-panel input:disabled,
swath-authoring-panel select:disabled { opacity: 0.4; cursor: not-allowed; }
swath-authoring-panel .swath-authoring-bands {
  display: flex;
  flex-wrap: wrap;
  gap: 2px 10px;
  margin: 2px 0 0;
}
swath-authoring-panel .swath-authoring-when {
  display: flex;
  flex-wrap: wrap;
  gap: 2px 10px;
  margin: 2px 0 0;
}
swath-authoring-panel .swath-authoring-when label {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  font: 12px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
swath-authoring-panel .swath-authoring-when input {
  display: inline-block;
  width: auto;
  margin: 0;
}
swath-authoring-panel .swath-authoring-bands label {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  font: 12px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  cursor: pointer;
}
swath-authoring-panel .swath-authoring-bands input {
  display: inline-block;
  width: auto;
  margin: 0;
}
swath-authoring-panel .swath-authoring-field-help {
  display: block;
  margin: 0 0 2px;
  font: 11px/1.4 system-ui, sans-serif;
  font-weight: 400;
  letter-spacing: normal;
  text-transform: none;
  color: rgb(148 163 184 / 75%);
}
swath-authoring-panel .swath-authoring-plain {
  display: block;
  margin: 0 0 6px;
  font: 11px/1.5 system-ui, sans-serif;
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
swath-authoring-panel .swath-authoring-preview {
  margin: 0 0 10px;
  padding: 0;
}
swath-authoring-panel .swath-authoring-preview img {
  display: block;
  width: 128px;
  height: 128px;
  border: 1px solid rgb(148 163 184 / 30%);
  border-radius: 6px;
  background:
    repeating-conic-gradient(rgb(148 163 184 / 12%) 0% 25%, rgb(15 23 42 / 60%) 0% 50%)
    0 0 / 16px 16px;
}
swath-authoring-panel .swath-authoring-preview img[hidden] { display: none; }
swath-authoring-panel .swath-authoring-preview figcaption {
  margin: 2px 0 0;
  font: 11px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 75%);
  overflow-wrap: anywhere;
}
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
swath-authoring-panel .swath-authoring-step-error {
  margin: 0 0 6px;
  font: 11px/1.5 system-ui, sans-serif;
  color: #fca5a5;
  overflow-wrap: anywhere;
}
swath-authoring-panel .swath-authoring-step-error:empty { display: none; }
swath-authoring-panel .swath-authoring-udf-drop {
  display: block;
  margin: 2px 0 0;
  padding: 8px;
  border: 1px dashed rgb(148 163 184 / 40%);
  border-radius: 6px;
  font: 11px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 85%);
}
swath-authoring-panel .swath-authoring-udf-drop[data-active] {
  border-color: #4ade80;
  background: rgb(74 222 128 / 8%);
}
swath-authoring-panel .swath-authoring-udf-drop input[type="file"] {
  margin-top: 4px;
  padding: 2px 0;
  border: 0;
  background: none;
}
swath-authoring-panel .swath-authoring-udf-module {
  display: block;
  margin: 2px 0 0;
  font: 11px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  color: #4ade80;
  overflow-wrap: anywhere;
}
swath-authoring-panel .swath-authoring-udf-module:empty { display: none; }
swath-authoring-panel textarea {
  display: block;
  width: 100%;
  box-sizing: border-box;
  min-height: 3em;
  margin-top: 1px;
  padding: 3px 6px;
  border: 1px solid rgb(148 163 184 / 30%);
  border-radius: 4px;
  background: rgb(15 23 42 / 60%);
  color: inherit;
  font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  resize: vertical;
}
swath-authoring-panel .swath-authoring-formula-row {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0 0 4px;
}
swath-authoring-panel .swath-authoring-formula-row .swath-authoring-formula-line {
  font: 700 11px/1.6 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  color: #4ade80;
  white-space: nowrap;
}
swath-authoring-panel .swath-authoring-formula-row select,
swath-authoring-panel .swath-authoring-formula-row input {
  margin-top: 0;
  min-width: 0;
}
swath-authoring-panel .swath-authoring-formula-row .swath-authoring-formula-op {
  flex: 0 0 52px;
}
swath-authoring-panel .swath-authoring-formula-row button {
  flex: none;
  border: none;
  background: none;
  cursor: pointer;
  color: rgb(148 163 184 / 80%);
  font: 12px/1 system-ui, sans-serif;
}
swath-authoring-panel .swath-authoring-formula-row button:hover { color: #f87171; }
swath-authoring-panel .swath-authoring-formula-add {
  display: block;
  margin: 0 0 6px;
  padding: 2px 8px;
  border: 1px dashed rgb(148 163 184 / 40%);
  border-radius: 999px;
  background: none;
  cursor: pointer;
  color: rgb(148 163 184 / 90%);
  font: 11px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
swath-authoring-panel .swath-authoring-formula-add:hover {
  background: rgb(148 163 184 / 12%);
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

/** The slice of a `GET /services` entry the panel reads. */
export interface ServiceItem {
  id: string;
  title?: string;
}

/** The slice of a `GET /collections` entry the panel reads: the id (the
 * collection-id picker's vocabulary) and the datacube band names (the
 * bands checkboxes' and band selects' vocabulary). */
export interface CollectionItem {
  id: string;
  bands: string[];
}

/** One pipeline card: a served process plus the user's raw field state.
 * `values` holds literal inputs as typed, parsed only at build time;
 * formula cards (`reduce_dimension`) carry their `rows` instead of a
 * reducer value. Data flow needs no per-card state: the canvas is a
 * linear chain, each cube input wired to the previous card. */
interface Card {
  process: ProcessDefinition;
  values: Map<string, string>;
  rows: FormulaRow[];
  /** Whether the card's advanced (defaulted/nullable) fields are shown. */
  advanced: boolean;
  /** The UDF card's picked module (#208): its name and size for the
   * card and the narrative (the `udf` value itself is the data URL),
   * `refused` when it was over the server's bound and never encoded. */
  udf?: { name: string; size: number; refused: boolean };
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

/** Does the parameter name a single band? Drives the band-select
 * widget (the loaded-band vocabulary promoted into the field, B7). */
function isBandName(schema: unknown): boolean {
  if (hasSubtype(schema, "band-name")) {
    return true;
  }
  return isBandArray(schema);
}

/** Does the parameter name an ARRAY of bands (`load_collection.bands`)?
 * Drives the band-checkbox widget. */
function isBandArray(schema: unknown): boolean {
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

/** The `[from, until]` date strings of a stored temporal-interval value
 * (`""` = open on that side; both `""` = no interval stored). */
function temporalBounds(raw: string): [string, string] {
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (Array.isArray(parsed) && parsed.length === 2) {
      return [
        typeof parsed[0] === "string" ? parsed[0] : "",
        typeof parsed[1] === "string" ? parsed[1] : "",
      ];
    }
  } catch {
    // Not an interval — treated as unset.
  }
  return ["", ""];
}

/** The stored temporal-interval value for two date-picker bounds:
 * `""` (field omitted → null, no filter) when both are empty, the
 * `[start, end]` JSON with `null` for an open side otherwise. */
function temporalValue(from: string, until: string): string {
  if (from === "" && until === "") {
    return "";
  }
  return JSON.stringify([from === "" ? null : from, until === "" ? null : until]);
}

/** The when line's plain words for a stored temporal-interval value. */
function temporalPhrase(raw: string): string {
  const [from, until] = temporalBounds(raw);
  if (from === "" && until === "") {
    return "everything available";
  }
  if (from === "") {
    return `until ${until}`;
  }
  if (until === "") {
    return `from ${from}`;
  }
  return `${from} until ${until}`;
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
  /** The permanent head: the Load card (`load_collection`), created
   * from the served definition — absent only when the server does not
   * serve one. */
  #loadCard: Card | undefined;
  /** The steps between Load and Output, in pipeline order. */
  #middle: Card[] = [];
  /** The permanent tail: the Output card (`save_result`) — B1's
   * "graph must end in save_result" made structural. */
  #saveCard: Card | undefined;
  /** The bands ticked on the Load card, in the order they were ticked
   * (the loaded-band order — for a composite that order is R, G, B). */
  #loadBands: string[] = [];
  #title = "";
  #error = "";
  #unavailable = false;
  /** The catalog reads (`/collections` + `/services`) ride behind the
   * canvas instead of gating it (issue #255): `pending` while they are
   * in flight, `failed` when they answered non-OK or threw — the canvas
   * stays up either way, and a failure re-arms the lazy load so the
   * next open retries (the add-data panel's #254 contract). */
  #catalogPending = false;
  #catalogFailed = false;
  /** Field keys (`s1-id`) the user has interacted with — inline
   * required/type messages only show for these, so a freshly inserted
   * card is not a wall of red; the disabled submit still counts them. */
  #touched = new Set<string>();
  /** Server diagnostics mapped onto fields (`s3-outputMin`) or whole
   * steps (`s3`); cleared per field on edit and wholesale on publish. */
  #serverNotes = new Map<string, string>();
  /** The draft preview (issue #169, ADR 0014 — B11's countermeasure):
   * the object URL of the last previewed PNG, the plain-words note when
   * there is no image to show, and the request body the state was
   * rendered from — previews are keyed on the composed graph, so
   * re-renders never refetch an unchanged draft. */
  #previewUrl = "";
  #previewNote = "";
  #previewedBody = "";
  #previewTimer: ReturnType<typeof setTimeout> | undefined;
  /** Field/step notes the PREVIEW filed (the UDF stage's fuel/trap
   * diagnostics, #208; a located compile diagnostic): cleared whenever
   * the next preview answers, so a fixed draft loses its stale note. */
  #previewNoteKeys = new Set<string>();

  /** Base URL of a Swath API (no trailing slash); same origin when the
   * `server` attribute is absent — mirroring `<swath-map>`. */
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

  connectedCallback(): void {
    injectStyles(this.ownerDocument);
    this.setAttribute("role", "group");
    if (!this.hasAttribute("aria-label")) {
      this.setAttribute("aria-label", "Author a layer");
    }
    this.#render();
  }

  disconnectedCallback(): void {
    clearTimeout(this.#previewTimer);
    this.#previewTimer = undefined;
    this.#clearPreview();
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

  async #load(): Promise<void> {
    // The canvas gates ONLY on `/processes` — a static definitions
    // document with no catalog round trip behind it. `/collections` and
    // `/services` both read the catalog, and under a concurrent live
    // tile-render burst (renders are inline on the runtime, ADR 0012)
    // those reads can be slow or transiently non-OK; the Load card's
    // readiness must not depend on them (issue #255). They start in
    // parallel here and hydrate the already-open canvas when they land.
    const catalog = this.#loadCatalog();
    try {
      const processes = await this.api.fetch("/processes", {
        headers: { accept: "application/json" },
      });
      if (!processes.ok) {
        throw new Error(`openEO surface answered ${processes.status}`);
      }
      const processBody = (await processes.json()) as { processes?: ProcessDefinition[] };
      this.#processes = (processBody.processes ?? []).filter((p) => typeof p.id === "string");
      this.#unavailable = false;
      // The permanent head and tail come from the served definitions —
      // schema honesty: no served `load_collection`/`save_result`, no
      // card. Existing cards survive a reload only if still served.
      const definition = (id: string): ProcessDefinition | undefined =>
        this.#processes.find((process) => process.id === id);
      const loadDef = definition("load_collection");
      const saveDef = definition("save_result");
      this.#loadCard =
        loadDef === undefined ? undefined : (this.#loadCard ?? this.#newCard(loadDef));
      this.#saveCard =
        saveDef === undefined ? undefined : (this.#saveCard ?? this.#newCard(saveDef));
      this.#middle = this.#middle.filter((card) => definition(card.process.id) !== undefined);
    } catch {
      // A failed load re-arms the lazy fetch: the next open retries
      // instead of showing the unreachable note forever (the add-data
      // panel's #254 contract).
      this.#unavailable = true;
      this.#loadStarted = false;
    }
    if (this.isConnected) {
      this.#render();
    }
    await catalog;
  }

  /** The catalog half of the lazy load: collections and services, off
   * the canvas's critical path (issue #255). A failure keeps the canvas
   * up, notes itself inline, and re-arms the lazy load so the next open
   * retries. */
  async #loadCatalog(): Promise<void> {
    this.#catalogPending = true;
    this.#catalogFailed = false;
    try {
      const [collections, services] = await Promise.all([
        this.api.fetch("/collections", { headers: { accept: "application/json" } }),
        this.api.fetch("/services", { headers: { accept: "application/json" } }),
      ]);
      if (!collections.ok || !services.ok) {
        throw new Error(`catalog reads answered ${collections.status}/${services.status}`);
      }
      const collectionBody = (await collections.json()) as {
        collections?: Record<string, unknown>[];
      };
      const serviceBody = (await services.json()) as { services?: ServiceItem[] };
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
      this.#catalogPending = false;
    } catch {
      this.#catalogPending = false;
      this.#catalogFailed = true;
      this.#loadStarted = false;
    }
    if (this.isConnected) {
      this.#render();
    }
  }

  async #refreshServices(): Promise<void> {
    try {
      const response = await this.api.fetch("/services", {
        headers: { accept: "application/json" },
      });
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

  #newCard(process: ProcessDefinition): Card {
    return {
      process,
      values: new Map(
        (process.parameters ?? [])
          .filter((param) => !isBandName(param.schema)) // band values come from vocabulary picks
          .map((param): [string, string] => [
            param.name,
            PROFILE_DEFAULTS[`${process.id}.${param.name}`] ?? defaultText(param),
          ])
          .filter(([, text]) => text !== ""),
      ),
      rows: [],
      advanced: false,
    };
  }

  /** The full pipeline, head to tail, in order — the single source of
   * step keys: the card at index `i` is `s${i + 1}`. */
  #cards(): Card[] {
    const cards: Card[] = [];
    if (this.#loadCard) {
      cards.push(this.#loadCard);
    }
    cards.push(...this.#middle);
    if (this.#saveCard) {
      cards.push(this.#saveCard);
    }
    return cards;
  }

  #keyOf(card: Card): string {
    return `s${this.#cards().indexOf(card) + 1}`;
  }

  /** The middle chain's process ids, for the stage table. */
  #middleIds(): string[] {
    return this.#middle.map((card) => card.process.id);
  }

  /** The pipeline's result stage (what reaches the Output card). */
  #resultStage(): Stage {
    return finalStage(this.#middleIds()) ?? { kind: "multi", scaled: false };
  }

  /** Inserts a served middle process at `gap` (0 = right after Load),
   * with newcomer prefills where the vocabulary suggests them (NDVI's
   * nir/red from the loaded bands, a first formula line). */
  #insertAt(gap: number, process: ProcessDefinition): void {
    const card = this.#newCard(process);
    if (process.id === "ndvi") {
      const nir = pickBand(this.#loadBands, [/nir/i, /8a$/i], "");
      const red = pickBand(this.#loadBands, [/red/i, /04$/i], "");
      if (nir !== "" && red !== "" && nir !== red) {
        card.values.set("nir", nir);
        card.values.set("red", red);
      }
    }
    if (process.id === "reduce_dimension") {
      card.rows.push({ op: "subtract", left: { ...EMPTY_OPERAND }, right: { ...EMPTY_OPERAND } });
    }
    this.#middle.splice(gap, 0, card);
    this.#serverNotes.clear(); // keys shift; stale notes would mislead
    this.#previewNoteKeys.clear();
    this.#render();
  }

  #removeMiddle(card: Card): void {
    this.#middle = this.#middle.filter((c) => c !== card);
    this.#serverNotes.clear();
    this.#previewNoteKeys.clear();
    this.#render();
  }

  /** Builds the NDVI starter pipeline: the first collection, nir/red
   * picked from its band vocabulary, the built-in layer's -1..1 → 0..255
   * scale and rdylgn colormap — a graph that renders, ready to modify. */
  #applyTemplate(): void {
    const definition = (id: string): ProcessDefinition | undefined =>
      this.#processes.find((process) => process.id === id);
    const collection = this.#collections[0];
    const ndviDef = definition("ndvi");
    const scaleDef = definition("linear_scale_range");
    if (!collection || !this.#loadCard || !this.#saveCard || !ndviDef || !scaleDef) {
      return; // the button only renders when everything exists
    }
    const nir = pickBand(collection.bands, [/nir/i, /8a$/i], "nir");
    const red = pickBand(collection.bands, [/red/i, /04$/i], "red");
    this.#loadCard.values.set("id", collection.id);
    this.#loadBands = [nir, red];
    const ndvi = this.#newCard(ndviDef);
    ndvi.values.set("nir", nir);
    ndvi.values.set("red", red);
    const scale = this.#newCard(scaleDef);
    scale.values.set("inputMin", "-1");
    scale.values.set("inputMax", "1");
    scale.values.set("outputMin", "0");
    scale.values.set("outputMax", "255");
    this.#middle = [ndvi, scale];
    this.#saveCard.values.set("format", "png");
    this.#saveCard.values.set("options", "rdylgn");
    this.#render();
  }

  // --- Validation (schema- and stage-driven, before any request) ---

  /** The chosen collection's band vocabulary (empty until chosen). */
  #collectionBands(): string[] {
    const chosen = (this.#loadCard?.values.get("id") ?? "").trim();
    return this.#collections.find((c) => c.id === chosen)?.bands ?? [];
  }

  /** What blocks this literal field, or `""`: required-but-empty (when
   * the schema offers no null alternative), non-numeric input into a
   * number-typed parameter, and the degenerate stretch range (B8). */
  #fieldIssue(card: Card, param: ProcessParameter): string {
    if (param.name === CUBE_PARAM[card.process.id]) {
      return ""; // wired to the previous card, never a field
    }
    if (card.process.id === "reduce_dimension") {
      // The formula card owns its parameters: `dimension` is pinned to
      // "bands" and `reducer` is composed from the rows —
      // [`formulaIssues`] is their validation.
      return "";
    }
    if (isBandArray(param.schema)) {
      return ""; // the bands checkboxes; counted as a pipeline issue
    }
    const raw = (card.values.get(param.name) ?? "").trim();
    if (card.process.id === "run_udf") {
      // The UDF card (#208): the module is picked, never typed — and a
      // module over the server's bound was refused before encoding, so
      // the field stays empty and says why.
      if (param.name === "udf") {
        if (card.udf?.refused) {
          return (
            `${card.udf.name} is ${formatMib(card.udf.size)} — the server accepts modules ` +
            `up to ${formatMib(UDF_MAX_BYTES)}; pick a smaller one`
          );
        }
        return raw === "" ? "upload a .wasm module" : "";
      }
      if (param.name === "context") {
        return contextIssue(raw);
      }
    }
    if (isBandName(param.schema)) {
      // Band selects: a value is always needed (the schema's "nir"/
      // "red" defaults are common names the loaded bands may not
      // carry), and it must be a currently loaded band (B7).
      if (raw === "") {
        return "pick a band";
      }
      return this.#loadBands.includes(raw) ? "" : `${raw} is not loaded any more`;
    }
    if (raw === "") {
      if (param.optional === true || allowsNull(param.schema)) {
        return "";
      }
      return "required";
    }
    if (isNumeric(param.schema) && !isStringArray(param.schema) && Number.isNaN(Number(raw))) {
      return "must be a number";
    }
    if (card.process.id === "linear_scale_range" && param.name === "inputMin") {
      const min = Number(raw);
      const max = Number((card.values.get("inputMax") ?? "").trim());
      if (!Number.isNaN(min) && !Number.isNaN(max) && min >= max) {
        return "the smallest value must be below the largest";
      }
    }
    return "";
  }

  /** Everything still blocking submit, spelled out in the user's words.
   * Structure is enforced by construction; what remains are the choices
   * only the user can make (B5's "which fix" included). */
  #submitIssues(): string[] {
    const issues: string[] = [];
    let blockedFields = 0;
    for (const card of this.#cards()) {
      for (const param of card.process.parameters ?? []) {
        if (this.#fieldIssue(card, param) !== "") {
          blockedFields += 1;
        }
      }
      if (card.process.id === "reduce_dimension") {
        blockedFields += formulaIssues(card.rows, this.#loadBands).length;
      }
    }
    if (blockedFields > 0) {
      issues.push(
        blockedFields === 1 ? "1 field needs a value" : `${blockedFields} fields need values`,
      );
    }
    if ((this.#loadCard?.values.get("id") ?? "").trim() === "") {
      issues.push("no collection chosen yet");
    } else if (this.#loadBands.length === 0) {
      issues.push("no bands ticked yet");
    }
    // B5, explained pre-submit: a multi-band result must be an RGB
    // composite; "which fix" (reduce, or load exactly 3) is the user's
    // call, so it gates rather than auto-corrects.
    const stage = this.#resultStage();
    if (stage.kind === "multi" && this.#loadBands.length > 0 && this.#loadBands.length !== 3) {
      const n = this.#loadBands.length;
      issues.push(
        `this pipeline produces ${n} channel${n === 1 ? "" : "s"}; a picture needs ` +
          "1 (add NDVI or a formula) or 3 (red, green, blue)",
      );
    }
    return issues;
  }

  /** Refreshes every inline note, the submit button, and its reason
   * line in place — called on each keystroke, no re-render, no lost
   * focus. */
  #updateValidity(): void {
    for (const card of this.#cards()) {
      const key = this.#keyOf(card);
      const stepNote = this.querySelector(`#swath-authoring-${key}-error`);
      if (stepNote) {
        stepNote.textContent = this.#serverNotes.get(key) ?? "";
      }
      if (card.process.id === "reduce_dimension") {
        const list = this.querySelector(`#swath-authoring-${key}-formula-issues`);
        if (list) {
          list.textContent = formulaIssues(card.rows, this.#loadBands).join("; ");
        }
      }
      for (const param of card.process.parameters ?? []) {
        const fieldKey = `${key}-${param.name}`;
        const note = this.querySelector(`#swath-authoring-${fieldKey}-note`);
        if (!note) {
          continue;
        }
        const server = this.#serverNotes.get(fieldKey);
        if (server !== undefined) {
          note.textContent = server;
          continue;
        }
        const issue = this.#fieldIssue(card, param);
        // Empty-required messages wait for a first touch (a fresh card
        // is not a wall of red); a FILLED field's issue — a stale band
        // pick, a degenerate range — always explains itself.
        const filled = (card.values.get(param.name) ?? "").trim() !== "";
        note.textContent = this.#touched.has(fieldKey) || filled ? issue : "";
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
    this.#schedulePreview();
  }

  // --- The draft preview (issue #169, ADR 0014 — B11's countermeasure) ---

  /** Schedules (or clears) the draft preview: whenever the pipeline is
   * complete — the same gate as publish — the composed graph is
   * debounced into the preview-bounded `POST /result`; anything less
   * shows no preview and makes no request. */
  #schedulePreview(): void {
    if (!this.querySelector("#swath-authoring-preview")) {
      return; // the canvas is not rendered (collapsed / unavailable)
    }
    if (this.#submitIssues().length > 0) {
      clearTimeout(this.#previewTimer);
      this.#previewTimer = undefined;
      this.#clearPreview();
      this.#clearPreviewNotes();
      this.#reflectPreview();
      return;
    }
    const body = JSON.stringify({ process: { process_graph: this.buildGraph() } });
    if (body === this.#previewedBody) {
      this.#reflectPreview(); // a re-render, not a new draft
      return;
    }
    clearTimeout(this.#previewTimer);
    this.#previewTimer = setTimeout(() => {
      void this.#loadPreview(body);
    }, PREVIEW_DEBOUNCE_MS);
    this.#reflectPreview();
  }

  #clearPreview(): void {
    if (this.#previewUrl !== "") {
      URL.revokeObjectURL(this.#previewUrl);
    }
    this.#previewUrl = "";
    this.#previewNote = "";
    this.#previewedBody = "";
  }

  /** POSTs the draft to `POST /result` and shows the returned PNG — or
   * the failure, in plain words. A refused preview never gates publish:
   * the server's budget bounds the preview, not the layer. */
  async #loadPreview(body: string): Promise<void> {
    this.#previewedBody = body;
    let url = "";
    let note = "";
    let failure: { code: string; message: string } | undefined;
    try {
      const response = await this.api.fetch("/result", {
        method: "POST",
        headers: { accept: "image/png", "content-type": "application/json" },
        body,
      });
      if (response.ok) {
        url = URL.createObjectURL(await response.blob());
      } else {
        failure = await readOpenEoBody(response);
        note = previewFailureNote(failure);
      }
    } catch {
      note = "The preview is unavailable right now — publishing still works.";
    }
    if (this.#previewedBody !== body) {
      // The draft moved on while this preview was in flight.
      if (url !== "") {
        URL.revokeObjectURL(url);
      }
      return;
    }
    if (this.#previewUrl !== "") {
      URL.revokeObjectURL(this.#previewUrl);
    }
    this.#previewUrl = url;
    if (failure === undefined) {
      this.#previewNote = note;
      this.#clearPreviewNotes(); // a fixed draft loses its stale diagnostic
    } else {
      this.#previewNote = this.#notePreviewFailure(failure, note);
    }
    this.#reflectPreview();
  }

  /** Files a preview failure where the user can act on it (#208): the
   * UDF stage's fuel/trap diagnostics (#206's registry codes) on the
   * module field in plain words; a compile diagnostic naming a node and
   * argument on that field (the same safety net publishing uses); the
   * rest stays on the preview caption. Preview-filed notes are replaced
   * wholesale on every answer, and NEVER gate publishing — the caption
   * always says where to look. */
  #notePreviewFailure(failure: { code: string; message: string }, caption: string): string {
    for (const key of this.#previewNoteKeys) {
      this.#serverNotes.delete(key);
    }
    this.#previewNoteKeys.clear();
    const cards = this.#cards();
    const udfCard = cards.find((card) => card.process.id === "run_udf");
    const diagnostic = udfDiagnostic(failure.code, failure.message);
    let noteKey: string | undefined;
    let text = "";
    if (udfCard && diagnostic !== undefined) {
      noteKey = `${this.#keyOf(udfCard)}-udf`;
      text = diagnostic;
    } else {
      const { node, argument } = locateServerError(failure.message);
      const index = node === undefined ? -1 : cards.findIndex((_, i) => `s${i + 1}` === node);
      const card = cards[index];
      if (card !== undefined) {
        const param = (card.process.parameters ?? []).find((p) => p.name === argument);
        noteKey = param ? `s${index + 1}-${param.name}` : `s${index + 1}`;
        text = `${failure.code}: ${failure.message}`;
        if (param && isAdvancedParam(card.process.id, param)) {
          card.advanced = true;
        }
      }
    }
    if (noteKey === undefined) {
      this.#updateValidity();
      return caption;
    }
    this.#serverNotes.set(noteKey, text);
    this.#previewNoteKeys.add(noteKey);
    const step = noteKey.split("-")[0] ?? noteKey;
    // A re-render, not just a validity pass: an advanced field forced
    // open needs its note element to exist before it can show anything.
    this.#render();
    return `The preview could not run this draft — see the note on step ${step}. Publishing is not blocked.`;
  }

  /** Drops every preview-filed note (a preview landed, or the draft is
   * no longer previewable) and refreshes the inline notes. */
  #clearPreviewNotes(): void {
    if (this.#previewNoteKeys.size === 0) {
      return;
    }
    for (const key of this.#previewNoteKeys) {
      this.#serverNotes.delete(key);
    }
    this.#previewNoteKeys.clear();
    this.#updateValidity();
  }

  /** Applies the preview state to the DOM in place (the elements are
   * re-created on every render; the state lives on the panel). */
  #reflectPreview(): void {
    const container = this.querySelector<HTMLElement>("#swath-authoring-preview");
    const image = this.querySelector<HTMLImageElement>("#swath-authoring-preview-image");
    const note = this.querySelector("#swath-authoring-preview-note");
    if (!container || !image || !note) {
      return;
    }
    if (this.#previewUrl === "") {
      image.hidden = true;
      image.removeAttribute("src");
    } else {
      image.hidden = false;
      if (image.getAttribute("src") !== this.#previewUrl) {
        image.src = this.#previewUrl;
      }
    }
    note.textContent =
      this.#previewUrl === "" ? this.#previewNote : "Preview — how the draft will look on the map.";
    container.hidden = this.#previewUrl === "" && this.#previewNote === "";
  }

  /** The pipeline in one plain sentence ("Load hls-s30 (bands …) →
   * compute NDVI (…) → …"), from the current field values. */
  #narrative(): string {
    const phrases = this.#cards().map((card) =>
      narrativePhrase(
        card.process.id,
        (name) =>
          name === "bands" && card === this.#loadCard
            ? this.#loadBands.join(",")
            : name === "udf" && card.process.id === "run_udf"
              ? card.udf?.refused === false
                ? card.udf.name
                : ""
              : (card.values.get(name) ?? "").trim(),
        card.process.id === "reduce_dimension" ? formulaPhrase(card.rows, this.#loadBands) : "",
      ),
    );
    const sentence = phrases.join(" → ");
    return sentence === "" ? "" : `${sentence.charAt(0).toUpperCase()}${sentence.slice(1)}.`;
  }

  /** The composed openEO process graph: one node per card in pipeline
   * order, each cube input wired to the previous card, the Output card
   * marked as the result — a graph that always ends in `save_result`
   * with nothing dangling, by construction (B1/B10). */
  buildGraph(): Record<string, unknown> {
    const cards = this.#cards();
    const stage = this.#resultStage();
    const graph: Record<string, unknown> = {};
    cards.forEach((card, index) => {
      const key = `s${index + 1}`;
      const previous = index > 0 ? `s${index}` : "";
      const args: Record<string, unknown> = {};
      if (card.process.id === "reduce_dimension") {
        args["data"] = { from_node: previous };
        args["dimension"] = "bands";
        args["reducer"] = { process_graph: buildReducerGraph(card.rows) };
      } else {
        for (const param of card.process.parameters ?? []) {
          if (param.name === CUBE_PARAM[card.process.id] && previous !== "") {
            args[param.name] = { from_node: previous };
            continue;
          }
          if (card === this.#loadCard && isBandArray(param.schema)) {
            if (this.#loadBands.length > 0) {
              args[param.name] = [...this.#loadBands];
            }
            continue;
          }
          if (
            card === this.#saveCard &&
            hasSubtype(param.schema, "output-format-options") &&
            stage.kind !== "gray"
          ) {
            // B6 made structural: a colormap never rides a composite —
            // nor a UDF result, which renders directly (ADR 0018).
            continue;
          }
          const raw = (card.values.get(param.name) ?? "").trim();
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
      }
      const node: Record<string, unknown> = { process_id: card.process.id, arguments: args };
      if (index === cards.length - 1) {
        node["result"] = true;
      }
      graph[key] = node;
    });
    return graph;
  }

  /** Files a server diagnostic where it belongs: on the named field
   * when the message locates a node and argument, on the step when only
   * a node, else the general inline error. Model B makes these nearly
   * unreachable; the mapping stays as the safety net. */
  #reportServerError(message: string): void {
    const { node, argument } = locateServerError(message);
    const cards = this.#cards();
    const index = node === undefined ? -1 : cards.findIndex((_, i) => `s${i + 1}` === node);
    const card = cards[index];
    if (card !== undefined) {
      const key = `s${index + 1}`;
      const param = (card.process.parameters ?? []).find((p) => p.name === argument);
      const noteKey = param ? `${key}-${param.name}` : key;
      this.#serverNotes.set(noteKey, message);
      if (param && isAdvancedParam(card.process.id, param)) {
        card.advanced = true; // surface the note even under the fold
      }
      this.#error = "";
      this.#render();
      this.querySelector(`[data-step="${key}"]`)?.scrollIntoView({ block: "nearest" });
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
    this.#previewNoteKeys.clear();
    const body: Record<string, unknown> = {
      type: "xyz",
      process: { process_graph: this.buildGraph() },
    };
    if (this.#title.trim() !== "") {
      body["title"] = this.#title.trim();
    }
    try {
      const response = await this.api.fetch("/services", {
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
        this.dispatchEvent(createSwathEvent("swath-service-created", { id }));
      }
      await this.#refreshServices();
    } catch (error) {
      this.#error = `request failed: ${String(error)}`;
      this.#render();
    }
  }

  async #delete(id: string): Promise<void> {
    try {
      const response = await this.api.fetch(`/services/${id}`, { method: "DELETE" });
      if (!response.ok) {
        this.#error = await readOpenEoError(response);
        this.#render();
        return;
      }
      this.#error = "";
      this.#render();
      this.dispatchEvent(createSwathEvent("swath-service-deleted", { id }));
      await this.#refreshServices();
    } catch (error) {
      this.#error = `request failed: ${String(error)}`;
      this.#render();
    }
  }

  // --- Rendering ---------------------------------------------------------

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
    if (!this.#loadCard || !this.#saveCard) {
      // Schema honesty: the canvas is only as capable as the served
      // definitions — no load_collection or save_result, no pipeline.
      const empty = document.createElement("p");
      empty.className = "swath-authoring-empty";
      empty.textContent =
        "The served process set cannot author a picture here " +
        "(it needs load_collection and save_result).";
      this.replaceChildren(toggle, empty);
      return;
    }

    const children: Element[] = [toggle, this.#renderForm()];
    if (this.#catalogFailed) {
      // The canvas stays up on a failed catalog read (issue #255); the
      // note says what is missing and how to retry (the next open
      // refetches — the add-data panel's #254 contract).
      const note = document.createElement("p");
      note.className = "swath-authoring-error";
      note.setAttribute("role", "alert");
      note.textContent =
        "The collections and published-services lists could not be loaded — " +
        "close and reopen to retry.";
      children.push(note);
    }
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

  #renderForm(): Element {
    const form = document.createElement("form");
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.#publish();
    });

    if (this.#middle.length === 0) {
      const hint = document.createElement("p");
      hint.className = "swath-authoring-hint";
      hint.textContent =
        "Every pipeline starts by loading imagery and ends by saving a picture. " +
        "Add steps in between to compute something from the bands.";
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
    }

    // The what-is-happening narrative: the pipeline in plain words,
    // always visible, refreshed live as fields change.
    const narrative = document.createElement("p");
    narrative.className = "swath-authoring-narrative";
    narrative.id = "swath-authoring-narrative";
    form.append(narrative);

    // The draft preview beside the narrative (B11's countermeasure,
    // ADR 0014): ground truth where the narrative can only retell —
    // filled in by #reflectPreview whenever the draft is complete.
    const preview = document.createElement("figure");
    preview.className = "swath-authoring-preview";
    preview.id = "swath-authoring-preview";
    preview.hidden = true;
    const previewImage = document.createElement("img");
    previewImage.id = "swath-authoring-preview-image";
    previewImage.alt = "Preview of the draft layer";
    previewImage.width = 128;
    previewImage.height = 128;
    previewImage.hidden = true;
    const previewNote = document.createElement("figcaption");
    previewNote.id = "swath-authoring-preview-note";
    preview.append(previewImage, previewNote);
    form.append(preview);

    const list = document.createElement("ol");
    list.className = "swath-authoring-steps";
    const cards = this.#cards();
    const served = new Set(this.#processes.map((process) => process.id));
    for (const [index, card] of cards.entries()) {
      list.append(this.#renderStep(card, `s${index + 1}`));
      // An insert gap after every card except the Output tail, showing
      // only the chips the stage table admits there (B2/B3/B4: what
      // does not fit is not offered, anywhere).
      if (card !== this.#saveCard) {
        const gap = index; // gap g = insert at middle position g
        const fits = insertableAt(this.#middleIds(), gap, served);
        if (fits.length > 0) {
          list.append(this.#renderInsert(gap, fits));
        }
      }
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

  /** One insert gap: a chip per process the stage table admits here. */
  #renderInsert(gap: number, fits: readonly string[]): Element {
    const item = document.createElement("li");
    item.className = "swath-authoring-insert";
    item.dataset["gap"] = String(gap);
    item.setAttribute("role", "group");
    item.setAttribute("aria-label", "Add a step here");
    for (const id of fits) {
      const process = this.#processes.find((p) => p.id === id);
      if (!process) {
        continue;
      }
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = `+ ${INSERT_LABELS[id] ?? id}`;
      button.dataset["process"] = id;
      if (process.summary !== undefined) {
        button.title = process.summary;
      }
      button.addEventListener("click", () => {
        this.#insertAt(gap, process);
      });
      item.append(button);
    }
    return item;
  }

  #renderStep(card: Card, key: string): Element {
    const item = document.createElement("li");
    item.className = "swath-authoring-step";
    item.dataset["step"] = key;
    item.dataset["process"] = card.process.id;
    const permanent = card === this.#loadCard || card === this.#saveCard;
    if (permanent) {
      // The permanent head and tail: not removable, not reorderable —
      // B1 ("must end in save_result") unconstructible by construction.
      item.setAttribute("data-permanent", "");
    }

    const header = document.createElement("p");
    header.className = "swath-authoring-step-header";
    const keySpan = document.createElement("span");
    keySpan.className = "swath-authoring-step-key";
    keySpan.textContent = key;
    const name = document.createElement("span");
    name.textContent = STEP_TITLES[card.process.id] ?? card.process.id;
    name.title = card.process.id;
    header.append(keySpan, name);
    if (!permanent) {
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "✕";
      remove.setAttribute("aria-label", `Remove step ${key}`);
      remove.addEventListener("click", () => {
        this.#removeMiddle(card);
      });
      header.append(remove);
    }
    item.append(header);

    if (card.process.summary !== undefined) {
      const summary = document.createElement("small");
      summary.className = "swath-authoring-step-summary";
      summary.textContent = card.process.summary;
      item.append(summary);
    }

    const stepError = document.createElement("p");
    stepError.className = "swath-authoring-step-error";
    stepError.id = `swath-authoring-${key}-error`;
    stepError.setAttribute("role", "alert");
    item.append(stepError);

    if (card.process.id === "reduce_dimension") {
      this.#renderFormula(card, key, item);
      return item;
    }

    // Progressive disclosure: fields that work when left alone collapse
    // under the card's "advanced" toggle; the default view is only the
    // choices a newcomer understands. A blocked or server-flagged
    // advanced field forces the section open so its message is visible.
    const parameters = (card.process.parameters ?? []).filter(
      (param) => param.name !== CUBE_PARAM[card.process.id], // wired, never a field
    );
    const basic = parameters.filter((param) => !isAdvancedParam(card.process.id, param));
    const advanced = parameters.filter((param) => isAdvancedParam(card.process.id, param));
    for (const param of basic) {
      item.append(...this.#renderField(card, key, param));
    }
    if (card === this.#loadCard) {
      // The plain-worded where/when line (design note §4): area stays an
      // expert field under advanced; time is the card's own "when"
      // control (ADR 0015) — the line says what the current choice
      // means, in the user's vocabulary.
      const plain = document.createElement("small");
      plain.className = "swath-authoring-plain";
      plain.id = `swath-authoring-${key}-extent-summary`;
      const custom = (name: string): boolean => (card.values.get(name) ?? "").trim() !== "";
      plain.textContent = `Area: ${
        custom("spatial_extent") ? "custom (see advanced)" : "everywhere the collection covers"
      } · Time: ${temporalPhrase(card.values.get("temporal_extent") ?? "")}`;
      item.append(plain);
    }
    const resultKind = this.#resultStage().kind;
    if (card === this.#saveCard && resultKind !== "gray") {
      // B6, explained before it can even be attempted: while the result
      // is multi-band (or a UDF result, which renders directly) the
      // colormap is greyed out and this line says why in the user's
      // words (buildGraph also never sends it).
      const plain = document.createElement("small");
      plain.className = "swath-authoring-plain";
      plain.id = `swath-authoring-${key}-composite-note`;
      plain.textContent =
        resultKind === "udf"
          ? "Your module's output renders directly — 1 value per pixel as gray, 3 as red, " +
            "green, and blue — so a colormap does not apply here."
          : this.#loadBands.length === 3
            ? "The three loaded bands become the picture's red, green, and blue. " +
              "A colormap maps one gray value per pixel, so it does not apply here."
            : "A colormap maps one gray value per pixel — add NDVI or a formula to " +
              "combine the bands into one value first.";
      item.append(plain);
    }
    if (advanced.length > 0) {
      const open =
        card.advanced ||
        advanced.some(
          (param) =>
            this.#serverNotes.has(`${key}-${param.name}`) || this.#fieldIssue(card, param) !== "",
        );
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "swath-authoring-advanced-toggle";
      toggle.textContent = `advanced (${advanced.length})`;
      toggle.setAttribute("aria-expanded", String(open));
      toggle.addEventListener("click", () => {
        card.advanced = !card.advanced;
        this.#render();
      });
      item.append(toggle);
      if (open) {
        for (const param of advanced) {
          item.append(...this.#renderField(card, key, param));
        }
      }
    }
    return item;
  }

  /** One generated field: the widget the parameter's schema (and the
   * vocabulary) calls for, its plain-language one-liner, and its inline
   * validation note. */
  #renderField(card: Card, key: string, param: ProcessParameter): Element[] {
    const fieldKey = `${key}-${param.name}`;
    const fieldId = `swath-authoring-${fieldKey}`;

    const label = document.createElement("label");
    label.htmlFor = fieldId;
    label.textContent = param.optional === true ? `${param.name} (optional)` : param.name;
    if (param.description !== undefined) {
      label.title = param.description;
    }
    // The plain-language one-liner, visible by default (the served
    // description stays available as the tooltip above).
    const help = fieldHelp(card.process.id, param.name);
    if (help !== "") {
      const line = document.createElement("small");
      line.className = "swath-authoring-field-help";
      line.textContent = help;
      label.append(line);
    }

    const touch = (): void => {
      this.#touched.add(fieldKey);
      this.#serverNotes.delete(fieldKey);
      this.#serverNotes.delete(key);
      this.#updateValidity();
    };

    if (card === this.#loadCard && isBandArray(param.schema)) {
      label.htmlFor = "";
      label.append(this.#renderBandChecks(fieldId, touch));
    } else if (card === this.#loadCard && hasSubtype(param.schema, "temporal-interval")) {
      label.htmlFor = "";
      label.append(this.#renderWhenControl(fieldId, card, param.name, touch));
    } else {
      label.append(this.#renderValueInput(card, key, param, fieldId, touch));
    }

    const note = document.createElement("small");
    note.className = "swath-authoring-field-note";
    note.id = `${fieldId}-note`;
    label.append(note);
    return [label];
  }

  /** The Load card's bands: one checkbox per band of the CHOSEN
   * collection — the vocabulary hint of #148 promoted into the widget
   * itself, so an unknown band is unconstructible (B7). Tick order is
   * loaded order. */
  #renderBandChecks(fieldId: string, touch: () => void): Element {
    const group = document.createElement("span");
    group.className = "swath-authoring-bands";
    group.id = fieldId;
    group.setAttribute("role", "group");
    group.setAttribute("aria-label", "Bands to load");
    const vocabulary = this.#collectionBands();
    if (vocabulary.length === 0) {
      const hint = document.createElement("small");
      hint.className = "swath-authoring-plain";
      hint.textContent = "Choose a dataset first — its bands appear here.";
      group.append(hint);
      return group;
    }
    for (const band of vocabulary) {
      const wrap = document.createElement("label");
      const box = document.createElement("input");
      box.type = "checkbox";
      box.id = `${fieldId}-${band}`;
      box.dataset["band"] = band;
      box.checked = this.#loadBands.includes(band);
      box.addEventListener("change", () => {
        if (box.checked) {
          this.#loadBands.push(band);
        } else {
          this.#loadBands = this.#loadBands.filter((b) => b !== band);
        }
        touch();
        this.#render(); // the loaded-band vocabulary feeds other widgets
      });
      const text = document.createElement("span");
      text.textContent = band;
      wrap.append(box, text);
      group.append(wrap);
    }
    return group;
  }

  /** The Load card's plain-words "when" control (design note §4, the
   * Model B extent treatment; ADR 0015): two calendar pickers over the
   * `temporal-interval` subtype — schema-derived in mechanism, curated
   * in wording, like the collection picker. Empty on both sides means
   * "everything available"; the stored value is the interval JSON the
   * graph emits verbatim, so an impossible date string is
   * unconstructible from the UI. */
  #renderWhenControl(fieldId: string, card: Card, name: string, touch: () => void): Element {
    const group = document.createElement("span");
    group.className = "swath-authoring-when";
    group.id = fieldId;
    group.setAttribute("role", "group");
    group.setAttribute("aria-label", "Dates to show");
    const bounds = temporalBounds(card.values.get(name) ?? "");
    const picker = (slot: "from" | "until", labelText: string): Element => {
      const wrap = document.createElement("label");
      const text = document.createElement("span");
      text.textContent = labelText;
      const input = document.createElement("input");
      input.type = "date";
      input.id = `${fieldId}-${slot}`;
      input.value = slot === "from" ? bounds[0] : bounds[1];
      input.addEventListener("change", () => {
        if (slot === "from") {
          bounds[0] = input.value;
        } else {
          bounds[1] = input.value;
        }
        card.values.set(name, temporalValue(bounds[0], bounds[1]));
        touch();
        this.#render(); // the when line and the narrative follow along
      });
      wrap.append(text, input);
      return wrap;
    };
    group.append(picker("from", "from"), picker("until", "until (not included)"));
    return group;
  }

  /** The UDF card's module control (#208): a file picker plus a drop
   * zone over `.wasm` files. The picked module is base64-encoded
   * client-side into the `udf` value as a `data:` URL — after the size
   * check, so an over-bound module is refused in plain words (via
   * `#fieldIssue`) without ever encoding 8+ MiB. */
  #renderUdfUpload(card: Card, fieldId: string, touch: () => void): HTMLElement {
    const zone = document.createElement("span");
    zone.className = "swath-authoring-udf-drop";
    zone.id = `${fieldId}-drop`;
    zone.textContent = "Drop a .wasm module here, or pick one:";
    const picker = document.createElement("input");
    picker.type = "file";
    picker.id = fieldId;
    picker.accept = ".wasm,application/wasm";
    picker.setAttribute("aria-label", "Upload a .wasm module");
    const status = document.createElement("small");
    status.className = "swath-authoring-udf-module";
    status.id = `${fieldId}-module`;
    if (card.udf && !card.udf.refused) {
      status.textContent = `${card.udf.name} · ${formatMib(card.udf.size)}`;
    }
    const pick = (file: File): void => {
      void this.#pickUdfModule(card, file, touch);
    };
    picker.addEventListener("change", () => {
      const file = picker.files?.[0];
      if (file) {
        pick(file);
      }
    });
    zone.addEventListener("dragover", (event) => {
      event.preventDefault();
      zone.setAttribute("data-active", "");
    });
    zone.addEventListener("dragleave", () => {
      zone.removeAttribute("data-active");
    });
    zone.addEventListener("drop", (event) => {
      event.preventDefault();
      zone.removeAttribute("data-active");
      const file = event.dataTransfer?.files[0];
      if (file) {
        pick(file);
      }
    });
    zone.append(picker, status);
    return zone;
  }

  /** Reads the picked module into the card: refused (never encoded)
   * over the server's bound, else encoded into the `udf` value. The
   * card re-renders so the module line and the narrative follow. */
  async #pickUdfModule(card: Card, file: File, touch: () => void): Promise<void> {
    if (file.size > UDF_MAX_BYTES) {
      card.udf = { name: file.name, size: file.size, refused: true };
      card.values.delete("udf");
    } else {
      const bytes = new Uint8Array(await file.arrayBuffer());
      card.udf = { name: file.name, size: file.size, refused: false };
      card.values.set("udf", wasmDataUrl(bytes));
    }
    touch();
    if (this.isConnected) {
      this.#render();
    }
  }

  #renderValueInput(
    card: Card,
    key: string,
    param: ProcessParameter,
    fieldId: string,
    touch: () => void,
  ): HTMLElement {
    const stored = card.values.get(param.name) ?? "";
    const dropdown = (
      values: readonly string[],
      placeholder: string | undefined,
      rerenderOnChange: boolean,
    ): HTMLSelectElement => {
      const select = document.createElement("select");
      select.id = fieldId;
      if (placeholder !== undefined) {
        const none = document.createElement("option");
        none.value = "";
        none.textContent = placeholder;
        select.append(none);
      }
      for (const value of values) {
        const option = document.createElement("option");
        option.value = value;
        option.textContent = value;
        select.append(option);
      }
      select.value = values.includes(stored)
        ? stored
        : placeholder !== undefined
          ? ""
          : (values[0] ?? "");
      select.addEventListener("change", () => {
        card.values.set(param.name, select.value);
        touch();
        if (rerenderOnChange) {
          this.#render(); // e.g. a collection choice refreshes band widgets
        }
      });
      return select;
    };

    if (hasSubtype(param.schema, "collection-id")) {
      if (this.#collections.length > 0) {
        // The collection picker: served ids only — an unknown collection
        // cannot be submitted from here.
        const select = dropdown(
          this.#collections.map((collection) => collection.id),
          "(choose a collection)",
          false,
        );
        select.addEventListener("change", () => {
          // A new collection means a new band vocabulary: drop picks that
          // no longer exist, then re-render the dependent widgets.
          this.#loadBands = this.#loadBands.filter((band) =>
            this.#collectionBands().includes(band),
          );
          this.#render();
        });
        return select;
      }
      if (this.#catalogPending) {
        // The collections read rides behind the canvas (issue #255):
        // the card is ready, the picker fills in when the list lands.
        const select = dropdown([], "(loading collections…)", false);
        select.disabled = true;
        return select;
      }
      // Loaded empty (or failed, noted inline): fall through to the
      // free-text input — there is nothing to choose from.
    }
    if (hasSubtype(param.schema, "output-format-options")) {
      // The subtype-specialized widget: the Swath colormap select. On a
      // multi-band (composite) result it greys out with the reduce-first
      // note (B6) — and buildGraph omits it, so the server rejection is
      // unconstructible, not merely explained.
      const select = dropdown(COLORMAPS, "(default colormap)", false);
      const kind = this.#resultStage().kind;
      if (kind !== "gray") {
        select.disabled = true;
        select.title =
          kind === "udf"
            ? "A UDF's output renders directly (1 plane gray, 3 planes RGB) — a colormap " +
              "does not apply."
            : "A colormap maps one gray value per pixel — add NDVI or a formula to reduce " +
              "the bands to one value first.";
      }
      return select;
    }
    if (hasSubtype(param.schema, "output-format")) {
      // The profile's format vocabulary (B9): a select, no free text.
      return dropdown(FORMATS, undefined, false);
    }
    if (hasSubtype(param.schema, "udf-runtime")) {
      // ADR 0018: runtime "wasm", version "1", only — vocabulary selects,
      // so InvalidRuntime / InvalidVersion are unconstructible.
      return dropdown(UDF_RUNTIMES, undefined, false);
    }
    if (hasSubtype(param.schema, "udf-runtime-version")) {
      return dropdown(UDF_VERSIONS, undefined, false);
    }
    if (card.process.id === "run_udf" && param.name === "udf") {
      return this.#renderUdfUpload(card, fieldId, touch);
    }
    if (card.process.id === "run_udf" && param.name === "context") {
      // The module's settings: a small JSON field (validated as an
      // object by `contextIssue`; passed through verbatim).
      const area = document.createElement("textarea");
      area.id = fieldId;
      area.rows = 2;
      area.placeholder = "{}";
      area.value = stored;
      area.addEventListener("input", () => {
        card.values.set(param.name, area.value);
        touch();
      });
      return area;
    }
    if (isBandName(param.schema)) {
      // Band parameters are selects over the LOADED bands (B7): the
      // compiler resolves them against load_collection's band list.
      return dropdown(this.#loadBands, "(pick a band)", false);
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
      card.values.set(param.name, input.value);
      touch();
      if (card === this.#loadCard) {
        this.#updateExtentSummary(key);
      }
    });
    return input;
  }

  /** Keeps the Load card's plain area/time line honest while the expert
   * extent fields are edited (no re-render, no lost focus). */
  #updateExtentSummary(key: string): void {
    const card = this.#loadCard;
    const line = this.querySelector(`#swath-authoring-${key}-extent-summary`);
    if (!card || !line) {
      return;
    }
    const custom = (name: string): boolean => (card.values.get(name) ?? "").trim() !== "";
    line.textContent = `Area: ${
      custom("spatial_extent") ? "custom (see advanced)" : "everywhere the collection covers"
    } · Time: ${custom("temporal_extent") ? "custom (see advanced)" : "everything available"}`;
  }

  // --- The formula builder (reduce_dimension's reducer child graph) ---

  /** The formula card: lines of `left op right` over band selects,
   * numbers, and earlier lines — the only place arithmetic and (via
   * band operands) `array_element` exist, exactly where the compiler
   * admits them (B2/B3). `dimension` is pinned to "bands" (the profile's
   * only reducible dimension) and the reducer child graph is composed,
   * never typed. */
  #renderFormula(card: Card, key: string, item: Element): void {
    const explain = document.createElement("small");
    explain.className = "swath-authoring-plain";
    explain.textContent =
      "Each pixel's bands are combined into one value: build the calculation line by " +
      "line — the last line is the result.";
    item.append(explain);

    card.rows.forEach((row, index) => {
      item.append(this.#renderFormulaRow(card, key, row, index));
    });

    const add = document.createElement("button");
    add.type = "button";
    add.className = "swath-authoring-formula-add";
    add.textContent = "+ add a line";
    add.addEventListener("click", () => {
      // A new line starts from the previous line's result — the common
      // "and then…" shape needs no select fiddling.
      const left: Operand =
        card.rows.length > 0 ? { kind: "row", index: card.rows.length - 1 } : { ...EMPTY_OPERAND };
      card.rows.push({ op: "divide", left, right: { ...EMPTY_OPERAND } });
      this.#render();
    });
    item.append(add);

    const issues = document.createElement("small");
    issues.className = "swath-authoring-field-note";
    issues.id = `swath-authoring-${key}-formula-issues`;
    item.append(issues);
  }

  #renderFormulaRow(card: Card, key: string, row: FormulaRow, index: number): Element {
    const wrap = document.createElement("p");
    wrap.className = "swath-authoring-formula-row";
    const lineNo = document.createElement("span");
    lineNo.className = "swath-authoring-formula-line";
    lineNo.textContent = `line ${index + 1}${index === card.rows.length - 1 ? " =" : ""}`;
    wrap.append(lineNo);

    const touch = (): void => {
      this.#serverNotes.delete(key);
      this.#updateValidity();
    };

    wrap.append(this.#renderOperand(card, key, row, index, "left", touch));

    const op = document.createElement("select");
    op.className = "swath-authoring-formula-op";
    op.id = `swath-authoring-${key}-row${index + 1}-op`;
    op.setAttribute("aria-label", `line ${index + 1} operation`);
    // Only SERVED arithmetic is offered — delete `divide` from the
    // served definitions and it disappears here too (schema honesty).
    const served = new Set(this.#processes.map((process) => process.id));
    const symbols: Record<FormulaOp, string> = {
      add: "+",
      subtract: "−",
      multiply: "×",
      divide: "÷",
    };
    for (const candidate of FORMULA_OPS) {
      if (!served.has(candidate)) {
        continue;
      }
      const option = document.createElement("option");
      option.value = candidate;
      option.textContent = symbols[candidate];
      option.title = candidate;
      op.append(option);
    }
    op.value = row.op;
    if (op.value !== row.op) {
      // The stored op is no longer served; fall back to the first.
      row.op = (op.value as FormulaOp) || "add";
    }
    op.addEventListener("change", () => {
      row.op = op.value as FormulaOp;
      touch();
    });
    wrap.append(op);

    wrap.append(this.#renderOperand(card, key, row, index, "right", touch));

    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "✕";
    remove.setAttribute("aria-label", `Remove line ${index + 1} of ${key}`);
    remove.addEventListener("click", () => {
      card.rows.splice(index, 1);
      // Later lines referencing at or past the removed one reset (the
      // issues list explains); references above it just shift down.
      for (const later of card.rows) {
        for (const side of ["left", "right"] as const) {
          const operand = later[side];
          if (operand.kind === "row" && operand.index >= index) {
            later[side] =
              operand.index > index
                ? { kind: "row", index: operand.index - 1 }
                : { ...EMPTY_OPERAND };
          }
        }
      }
      this.#render();
    });
    wrap.append(remove);
    return wrap;
  }

  /** One operand cell: a select over the loaded bands, earlier lines,
   * and "a number…" (which reveals a number input beside it). */
  #renderOperand(
    card: Card,
    key: string,
    row: FormulaRow,
    index: number,
    side: "left" | "right",
    touch: () => void,
  ): Element {
    const holder = document.createElement("span");
    holder.style.display = "contents";
    const operand = row[side];
    const select = document.createElement("select");
    select.id = `swath-authoring-${key}-row${index + 1}-${side}`;
    select.setAttribute("aria-label", `line ${index + 1} ${side} value`);

    const none = document.createElement("option");
    none.value = "";
    none.textContent = "(pick)";
    select.append(none);
    for (const band of this.#loadBands) {
      const option = document.createElement("option");
      option.value = `band:${band}`;
      option.textContent = band;
      select.append(option);
    }
    for (let earlier = 0; earlier < index; earlier += 1) {
      const option = document.createElement("option");
      option.value = `row:${earlier}`;
      option.textContent = `line ${earlier + 1}`;
      select.append(option);
    }
    const numberOption = document.createElement("option");
    numberOption.value = "number";
    numberOption.textContent = "a number…";
    select.append(numberOption);

    const number = document.createElement("input");
    number.type = "number";
    number.step = "any";
    number.id = `swath-authoring-${key}-row${index + 1}-${side}-number`;
    number.setAttribute("aria-label", `line ${index + 1} ${side} number`);
    number.style.display = "none";

    const reflect = (current: Operand): void => {
      switch (current.kind) {
        case "band":
          select.value = current.band === "" ? "" : `band:${current.band}`;
          if (select.value !== `band:${current.band}` && current.band !== "") {
            select.value = ""; // the band is no longer loaded; the issues list explains
          }
          number.style.display = "none";
          break;
        case "number":
          select.value = "number";
          number.style.display = "";
          number.value = current.text;
          break;
        case "row":
          select.value = `row:${current.index}`;
          number.style.display = "none";
          break;
      }
    };
    reflect(operand);

    select.addEventListener("change", () => {
      const value = select.value;
      if (value === "number") {
        row[side] = { kind: "number", text: number.value };
        number.style.display = "";
      } else if (value.startsWith("band:")) {
        row[side] = { kind: "band", band: value.slice("band:".length) };
        number.style.display = "none";
      } else if (value.startsWith("row:")) {
        row[side] = { kind: "row", index: Number(value.slice("row:".length)) };
        number.style.display = "none";
      } else {
        row[side] = { ...EMPTY_OPERAND };
        number.style.display = "none";
      }
      touch();
    });
    number.addEventListener("input", () => {
      row[side] = { kind: "number", text: number.value };
      touch();
    });

    holder.append(select, number);
    return holder;
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

/** A preview failure in the user's words: the server's budget refusal
 * (`ProcessGraphComplexity`, ADR 0014) says what to do about it — and
 * that publishing is unaffected; anything else falls back to the
 * standardized error line. (A UDF's own budget refusal is the module's
 * fault, not the area's — `udfDiagnostic` claims it first, on the
 * module field.) */
function previewFailureNote(failure: { code: string; message: string }): string {
  if (
    failure.code === "ProcessGraphComplexity" &&
    udfDiagnostic(failure.code, failure.message) === undefined
  ) {
    return (
      "This draft covers too much data to preview at once — narrow the area " +
      "(Load imagery → advanced), or publish and look at the map itself."
    );
  }
  return `The preview failed: ${failure.code}: ${failure.message}`;
}

/** The standardized openEO error body (`{code, message}`); a non-openEO
 * body reads as an `HttpError` carrying the status line. */
async function readOpenEoBody(response: Response): Promise<{ code: string; message: string }> {
  const problem = await ApiProblem.from(response);
  if (problem.title !== "" && problem.detail !== "") {
    return { code: problem.title, message: problem.detail };
  }
  return { code: "HttpError", message: `request failed with HTTP ${response.status}` };
}

/** [`readOpenEoBody`] rendered as one line. */
async function readOpenEoError(response: Response): Promise<string> {
  const { code, message } = await readOpenEoBody(response);
  return code === "HttpError" ? message : `${code}: ${message}`;
}

/** Registers `<swath-authoring-panel>`; safe to call more than once. */
export function defineSwathAuthoringPanel(): void {
  if (!customElements.get(SwathAuthoringPanel.tagName)) {
    customElements.define(SwathAuthoringPanel.tagName, SwathAuthoringPanel);
  }
}
