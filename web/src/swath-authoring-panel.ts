// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-authoring-panel>` — the openEO authoring panel (issue #109,
 * ADR 0010): compose a process graph as a linear pipeline of steps,
 * publish it as an XYZ secondary service, delete published services.
 *
 * Plain Custom Element, light DOM, no framework (ADR 0005). The panel is
 * a pure client of the openEO surface — it invents nothing:
 *
 * - The **palette** (which processes can be added) and every **form
 *   field** are generated from the parameter schemas `GET /processes`
 *   serves. There are no hand-maintained per-process form definitions:
 *   delete a process from the served definitions and its palette entry
 *   and form disappear (a vitest pins exactly that).
 * - Field widgets are chosen by schema shape alone: number → number
 *   input, array-of-strings → comma-separated text, everything else a
 *   text input parsed as JSON when it looks like JSON. One subtype gets
 *   a dedicated widget: `output-format-options` renders as the Swath
 *   colormap `<select>` (grayscale/viridis/magma/rdylgn — the render
 *   profile's vocabulary, keyed on the schema subtype, not on any
 *   process id).
 * - Data flow is a per-parameter **source** select: a parameter either
 *   holds a literal value or references an earlier step's output
 *   (`from_node`). Defaults are derived from the schemas too: raster-cube
 *   parameters — and the first required parameter of any step after the
 *   first — default to the previous step, so a straight pipeline needs
 *   no source fiddling.
 * - `POST /services` publishes; the standardized openEO error
 *   (`{code, message}`) from a rejected graph renders inline, verbatim.
 *   Success announces a bubbling `swath-service-created` (detail:
 *   `{id}`); the app shell switches the map, which refreshes the layer
 *   list — the authored layer appears in the browser with no reload.
 * - Published services are listed from `GET /services` with a delete
 *   button each; `DELETE /services/{id}` announces
 *   `swath-service-deleted` the same way.
 */

const STYLE_ELEMENT_ID = "swath-authoring-panel-styles";

/** The render profile's colormap vocabulary, offered for the
 * `output-format-options` schema subtype (`save_result`'s `options`).
 * This is a widget for a subtype — the only schema the panel narrows. */
const COLORMAPS = ["grayscale", "viridis", "magma", "rdylgn"] as const;

