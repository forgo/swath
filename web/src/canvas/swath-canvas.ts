// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-canvas>` (issue #290): the pan/zoom surface the DAG editor sits
 * on. Viewport `x` / `y` / `k`, a grid, an SVG layer for `edges` (each
 * with a 12 px hit twin), rubber-band marquee selection, `fit()`.
 * Interaction only — what may connect is the consumer's answer to
 * `swath-port-connect-end` (M11). Slotted `<swath-canvas-node>`s are
 * positioned by their canvas `x`/`y` under the viewport transform.
 *
 * Keyboard: arrows pan (Shift: nudge the selection instead), + / − / 0
 * zoom in / out / fit, Tab roves between nodes, Enter on an armed port
 * completes a connection, Esc cancels, Delete asks. Pointer: primary drag
 * on empty canvas = marquee, middle / Space+drag / one-finger touch = pan,
 * wheel = zoom around the pointer, two-finger pinch = zoom around the
 * midpoint (pointer-event pairs, no gesture events), long-press (500 ms,
 * < 8 px drift) = context. `touch-action: none`.
 */
import { el } from "../ui/dom.js";
import { SwathElement } from "../ui/element.js";
import { css } from "../ui/styles.js";
import {
  clampZoom,
  edgePath,
  fitViewport,
  intersects,
  type Point,
  type PortSide,
  portAnchor,
  type Rect,
  rectFrom,
  toCanvas,
  toScreen,
  union,
  type Viewport,
  zoomAround,
} from "./canvas-geometry.js";
import { DRAG_THRESHOLD_PX, SwathCanvasNode } from "./swath-canvas-node.js";
import { SwathCanvasPort } from "./swath-canvas-port.js";

export interface CanvasEdge {
  id: string;
  from: { node: string; port: string };
  to: { node: string; port: string };
}

interface PortRef {
  node: string;
  port: string;
  side: PortSide;
}

const SVG_NS = "http://www.w3.org/2000/svg";
const LONG_PRESS_MS = 500;
const EDGE_HIT_PX = 12;
const WHEEL_ZOOM = 1.0015;

export class SwathCanvas extends SwathElement {
  static override tagName = "swath-canvas";
  static override styles = [
    css`
      :host {
        display: block;
        position: relative;
        inline-size: 100%;
        block-size: 100%;
        overflow: hidden;
        touch-action: none;
        user-select: none;
        background-color: var(--swath-color-bg);
        background-image:
          linear-gradient(var(--swath-color-line) 1px, transparent 1px),
          linear-gradient(90deg, var(--swath-color-line) 1px, transparent 1px);
        outline: none;
      }
      :host(:focus-visible) { outline: var(--swath-border-focus); outline-offset: -2px; }
      [part="world"] { position: absolute; inset: 0; transform-origin: 0 0; }
      [part="edges"] { position: absolute; inset: 0; overflow: visible; pointer-events: none; }
      [part="edges"] path.edge { fill: none; stroke: var(--swath-color-fg-muted); stroke-width: 2; }
      [part="edges"] path.edge[data-selected] { stroke: var(--swath-color-accent); }
      [part="edges"] path.hit { fill: none; stroke: transparent; stroke-width: 12; pointer-events: stroke; cursor: pointer; }
      [part="edges"] path.pending { fill: none; stroke: var(--swath-color-accent); stroke-width: 2; stroke-dasharray: 6 4; }
      [part="marquee"] {
        position: absolute;
        border: 1px dashed var(--swath-color-accent-border);
        background: var(--swath-color-accent-bg);
        pointer-events: none;
      }
      [part="marquee"][hidden] { display: none; }
    `,
  ];
  static override properties = {
    x: { type: "number", reflect: true },
    y: { type: "number", reflect: true },
    k: { type: "number", reflect: true },
    grid: { type: "number", reflect: true },
  } as const;

  declare x: number | undefined;
  declare y: number | undefined;
  declare k: number | undefined;
  declare grid: number | undefined;

