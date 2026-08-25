// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The compare swipe's DOM machinery (issue #210): a second MapLibre map
 * clipped to the right of the handle, view-synced to the primary, plus
 * the handle itself and the per-side identity chips.
 *
 * # Owned by `<swath-map>`, like the x-ray overlay and the time slider
 *
 * Same pattern as [`XRayOverlay`](./swath-xray.ts) and
 * [`TimeSlider`](./time-slider.ts): a module the map component owns and
 * positions over its container — a swipe is meaningless without a map
 * to split. The pure pieces (mode resolution, side identity, swipe
 * clamping) live in compare-model.ts; this module only renders them.
 *
 * # Two real maps, one gesture
 *
 * MapLibre has no per-layer scissor for raster sources, so the right
 * side is a second `Map` in a `clip-path`-clipped container over the
 * primary — the same construction the classic compare plugins use, with
 * one deliberate simplification: the right map is non-interactive and
 * its container ignores pointer events entirely, so every pan/zoom
 * lands on the primary map underneath and the sync is strictly one-way
 * (primary drives, right follows via `jumpTo` on `move`). No feedback
 * loops, no double-handling, and the right side can never drift.
 *
 * The handle is a real focusable `role="slider"` (arrow keys nudge,
 * Home/End jump) whose drag reports fractions back through the
 * [`CompareViewHooks`] seam — the host reflects them into its `swipe`
 * attribute, which remains the single source of truth.
 */

import { Map as MapLibreMap, type RasterTileSource } from "maplibre-gl";
import { clampSwipe } from "./compare-model.js";

/** Keyboard nudge per arrow-key press: 2% of the map width. */
const KEY_STEP = 0.02;

/** How the compare view talks back to its host (`<swath-map>`). */
export interface CompareViewHooks {
  /** The user moved the handle (drag or keys): the host reflects the
   * fraction into its `swipe` attribute. */
  onSwipe(fraction: number): void;
}

/**
 * The right-side map + handle + chips. Created when a compare activates,
 * disposed when it ends; the host re-feeds style/labels/fraction through
 * the setters whenever its own state changes.
 */
export class CompareView {
  readonly #host: HTMLElement;
  readonly #primary: MapLibreMap;
  readonly #hooks: CompareViewHooks;
  readonly #clip: HTMLDivElement;
  readonly #mapContainer: HTMLDivElement;
  readonly #handle: HTMLDivElement;
  readonly #labelLeft: HTMLSpanElement;
  readonly #labelRight: HTMLSpanElement;
  readonly #onPrimaryMove: () => void;
  #map: MapLibreMap | undefined;
  #fraction = clampSwipe(undefined);
  #disposed = false;

