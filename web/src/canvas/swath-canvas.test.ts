// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The canvas interaction suite (issue #290): pointer, keyboard and touch
// gestures over a real DOM — no graph semantics, the consumer answers
// swath-port-connect-end.
import { afterEach, beforeAll, expect, test } from "vitest";
import { userEvent } from "vitest/browser";
import { SwathCanvas } from "./swath-canvas.js";
import type { SwathCanvasNode } from "./swath-canvas-node.js";
import type { SwathCanvasPort } from "./swath-canvas-port.js";

beforeAll(() => {
  SwathCanvas.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

const FIXTURE = `
<swath-canvas style="width:800px;height:600px">
  <swath-canvas-node node-id="load" title="Load" x="40" y="40">
    <swath-canvas-port slot="outputs" side="output" name="data" label="data"></swath-canvas-port>
  </swath-canvas-node>
  <swath-canvas-node node-id="ndvi" title="NDVI" x="360" y="120">
    <swath-canvas-port slot="inputs" side="input" name="data" label="data"></swath-canvas-port>
    <swath-canvas-port slot="outputs" side="output" name="out" label="out"></swath-canvas-port>
  </swath-canvas-node>
</swath-canvas>`;

async function mount(): Promise<SwathCanvas> {
  const host = document.createElement("div");
  host.innerHTML = FIXTURE;
  document.body.append(host);
  const canvas = host.querySelector("swath-canvas") as SwathCanvas;
  await canvas.updateComplete;
  for (const node of canvas.nodes) {
    await node.updateComplete;
  }
  return canvas;
}

const node = (canvas: SwathCanvas, id: string): SwathCanvasNode =>
  canvas.querySelector(`swath-canvas-node[node-id="${id}"]`) as SwathCanvasNode;
const port = (canvas: SwathCanvas, id: string, name: string): SwathCanvasPort =>
  node(canvas, id).querySelector(`swath-canvas-port[name="${name}"]`) as SwathCanvasPort;

function pointer(
  target: Element,
  type: string,
  init: {
    x: number;
    y: number;
    id?: number;
    pointerType?: string;
    button?: number;
    shiftKey?: boolean;
  },
): void {
  target.dispatchEvent(
    new PointerEvent(type, {
      bubbles: true,
      composed: true,
      cancelable: true,
      pointerId: init.id ?? 1,
      pointerType: init.pointerType ?? "mouse",
      isPrimary: true,
      button: init.button ?? 0,
      clientX: init.x,
      clientY: init.y,
      shiftKey: init.shiftKey ?? false,
    }),
  );
}

const origin = (canvas: SwathCanvas) => canvas.getBoundingClientRect();

test("nodes sit at their canvas x/y under the viewport transform; fit() frames them all", async () => {
  const canvas = await mount();
  expect(node(canvas, "load").style.left).toBe("40px");
  canvas.view = { x: 10, y: 20, k: 2 };
  await canvas.updateComplete;
  const world = canvas.shadowRoot?.querySelector<HTMLElement>('[part="world"]');
  expect(world?.style.transform).toBe("scale(2) translate(10px, 20px)");
  const changes: number[] = [];
  canvas.addEventListener("swath-canvas-change", (e) => changes.push(e.detail.k));
  canvas.fit();
  await canvas.updateComplete;
  expect(canvas.view.k).toBeLessThanOrEqual(1);
  expect(changes).toHaveLength(1);
  const load = node(canvas, "load").getBoundingClientRect();
  const ndvi = node(canvas, "ndvi").getBoundingClientRect();
  const box = origin(canvas);
  expect(load.left).toBeGreaterThanOrEqual(box.left);
  expect(ndvi.right).toBeLessThanOrEqual(box.right);
});

test("edges: an SVG path per edge with a 12px hit twin; clicking the twin selects the edge", async () => {
  const canvas = await mount();
  canvas.edges = [
    { id: "e1", from: { node: "load", port: "data" }, to: { node: "ndvi", port: "data" } },
  ];
  await canvas.updateComplete;
  const svg = canvas.shadowRoot?.querySelector('[part="edges"]');
  expect(svg?.querySelectorAll("path.edge")).toHaveLength(1);
  const hit = svg?.querySelector<SVGPathElement>("path.hit");
  expect(hit?.getAttribute("stroke-width")).toBe("12");
  const selections: { nodes: string[]; edges: string[] }[] = [];
  canvas.addEventListener("swath-canvas-select", (e) => selections.push(e.detail));
  hit?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, composed: true, button: 0 }));
  expect(selections).toEqual([{ nodes: [], edges: ["e1"] }]);
  await canvas.updateComplete;
  expect(svg?.querySelector("path.edge")?.hasAttribute("data-selected")).toBe(true);
});

