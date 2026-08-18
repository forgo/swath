// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-add-data-panel>` — the dataset-creation API's face (issue #197):
 * paste a link to a COG or a STAC Item (or, in local mode, drop a file) →
 * the #196 registration flow → a quick-look layer authored through the
 * openEO services surface → serving, traced, on the map. Everything goes
 * through the engine; the panel never decodes a pixel (ADR 0019).
 *
 * Plain Custom Element, light DOM, no framework (ADR 0005). Lazy like the
 * dataset browser: collapsed by default, no requests until opened. The
 * first open fetches `GET /` once — the capabilities document (#198)
 * decides what renders: no `POST /datasets` advertised (read-only, or a
 * catalog-less server) renders a plain "viewing only" note instead of the
 * form, and the file drop appears only where `PUT /uploads/{filename}` is
 * mounted. Capabilities-driven, never probed or hardcoded.
 *
 * A `stac` attribute (the `/?stac=<item-url>` deep link) opens the panel
 * on load, pre-fills the link, and fetches the item — the user still
 * reviews and clicks Add; nothing registers on page load.
 *
 * Events (bubbling):
 * - `swath-data-added` `{ dataset, layer }` — the quick-look service is
 *   published; the shell switches the map onto it.
 *
 * Attributes: `server` (API base URL, default same origin), `stac` (the
 * deep-linked STAC Item URL).
 */

import {
  type AddDataCapabilities,
  type AddDataDraft,
  bandIssue,
  brightestIssue,
  classifyLink,
  cogDraft,
  datasetBody,
  datasetIdIssue,
  datetimeIssue,
  granuleBody,
  granuleIdIssue,
  mapProblem,
  type ProblemField,
  parseCapabilities,
  quicklookService,
  stacDraft,
} from "./add-data-model.js";

/** The read-only state, exported so tests assert the exact contract
 * string (the #198 capabilities document drives it). */
export const READ_ONLY_NOTE =
  "This server is read-only: data can be viewed here but not added. " +
  "Serve without --read-only to register datasets.";

/** Shown while nothing is pasted yet — the panel's one-line pitch. */
export const LINK_HELP =
  "Paste a link to a cloud-optimized GeoTIFF, or to a STAC item (.json). " +
  "Swath reads files where they live — nothing is copied.";

const STYLE_ELEMENT_ID = "swath-add-data-panel-styles";

/** Dark-telemetry skin matching the rail's other panels; layout belongs
 * to the page. */
const PANEL_CSS = `
swath-add-data-panel { display: block; }
swath-add-data-panel .swath-add-data-toggle {
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
swath-add-data-panel .swath-add-data-toggle::before { content: "▸ "; }
swath-add-data-panel .swath-add-data-toggle[aria-expanded="true"]::before { content: "▾ "; }
swath-add-data-panel .swath-add-data-toggle:focus-visible {
  outline: 2px solid #4ade80;
  outline-offset: 1px;
}
swath-add-data-panel form {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
swath-add-data-panel label {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font: 600 12px/1.4 system-ui, sans-serif;
}
swath-add-data-panel input {
  padding: 5px 8px;
  border: 1px solid rgb(148 163 184 / 30%);
  border-radius: 4px;
  background: rgb(15 23 42 / 60%);
  color: inherit;
  font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
swath-add-data-panel input:focus-visible {
  outline: 2px solid #4ade80;
  outline-offset: 1px;
}
swath-add-data-panel .swath-add-data-help {
  font: 11px/1.5 system-ui, sans-serif;
  font-weight: 400;
  color: rgb(148 163 184 / 80%);
}
swath-add-data-panel .swath-add-data-note {
  font: 11px/1.5 system-ui, sans-serif;
  font-weight: 400;
  color: #fca5a5;
}
swath-add-data-panel .swath-add-data-drop {
  padding: 10px;
  border: 1px dashed rgb(148 163 184 / 30%);
  border-radius: 6px;
  text-align: center;
  font: 12px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 80%);
}
swath-add-data-panel .swath-add-data-drop[data-active] {
  border-color: rgb(74 222 128 / 45%);
  background: rgb(74 222 128 / 10%);
}
swath-add-data-panel .swath-add-data-submit {
  padding: 6px 10px;
  border: 1px solid rgb(74 222 128 / 45%);
  border-radius: 6px;
  background: rgb(74 222 128 / 10%);
  color: inherit;
  font: 600 12px/1.4 system-ui, sans-serif;
  cursor: pointer;
}
swath-add-data-panel .swath-add-data-submit:disabled {
  border-color: rgb(148 163 184 / 20%);
  background: none;
  color: rgb(148 163 184 / 75%);
  cursor: default;
}
swath-add-data-panel .swath-add-data-submit:focus-visible {
  outline: 2px solid #4ade80;
  outline-offset: 1px;
}
swath-add-data-panel .swath-add-data-reason,
swath-add-data-panel .swath-add-data-status,
swath-add-data-panel .swath-add-data-error,
swath-add-data-panel .swath-add-data-readonly {
  margin: 0;
  font: 12px/1.5 system-ui, sans-serif;
  color: rgb(148 163 184 / 80%);
}
swath-add-data-panel .swath-add-data-error { color: #fca5a5; }
swath-add-data-panel .swath-add-data-status { color: #4ade80; }
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

/** One editable field of the draft form. */
interface Field {
  key: ProblemField | "brightest" | "title";
  label: string;
  help: string;
  get: (panel: SwathAddDataPanel) => string;
  set: (panel: SwathAddDataPanel, value: string) => void;
  issue: (value: string) => string;
  /** Renders only for the direct (COG) form. */
  cogOnly?: boolean;
}

export class SwathAddDataPanel extends HTMLElement {
  static readonly tagName = "swath-add-data-panel";

  #open = false;
  /** undefined until the first open's capabilities fetch settles. */
  #capabilities: AddDataCapabilities | undefined;
  #capabilitiesError: string | undefined;
  #link = "";
  #linkNote = "";
  #draft: AddDataDraft | undefined;
  /** Editable overlay onto the draft. */
  #brightest = "10000";
  #inspecting = false;
  #submitting = false;
  #uploading = false;
  #flowError = "";
  #status = "";
  #serverNotes = new Map<string, string>();
  #touched = new Set<string>();
  /** Settles when the last user-triggered flow has rendered (never
   * rejects — failures render as notes). Test seam. */
  #ready: Promise<void> = Promise.resolve();

  /** Test seam: assign a stub BEFORE the element connects. */
  fetchImpl: typeof fetch | undefined;

  /** Base URL of the Swath API (no trailing slash); same origin when the
   * `server` attribute is absent. */
  get server(): string {
    return (this.getAttribute("server") ?? "").replace(/\/+$/, "");
  }

  /** `await el.ready` before inspecting the DOM after an interaction. */
  get ready(): Promise<void> {
    return this.#ready;
  }

  connectedCallback(): void {
    injectStyles(this.ownerDocument);
    this.setAttribute("role", "group");
    if (!this.hasAttribute("aria-label")) {
      this.setAttribute("aria-label", "Add data");
    }
    const stac = this.getAttribute("stac");
    if (stac !== null && stac !== "") {
      // The deep link pre-fills and fetches — registering stays a click.
      this.#open = true;
      this.#link = stac;
      this.#ready = this.#ensureCapabilities().then(() => this.#inspect());
    }
    this.#render();
  }

  #fetch(input: string, init?: RequestInit): Promise<Response> {
    const call = this.fetchImpl ?? fetch;
    return call(input, init);
  }

  #api(path: string, init?: RequestInit): Promise<Response> {
    return this.#fetch(`${this.server}${path}`, init);
  }

  #togglePanel(): void {
    this.#open = !this.#open;
    if (this.#open && this.#capabilities === undefined) {
      this.#ready = this.#ensureCapabilities();
    }
    this.#render();
  }

  /** One capabilities fetch per panel life: `GET /` (JSON — the browser
   * negotiation needs an explicit text/html, which this is not). */
  async #ensureCapabilities(): Promise<void> {
    if (this.#capabilities !== undefined) {
      return;
    }
    try {
      const response = await this.#api("/", { headers: { accept: "application/json" } });
      if (!response.ok) {
        throw new Error(`GET ${this.server}/ failed: ${response.status}`);
      }
      this.#capabilities = parseCapabilities(await response.json());
    } catch (error) {
      this.#capabilitiesError = error instanceof Error ? error.message : String(error);
    }
    this.#render();
  }

  /** Resolves the pasted link into a draft: a `.json` link is fetched
   * in-browser (the server never fetches URLs — no SSRF surface); any
   * other link is handed to the server as the asset reference. */
  async #inspect(): Promise<void> {
    const link = this.#link.trim();
    this.#draft = undefined;
    this.#flowError = "";
    this.#status = "";
    this.#serverNotes.clear();
    this.#touched.clear();
    if (link === "") {
      this.#linkNote = "paste a link first";
      this.#render();
      return;
    }
    this.#linkNote = "";
    if (classifyLink(link) === "cog") {
      this.#draft = cogDraft(link);
      this.#render();
      return;
    }
    this.#inspecting = true;
    this.#render();
    let draft: AddDataDraft | string;
    try {
      const response = await this.#fetch(link, { headers: { accept: "application/json" } });
      if (!response.ok) {
        throw new Error(`the link answered HTTP ${response.status}`);
      }
      draft = stacDraft(await response.json());
    } catch (error) {
      draft = `could not read the item: ${error instanceof Error ? error.message : String(error)}`;
    }
    this.#inspecting = false;
    if (typeof draft === "string") {
      this.#linkNote = draft;
    } else {
      this.#draft = draft;
    }
    this.#render();
  }

  /** Local-mode file drop: upload into the serving store, then continue
   * as a pasted link to the returned store key. */
  async #uploadFile(file: File): Promise<void> {
    const name = file.name.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^[.-]+/, "");
    if (name === "") {
      this.#linkNote = "that file has no usable name";
      this.#render();
      return;
    }
    this.#uploading = true;
    this.#flowError = "";
    this.#status = "";
    this.#render();
    try {
      const response = await this.#api(`/uploads/${encodeURIComponent(name)}`, {
        method: "PUT",
        body: file,
      });
      if (!response.ok) {
        const problem = mapProblem(response.status, await response.json().catch(() => undefined));
        throw new Error(problem.note);
      }
      const body = (await response.json()) as { href?: string };
      if (typeof body.href !== "string") {
        throw new Error("the server answered without an href");
      }
      this.#uploading = false;
      this.#link = body.href;
      await this.#inspect();
      return;
    } catch (error) {
      this.#uploading = false;
      this.#linkNote = `upload failed: ${error instanceof Error ? error.message : String(error)}`;
    }
    this.#render();
  }

  /** The whole flow: register the dataset (409 = already there — fine),
   * register the granule, publish the quick look, announce the layer. */
  async #register(): Promise<void> {
    const draft = this.#draft;
    if (draft === undefined || this.#issues().length > 0) {
      return;
    }
    this.#submitting = true;
    this.#flowError = "";
    this.#status = "";
    this.#serverNotes.clear();
    this.#render();
    try {
      const json = { "content-type": "application/json" };
      const created = await this.#api("/datasets", {
        method: "POST",
        headers: json,
        body: JSON.stringify(datasetBody(draft)),
      });
      if (!created.ok && created.status !== 409) {
        // 409 means the dataset exists — adding to it is the point.
        this.#reportProblem(created.status, await created.json().catch(() => undefined));
        return;
      }
      const granule = await this.#api(`/datasets/${encodeURIComponent(draft.datasetId)}/granules`, {
        method: "POST",
        headers: json,
        body: JSON.stringify(granuleBody(draft)),
      });
      if (!granule.ok) {
        this.#reportProblem(granule.status, await granule.json().catch(() => undefined));
        return;
      }
      const service = await this.#api("/services", {
        method: "POST",
        headers: json,
        body: JSON.stringify(
          quicklookService(draft.datasetId, draft.title, draft.bands, Number(this.#brightest)),
        ),
      });
      if (!service.ok) {
        const body = (await service.json().catch(() => undefined)) as
          | { code?: unknown; message?: unknown }
          | undefined;
        this.#flowError =
          typeof body?.code === "string" && typeof body?.message === "string"
            ? `registered, but the quick look failed — ${body.code}: ${body.message}`
            : `registered, but the quick look failed with HTTP ${service.status}`;
        return;
      }
      const layer = service.headers.get("openeo-identifier") ?? "";
      this.#status = `Serving: ${draft.datasetId} is registered and its quick look is on the map.`;
      this.dispatchEvent(
        new CustomEvent("swath-data-added", {
          detail: { dataset: draft.datasetId, layer },
          bubbles: true,
        }),
      );
    } catch (error) {
      this.#flowError = `registration failed: ${
        error instanceof Error ? error.message : String(error)
      }`;
    } finally {
      this.#submitting = false;
      this.#render();
    }
  }

  #reportProblem(status: number, body: unknown): void {
    const problem = mapProblem(status, body);
    if (problem.field === "") {
      this.#flowError = problem.note;
    } else if (problem.field === "link") {
      this.#linkNote = problem.note;
    } else {
      this.#serverNotes.set(problem.field, problem.note);
    }
  }

  // --- Validation ---

  #fields(): Field[] {
    const draftField = (
      key: Field["key"],
      label: string,
      help: string,
      get: (d: AddDataDraft) => string,
      set: (d: AddDataDraft, v: string) => void,
      issue: (v: string) => string,
      cogOnly = false,
    ): Field => ({
      key,
      label,
      help,
      get: (panel) => (panel.#draft === undefined ? "" : get(panel.#draft)),
      set: (panel, value) => {
        if (panel.#draft !== undefined) {
          set(panel.#draft, value);
        }
      },
      issue,
      cogOnly,
    });
    return [
      draftField(
        "dataset",
        "Dataset id",
        "Groups files that belong together; new id = new dataset, existing id adds to it.",
        (d) => d.datasetId,
        (d, v) => {
          d.datasetId = v;
        },
        datasetIdIssue,
      ),
      draftField(
        "title",
        "Title",
        "How the dataset reads in lists and on the map.",
        (d) => d.title,
        (d, v) => {
          d.title = v;
        },
        () => "",
      ),
      draftField(
        "band",
        "Band name",
        "What this file measures — one word, like red or b04.",
        (d) => d.bands[0] ?? "",
        (d, v) => {
          d.bands = [v];
        },
        bandIssue,
        true,
      ),
      draftField(
        "granule",
        "Granule id",
        "Names this one acquisition within the dataset.",
        (d) => d.granuleId,
        (d, v) => {
          d.granuleId = v;
        },
        granuleIdIssue,
        true,
      ),
      draftField(
        "datetime",
        "Acquired at",
        "When the scene was captured, like 2024-06-06T17:54:00Z.",
        (d) => d.datetime,
        (d, v) => {
          d.datetime = v;
        },
        datetimeIssue,
        true,
      ),
      {
        key: "brightest",
        label: "Brightest value",
        help: "The pixel value shown at full brightness — 10000 fits HLS reflectance.",
        get: (panel) => panel.#brightest,
        set: (panel, value) => {
          panel.#brightest = value;
        },
        issue: brightestIssue,
      },
    ];
  }

  #issues(): string[] {
    const draft = this.#draft;
    if (draft === undefined) {
      return ["paste a link first"];
    }
    const issues: string[] = [];
    for (const field of this.#fields()) {
      if (field.cogOnly === true && draft.kind !== "cog") {
        continue;
      }
      const issue = field.issue(field.get(this));
      if (issue !== "") {
        issues.push(`${field.label.toLowerCase()}: ${issue}`);
      }
    }
    return issues;
  }

  // --- Rendering ---

  #render(): void {
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "swath-add-data-toggle";
    toggle.textContent = "Add data";
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
    if (this.#capabilitiesError !== undefined) {
      return [
        this.#note(
          "swath-add-data-error",
          `Cannot reach the server: ${this.#capabilitiesError}. Close and re-open to retry.`,
        ),
      ];
    }
    if (this.#capabilities === undefined) {
      return [this.#note("swath-add-data-status", "Checking what this server allows…")];
    }
    if (!this.#capabilities.register) {
      // Capabilities-driven absence (#198): no write surface, no form.
      return [this.#note("swath-add-data-readonly", READ_ONLY_NOTE)];
    }
    const form = document.createElement("form");
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      this.#ready = this.#register();
    });
    form.append(this.#linkRow());
    if (this.#capabilities.upload) {
      form.append(this.#dropZone());
    }
    if (this.#inspecting) {
      form.append(this.#note("swath-add-data-status", "Reading the item…"));
    }
    if (this.#uploading) {
      form.append(this.#note("swath-add-data-status", "Uploading…"));
    }
    if (this.#draft !== undefined) {
      for (const field of this.#fields()) {
        if (field.cogOnly === true && this.#draft.kind !== "cog") {
          continue;
        }
        form.append(this.#fieldRow(field));
      }
      form.append(...this.#submitRow());
    }
    if (this.#flowError !== "") {
      form.append(this.#note("swath-add-data-error", this.#flowError));
    }
    if (this.#status !== "") {
      form.append(this.#note("swath-add-data-status", this.#status));
    }
    return [form];
  }

  #linkRow(): HTMLLabelElement {
    const label = document.createElement("label");
    const caption = document.createElement("span");
    caption.textContent = "Link to data";
    const input = document.createElement("input");
    input.id = "swath-add-data-link";
    input.type = "text";
    input.placeholder = "https://…/scene-b04.tif or …/item.json";
    input.value = this.#link;
    input.addEventListener("input", () => {
      this.#link = input.value;
    });
    input.addEventListener("change", () => {
      this.#ready = this.#inspect();
    });
    const help = document.createElement("small");
    help.className = "swath-add-data-help";
    help.textContent = LINK_HELP;
    const note = document.createElement("small");
    note.className = "swath-add-data-note";
    note.id = "swath-add-data-link-note";
    note.textContent = this.#linkNote;
    label.append(caption, input, help, note);
    return label;
  }

  #dropZone(): HTMLElement {
    const zone = document.createElement("div");
    zone.className = "swath-add-data-drop";
    zone.textContent = "…or drop a file here to upload it into the server's store";
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
        this.#ready = this.#uploadFile(file);
      }
    });
    const picker = document.createElement("input");
    picker.id = "swath-add-data-file";
    picker.type = "file";
    picker.setAttribute("aria-label", "Upload a file");
    picker.addEventListener("change", () => {
      const file = picker.files?.[0];
      if (file) {
        this.#ready = this.#uploadFile(file);
      }
    });
    zone.append(document.createElement("br"), picker);
    return zone;
  }

  #fieldRow(field: Field): HTMLLabelElement {
    const label = document.createElement("label");
    const caption = document.createElement("span");
    caption.textContent = field.label;
    const input = document.createElement("input");
    input.id = `swath-add-data-${field.key}`;
    input.type = "text";
    input.value = field.get(this);
    input.addEventListener("input", () => {
      field.set(this, input.value);
      this.#touched.add(field.key);
      this.#serverNotes.delete(field.key);
      this.#updateValidity();
    });
    const help = document.createElement("small");
    help.className = "swath-add-data-help";
    help.textContent = field.help;
    const note = document.createElement("small");
    note.className = "swath-add-data-note";
    note.id = `swath-add-data-${field.key}-note`;
    note.textContent = this.#fieldNote(field);
    label.append(caption, input, help, note);
    return label;
  }

  /** Server note first; then the local issue — but an untouched empty
   * field stays quiet (a fresh form is not a wall of red). */
  #fieldNote(field: Field): string {
    const server = this.#serverNotes.get(field.key);
    if (server !== undefined) {
      return server;
    }
    const value = field.get(this);
    const issue = field.issue(value);
    return this.#touched.has(field.key) || value.trim() !== "" ? issue : "";
  }

  #submitRow(): HTMLElement[] {
    const issues = this.#issues();
    const submit = document.createElement("button");
    submit.type = "submit";
    submit.className = "swath-add-data-submit";
    submit.textContent = this.#submitting ? "Registering…" : "Add to Swath";
    submit.disabled = issues.length > 0 || this.#submitting;
    const reason = document.createElement("p");
    reason.className = "swath-add-data-reason";
    reason.id = "swath-add-data-reason";
    reason.textContent = issues.length > 0 ? `To add: ${issues.join("; ")}.` : "";
    return [submit, reason];
  }

  /** Patches notes and the submit gate in place — no re-render, no lost
   * focus. */
  #updateValidity(): void {
    for (const field of this.#fields()) {
      const note = this.querySelector(`#swath-add-data-${field.key}-note`);
      if (note !== null) {
        note.textContent = this.#fieldNote(field);
      }
    }
    const issues = this.#issues();
    const submit = this.querySelector<HTMLButtonElement>(".swath-add-data-submit");
    if (submit !== null) {
      submit.disabled = issues.length > 0 || this.#submitting;
    }
    const reason = this.querySelector("#swath-add-data-reason");
    if (reason !== null) {
      reason.textContent = issues.length > 0 ? `To add: ${issues.join("; ")}.` : "";
    }
  }

  #note(className: string, text: string): HTMLParagraphElement {
    const note = document.createElement("p");
    note.className = className;
    note.textContent = text;
    return note;
  }
}

/** Registers `<swath-add-data-panel>`; safe to call more than once. */
export function defineSwathAddDataPanel(): void {
  if (!customElements.get(SwathAddDataPanel.tagName)) {
    customElements.define(SwathAddDataPanel.tagName, SwathAddDataPanel);
  }
}
