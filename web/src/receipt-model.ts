// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The publish receipt's pure model (issue #395).
 *
 * Publishing is the moment a person most wants proof, and it was the moment
 * the product said the least. The receipt is the glass box opening at publish
 * time: what was published, where it can be fetched, which granules the first
 * tile read, and what that tile cost.
 *
 * Everything here is derived from two server answers — the tileset document
 * and one rendered tile's `x-swath-trace` header. Nothing is inferred from
 * the graph the client just sent: a client-side guess that happens to agree
 * with the server is still a guess.
 */
import type { JoinLabel } from "./authoring-dag.js";
import type { PublishReceipt } from "./explain-model.js";
import type { GranuleBbox } from "./granule-footprints.js";
import type { TraceEnvelope, TraceJson } from "./swath-xray.js";
import { centerTile } from "./tms.js";

/** The tileset document, reduced to the fields the receipt reads. */
export interface TilesetDocument {
  boundingBox?: { lowerLeft?: unknown; upperRight?: unknown } | undefined;
  links?: readonly { rel?: unknown; href?: unknown; templated?: unknown }[] | undefined;
  "swath:window"?: unknown;
  "swath:sources"?: unknown;
}

/** The tile the receipt renders: the one at the centre of what was
 * published, at a zoom where the whole extent is in view.
 *
 * A fixed zoom would render an ocean tile for a small layer and a
 * sub-pixel one for a global layer; framing on the published extent is the
 * same rule the granule preview uses (#276). */
export function receiptTile(bbox: GranuleBbox | undefined): { z: number; x: number; y: number } {
  if (!bbox) {
    return { z: 0, x: 0, y: 0 };
  }
  const [west, south, east, north] = bbox;
  const span = Math.max(Math.abs(east - west), Math.abs(north - south));
  // The deepest level whose tile is at least as wide as the extent: 360° at
  // z0, halving each level. Clamped by `centerTile`.
  const z = span > 0 ? Math.max(0, Math.floor(Math.log2(360 / span))) : 12;
  return centerTile((west + east) / 2, (south + north) / 2, z);
}

/** `[west, south, east, north]` from a tileset's `boundingBox`, when it
 * carries one in the shape OGC 20-057 defines. */
export function tilesetBbox(doc: TilesetDocument): GranuleBbox | undefined {
  const pair = (raw: unknown): [number, number] | undefined => {
    if (!Array.isArray(raw) || raw.length < 2) {
      return undefined;
    }
    const [a, b] = raw;
    return typeof a === "number" &&
      typeof b === "number" &&
      Number.isFinite(a) &&
      Number.isFinite(b)
      ? [a, b]
      : undefined;
  };
  const lower = pair(doc.boundingBox?.lowerLeft);
  const upper = pair(doc.boundingBox?.upperRight);
  return lower && upper ? [lower[0], lower[1], upper[0], upper[1]] : undefined;
}

/** The receipt's addressable half, from the tileset document.
 *
 * The URLs are the server's own links, copied verbatim — the receipt must
 * hand over what the server serves, not a template the client assembled and
 * hopes matches. A missing link stays missing, and the card renders an em
 * dash for it.
 */
export function receiptFrom(doc: TilesetDocument, id: string, origin: string): PublishReceipt {
  const receipt: PublishReceipt = {};
  const links = doc.links ?? [];
  const href = (rel: string): string | undefined => {
    const found = links.find((link) => link.rel === rel && typeof link.href === "string");
    return typeof found?.href === "string" ? found.href : undefined;
  };
  // OGC's canonical tileset link; `self` is the document itself, which is
  // the address an OGC client starts from.
  const ogc = href("self") ?? `${origin}/tilesets/${encodeURIComponent(id)}`;
  receipt.ogc = ogc;
  // The XYZ template the map consumes. Served as a templated `item` link
  // where the server offers one; otherwise the documented path, which is
  // stable API surface rather than a guess.
  const item = links.find((link) => link.rel === "item" && typeof link.href === "string");
  receipt.xyz =
    typeof item?.href === "string"
      ? item.href
      : `${origin}/tilesets/${encodeURIComponent(id)}/tiles/{z}/{y}/{x}`;
  const window = doc["swath:window"];
  if (Array.isArray(window) && window.length === 2) {
    const [start, end] = window;
    receipt.window = `${typeof start === "string" ? start : "…"}/${typeof end === "string" ? end : "…"}`;
  } else if (typeof window === "string") {
    receipt.window = window;
  }
  const sources = doc["swath:sources"];
  if (Array.isArray(sources)) {
    const named = sources.filter((s): s is string => typeof s === "string");
    if (named.length > 0) {
      receipt.sources = named;
    }
  } else if (typeof sources === "number" && Number.isFinite(sources)) {
    // The documented shape is a branch COUNT for a two-source layer, not a
    // list of granules. Saying "2 sources" is true; naming granules we were
    // not given would not be.
    receipt.sources = [`${sources} source${sources === 1 ? "" : "s"}`];
  }
  return receipt;
}