test("node drag: below 8px it is a click (select + focus); beyond it moves in canvas units and reports on release", async () => {
  const canvas = await mount();
  const box = origin(canvas);
  const header = node(canvas, "load").shadowRoot?.querySelector('[part="header"]') as HTMLElement;
  const at = header.getBoundingClientRect();
  const moves: { id: string; x: number; y: number }[] = [];
  canvas.addEventListener("swath-node-move", (e) => moves.push(e.detail));
  pointer(header, "pointerdown", { x: at.left + 5, y: at.top + 5 });
  pointer(header, "pointermove", { x: at.left + 9, y: at.top + 5 });
  pointer(header, "pointerup", { x: at.left + 9, y: at.top + 5 });
  expect(moves).toEqual([]);
  expect(canvas.selection.nodes).toEqual(["load"]);
  expect(node(canvas, "load").shadowRoot?.activeElement).not.toBeNull();

  canvas.view = { x: 0, y: 0, k: 2 };
  await canvas.updateComplete;
  const at2 = header.getBoundingClientRect();
  pointer(header, "pointerdown", { x: at2.left + 5, y: at2.top + 5 });
  pointer(header, "pointermove", { x: at2.left + 45, y: at2.top + 25 });
  expect(node(canvas, "load").dragging).toBe(true);
  pointer(header, "pointerup", { x: at2.left + 45, y: at2.top + 25 });
  expect(node(canvas, "load").x).toBe(60); // 40 screen px at k=2 → 20 units
  expect(node(canvas, "load").y).toBe(50);
  expect(moves).toEqual([{ id: "load", x: 60, y: 50 }]);
  expect(node(canvas, "load").dragging).toBe(false);
  expect(box.width).toBe(800);
});

test("marquee on empty canvas selects intersecting nodes; a tiny drag clears", async () => {
  const canvas = await mount();
  const box = origin(canvas);
  pointer(canvas, "pointerdown", { x: box.left + 10, y: box.top + 10 });
  pointer(canvas, "pointermove", { x: box.left + 300, y: box.top + 300 });
  const marquee = canvas.shadowRoot?.querySelector<HTMLElement>('[part="marquee"]');
  expect(marquee?.hidden).toBe(false);
  pointer(canvas, "pointerup", { x: box.left + 300, y: box.top + 300 });
  expect(canvas.selection.nodes).toEqual(["load"]);
  expect(marquee?.hidden).toBe(true);
  pointer(canvas, "pointerdown", { x: box.left + 700, y: box.top + 500 });
  pointer(canvas, "pointerup", { x: box.left + 702, y: box.top + 501 });
  expect(canvas.selection.nodes).toEqual([]);
});

test("keyboard: arrows pan, +/−/0 zoom and fit, Tab roves nodes, Delete asks, Esc clears", async () => {
  const canvas = await mount();
  canvas.focus();
  const before = canvas.view;
  await userEvent.keyboard("{ArrowRight}");
  expect(canvas.view.x).toBeLessThan(before.x);
  await userEvent.keyboard("+");
  expect(canvas.view.k).toBeCloseTo(1.25);
  await userEvent.keyboard("-");
  expect(canvas.view.k).toBeCloseTo(1);
  canvas.view = { x: 300, y: 300, k: 3 };
  await userEvent.keyboard("0");
  expect(canvas.view.k).toBeLessThanOrEqual(1);
  await userEvent.keyboard("{Tab}");
  expect(node(canvas, "load").shadowRoot?.activeElement).not.toBeNull();
  await userEvent.keyboard("{Tab}");
  expect(node(canvas, "ndvi").shadowRoot?.activeElement).not.toBeNull();
  // Nudge from the node, Delete asks for the selection.
  const moves: { id: string; x: number }[] = [];
  canvas.addEventListener("swath-node-move", (e) => moves.push(e.detail));
  await userEvent.keyboard("{Shift>}{ArrowRight}{/Shift}");
  expect(moves).toEqual([{ id: "ndvi", x: 370, y: 120 }]);
  const asks: { nodes: string[] }[] = [];
  canvas.addEventListener("swath-delete-request", (e) => asks.push(e.detail));
  await userEvent.keyboard("{Delete}");
  expect(asks).toEqual([{ nodes: ["ndvi"], edges: [] }]);
  canvas.select(["load"]);
  canvas.focus();
  await userEvent.keyboard("{Escape}");
  expect(canvas.selection.nodes).toEqual([]);
});

test("tap-to-connect: first tap arms, second completes with swath-port-connect-end; Esc cancels", async () => {
  const canvas = await mount();
  const events: string[] = [];
  canvas.addEventListener("swath-port-connect-start", (e) =>
    events.push(`start:${e.detail.node}.${e.detail.port}`),
  );
  canvas.addEventListener("swath-port-connect-end", (e) =>
    events.push(
      `end:${e.detail.from.node}→${e.detail.to?.node ?? "null"}.${e.detail.to?.port ?? ""}`,
    ),
  );
  const from = port(canvas, "load", "data");
  const to = port(canvas, "ndvi", "data");
  from.shadowRoot?.querySelector("button")?.click();
  expect(from.armed).toBe(true);
  expect(canvas.armedPort?.node).toBe("load");
  canvas.focus();
  await userEvent.keyboard("{Escape}");
  expect(from.armed).toBe(false);
  expect(canvas.armedPort).toBeUndefined();
  from.shadowRoot?.querySelector("button")?.click();
  to.shadowRoot?.querySelector("button")?.click();
  expect(events).toEqual(["start:load.data", "start:load.data", "end:load→ndvi.data"]);
  expect(canvas.armedPort).toBeUndefined();
});