  #edges: readonly CanvasEdge[] = [];
  #world: HTMLElement | undefined;
  #svg: SVGSVGElement | undefined;
  #marquee: HTMLElement | undefined;
  #selectedNodes = new Set<string>();
  #selectedEdges = new Set<string>();
  #pointers = new Map<number, Point>();
  #gesture:
    | { kind: "pan"; last: Point }
    | { kind: "pinch"; distance: number; k: number }
    | { kind: "marquee"; origin: Point; current: Point }
    | { kind: "node"; node: SwathCanvasNode; origin: Point; start: Point; moved: boolean }
    | { kind: "connect"; from: PortRef; origin: Point; current: Point; moved: boolean }
    | undefined;
  #armed: PortRef | undefined;
  #longPress: number | undefined;
  #pressOrigin: Point | undefined;
  #spaceHeld = false;

  constructor() {
    super();
    SwathCanvasNode.define();
    SwathCanvasPort.define();
  }

  get view(): Viewport {
    return { x: this.x ?? 0, y: this.y ?? 0, k: clampZoom(this.k ?? 1) };
  }

  set view(view: Viewport) {
    this.x = view.x;
    this.y = view.y;
    this.k = clampZoom(view.k);
    this.emit("swath-canvas-change", { x: this.x, y: this.y, k: this.k });
  }

  get edges(): readonly CanvasEdge[] {
    return this.#edges;
  }

  set edges(edges: readonly CanvasEdge[]) {
    this.#edges = edges;
    this.requestUpdate();
  }

  get selection(): { nodes: string[]; edges: string[] } {
    return { nodes: [...this.#selectedNodes], edges: [...this.#selectedEdges] };
  }

  get armedPort(): PortRef | undefined {
    return this.#armed;
  }

  /** Nodes in slot order. */
  get nodes(): SwathCanvasNode[] {
    return [...this.querySelectorAll("swath-canvas-node")];
  }

  /** Screen px (container-relative) → canvas units. */
  toCanvas(screen: Point): Point {
    return toCanvas(this.view, screen);
  }

  /** The screen position of a node's port anchor. */
  portAnchor(node: string, port: string): Point | undefined {
    const element = this.#node(node);
    if (!element) {
      return undefined;
    }
    const rect = this.#nodeRect(element);
    const portEl = element.querySelector<SwathCanvasPort>(
      `swath-canvas-port[name="${CSS.escape(port)}"]`,
    );
    if (!portEl) {
      return undefined;
    }
    const side: PortSide = portEl.side === "input" ? "input" : "output";
    const siblings = [
      ...element.querySelectorAll<SwathCanvasPort>(`swath-canvas-port[side="${side}"]`),
    ];
    const anchor = portAnchor(rect, side, siblings.indexOf(portEl), siblings.length);
    return toScreen(this.view, anchor);
  }

  /** Fit every node into view. */
  fit(): void {
    const rects = this.nodes.map((node) => this.#nodeRect(node));
    const bounds = union(rects);
    if (!bounds) {
      this.view = { x: 0, y: 0, k: 1 };
      return;
    }
    this.view = fitViewport(bounds, { width: this.clientWidth, height: this.clientHeight });
  }

  zoomBy(factor: number, around?: Point): void {
    const at = around ?? { x: this.clientWidth / 2, y: this.clientHeight / 2 };
    this.view = zoomAround(this.view, factor, at);
  }

  select(nodes: readonly string[], edges: readonly string[] = []): void {
    this.#selectedNodes = new Set(nodes);
    this.#selectedEdges = new Set(edges);
    for (const node of this.nodes) {
      node.selected = this.#selectedNodes.has(node.nodeId ?? "");
    }
    this.emit("swath-canvas-select", this.selection);
    this.requestUpdate();
  }

  /** Cancel an armed / dragging connection. */
  cancelConnect(): void {
    if (this.#armed) {
      this.#port(this.#armed)?.removeAttribute("armed");
      this.#armed = undefined;
    }
    if (this.#gesture?.kind === "connect") {
      this.#gesture = undefined;
    }
    this.requestUpdate();
  }

  #node(id: string): SwathCanvasNode | undefined {
    return (
      this.querySelector<SwathCanvasNode>(`swath-canvas-node[node-id="${CSS.escape(id)}"]`) ??
      undefined
    );
  }

  #port(ref: PortRef): SwathCanvasPort | undefined {
    return (
      this.#node(ref.node)?.querySelector<SwathCanvasPort>(
        `swath-canvas-port[name="${CSS.escape(ref.port)}"]`,
      ) ?? undefined
    );
  }