/** The summary `x-swath-trace` header, as an envelope the explain card can
 * render.
 *
 * The header is a summary — decision, bytes, total, ingest→pixel — not the
 * full trace, so the envelope it produces carries no plan and no per-stage
 * timings. That is deliberate: the card renders em dashes for them rather
 * than the client inventing a breakdown the server did not send.
 */
export function envelopeFromHeader(
  header: string | null,
  tile: { z: number; x: number; y: number },
  layer: string,
): TraceEnvelope | undefined {
  if (header === null || header.trim() === "") {
    return undefined;
  }
  let summary: Record<string, unknown>;
  try {
    summary = JSON.parse(header) as Record<string, unknown>;
  } catch {
    return undefined;
  }
  const num = (key: string): number | null => {
    const value = summary[key];
    return typeof value === "number" && Number.isFinite(value) ? value : null;
  };
  const decision = summary["decision"];
  const trace = {
    decision: (typeof decision === "string" || (typeof decision === "object" && decision !== null)
      ? decision
      : "live") as TraceJson["decision"],
    source: "",
    sources: [],
    crs_from: 0,
    crs_to: 0,
    bytes_read: num("bytes_read") ?? Number.NaN,
    provenance: [],
    timings: {
      read_ms: Number.NaN,
      warp_ms: Number.NaN,
      pixel_ops_ms: Number.NaN,
      encode_ms: Number.NaN,
      total_ms: num("total_ms") ?? Number.NaN,
    },
    ingest_to_pixel_ms: num("ingest_to_pixel_ms"),
    plan: null,
  } satisfies TraceJson;
  return { tile: `${tile.z}/${tile.x}/${tile.y}`, layer, trace };
}

/** What the SERVER says a published layer's join is (#405): its compiled
 * window and how many sources it joined, read from the tileset document.
 *
 * The client infers both while composing (`inferredJoin`); this replaces
 * that inference the moment there is a published layer, because the label
 * is the claim an operator will quote. A guess that happens to agree with
 * the server is still a guess.
 *
 * `undefined` when the document carries neither field — a static layer, or
 * one the server did not compile a window for.
 */
export function publishedJoin(doc: TilesetDocument): JoinLabel | undefined {
  const raw = doc["swath:window"];
  const window: [string | null, string | null] | undefined =
    Array.isArray(raw) && raw.length === 2
      ? [typeof raw[0] === "string" ? raw[0] : null, typeof raw[1] === "string" ? raw[1] : null]
      : undefined;
  const sources = doc["swath:sources"];
  const branches = typeof sources === "number" && Number.isFinite(sources) ? sources : undefined;
  if (window === undefined && branches === undefined) {
    return undefined;
  }
  return { window: window ?? [null, null], branches: branches ?? 1 };
}

/** Whether the server's answer differs from what the client inferred — the
 * cue to say the label was updated rather than silently swapping it. */
export function joinLabelDiffers(inferred: JoinLabel | undefined, published: JoinLabel): boolean {
  if (inferred === undefined) {
    return false; // nothing was claimed, so nothing was corrected
  }
  return (
    inferred.branches !== published.branches ||
    inferred.window[0] !== published.window[0] ||
    inferred.window[1] !== published.window[1]
  );
}