  constructor(host: HTMLElement, primary: MapLibreMap, hooks: CompareViewHooks) {
    this.#host = host;
    this.#primary = primary;
    this.#hooks = hooks;
    const doc = host.ownerDocument;

    this.#clip = doc.createElement("div");
    this.#clip.className = "swath-map-compare";
    this.#mapContainer = doc.createElement("div");
    this.#mapContainer.className = "swath-map-compare-map";
    this.#clip.append(this.#mapContainer);

    this.#handle = doc.createElement("div");
    this.#handle.className = "swath-map-compare-handle";
    this.#handle.setAttribute("role", "slider");
    this.#handle.setAttribute("aria-label", "Compare swipe");
    this.#handle.setAttribute("aria-valuemin", "0");
    this.#handle.setAttribute("aria-valuemax", "100");
    this.#handle.tabIndex = 0;
    this.#labelLeft = doc.createElement("span");
    this.#labelLeft.className = "swath-map-compare-label";
    this.#labelLeft.dataset["side"] = "left";
    this.#labelRight = doc.createElement("span");
    this.#labelRight.className = "swath-map-compare-label";
    this.#labelRight.dataset["side"] = "right";
    const grip = doc.createElement("span");
    grip.className = "swath-map-compare-grip";
    grip.textContent = "⇆";
    grip.setAttribute("aria-hidden", "true");
    this.#handle.append(this.#labelLeft, grip, this.#labelRight);

    this.#handle.addEventListener("pointerdown", (event) => {
      event.preventDefault();
      this.#handle.setPointerCapture(event.pointerId);
    });
    this.#handle.addEventListener("pointermove", (event) => {
      if (!this.#handle.hasPointerCapture(event.pointerId)) {
        return;
      }
      const rect = this.#host.getBoundingClientRect();
      if (rect.width > 0) {
        this.#hooks.onSwipe(clampSwipe((event.clientX - rect.left) / rect.width));
      }
    });
    this.#handle.addEventListener("keydown", (event) => {
      const nudged = this.#keyFraction(event.key);
      if (nudged !== undefined) {
        event.preventDefault();
        this.#hooks.onSwipe(nudged);
      }
    });

    // Stacking by DOM order (no z-index anywhere on the clip): the right
    // map goes IMMEDIATELY after the primary map's container, so it
    // paints over the primary canvas and under whatever the host
    // appended after that container — the x-ray overlay root above all,
    // whichever of compare/x-ray was enabled first. The handle carries
    // its own z-index and can go last.
    primary.getContainer().after(this.#clip);
    host.append(this.#handle);
    this.#applyFraction();

    this.#onPrimaryMove = () => this.#sync();
    this.#primary.on("move", this.#onPrimaryMove);
  }

  /** The right-side MapLibre map (undefined until the first style lands).
   * Same UNSTABLE escape hatch as `SwathMap.map`. */
  get map(): MapLibreMap | undefined {
    return this.#map;
  }

  /** The current handle fraction. */
  get fraction(): number {
    return this.#fraction;
  }

  /** (Re)applies the right side's full style — creates the map on first
   * call, mirroring the primary's current view exactly. */
  setStyle(style: object): void {
    if (this.#disposed) {
      return;
    }
    if (!this.#map) {
      this.#map = new MapLibreMap({
        container: this.#mapContainer,
        style: style as never,
        center: this.#primary.getCenter(),
        zoom: this.#primary.getZoom(),
        bearing: this.#primary.getBearing(),
        pitch: this.#primary.getPitch(),
        interactive: false,
        attributionControl: false,
      });
      return;
    }
    this.#map.setStyle(style as never);
    this.#sync();
  }

  /** The `datetime` fast path, right-side edition: re-points the raster
   * source in place, deferring past a mid-apply style diff exactly like
   * the host's own repoint. */
  repoint(sourceId: string, template: string): void {
    const map = this.#map;
    if (!map) {
      return;
    }
    const repoint = (): void => {
      const source = map.getSource(sourceId) as RasterTileSource | undefined;
      source?.setTiles([template]);
    };
    if (map.getSource(sourceId)) {
      repoint();
    } else {
      map.once("styledata", repoint);
    }
  }

  /** Moves the handle and the clip (no event: the host's `swipe`
   * attribute is the source of truth, this just mirrors it). */
  setFraction(fraction: number): void {
    this.#fraction = clampSwipe(fraction);
    this.#applyFraction();
  }

  /** The per-side identity chips (frames in date mode, layer ids in
   * layer mode) and the mode marker the tests key on. */
  setLabels(mode: string, left: string, right: string): void {
    this.#handle.dataset["mode"] = mode;
    this.#labelLeft.textContent = left;
    this.#labelRight.textContent = right;
  }

  /** Tears the compare down: right map, handle, listeners — all of it. */
  dispose(): void {
    this.#disposed = true;
    this.#primary.off("move", this.#onPrimaryMove);
    this.#map?.remove();
    this.#map = undefined;
    this.#clip.remove();
    this.#handle.remove();
  }

  #keyFraction(key: string): number | undefined {
    switch (key) {
      case "ArrowLeft":
      case "ArrowDown":
        return clampSwipe(this.#fraction - KEY_STEP);
      case "ArrowRight":
      case "ArrowUp":
        return clampSwipe(this.#fraction + KEY_STEP);
      case "Home":
        return 0;
      case "End":
        return 1;
      default:
        return undefined;
    }
  }

  #applyFraction(): void {
    const percent = this.#fraction * 100;
    this.#clip.style.clipPath = `inset(0 0 0 ${percent}%)`;
    this.#handle.style.left = `${percent}%`;
    this.#handle.dataset["fraction"] = String(this.#fraction);
    this.#handle.setAttribute("aria-valuenow", String(Math.round(percent)));
    this.#handle.setAttribute("aria-valuetext", `${Math.round(percent)}%`);
  }

  /** One-way view sync: the primary map drives, the right map follows. */
  #sync(): void {
    this.#map?.jumpTo({
      center: this.#primary.getCenter(),
      zoom: this.#primary.getZoom(),
      bearing: this.#primary.getBearing(),
      pitch: this.#primary.getPitch(),
    });
  }
}
