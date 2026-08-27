// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The status bar's cell text (issue #287): pure formatting beside
 * `view-state.ts`. lat/lon · zoom · CRS · ingest→pixel — the glass-box
 * number is always on screen, not only while the x-ray is on.
 */
import { formatZoom } from "./view-state.js";

/** The tiling scheme every layer serves in (crates/swath-api model.rs). */
export const TILE_MATRIX_SET = "WebMercatorQuad";
export const TILE_CRS = "EPSG:3857";

/** Cursor precision: 4 decimals ≈ 11 m — a readout, not a share link. */
const CURSOR_DECIMALS = 4;

export interface Cursor {
  lng: number;
  lat: number;
  /** `pointer` while a mouse is over the map, `center` otherwise (touch). */
  source: "pointer" | "center";
}

function trim(value: number, decimals: number): string {
  return value.toFixed(decimals).replace(/\.?0+$/, "");
}

/** "lon, lat" with the cursor precision; longitude wrapped to ±180. */
export function formatLonLat(lng: number, lat: number): string {
  let wrapped = ((((lng + 180) % 360) + 360) % 360) - 180;
  if (Object.is(wrapped, -0)) {
    wrapped = 0;
  }
  return `${trim(wrapped, CURSOR_DECIMALS)}, ${trim(lat, CURSOR_DECIMALS)}`;
}

/** Zoom with view-state's precision (2 decimals, trailing zeros trimmed). */
export function formatZoomCell(zoom: number): string {
  return formatZoom(zoom);
}

/** "— " until the first traced tile; then the best ingest→pixel seen. */
export function formatIngest(ms: number | undefined): string {
  return ms === undefined ? "—" : `${Math.round(ms)} ms`;
}

/** The CRS cell: scheme and EPSG code. */
export function formatCrs(): string {
  return `${TILE_MATRIX_SET} · ${TILE_CRS}`;
}
