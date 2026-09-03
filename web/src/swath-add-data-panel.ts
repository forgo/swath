// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-add-data-panel>` — Data mode's "Add data" drawer (issue #289,
 * rebuilt on the primitives; issue #197 for the flow): paste a link to a
 * cloud-optimized GeoTIFF or a STAC item (or drop a file where uploads
 * are mounted), review the draft, register — the dataset, the granule,
 * then a quick-look service the shell puts on the map. Everything goes
 * through the engine (ADR 0019). Capabilities-driven (#198): the first
 * open reads `GET /` once through `SwathApi.capabilities()`; a read-only
 * server shows a note and no form. Server refusals route onto the field
 * that caused them (`mapProblem` → `swath-field error`). The `stac`
 * attribute (the `?stac=` deep link) opens pre-filled; registering stays
 * a click. Lazy by contract: a closed panel issues zero requests.
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
import { SwathApi } from "./api.js";
import { SwathButton } from "./ui/button.js";
import { el } from "./ui/dom.js";
import { SwathDrawer } from "./ui/drawer.js";
import { SwathElement } from "./ui/element.js";
import { SwathField } from "./ui/field.js";
import { css } from "./ui/styles.js";

export const READ_ONLY_NOTE =
  "This server is read-only: data can be viewed here but not added. " +
  "Serve without --read-only to register datasets.";

export const LINK_HELP =
  "Paste a link to a cloud-optimized GeoTIFF, or to a STAC item (.json). " +
  "Swath reads files where they live — nothing is copied.";

/** One form field: which draft value it edits, how it is explained, and
 * the client-side issue that blocks submission. */
interface Field {
  key: ProblemField | "brightest" | "title";
  label: string;
  help: string;
  get: (panel: SwathAddDataPanel) => string;
  set: (panel: SwathAddDataPanel, value: string) => void;
  issue: (value: string) => string;
  cogOnly?: boolean;
  readOnly?: boolean;
}

export class SwathAddDataPanel extends SwathElement {
  static override tagName = "swath-add-data-panel";
  static override styles = [
    css`
      :host { display: block; }
      [part="toggle"] { inline-size: 100%; }
      form {
        display: grid;
        gap: var(--swath-space-3);
        margin: 0;
      }
      .swath-add-data-drop {
        padding: var(--swath-space-3);
        border: var(--swath-border-hairline);
        border-style: dashed;
        border-radius: var(--swath-radius-sm);
        font-size: var(--swath-text-sm);
        color: var(--swath-color-fg-muted);
        text-align: center;
      }
      .swath-add-data-drop[data-active] {
        border-color: var(--swath-color-accent-border);
        background: var(--swath-color-accent-bg);
      }
      .swath-add-data-drop input { margin-block-start: var(--swath-space-2); font: inherit; color: inherit; }
      .swath-add-data-status, .swath-add-data-error, .swath-add-data-readonly, .swath-add-data-reason {
        margin: 0;
        font-size: var(--swath-text-sm);
        line-height: var(--swath-leading-normal);
        color: var(--swath-color-fg-muted);
      }
      .swath-add-data-status { color: var(--swath-color-accent); }
      .swath-add-data-error { color: var(--swath-color-danger); }
      .swath-add-data-reason { font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); }
      .swath-add-data-help { margin: 0; font-size: var(--swath-text-xs); color: var(--swath-color-fg-muted); }
      [slot="header"] {
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        font-weight: var(--swath-weight-label);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
      }
    `,
  ];
  static override properties = {
    open: { type: "boolean", reflect: true },
    /** The `?stac=` deep link: opens pre-filled, sends nothing. */
    stac: { type: "string" },
    server: { type: "string" },
  } as const;

  declare open: boolean;
  declare stac: string | undefined;
  declare server: string | undefined;

  #api: SwathApi | undefined;
  #capabilities: AddDataCapabilities | undefined;
  #capabilitiesError: string | undefined;
  #link = "";
  #linkNote = "";
  #draft: AddDataDraft | undefined;
  #brightest = "10000";
  #inspecting = false;
  #inspectToken = 0;
  #submitting = false;
  #uploading = false;
  #flowError = "";
  #status = "";
  #serverNotes = new Map<string, string>();
  #touched = new Set<string>();
  /** Settles when the last user-triggered flow has rendered (never
   * rejects — failures render as notes). Test seam. */
  #ready: Promise<void> = Promise.resolve();
  #started = false;

  constructor() {
    super();
    SwathButton.define();
    SwathDrawer.define();
    SwathField.define();
  }

  get api(): SwathApi {
    this.#api ??= new SwathApi({ base: this.server ?? "" });
    return this.#api;
  }

  set api(api: SwathApi) {
    this.#api = api;
  }

  /** `await el.ready` before inspecting the DOM after an interaction. */
  get ready(): Promise<void> {
    return this.#ready.then(() => this.updateComplete);
  }

  override connectedCallback(): void {
    super.connectedCallback();
    this.render(); // synchronous first paint: the toggle exists the moment the panel does
    this.setAttribute("role", "group");
    if (!this.hasAttribute("aria-label")) {
      this.setAttribute("aria-label", "Add data");
    }
    const stac = this.stac ?? "";
    if (stac !== "" && !this.#started) {
      this.#started = true;
      this.open = true;
      this.#link = stac;
      this.#ready = this.#ensureCapabilities().then(() => this.#inspect());
    }
  }

  #togglePanel(): void {
    this.open = !this.open;
    if (this.open && this.#capabilities === undefined) {
      this.#ready = this.#ensureCapabilities();
    }
  }

  /** One capabilities read per success, shared through `SwathApi`; a
   * failure clears the client's cache, so "close and re-open to retry" is
   * a real retry, never a bricked panel. */
  async #ensureCapabilities(): Promise<void> {
    if (this.#capabilities !== undefined) {
      return;
    }
    this.#capabilitiesError = undefined;
    try {
      this.#capabilities = parseCapabilities(await this.api.capabilities());
    } catch (error) {
      this.#capabilitiesError = error instanceof Error ? error.message : String(error);
    }
    this.requestUpdate();
  }

  async #inspect(): Promise<void> {
    const token = ++this.#inspectToken;
    // This run owns the spinner from here on — a superseded run's flag
    // must not outlive it (its early return below never clears state).
    this.#inspecting = false;
    const link = this.#link.trim();
    this.#draft = undefined;
    this.#flowError = "";
    this.#status = "";
    this.#serverNotes.clear();
    this.#touched.clear();
    if (link === "") {
      this.#linkNote = "paste a link first";
      this.requestUpdate();
      return;
    }
    this.#linkNote = "";
    if (classifyLink(link) === "cog") {
      this.#draft = cogDraft(link);
      this.requestUpdate();
      return;
    }
    this.#inspecting = true;
    this.requestUpdate();
    let draft: AddDataDraft | string;
    try {
      const response = await this.api.fetch(link, { headers: { accept: "application/json" } });
      if (!response.ok) {
        throw new Error(`the link answered HTTP ${response.status}`);
      }
      draft = stacDraft(await response.json());
    } catch (error) {
      draft = `could not read the item: ${error instanceof Error ? error.message : String(error)}`;
    }
    if (token !== this.#inspectToken) {
      return; // stale: the user pasted something newer while this fetched
    }
    this.#inspecting = false;
    if (typeof draft === "string") {
      this.#linkNote = draft;
    } else {
      this.#draft = draft;
    }
    this.requestUpdate();
  }

  /** Local-mode file drop: upload into the serving store, then continue
   * as a pasted link to the returned store key. */
  async #uploadFile(file: File): Promise<void> {
    const name = file.name.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^[.-]+/, "");
    if (name === "") {
      this.#linkNote = "that file has no usable name";
      this.requestUpdate();
      return;
    }
    this.#uploading = true;
    this.#flowError = "";
    this.#status = "";
    this.requestUpdate();
    try {
      const response = await this.api.fetch(`/uploads/${encodeURIComponent(name)}`, {
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
    this.requestUpdate();
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
    this.requestUpdate();
    try {
      const json = { "content-type": "application/json" };
      const created = await this.api.fetch("/datasets", {
        method: "POST",
        headers: json,
        body: JSON.stringify(datasetBody(draft)),
      });
      if (!created.ok && created.status !== 409) {
        // 409 means the dataset exists — adding to it is the point.
        this.#reportProblem(created.status, await created.json().catch(() => undefined));
        return;
      }
      const granule = await this.api.fetch(
        `/datasets/${encodeURIComponent(draft.datasetId)}/granules`,
        {
          method: "POST",
          headers: json,
          body: JSON.stringify(granuleBody(draft)),
        },
      );
      if (!granule.ok) {
        this.#reportProblem(granule.status, await granule.json().catch(() => undefined));
        return;
      }
      const service = await this.api.fetch("/services", {
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
      this.emit("swath-data-added", { dataset: draft.datasetId, layer });
    } catch (error) {
      this.#flowError = `registration failed: ${
        error instanceof Error ? error.message : String(error)
      }`;
    } finally {
      this.#submitting = false;
      this.requestUpdate();
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
    // A STAC item names its own collection, and registration must match
    // it (the server refuses a mismatch) — so the id is shown, not
    // editable, and the help says why in plain words.
    const stac = this.#draft?.kind === "stac";
    const datasetField = draftField(
      "dataset",
      "Dataset id",
      stac
        ? "Named by the item's collection — the granule registers into exactly this dataset."
        : "Groups files that belong together; new id = new dataset, existing id adds to it.",
      (d) => d.datasetId,
      (d, v) => {
        d.datasetId = v;
      },
      datasetIdIssue,
    );
    datasetField.readOnly = stac;
    return [
      datasetField,
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

  #fieldNote(field: Field): string {
    const server = this.#serverNotes.get(field.key);
    if (server !== undefined) {
      return server;
    }
    const value = field.get(this);
    const issue = field.issue(value);
    return this.#touched.has(field.key) || value.trim() !== "" ? issue : "";
  }

  #note(className: string, text: string): HTMLParagraphElement {
    return el("p", { class: className }, text);
  }

  #linkField(): SwathField {
    const field = el("swath-field", {
      id: "swath-add-data-link",
      name: "link",
      label: "Link to data",
      placeholder: "https://…/scene-b04.tif or …/item.json",
      value: this.#link,
    });
    field.append(el("span", { slot: "help" }, LINK_HELP));
    if (this.#linkNote !== "") {
      field.error = this.#linkNote;
    }
    field.addEventListener("swath-input", (event) => {
      event.stopPropagation();
      this.#link = String(event.detail.value);
    });
    field.addEventListener("swath-change", (event) => {
      event.stopPropagation();
      this.#link = String(event.detail.value);
      this.#ready = this.#inspect();
    });
    return field;
  }

  #dropZone(): HTMLElement {
    const zone = el(
      "div",
      { class: "swath-add-data-drop" },
      "…or drop a file here to upload it into the server's store",
    );
    zone.addEventListener("dragover", (event) => {
      event.preventDefault();
      zone.setAttribute("data-active", "");
    });
    zone.addEventListener("dragleave", () => zone.removeAttribute("data-active"));
    zone.addEventListener("drop", (event) => {
      event.preventDefault();
      zone.removeAttribute("data-active");
      const file = event.dataTransfer?.files[0];
      if (file) {
        this.#ready = this.#uploadFile(file);
      }
    });
    const picker = el("input", {
      id: "swath-add-data-file",
      type: "file",
      "aria-label": "Upload a file",
    });
    picker.addEventListener("change", () => {
      const file = picker.files?.[0];
      if (file) {
        this.#ready = this.#uploadFile(file);
      }
    });
    zone.append(el("br"), picker);
    return zone;
  }

  #fieldRow(field: Field): SwathField {
    const row = el("swath-field", {
      id: `swath-add-data-${field.key}`,
      name: field.key,
      label: field.label,
      help: field.help,
      value: field.get(this),
      readonly: field.readOnly === true,
    });
    const note = this.#fieldNote(field);
    if (note !== "") {
      row.error = note;
    }
    row.addEventListener("swath-input", (event) => {
      event.stopPropagation();
      field.set(this, String(event.detail.value));
      this.#touched.add(field.key);
      this.#serverNotes.delete(field.key);
      this.#updateValidity();
    });
    row.addEventListener("swath-change", (event) => event.stopPropagation());
    return row;
  }

  /** Live validity without a re-render (typing must not re-create the
   * field that has focus): notes, the submit state, the reason line. */
  #updateValidity(): void {
    for (const field of this.#fields()) {
      const row = this.renderRoot.querySelector<SwathField>(`#swath-add-data-${field.key}`);
      if (row) {
        const note = this.#fieldNote(field);
        row.error = note === "" ? undefined : note;
      }
    }
    const issues = this.#issues();
    const submit = this.renderRoot.querySelector<SwathButton>(".swath-add-data-submit");
    if (submit) {
      submit.disabled = issues.length > 0 || this.#submitting;
    }
    const reason = this.renderRoot.querySelector("#swath-add-data-reason");
    if (reason) {
      reason.textContent = issues.length > 0 ? `To add: ${issues.join("; ")}.` : "";
    }
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
      return [this.#note("swath-add-data-readonly", READ_ONLY_NOTE)];
    }
    const form = el("form");
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      this.#ready = this.#register();
    });
    form.append(this.#linkField());
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
      const issues = this.#issues();
      const submit = el(
        "swath-button",
        {
          class: "swath-add-data-submit",
          variant: "accent",
          disabled: issues.length > 0 || this.#submitting,
        },
        this.#submitting ? "Registering…" : "Add to Swath",
      );
      submit.addEventListener("click", () => form.requestSubmit());
      form.append(
        submit,
        el(
          "p",
          { class: "swath-add-data-reason", id: "swath-add-data-reason" },
          issues.length > 0 ? `To add: ${issues.join("; ")}.` : "",
        ),
      );
    }
    if (this.#flowError !== "") {
      form.append(this.#note("swath-add-data-error", this.#flowError));
    }
    if (this.#status !== "") {
      form.append(this.#note("swath-add-data-status", this.#status));
    }
    return [form];
  }

  protected render(): void {
    const toggle = el(
      "swath-button",
      { part: "toggle", class: "swath-add-data-toggle", size: "sm", pressed: this.open },
      "Add data",
    );
    toggle.addEventListener("click", () => this.#togglePanel());
    if (!this.open) {
      this.renderRoot.replaceChildren(toggle);
      return;
    }
    const drawer = el("swath-drawer", { edge: "right", open: true, label: "Add data" });
    drawer.append(el("span", { slot: "header" }, "Add data"), ...this.#body());
    drawer.addEventListener("swath-drawer-close", (event) => {
      event.stopPropagation();
      this.open = false;
    });
    this.renderRoot.replaceChildren(toggle, drawer);
  }
}

/** Registers `<swath-add-data-panel>`; safe to call more than once. */
export function defineSwathAddDataPanel(): void {
  SwathAddDataPanel.define();
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-add-data-panel": SwathAddDataPanel;
  }
}