  /** A node's rect in canvas units (size measured on screen, unscaled). */
  #nodeRect(node: SwathCanvasNode): Rect {
    return {
      x: node.x ?? 0,
      y: node.y ?? 0,
      width: node.offsetWidth || 160,
      height: node.offsetHeight || 64,
    };
  }

  #screenPoint(event: PointerEvent | MouseEvent | WheelEvent): Point {
    const rect = this.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  override connectedCallback(): void {
    super.connectedCallback();
    this.render();
    this.tabIndex = this.tabIndex < 0 ? 0 : this.tabIndex;
    this.addEventListener("pointerdown", this.#onPointerDown, { signal: this.disconnected });
    this.addEventListener("pointermove", this.#onPointerMove, { signal: this.disconnected });
    this.addEventListener("pointerup", this.#onPointerUp, { signal: this.disconnected });
    this.addEventListener("pointercancel", this.#onPointerUp, { signal: this.disconnected });
    this.addEventListener("wheel", this.#onWheel, { signal: this.disconnected, passive: false });
    this.addEventListener("keydown", this.#onKeyDown, { signal: this.disconnected });
    this.addEventListener("keyup", this.#onKeyUp, { signal: this.disconnected });
    this.addEventListener("contextmenu", this.#onContextMenu, { signal: this.disconnected });
    this.addEventListener("swath-port-connect-start", this.#onConnectStart, {
      signal: this.disconnected,
    });
    this.addEventListener("swath-port-tap", this.#onPortTap, { signal: this.disconnected });
    this.addEventListener("swath-node-move", () => this.requestUpdate(), {
      signal: this.disconnected,
    });
    this.addEventListener("slotchange", () => this.requestUpdate(), { signal: this.disconnected });
  }

  readonly #onContextMenu = (event: MouseEvent): void => {
    event.preventDefault();
    const at = this.#screenPoint(event);
    const node = (event.target as Element | null)?.closest?.(
      "swath-canvas-node",
    ) as SwathCanvasNode | null;
    this.emit("swath-canvas-context", { node: node?.nodeId ?? null, ...this.toCanvas(at) });
  };

  readonly #onConnectStart = (event: CustomEvent<PortRef>): void => {
    this.#pointers.clear();
    const anchor = this.portAnchor(event.detail.node, event.detail.port) ?? { x: 0, y: 0 };
    this.#gesture = {
      kind: "connect",
      from: event.detail,
      origin: anchor,
      current: anchor,
      moved: false,
    };
    this.requestUpdate();
  };

  readonly #onPortTap = (event: Event): void => {
    const port = (event.target as Element).closest("swath-canvas-port") as SwathCanvasPort | null;
    if (port) {
      this.#tap(port.ref);
    }
  };

  /** Tap-to-connect: the first tap arms a port, the second completes to
   * the tapped one. Reached from a press that never moved (pointerup —
   * touch or mouse) and from the port's non-pointer activations (Enter,
   * a synthetic click). Never from a pointer `click`: after a touch pan
   * Chromium on Linux swallows the next tap's click while its pointer
   * events still arrive, which left the first port unarmed in CI. */
  #tap(ref: PortRef): void {
    if (!this.#armed) {
      this.#armed = ref;
      const port = this.#port(ref);
      if (port) {
        port.armed = true;
      }
      this.emit("swath-port-connect-start", ref);
    } else {
      const from = this.#armed;
      this.cancelConnect();
      this.emit("swath-port-connect-end", { from, to: ref });
    }
  }

  readonly #onPointerDown = (event: PointerEvent): void => {
    const at = this.#screenPoint(event);
    this.#pointers.set(event.pointerId, at);
    try {
      this.setPointerCapture(event.pointerId);
    } catch {
      // A synthetic event has no active pointer to capture; the gesture
      // still works through the bubbling move/up on the canvas.
    }
    if (this.#gesture?.kind === "connect") {
      return; // the port started it; we track the pointer in move/up
    }
    if (this.#pointers.size === 2) {
      const [a, b] = [...this.#pointers.values()];
      this.#gesture = {
        kind: "pinch",
        distance: Math.hypot((a as Point).x - (b as Point).x, (a as Point).y - (b as Point).y),
        k: this.view.k,
      };
      this.#clearLongPress();
      return;
    }
    const nodeEl = (event.target as Element | null)?.closest?.(
      "swath-canvas-node",
    ) as SwathCanvasNode | null;
    const pan = event.button === 1 || this.#spaceHeld || (event.pointerType === "touch" && !nodeEl);
    if (pan) {
      this.#gesture = { kind: "pan", last: at };
    } else if (nodeEl && event.button === 0) {
      this.#gesture = {
        kind: "node",
        node: nodeEl,
        origin: at,
        start: { x: nodeEl.x ?? 0, y: nodeEl.y ?? 0 },
        moved: false,
      };
    } else if (event.button === 0) {
      this.#gesture = { kind: "marquee", origin: at, current: at };
    }
    if (event.pointerType !== "mouse") {
      this.#pressOrigin = at;
      this.#longPress = window.setTimeout(() => {
        this.#longPress = undefined;
        this.#gesture = undefined;
        this.emit("swath-canvas-context", { node: nodeEl?.nodeId ?? null, ...this.toCanvas(at) });
      }, LONG_PRESS_MS);
    }
  };

  #clearLongPress(): void {
    if (this.#longPress !== undefined) {
      window.clearTimeout(this.#longPress);
      this.#longPress = undefined;
    }
  }

  readonly #onPointerMove = (event: PointerEvent): void => {
    const at = this.#screenPoint(event);
    if (this.#pointers.has(event.pointerId)) {
      this.#pointers.set(event.pointerId, at);
    }
    if (
      this.#pressOrigin &&
      Math.hypot(at.x - this.#pressOrigin.x, at.y - this.#pressOrigin.y) >= DRAG_THRESHOLD_PX
    ) {
      this.#clearLongPress();
    }
    const g = this.#gesture;
    if (!g) {
      return;
    }
    switch (g.kind) {
      case "pan": {
        const view = this.view;
        this.view = {
          x: view.x + (at.x - g.last.x) / view.k,
          y: view.y + (at.y - g.last.y) / view.k,
          k: view.k,
        };
        g.last = at;
        break;
      }
      case "pinch": {
        if (this.#pointers.size < 2) {
          return;
        }
        const [a, b] = [...this.#pointers.values()] as [Point, Point];
        const distance = Math.hypot(a.x - b.x, a.y - b.y);
        const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
        const target = clampZoom(g.k * (distance / g.distance));
        this.view = zoomAround(this.view, target / this.view.k, mid);
        break;
      }
      case "marquee":
        g.current = at;
        this.#paintMarquee(rectFrom(g.origin, g.current));
        break;
      case "node": {
        const dx = at.x - g.origin.x;
        const dy = at.y - g.origin.y;
        if (!g.moved && Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) {
          return;
        }
        g.moved = true;
        g.node.dragging = true;
        const k = this.view.k;
        g.node.x = Math.round(g.start.x + dx / k);
        g.node.y = Math.round(g.start.y + dy / k);
        this.requestUpdate();
        break;
      }
      case "connect":
        g.current = at;
        if (Math.hypot(at.x - g.origin.x, at.y - g.origin.y) >= DRAG_THRESHOLD_PX) {
          g.moved = true;
        }
        this.requestUpdate();
        break;
    }
  };

  readonly #onPointerUp = (event: PointerEvent): void => {
    const at = this.#screenPoint(event);
    this.#pointers.delete(event.pointerId);
    const g = this.#gesture;
    const pressed = this.#longPress !== undefined;
    this.#clearLongPress();
    this.#pressOrigin = undefined;
    if (!g) {
      return;
    }
    if (g.kind === "pinch") {
      if (this.#pointers.size === 0) {
        this.#gesture = undefined;
      }
      return;
    }
    this.#gesture = undefined;
    switch (g.kind) {
      case "marquee": {
        const box = rectFrom(g.origin, at);
        if (box.width < DRAG_THRESHOLD_PX && box.height < DRAG_THRESHOLD_PX) {
          if (this.#armed) {
            this.cancelConnect();
          }
          this.select([]);
        } else {
          const a = this.toCanvas({ x: box.x, y: box.y });
          const b = this.toCanvas({ x: box.x + box.width, y: box.y + box.height });
          const world = rectFrom(a, b);
          this.select(
            this.nodes
              .filter((n) => intersects(world, this.#nodeRect(n)))
              .map((n) => n.nodeId ?? ""),
          );
        }
        this.#paintMarquee(undefined);
        break;
      }
      case "node": {
        g.node.dragging = false;
        if (g.moved) {
          this.emit("swath-node-move", {
            id: g.node.nodeId ?? "",
            x: g.node.x ?? 0,
            y: g.node.y ?? 0,
          });
        } else if (pressed || event.pointerType === "mouse") {
          const id = g.node.nodeId ?? "";
          if (event.shiftKey) {
            const next = new Set(this.#selectedNodes);
            next.has(id) ? next.delete(id) : next.add(id);
            this.select([...next]);
          } else {
            this.select([id]);
          }
          g.node.focus();
        }
        break;
      }
      case "connect": {
        const from = g.from;
        this.requestUpdate();
        // A press that never moved is a tap (touch, or a click).
        if (!g.moved) {
          this.#tap(from);
          break;
        }
        const under = document
          .elementsFromPoint(event.clientX, event.clientY)
          .find((e) => e.closest("swath-canvas-port")) as Element | undefined;
        const portEl = under?.closest("swath-canvas-port") as SwathCanvasPort | null;
        const to = portEl ? portEl.ref : null;
        if (to && to.node === from.node && to.port === from.port) {
          break; // released on itself: nothing to connect
        }
        this.emit("swath-port-connect-end", { from, to });
        break;
      }
      default:
        break;
    }
  };

  readonly #onWheel = (event: WheelEvent): void => {
    event.preventDefault();
    const factor = WHEEL_ZOOM ** -event.deltaY;
    this.zoomBy(factor, this.#screenPoint(event));
  };

  readonly #onKeyUp = (event: KeyboardEvent): void => {
    if (event.key === " ") {
      this.#spaceHeld = false;
    }
  };

  readonly #onKeyDown = (event: KeyboardEvent): void => {
    if (event.key === " ") {
      this.#spaceHeld = true;
      if (event.target === this) {
        event.preventDefault();
      }
      return;
    }
    if (event.key === "Escape") {
      if (this.#armed || this.#gesture?.kind === "connect") {
        this.cancelConnect();
      } else {
        this.select([]);
      }
      event.preventDefault();
      return;
    }
    if (event.key === "Tab" && this.nodes.length > 0) {
      const focused = this.nodes.findIndex(
        (n) => n.contains(document.activeElement) || n.shadowRoot?.activeElement,
      );
      const next = (focused + (event.shiftKey ? -1 : 1) + this.nodes.length) % this.nodes.length;
      if (focused !== -1 || event.target === this) {
        event.preventDefault();
        this.nodes[next]?.focus();
      }
      return;
    }
    if (event.target !== this) {
      return; // node keys are the node's
    }
    const pan = 40 / this.view.k;
    switch (event.key) {
      case "ArrowLeft":
        this.view = { ...this.view, x: this.view.x + pan };
        break;
      case "ArrowRight":
        this.view = { ...this.view, x: this.view.x - pan };
        break;
      case "ArrowUp":
        this.view = { ...this.view, y: this.view.y + pan };
        break;
      case "ArrowDown":
        this.view = { ...this.view, y: this.view.y - pan };
        break;
      case "+":
      case "=":
        this.zoomBy(1.25);
        break;
      case "-":
        this.zoomBy(0.8);
        break;
      case "0":
        // biome-ignore lint/suspicious/noFocusedTests: `fit()` is the canvas API (ui-system.md §5), not a test focus
        this.fit();
        break;
      case "Delete":
      case "Backspace":
        if (this.#selectedNodes.size > 0 || this.#selectedEdges.size > 0) {
          this.emit("swath-delete-request", this.selection);
        }
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  #paintMarquee(box: Rect | undefined): void {
    const marquee = this.#marquee;
    if (!marquee) {
      return;
    }
    if (!box) {
      marquee.hidden = true;
      return;
    }
    marquee.hidden = false;
    marquee.style.left = `${box.x}px`;
    marquee.style.top = `${box.y}px`;
    marquee.style.width = `${box.width}px`;
    marquee.style.height = `${box.height}px`;
  }

  protected render(): void {
    if (!this.#world) {
      this.#svg = document.createElementNS(SVG_NS, "svg");
      this.#svg.setAttribute("part", "edges");
      this.#world = el("div", { part: "world" }, el("slot"));
      this.#marquee = el("div", { part: "marquee", hidden: true });
      this.renderRoot.replaceChildren(this.#svg, this.#world, this.#marquee);
    }
    const view = this.view;
    const grid = (this.grid ?? 24) * view.k;
    this.style.backgroundSize = `${grid}px ${grid}px`;
    this.style.backgroundPosition = `${view.x * view.k}px ${view.y * view.k}px`;
    this.#world.style.transform = `scale(${view.k}) translate(${view.x}px, ${view.y}px)`;
    for (const node of this.nodes) {
      node.style.left = `${node.x ?? 0}px`;
      node.style.top = `${node.y ?? 0}px`;
    }
    const svg = this.#svg as SVGSVGElement;
    const children: SVGElement[] = [];
    for (const edge of this.#edges) {
      const from = this.portAnchor(edge.from.node, edge.from.port);
      const to = this.portAnchor(edge.to.node, edge.to.port);
      if (!from || !to) {
        continue;
      }
      const d = edgePath(from, to);
      const line = document.createElementNS(SVG_NS, "path");
      line.setAttribute("class", "edge");
      line.setAttribute("d", d);
      line.dataset["edge"] = edge.id;
      if (this.#selectedEdges.has(edge.id)) {
        line.dataset["selected"] = "";
      }
      const hit = document.createElementNS(SVG_NS, "path");
      hit.setAttribute("class", "hit");
      hit.setAttribute("d", d);
      hit.dataset["edge"] = edge.id;
      hit.setAttribute("stroke-width", String(EDGE_HIT_PX));
      hit.addEventListener("pointerdown", (event) => {
        event.stopPropagation();
        this.select([], [edge.id]);
      });
      children.push(line, hit);
    }
    const g = this.#gesture;
    if (g?.kind === "connect") {
      const from = this.portAnchor(g.from.node, g.from.port);
      if (from) {
        const pending = document.createElementNS(SVG_NS, "path");
        pending.setAttribute("class", "pending");
        pending.setAttribute("d", edgePath(from, g.current));
        children.push(pending);
      }
    }
    svg.replaceChildren(...children);
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-canvas": SwathCanvas;
  }
}