/** Panel chrome, matching the layer panel's dark-telemetry look. */
const PANEL_CSS = `
swath-authoring-panel { display: block; }
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
  margin: 0 0 6px;
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
swath-authoring-panel .swath-authoring-submit:hover { background: rgb(74 222 128 / 20%); }
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

/** One pipeline step: a chosen process plus the user's raw field state.
 * `sources` maps a parameter to the step key it references (`""` =
 * explicit literal, absent = derive the schema default); `values` holds
 * the literal inputs as typed, parsed only at build time. */
interface Step {
  key: string;
  process: ProcessDefinition;
  sources: Map<string, string>;
  values: Map<string, string>;
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

export class SwathAuthoringPanel extends HTMLElement {
  static readonly tagName = "swath-authoring-panel";

  #processes: readonly ProcessDefinition[] = [];
  #services: readonly ServiceItem[] = [];
  #steps: Step[] = [];
  #counter = 0;
  #title = "";
  #error = "";
  #unavailable = false;

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
    void this.#load();
  }

  /** Refetches the process definitions and the published-services list
   * (what `connectedCallback` does); exposed for hosts and tests. */
  async reload(): Promise<void> {
    await this.#load();
  }

  #fetch(path: string, init?: RequestInit): Promise<Response> {
    const call = this.fetchImpl ?? fetch;
    return call(`${this.server}${path}`, init);
  }

  async #load(): Promise<void> {
    try {
      const [processes, services] = await Promise.all([
        this.#fetch("/processes", { headers: { accept: "application/json" } }),
        this.#fetch("/services", { headers: { accept: "application/json" } }),
      ]);
      if (!processes.ok || !services.ok) {
        throw new Error(`openEO surface answered ${processes.status}/${services.status}`);
      }
      const processBody = (await processes.json()) as { processes?: ProcessDefinition[] };
      const serviceBody = (await services.json()) as { services?: ServiceItem[] };
      this.#processes = (processBody.processes ?? []).filter((p) => typeof p.id === "string");
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

  #addStep(process: ProcessDefinition): void {
    this.#counter += 1;
    this.#steps.push({
      key: `s${this.#counter}`,
      process,
      sources: new Map(),
      values: new Map(
        (process.parameters ?? [])
          .map((param): [string, string] => [param.name, defaultText(param)])
          .filter(([, text]) => text !== ""),
      ),
    });
    this.#render();
  }

  #removeStep(key: string): void {
    this.#steps = this.#steps.filter((step) => step.key !== key);
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
          // Required without a null alternative and left empty: omitted —
          // the server's inline error names exactly what is missing.
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

  async #publish(): Promise<void> {
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
        this.#error = await readOpenEoError(response);
        this.#render();
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
    const heading = document.createElement("h2");
    heading.className = "swath-authoring-heading";
    heading.textContent = "Author a layer";

    if (this.#unavailable) {
      const empty = document.createElement("p");
      empty.className = "swath-authoring-empty";
      empty.textContent = "The openEO authoring surface is not reachable.";
      this.replaceChildren(heading, empty);
      return;
    }
    if (this.#processes.length === 0) {
      const empty = document.createElement("p");
      empty.className = "swath-authoring-empty";
      empty.textContent = "Waiting for the server's process definitions…";
      this.replaceChildren(heading, empty);
      return;
    }

    const children: Element[] = [heading, this.#renderPalette(), this.#renderForm()];
    if (this.#error !== "") {
      const error = document.createElement("p");
      error.className = "swath-authoring-error";
      error.setAttribute("role", "alert");
      error.textContent = this.#error;
      children.push(error);
    }
    children.push(...this.#renderServices());
    this.replaceChildren(...children);
  }

  /** The palette: one add-button per served process definition. */
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
      hint.textContent = "Add process steps above; the last step is the result.";
      form.append(hint);
      return form;
    }

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

    form.append(titleLabel, submit);
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

    for (const param of step.process.parameters ?? []) {
      item.append(...this.#renderField(step, index, param));
    }
    return item;
  }

  /** One generated field: an optional source select (steps after the
   * first can wire any parameter to an earlier step's output) plus the
   * literal input the parameter's schema calls for. */
  #renderField(step: Step, index: number, param: ProcessParameter): Element[] {
    const fieldId = `swath-authoring-${step.key}-${param.name}`;
    const source = this.#effectiveSource(step, index, param);
    const elements: Element[] = [];

    const label = document.createElement("label");
    label.htmlFor = fieldId;
    label.textContent = param.optional === true ? `${param.name} (optional)` : param.name;
    if (param.description !== undefined) {
      label.title = param.description;
    }

    const input = this.#renderValueInput(step, param, fieldId);
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
      });
      label.append(select);
    }

    label.append(input);
    elements.push(label);
    return elements;
  }

  #renderValueInput(
    step: Step,
    param: ProcessParameter,
    fieldId: string,
  ): HTMLInputElement | HTMLSelectElement {
    const stored = step.values.get(param.name) ?? "";
    if (hasSubtype(param.schema, "output-format-options")) {
      // The one subtype-specialized widget: the Swath colormap select.
      const select = document.createElement("select");
      select.id = fieldId;
      const none = document.createElement("option");
      none.value = "";
      none.textContent = "(default colormap)";
      select.append(none);
      for (const colormap of COLORMAPS) {
        const option = document.createElement("option");
        option.value = colormap;
        option.textContent = colormap;
        select.append(option);
      }
      select.value = COLORMAPS.includes(stored as (typeof COLORMAPS)[number]) ? stored : "";
      select.addEventListener("change", () => {
        step.values.set(param.name, select.value);
      });
      return select;
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
