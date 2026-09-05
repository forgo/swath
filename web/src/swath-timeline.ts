// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-timeline>` (#411): two stacked bands over one time axis — what
 * the collection holds, and what survives the current filters. The gap
 * between them is the filter's effect, made visible, which is the single
 * most useful thing a search UI can show.
 *
 * Both bands come from the counts endpoint; nothing here is inferred from
 * a fetched page. With no filters active the bands are identical and the
 * control says so in words rather than drawing a confusing empty second
 * band.
 *
 * Dragging across the axis emits `swath-dates` with whole-day bounds —
 * the host turns that into the URL's date chip. The control is
 * keyboard-reachable: each bucket is a button, and Enter or Space picks
 * it; shift-picking a second bucket makes the range.
 *
 * Parts: `base hint axis bucket exists survives note`.
 */
import {
  bandNote,
  barHeight,
  dayOf,
  EMPTY_TIMELINE,
  rangeOf,
  TIMELINE_HINT,
  type Timeline,
} from "./timeline-model.js";
import { el } from "./ui/dom.js";
import { SwathElement } from "./ui/element.js";
import { css } from "./ui/styles.js";

export class SwathTimeline extends SwathElement {
  static override tagName = "swath-timeline";
  static override styles = [
    css`
      :host { display: block; }
      [part="hint"], [part="note"] {
        margin: 0;
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
      }
      [part="axis"] {
        display: flex;
        align-items: flex-end;
        gap: 1px;
        block-size: var(--swath-space-8);
        margin-block: var(--swath-space-1);
      }
      [part="bucket"] {
        flex: 1 1 0;
        min-inline-size: 2px;
        block-size: 100%;
        display: flex;
        flex-direction: column;
        justify-content: flex-end;
        padding: 0;
        border: 0;
        background: none;
        cursor: pointer;
      }
      [part="bucket"]:focus-visible {
        outline: var(--swath-focus-ring);
        outline-offset: 1px;
      }
      [part="bucket"][aria-pressed="true"] { background: var(--swath-color-accent-bg); }
      [part="exists"] {
        display: block;
        background: var(--swath-color-bg-raised);
        border-block-start: var(--swath-border-hairline);
      }
      [part="survives"] {
        display: block;
        background: var(--swath-color-accent);
      }
    `,
  ];

  #timeline: Timeline = EMPTY_TIMELINE;
  #anchor: number | undefined;
  #selection: [number, number] | undefined;

  /** The bands to draw. Setting them clears any in-progress drag: the
   * axis it referred to is gone. */
  set timeline(value: Timeline) {
    this.#timeline = value;
    this.#anchor = undefined;
    this.#selection = undefined;
    this.requestUpdate();
  }

  get timeline(): Timeline {
    return this.#timeline;
  }

  #pick(index: number, extend: boolean): void {
    if (extend && this.#anchor !== undefined) {
      this.#selection = [this.#anchor, index];
    } else {
      this.#anchor = index;
      this.#selection = [index, index];
    }
    const [a, b] = this.#selection;
    const range = rangeOf(this.#timeline, a, b);
    if (range !== undefined) {
      this.emit("swath-dates", { from: range.from, to: range.to });
    }
    this.requestUpdate();
  }

  #selected(index: number): boolean {
    if (this.#selection === undefined) {
      return false;
    }
    const [a, b] = this.#selection;
    return index >= Math.min(a, b) && index <= Math.max(a, b);
  }

  protected render(): void {
    const { buckets, peak } = this.#timeline;
    const hint = el("p", { part: "hint" }, TIMELINE_HINT);
    const bars = buckets.map((bucket, index) => {
      const bar = el("button", {
        part: "bucket",
        type: "button",
        "aria-pressed": this.#selected(index) ? "true" : "false",
        // The accessible name is the date and both counts, so the axis is
        // readable without seeing it.
        "aria-label": `${dayOf(bucket.start)}: ${bucket.survives} of ${bucket.exists}`,
      });
      const exists = el("span", { part: "exists" });
      exists.style.blockSize = `${barHeight(bucket.exists, peak) * 100}%`;
      const survives = el("span", { part: "survives" });
      survives.style.blockSize = `${barHeight(bucket.survives, peak) * 100}%`;
      bar.append(exists, survives);
      bar.addEventListener("pointerdown", (event) => {
        this.#pick(index, event.shiftKey);
      });
      bar.addEventListener("pointerenter", (event) => {
        // Buttons=1 is a drag in progress: extend from the anchor.
        if (event.buttons === 1 && this.#anchor !== undefined) {
          this.#pick(index, true);
        }
      });
      bar.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          this.#pick(index, event.shiftKey);
        }
      });
      return bar;
    });
    const axis = el("div", { part: "axis", role: "group", "aria-label": TIMELINE_HINT }, ...bars);
    const note = el("p", { part: "note", role: "status" }, bandNote(this.#timeline));
    this.renderRoot.replaceChildren(el("div", { part: "base" }, hint, axis, note));
  }
}

export function defineSwathTimeline(): void {
  SwathTimeline.define();
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-timeline": SwathTimeline;
  }
}
