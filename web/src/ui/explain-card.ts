// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The tethered explain card (issue #394) — one component at three densities.
 *
 * `concept` is a definition from the glossary. `measured` is what the planner
 * actually decided for the thing under the pointer, from a trace envelope.
 * `published` is the same card at its fullest, after a publish.
 *
 * One component rather than three, and one card rather than a card plus a
 * separate receipt: it is less to build and less to learn — one tether, one
 * dismissal rule, one place where explanation lives.
 *
 * The content comes from `explain-model.ts`; this file is the surface only.
 * Every figure it renders was formatted there from something the server said,
 * so there is no arithmetic here to get wrong.
 */
import type { ExplainContent } from "../explain-model.js";
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

export class SwathExplainCard extends SwathElement {
  static override tagName = "swath-explain-card";
  static override properties = {
    open: { type: "boolean", reflect: true },
    density: { type: "string", reflect: true },
  } as const;

  declare open: boolean;
  declare density: string | undefined;

  static override styles = [
    css`
      :host {
        position: fixed;
        z-index: var(--swath-z-hud);
        display: block;
        inline-size: max-content;
        max-inline-size: 34ch;
      }
      :host(:not([open])) { display: none; }
      [part="base"] {
        display: flex;
        flex-direction: column;
        gap: var(--swath-space-2);
        padding: var(--swath-space-3);
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-md);
        background: var(--swath-color-bg-hud);
        backdrop-filter: var(--swath-blur-hud);
        box-shadow: var(--swath-shadow-hud);
      }
      [part="header"] {
        display: flex;
        align-items: baseline;
        justify-content: space-between;
        gap: var(--swath-space-2);
      }
      [part="title"] {
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        font-weight: var(--swath-weight-label);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg);
      }
      [part="dismiss"] {
        border: 0;
        padding: 0;
        background: none;
        color: var(--swath-color-fg-muted);
        cursor: pointer;
        font: inherit;
      }
      [part="dismiss"]:hover { color: var(--swath-color-fg); }
      [part="definition"] {
        margin: 0;
        font-size: var(--swath-text-sm);
        color: var(--swath-color-fg);
      }
      [part="rows"] {
        display: grid;
        grid-template-columns: auto 1fr;
        gap: var(--swath-space-1) var(--swath-space-3);
        font-size: var(--swath-text-xs);
      }
      [part="label"] { color: var(--swath-color-fg-muted); }
      [part="value"] {
        color: var(--swath-color-fg);
        overflow-wrap: anywhere;
      }
      [part="value"][data-mono] { font-family: var(--swath-font-mono); }
      [part="candidates"] {
        display: grid;
        grid-template-columns: auto auto 1fr;
        gap: var(--swath-space-1) var(--swath-space-2);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
      }
      [part="candidate"][data-admissible="false"] { opacity: 0.7; }
      [part="fix"] {
        margin: 0;
        padding-block-start: var(--swath-space-2);
        border-block-start: var(--swath-border-hairline);
        font-size: var(--swath-text-sm);
        color: var(--swath-color-fg);
      }
    `,
  ];

  #content: ExplainContent | undefined;

  /** What to say. Assigning re-renders and sets the density. */
  get content(): ExplainContent | undefined {
    return this.#content;
  }

  set content(next: ExplainContent | undefined) {
    this.#content = next;
    if (next) {
      this.density = next.density;
    }
    this.requestUpdate();
  }

  /** Position the card beside `anchor`, kept inside the viewport.
   *
   * It never covers the map's centre: a card that explains the pixels by
   * hiding them is not an explanation. When there is no room beside the
   * anchor it flips to the other side rather than drifting inward. */
  tetherTo(anchor: Element): void {
    const rect = anchor.getBoundingClientRect();
    const own = this.getBoundingClientRect();
    const margin = 8;
    const room = window.innerWidth - rect.right;
    const left =
      room > own.width + margin
        ? rect.right + margin
        : Math.max(margin, rect.left - own.width - margin);
    const top = Math.min(
      Math.max(margin, rect.top),
      Math.max(margin, window.innerHeight - own.height - margin),
    );
    this.style.left = `${Math.round(left)}px`;
    this.style.top = `${Math.round(top)}px`;
  }

  override connectedCallback(): void {
    super.connectedCallback();
    // Escape and an outside click both dismiss; the host decides what that
    // means for its own state. Listeners hang off the disconnect signal.
    const signal = this.disconnected;
    window.addEventListener(
      "keydown",
      (event) => {
        if (event.key === "Escape" && this.open) {
          this.#dismiss();
        }
      },
      { signal },
    );
    document.addEventListener(
      "pointerdown",
      (event) => {
        if (this.open && !event.composedPath().includes(this)) {
          this.#dismiss();
        }
      },
      { signal },
    );
  }

  #dismiss(): void {
    this.open = false;
    this.emit("swath-explain-dismiss", {});
  }

  protected render(): void {
    const content = this.#content;
    if (!content) {
      this.renderRoot.replaceChildren();
      return;
    }
    const base = el("div", { part: "base", role: "dialog", "aria-label": content.title });
    const dismiss = el("button", { part: "dismiss", type: "button", "aria-label": "close" });
    const icon = document.createElement("swath-icon");
    icon.setAttribute("name", "close");
    icon.setAttribute("size", "sm");
    dismiss.append(icon);
    dismiss.addEventListener("click", () => {
      this.#dismiss();
    });
    base.append(
      el("div", { part: "header" }, el("span", { part: "title" }, content.title), dismiss),
    );

    if (content.definition !== undefined) {
      base.append(el("p", { part: "definition" }, content.definition));
    }
    if (content.rows.length > 0) {
      const rows = el("dl", { part: "rows" });
      for (const row of content.rows) {
        rows.append(
          el("dt", { part: "label" }, row.label),
          el("dd", { part: "value", "data-mono": row.mono === true }, row.value),
        );
      }
      base.append(rows);
    }
    if (content.candidates.length > 0) {
      const table = el("div", { part: "candidates", role: "table" });
      for (const candidate of content.candidates) {
        table.append(
          el(
            "span",
            { part: "candidate", "data-admissible": String(candidate.admissible) },
            candidate.strategy,
          ),
          el("span", { part: "candidate-cost" }, candidate.cost),
          el("span", { part: "candidate-reason" }, candidate.reason),
        );
      }
      base.append(table);
    }
    if (content.fix !== undefined) {
      base.append(el("p", { part: "fix" }, content.fix));
    }
    this.renderRoot.replaceChildren(base);
  }
}
