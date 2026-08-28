// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The canvas fixture's wiring (issue #290): one edge to start with, and a
// consumer that ACCEPTS every connection — the primitives carry no port
// typing; M11's graph model answers `swath-port-connect-end` for real.
import { SwathCanvas } from "../src/canvas/swath-canvas.js";

SwathCanvas.define();
const canvas = document.querySelector("swath-canvas");
const log = document.querySelector("#log");
const say = (line: string): void => {
  if (log) {
    log.textContent = `${line}\n${log.textContent ?? ""}`.slice(0, 4000);
  }
};
if (canvas instanceof SwathCanvas) {
  let edges = [
    { id: "e1", from: { node: "load", port: "data" }, to: { node: "ndvi", port: "data" } },
  ];
  canvas.edges = edges;
  canvas.addEventListener("swath-port-connect-end", (event) => {
    const { from, to } = event.detail;
    say(`connect ${from.node}.${from.port} → ${to ? `${to.node}.${to.port}` : "nothing"}`);
    if (to && to.node !== from.node) {
      edges = [
        ...edges,
        {
          id: `e${edges.length + 1}`,
          from: { node: from.node, port: from.port },
          to: { node: to.node, port: to.port },
        },
      ];
      canvas.edges = edges;
    }
  });
  canvas.addEventListener("swath-node-move", (event) =>
    say(`move ${event.detail.id} → ${event.detail.x},${event.detail.y}`),
  );
  canvas.addEventListener("swath-canvas-select", (event) =>
    say(`select ${JSON.stringify(event.detail)}`),
  );
  canvas.addEventListener("swath-delete-request", (event) => {
    say(`delete ${JSON.stringify(event.detail)}`);
    edges = edges.filter((e) => !event.detail.edges.includes(e.id));
    canvas.edges = edges;
  });
  canvas.addEventListener("swath-canvas-context", (event) =>
    say(`context ${event.detail.node ?? "canvas"}`),
  );
  canvas.addEventListener("swath-node-activate", (event) => say(`activate ${event.detail.id}`));
  canvas.addEventListener("swath-canvas-change", (event) => {
    canvas.dataset["view"] =
      `${event.detail.x.toFixed(1)},${event.detail.y.toFixed(1)},${event.detail.k.toFixed(2)}`;
  });
  requestAnimationFrame(() => canvas.fit());
}
