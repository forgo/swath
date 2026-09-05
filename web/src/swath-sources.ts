// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-sources>` (#418, ADR 0030): what is watching, what is broken,
 * and what each origin has actually done.
 *
 * Every state on this screen traces to a served field. Nothing is
 * inferred client-side: the state, the counts and the reachability come
 * from `GET /sources`, freshness is computed from the server's own
 * `lastEvent` instant, and where the server said nothing the row says an
 * em dash. A broken source reads as broken in **form** as well as in
 * number — the row carries a tone, not only a smaller figure.
 *
 * The empty state does work rather than apologising: it offers to
 * register the committed fixture stack, which is what a new operator
 * needs in their first minute.
 *
 * Parts: `list row title meta state counts empty action report error`.
 */
import { SwathApi } from "./api.js";
import { describeReport, loadFixtureStack } from "./fixture-stack.js";
import {
  countsLine,
  credentialNote,
  freshness,
  isFirstRun,
  parseSources,
  type SourceRow,
  stateLabel,
  stateTone,
  UNKNOWN,
} from "./sources-model.js";
import { SwathButton } from "./ui/button.js";
import { el } from "./ui/dom.js";
import { SwathElement } from "./ui/element.js";
import { css } from "./ui/styles.js";

export class SwathSources extends SwathElement {
  static override tagName = "swath-sources";
  static override styles = [
    css`
      :host { display: block; }
      [part="list"] {
        display: grid;
        gap: var(--swath-space-2);
        margin: 0;
        padding: 0;
        list-style: none;
      }
      [part="row"] {
        display: grid;
        gap: var(--swath-space-1);
        padding: var(--swath-space-2);
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-md);
      }
      [part="row"][data-tone="danger"] {
        border-color: var(--swath-color-danger);
      }
      [part="title"] {
        display: flex;
        align-items: baseline;
        gap: var(--swath-space-2);
        font-size: var(--swath-text-sm);
      }
      [part="meta"], [part="counts"] {
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
        font-variant-numeric: tabular-nums;
      }
      [part="state"] {
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
      }
      [part="state"][data-tone="ok"] { color: var(--swath-color-accent); }
      [part="state"][data-tone="danger"] { color: var(--swath-color-danger); }
      [part="error"] { color: var(--swath-color-danger); font-size: var(--swath-text-xs); }
      [part="empty"], [part="report"] {
        margin: 0 0 var(--swath-space-2);
        font-size: var(--swath-text-sm);
        color: var(--swath-color-fg-muted);
      }
    `,
  ];
  static override properties = {
    /** Fetches begin when true (the host sets it on entering the mode). */
    active: { type: "boolean", reflect: true },
    server: { type: "string" },
  } as const;

  declare active: boolean;
  declare server: string | undefined;

  #api: SwathApi | undefined;
  #rows: SourceRow[] = [];
  #error: string | undefined;
  #loaded = false;
  #report: string | undefined;
  #busy = false;
  /** The clock freshness is measured against. Injectable so the tests
   * assert an age rather than race one. */
  now: () => number = () => Date.now();

  set api(value: SwathApi) {
    this.#api = value;
  }

  get api(): SwathApi {
    this.#api ??= new SwathApi({ base: this.server ?? "" });
    return this.#api;
  }

  /** The rows as served (test seam). */
  get rows(): readonly SourceRow[] {
    return this.#rows;
  }

  /** Re-read `GET /sources`. A failure is shown in the server's words;
   * the screen never guesses a state to fill the gap. */
  async refresh(): Promise<void> {
    this.#loaded = true;
    try {
      this.#rows = parseSources(await this.api.json("/sources"));
      this.#error = undefined;
    } catch (error) {
      this.#rows = [];
      this.#error = error instanceof Error ? error.message : String(error);
    }
    this.requestUpdate();
  }

  /** The first-run action: register the committed fixtures, then re-read
   * so what the screen shows is what the server now says. */
  async loadFixtures(): Promise<void> {
    if (this.#busy) {
      return;
    }
    this.#busy = true;
    this.requestUpdate();
    const report = await loadFixtureStack(this.api);
    this.#report = describeReport(report);
    this.#busy = false;
    await this.refresh();
  }

  #row(row: SourceRow): HTMLElement {
    const tone = stateTone(row);
    // Form as well as number: the state carries a tone the row's border
    // and the chip both read, so a broken source is visibly broken.
    const state = el("span", { part: "state", "data-tone": tone }, stateLabel(row));
    const item = el(
      "li",
      { part: "row", "data-tone": tone, "data-source": row.id },
      el("div", { part: "title" }, el("span", {}, row.title), state),
      el(
        "p",
        { part: "meta" },
        `${row.kind} · ${row.scheme} · ${row.origin} · last event ${freshness(row, this.now())}`,
      ),
      el("p", { part: "counts" }, countsLine(row)),
    );
    if (row.lastError !== undefined) {
      item.append(el("p", { part: "error", role: "alert" }, row.lastError));
    }
    const credential = credentialNote(row);
    if (credential !== undefined) {
      item.append(el("p", { part: "meta" }, credential));
    }
    if (row.datasets.length > 0) {
      item.append(el("p", { part: "meta" }, `feeds ${row.datasets.join(", ")}`));
    }
    return item;
  }

  #emptyState(): HTMLElement {
    const action = el("swath-button", {
      part: "action",
      variant: "primary",
      label: "Load the fixture stack",
      disabled: this.#busy,
    });
    action.textContent = this.#busy ? "Loading…" : "Load the fixture stack";
    action.addEventListener("click", () => {
      void this.loadFixtures();
    });
    return el(
      "div",
      {},
      el(
        "p",
        { part: "empty" },
        this.#rows.length === 0
          ? "No sources are configured. Load the committed fixtures to have something on the map."
          : "Nothing has been ingested yet. Load the committed fixtures to have something on the map.",
      ),
      action,
    );
  }

  protected render(): void {
    // Lazy by contract, as the catalog is: nothing is fetched until the
    // mode is entered.
    if (this.active && !this.#loaded) {
      void this.refresh();
    }
    const children: HTMLElement[] = [];
    if (this.#error !== undefined) {
      children.push(
        el("p", { part: "error", role: "alert" }, `Could not list sources: ${this.#error}`),
      );
    }
    if (this.#report !== undefined) {
      children.push(el("p", { part: "report", role: "status" }, this.#report));
    }
    if (this.#loaded && this.#error === undefined && isFirstRun(this.#rows)) {
      children.push(this.#emptyState());
    }
    if (this.#rows.length > 0) {
      children.push(el("ul", { part: "list" }, ...this.#rows.map((row) => this.#row(row))));
    } else if (!this.#loaded) {
      children.push(el("p", { part: "empty" }, UNKNOWN));
    }
    this.renderRoot.replaceChildren(...children);
  }
}

export function defineSwathSources(): void {
  SwathButton.define();
  SwathSources.define();
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-sources": SwathSources;
  }
}
