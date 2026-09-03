// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** The authoring panel's outcomes routed to the map (issue #109), lifted
 * out of the entry script (#355). */
import type { SwathAuthoringPanel } from "../swath-authoring-panel";
import type { SwathMap } from "../swath-map";
import type { SwathExplainCard } from "../ui/explain-card.js";
import { showReceipt } from "./receipt.js";

// The authoring panel (issue #109) is a pure openEO client; the shell
// only routes its outcomes to the map. A created service becomes the
// viewed layer (the switch refetches /tilesets, so the layer browser
// lists it immediately — no reload); a deleted one falls back to the
// server's default layer when it was the viewed one, else just refreshes
// the layer list.
export function wireAuthoring(
  map: SwathMap,
  authoring: SwathAuthoringPanel,
  receipt?: SwathExplainCard | null,
): void {
  authoring.addEventListener("swath-service-created", (event) => {
    const id = event.detail.id;
    map.setLayer(id).catch(() => undefined);
    // The receipt is proof, not a step (#395): publishing has already
    // succeeded, so a failure to render it leaves the card closed and
    // changes nothing else.
    if (receipt) {
      void showReceipt(map.api, receipt, id, window.location.origin);
    }
  });
  authoring.addEventListener("swath-service-deleted", (event) => {
    onServiceDeleted(map, event.detail.id);
  });
}

/** A service is gone (deleted from the authoring panel or a layer row's
 * kebab): the viewed layer falls back to the server default, any other
 * just refreshes the list. */
export function onServiceDeleted(map: SwathMap, id: string): void {
  if (map.getAttribute("layer") === id) {
    map.removeAttribute("layer"); // re-applies with the server default
  } else {
    map.refresh();
  }
}
