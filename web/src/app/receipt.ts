// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The publish receipt (issue #395): after a publish, render one tile of what
 * was published and show the explain card at its `published` density.
 *
 * Two server answers, no client inference: the tileset document says where
 * the layer can be fetched and what window it compiled to; one rendered
 * tile's `x-swath-trace` header says what that tile cost. Anything neither
 * of them carries renders as an em dash.
 */
import type { SwathApi } from "../api.js";
import { publishedContent } from "../explain-model.js";
import { envelopeFromHeader, receiptFrom, receiptTile, tilesetBbox } from "../receipt-model.js";
import type { SwathExplainCard } from "../ui/explain-card.js";

/** Fetches the tileset document, renders its centre tile, and fills `card`.
 *
 * A failure at any step leaves the card closed rather than showing a receipt
 * with invented content: a receipt that cannot be trusted is worse than none.
 * Publishing has already succeeded by this point, so nothing here can fail
 * the publish — this is proof, not a step.
 */
export async function showReceipt(
  api: SwathApi,
  card: SwathExplainCard,
  id: string,
  origin: string,
): Promise<boolean> {
  try {
    const path = `/tilesets/${encodeURIComponent(id)}`;
    const response = await api.fetch(path, { headers: { accept: "application/json" } });
    if (!response.ok) {
      return false;
    }
    const doc = (await response.json()) as Parameters<typeof receiptFrom>[0];
    const receipt = receiptFrom(doc, id, origin);
    const tile = receiptTile(tilesetBbox(doc));
    // OGC path order is z/row/col — `{z}/{y}/{x}` — not the XYZ order the
    // tile address is written in.
    const tileResponse = await api.fetch(`${path}/tiles/${tile.z}/${tile.y}/${tile.x}`, {
      headers: { accept: "image/png" },
    });
    const envelope = envelopeFromHeader(tileResponse.headers.get("x-swath-trace"), tile, id);
    if (!envelope) {
      return false;
    }
    card.content = publishedContent(envelope, receipt);
    card.open = true;
    return true;
  } catch {
    return false;
  }
}
