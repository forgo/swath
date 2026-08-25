// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The time slider (issue #182) — the charter-promised animation: scrub
 * and play a layer's temporal extent, one cacheable `datetime=` frame
 * per granule (ADR 0015).
 *
 * # Domain from the granules API, discovery from tileset metadata
 *
 * The slider's stops are the layer's granule acquisition datetimes:
 * `<swath-map>` follows the `granules` link its tileset metadata carries
 * (catalog-backed layers only) to `GET /datasets/{id}/granules` and
 * hands the parsed, ascending domain to this control. Fewer than two
 * dates means there is nothing to scrub — the control stays hidden, so
 * the zero-config landing page (single-granule fixture layers) is
 * visually untouched.
 *
 * # Owned by `<swath-map>`, like the x-ray overlay
 *
 * Same pattern as [`XRayOverlay`](./swath-xray.ts): a module the map
 * component owns and positions over its container, not a separate
 * element — a time slider is meaningless without a map whose source it
 * re-points. The pure pieces (domain parsing, frame resolution) are
 * exported for unit tests; the control talks back to its host through
 * the two-callback [`TimeSliderHooks`] seam, so tests drive it with no
 * map at all.
 *
 * # Scrub vs play
 *
 * Scrubbing re-points the raster source at the chosen frame and nothing
 * else — the first pass over a cold cache renders every frame live (the
 * x-ray shows `live` badges), which is the honest demo. Play advances a
 * frame per tick and additionally *prefetches* the frame after next
 * (the host fetches its visible tile URLs), so once the season has been
 * seen the loop replays from the server's tile cache without a stutter.
 */

/** Milliseconds per frame while playing: slow enough to read a frame's
 * badges, fast enough that a six-date season loops in ~7 s. */
export const PLAY_INTERVAL_MS = 1200;

/** The slice of the granules listing the slider consumes. */
interface GranulesBody {
  granules?: { datetime?: unknown }[];
}

/**
 * The temporal domain a granules listing carries: the granules'
 * acquisition datetimes, ascending and de-duplicated (two granules of
 * one instant are one frame — `datetime=` cannot tell them apart).
 * Malformed entries are skipped, never fatal.
 */
export function parseGranuleDatetimes(body: unknown): string[] {
  const granules = (body as GranulesBody)?.granules;
  if (!Array.isArray(granules)) {
    return [];
  }
  const valid = granules
    .map((granule) => granule?.datetime)
    .filter(
      (value): value is string => typeof value === "string" && !Number.isNaN(Date.parse(value)),
    );
  return [...new Set(valid)].sort((a, b) => Date.parse(a) - Date.parse(b));
}

/**
 * The frame a `datetime=` instant displays: the latest frame at or
 * before `t` — exactly the server's resolution rule (ADR 0015,
 * latest-at-or-before), so the slider's thumb always points at the
 * granule actually backing the imagery. Before the first frame (the
 * server's honest 404 window) and for a missing/malformed `t` the thumb
 * rests at 0; past the last frame it rests at the end.
 */
export function frameIndexForTime(frames: readonly string[], t: string | null): number {
  if (t === null) {
    return Math.max(0, frames.length - 1); // absent = latest
  }
  const instant = Date.parse(t);
  if (Number.isNaN(instant)) {
    return Math.max(0, frames.length - 1);
  }
  let index = 0;
  for (const [i, frame] of frames.entries()) {
    if (Date.parse(frame) <= instant) {
      index = i;
    }
  }
  return index;
}

/** How the control talks back to its host (`<swath-map>`). */
export interface TimeSliderHooks {
  /** Show this frame: the host re-points its raster source with
   * `datetime=` (and reflects its `datetime` attribute). */
  scrubTo(datetime: string): void;
  /** Warm this frame: the host fetches its visible tile URLs so the
   * server's write-through cache holds them before the frame displays. */
  prefetch(datetime: string): void;
  /** May the play loop advance right now? A tick that lands while the
   * current frame is still painting (cold cache, slow renderer) is
   * skipped and retried next interval — the loop never runs ahead of
   * the imagery, and a cold first pass adapts to the server's pace
   * instead of thrashing it. Absent = always. */
  canAdvance?(): boolean;
  /** The user touched the control (play/pause click, a scrub): the
   * host's cue that any automated playback — the cinematic landing loop
   * (issue #211) — is now the user's. Called BEFORE the act itself, so
   * the host can attribute the resulting frame change to the user. */
  interact?(): void;
}

/**
 * The DOM control: play/pause button, a native range input (one stop
 * per frame — accessible for free: arrow keys scrub), and the active
 * frame's datetime as a label. Hidden until [`setDomain`] hands it two
 * or more frames. Exact state rides `data-*` attributes for the tests.
 *
 * [`setDomain`]: TimeSlider.setDomain
 */
