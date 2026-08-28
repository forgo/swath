// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-granule-card>` (issue #288): one granule as a picture the engine
 * rendered — a `<swath-card interactive>` whose media is the preview the
 * host feeds it (`thumbnail` = an object URL, or a `note` when the server
 * refused the preview, in plain words; never a client decode, ADR 0019).
 * `granule-id` is mirrored as `data-granule` on the host; activation
 * emits `swath-activate` (the catalog turns it into `swath-granule-zoom`).
 */
import { SwathCard } from "./card.js";
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

export class SwathGranuleCard extends SwathElement {
  static override tagName = "swath-granule-card";
  static override styles = [
    css`
      :host { display: block; }
      [part="media"] {
        display: block;
        aspect-ratio: 1;
        inline-size: 100%;
        background: var(--swath-color-bg);
        object-fit: cover;
      }
      [part="pending"], [part="note"] {
        display: grid;
        place-items: center;
        aspect-ratio: 1;
        padding: var(--swath-space-2);
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
        text-align: center;
        background: var(--swath-color-bg);
      }
      [part="note"] { color: var(--swath-color-warn); }
      [part="title"] { font-size: var(--swath-text-sm); color: var(--swath-color-fg); }
      [part="meta"] {
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
        word-break: break-all;
      }
      :host([layout="list"]) swath-card::part(base) { flex-direction: row; align-items: center; }
      :host([layout="list"]) [part="media"], :host([layout="list"]) [part="pending"], :host([layout="list"]) [part="note"] {
        inline-size: var(--swath-space-8);
        block-size: var(--swath-space-8);
        aspect-ratio: auto;
        flex: none;
      }
    `,
  ];
  static override properties = {
    granuleId: { type: "string", attribute: "granule-id", reflect: true },
    datasetId: { type: "string", attribute: "dataset-id", reflect: true },
    datetime: { type: "string" },
    /** Object URL of the engine's preview; absent while pending. */
    thumbnail: { type: "string" },
    /** Why there is no preview (the server's refusal, in plain words). */
    note: { type: "string" },
    kind: { type: "string" },
    layout: { type: "string", reflect: true },
    selected: { type: "boolean", reflect: true },
  } as const;

  declare granuleId: string | undefined;
  declare datasetId: string | undefined;
  declare datetime: string | undefined;
  declare thumbnail: string | undefined;
  declare note: string | undefined;
  declare kind: string | undefined;
  declare layout: string | undefined;
  declare selected: boolean;

  #card: SwathCard | undefined;

  constructor() {
    super();
    SwathCard.define();
  }

  #ensure(): SwathCard {
    if (this.#card) {
      return this.#card;
    }
    const card = el("swath-card", { interactive: true, dense: true });
    card.addEventListener("swath-activate", (event) => {
      event.stopPropagation();
      this.emit("swath-activate", { id: this.granuleId ?? "", long: event.detail.long });
    });
    this.#card = card;
    this.renderRoot.replaceChildren(card);
    return card;
  }

  protected render(): void {
    const card = this.#ensure();
    const id = this.granuleId ?? "";
    this.dataset["granule"] = id;
    card.id = id;
    card.selected = this.selected;
    const media =
      this.thumbnail !== undefined && this.thumbnail !== ""
        ? el("img", {
            part: "media",
            slot: "media",
            src: this.thumbnail,
            alt: `Engine preview of ${id}`,
          })
        : this.note !== undefined && this.note !== ""
          ? el("div", { part: "note", slot: "media", role: "note" }, this.note)
          : el("div", { part: "pending", slot: "media", "aria-busy": "true" }, "rendering…");
    const when = (this.datetime ?? "").replace("T", " ").replace(/:\d\d(\.\d+)?Z$/, "Z");
    card.replaceChildren(
      media,
      el("div", { part: "title" }, when === "" ? id : when),
      el(
        "div",
        { part: "meta" },
        `${id}${this.kind ? ` · ${this.kind} preview · current frame` : ""}`,
      ),
    );
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-granule-card": SwathGranuleCard;
  }
}