test("drag-to-connect: pointer down on a port draws a pending edge; release over a port ends with it, elsewhere with null", async () => {
  const canvas = await mount();
  const ends: (string | null)[] = [];
  canvas.addEventListener("swath-port-connect-end", (e) =>
    ends.push(e.detail.to ? `${e.detail.to.node}.${e.detail.to.port}` : null),
  );
  const from = port(canvas, "load", "data").shadowRoot?.querySelector("button") as HTMLElement;
  const to = port(canvas, "ndvi", "data").shadowRoot?.querySelector("button") as HTMLElement;
  const a = from.getBoundingClientRect();
  const b = to.getBoundingClientRect();
  pointer(from, "pointerdown", { x: a.left + 5, y: a.top + 5 });
  pointer(canvas, "pointermove", { x: b.left + 5, y: b.top + 5 });
  await canvas.updateComplete;
  expect(canvas.shadowRoot?.querySelector("path.pending")).not.toBeNull();
  pointer(canvas, "pointerup", { x: b.left + 5, y: b.top + 5 });
  expect(ends).toEqual(["ndvi.data"]);
  const box = origin(canvas);
  pointer(from, "pointerdown", { x: a.left + 5, y: a.top + 5 });
  pointer(canvas, "pointerup", { x: box.left + 700, y: box.top + 550 });
  expect(ends).toEqual(["ndvi.data", null]);
  await canvas.updateComplete;
  expect(canvas.shadowRoot?.querySelector("path.pending")).toBeNull();
});

test("touch: one finger pans, two fingers pinch around the midpoint, a long-press asks for context", async () => {
  const canvas = await mount();
  const box = origin(canvas);
  pointer(canvas, "pointerdown", {
    x: box.left + 100,
    y: box.top + 100,
    id: 1,
    pointerType: "touch",
  });
  pointer(canvas, "pointermove", {
    x: box.left + 150,
    y: box.top + 130,
    id: 1,
    pointerType: "touch",
  });
  pointer(canvas, "pointerup", {
    x: box.left + 150,
    y: box.top + 130,
    id: 1,
    pointerType: "touch",
  });
  expect(canvas.view).toMatchObject({ x: 50, y: 30, k: 1 });

  canvas.view = { x: 0, y: 0, k: 1 };
  pointer(canvas, "pointerdown", {
    x: box.left + 200,
    y: box.top + 300,
    id: 1,
    pointerType: "touch",
  });
  pointer(canvas, "pointerdown", {
    x: box.left + 400,
    y: box.top + 300,
    id: 2,
    pointerType: "touch",
  });
  pointer(canvas, "pointermove", {
    x: box.left + 100,
    y: box.top + 300,
    id: 1,
    pointerType: "touch",
  });
  // Each step zooms around the fingers' CURRENT midpoint: the content under
  // it before the step is still under it after.
  const under = canvas.toCanvas({ x: 300, y: 300 }); // step 2's midpoint, before step 2
  pointer(canvas, "pointermove", {
    x: box.left + 500,
    y: box.top + 300,
    id: 2,
    pointerType: "touch",
  });
  expect(canvas.view.k).toBeCloseTo(2);
  const after = canvas.toCanvas({ x: 300, y: 300 });
  expect(after.x).toBeCloseTo(under.x, 5);
  expect(after.y).toBeCloseTo(under.y, 5);
  pointer(canvas, "pointerup", {
    x: box.left + 100,
    y: box.top + 300,
    id: 1,
    pointerType: "touch",
  });
  pointer(canvas, "pointerup", {
    x: box.left + 500,
    y: box.top + 300,
    id: 2,
    pointerType: "touch",
  });

  const contexts: { node: string | null }[] = [];
  canvas.addEventListener("swath-canvas-context", (e) => contexts.push({ node: e.detail.node }));
  pointer(canvas, "pointerdown", {
    x: box.left + 600,
    y: box.top + 500,
    id: 3,
    pointerType: "touch",
  });
  pointer(canvas, "pointermove", {
    x: box.left + 603,
    y: box.top + 502,
    id: 3,
    pointerType: "touch",
  }); // < 8px drift
  await new Promise((r) => setTimeout(r, 560));
  expect(contexts).toEqual([{ node: null }]);
  pointer(canvas, "pointerup", {
    x: box.left + 603,
    y: box.top + 502,
    id: 3,
    pointerType: "touch",
  });
});