export class TimeSlider {
  readonly element: HTMLElement;
  readonly #hooks: TimeSliderHooks;
  readonly #play: HTMLButtonElement;
  readonly #range: HTMLInputElement;
  readonly #label: HTMLSpanElement;
  #frames: readonly string[] = [];
  #index = 0;
  #timer: number | undefined;

  constructor(doc: Document, hooks: TimeSliderHooks) {
    this.#hooks = hooks;
    this.element = doc.createElement("div");
    this.element.className = "swath-map-time";
    this.element.setAttribute("role", "group");
    this.element.setAttribute("aria-label", "Time");
    this.element.hidden = true;

    this.#play = doc.createElement("button");
    this.#play.type = "button";
    this.#play.className = "swath-map-time-play";
    this.#play.textContent = "play";
    this.#play.setAttribute("aria-label", "Play the layer's time series");
    this.#play.setAttribute("aria-pressed", "false");
    this.#play.addEventListener("click", () => {
      this.#hooks.interact?.();
      if (this.playing) {
        this.pause();
      } else {
        this.play();
      }
    });

    this.#range = doc.createElement("input");
    this.#range.type = "range";
    this.#range.min = "0";
    this.#range.step = "1";
    this.#range.setAttribute("aria-label", "Frame");
    this.#range.addEventListener("input", () => {
      const index = Number(this.#range.value);
      const frame = this.#frames[index];
      if (frame !== undefined && index !== this.#index) {
        this.#hooks.interact?.();
        this.#show(index);
        this.#hooks.scrubTo(frame);
      }
    });

    this.#label = doc.createElement("span");
    this.#label.className = "swath-map-time-label";

    this.element.append(this.#play, this.#range, this.#label);
  }

  /** The frames currently scrubbed over (ascending). */
  get frames(): readonly string[] {
    return this.#frames;
  }

  /** Whether the play loop is running. */
  get playing(): boolean {
    return this.#timer !== undefined;
  }

  /**
   * (Re)sets the temporal domain. Fewer than two frames hides the
   * control (nothing to scrub — the zero-config landing page stays
   * untouched); the active thumb is re-resolved against `active` (the
   * host's current `datetime` attribute, null = latest).
   */
  setDomain(frames: readonly string[], active: string | null): void {
    this.#frames = [...frames];
    if (frames.length < 2) {
      this.pause();
      this.element.hidden = true;
      delete this.element.dataset["frames"];
      return;
    }
    this.element.hidden = false;
    this.element.dataset["frames"] = String(frames.length);
    this.#range.max = String(frames.length - 1);
    this.#show(frameIndexForTime(frames, active));
  }

  /** Moves the thumb to the frame `datetime` displays (no scrub-back:
   * the host's attribute is the source of truth, this just mirrors it). */
  setActive(datetime: string | null): void {
    if (this.#frames.length >= 2) {
      this.#show(frameIndexForTime(this.#frames, datetime));
    }
  }

  /** Starts the loop: each tick advances one frame (wrapping) and
   * prefetches the frame after next, so the next advance's tiles are
   * already warm by the time it lands. A tick the host vetoes
   * (`canAdvance` false: the frame is still painting) is skipped. */
  play(): void {
    if (this.playing || this.#frames.length < 2) {
      return;
    }
    this.#play.textContent = "pause";
    this.#play.setAttribute("aria-label", "Pause the time series");
    this.#play.setAttribute("aria-pressed", "true");
    const next = (from: number): number => (from + 1) % this.#frames.length;
    const prefetch = this.#frames[next(this.#index)];
    if (prefetch !== undefined) {
      this.#hooks.prefetch(prefetch);
    }
    this.#timer = window.setInterval(() => {
      if (this.#hooks.canAdvance?.() === false) {
        return; // the current frame is still painting — hold it
      }
      const index = next(this.#index);
      const frame = this.#frames[index];
      if (frame === undefined) {
        return;
      }
      this.#show(index);
      this.#hooks.scrubTo(frame);
      const ahead = this.#frames[next(index)];
      if (ahead !== undefined) {
        this.#hooks.prefetch(ahead);
      }
    }, PLAY_INTERVAL_MS);
  }

  /** Stops the loop (idempotent). */
  pause(): void {
    if (this.#timer !== undefined) {
      window.clearInterval(this.#timer);
      this.#timer = undefined;
    }
    this.#play.textContent = "play";
    this.#play.setAttribute("aria-label", "Play the layer's time series");
    this.#play.setAttribute("aria-pressed", "false");
  }

  /** Tears the control down: stops the loop, removes the DOM. */
  dispose(): void {
    this.pause();
    this.element.remove();
  }

  #show(index: number): void {
    const frame = this.#frames[index];
    if (frame === undefined) {
      return;
    }
    this.#index = index;
    this.#range.value = String(index);
    this.#range.setAttribute("aria-valuetext", frame);
    this.#label.textContent = `${frame} (${index + 1}/${this.#frames.length})`;
    this.element.dataset["index"] = String(index);
    this.element.dataset["datetime"] = frame;
  }
}
