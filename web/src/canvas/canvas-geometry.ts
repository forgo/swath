// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure geometry for the DAG canvas (issue #290): the viewport transform,
 * port anchors, cubic edge paths, and hit tests. No DOM, no graph
 * semantics — M11's design note owns what may connect to what.
 */

/** The viewport: canvas = screen / k − (x, y) … i.e. screen = (c + t) · k. */
export interface Viewport {
  /** Pan, in canvas units. */
  x: number;
  y: number;
  /** Zoom factor (1 = 1 canvas unit per CSS px). */
  k: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export const MIN_ZOOM = 0.25;
export const MAX_ZOOM = 4;

export function clampZoom(k: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, k));
}

/** Screen (container-relative CSS px) → canvas units. */
export function toCanvas(view: Viewport, screen: Point): Point {
  return { x: screen.x / view.k - view.x, y: screen.y / view.k - view.y };
}

/** Canvas units → screen px. */
export function toScreen(view: Viewport, canvas: Point): Point {
  return { x: (canvas.x + view.x) * view.k, y: (canvas.y + view.y) * view.k };
}

/** Zoom by `factor` keeping the screen point `around` fixed. */
export function zoomAround(view: Viewport, factor: number, around: Point): Viewport {
  const k = clampZoom(view.k * factor);
  const before = toCanvas(view, around);
  const next = { x: view.x, y: view.y, k };
  const after = toCanvas(next, around);
  return { x: view.x + (after.x - before.x), y: view.y + (after.y - before.y), k };
}

/** The viewport that shows `bounds` (canvas units) inside a `size` px
 * container with `padding` px around, centred; a degenerate bounds
 * yields zoom 1 centred on its point. */
export function fitViewport(
  bounds: Rect,
  size: { width: number; height: number },
  padding = 24,
): Viewport {
  const width = Math.max(bounds.width, 1);
  const height = Math.max(bounds.height, 1);
  const k = clampZoom(
    Math.min((size.width - 2 * padding) / width, (size.height - 2 * padding) / height, 1),
  );
  const cx = bounds.x + bounds.width / 2;
  const cy = bounds.y + bounds.height / 2;
  return { x: size.width / (2 * k) - cx, y: size.height / (2 * k) - cy, k };
}

/** The bounding rect of several rects (`undefined` for none). */
export function union(rects: readonly Rect[]): Rect | undefined {
  if (rects.length === 0) {
    return undefined;
  }
  let x1 = Number.POSITIVE_INFINITY;
  let y1 = Number.POSITIVE_INFINITY;
  let x2 = Number.NEGATIVE_INFINITY;
  let y2 = Number.NEGATIVE_INFINITY;
  for (const r of rects) {
    x1 = Math.min(x1, r.x);
    y1 = Math.min(y1, r.y);
    x2 = Math.max(x2, r.x + r.width);
    y2 = Math.max(y2, r.y + r.height);
  }
  return { x: x1, y: y1, width: x2 - x1, height: y2 - y1 };
}

export type PortSide = "input" | "output";

/** Where an edge attaches: the middle of the node's left (input) or
 * right (output) edge at the port's row. */
export function portAnchor(node: Rect, side: PortSide, index: number, count: number): Point {
  const step = node.height / (count + 1);
  return { x: side === "input" ? node.x : node.x + node.width, y: node.y + step * (index + 1) };
}

/** An S-shaped cubic between two anchors (SVG path data). */
export function edgePath(from: Point, to: Point): string {
  const dx = Math.max(Math.abs(to.x - from.x) / 2, 24);
  return `M ${from.x} ${from.y} C ${from.x + dx} ${from.y}, ${to.x - dx} ${to.y}, ${to.x} ${to.y}`;
}

/** Points along the cubic at `steps` samples (for hit tests without the DOM). */
export function sampleEdge(from: Point, to: Point, steps = 16): Point[] {
  const dx = Math.max(Math.abs(to.x - from.x) / 2, 24);
  const c1 = { x: from.x + dx, y: from.y };
  const c2 = { x: to.x - dx, y: to.y };
  const out: Point[] = [];
  for (let i = 0; i <= steps; i += 1) {
    const t = i / steps;
    const u = 1 - t;
    out.push({
      x: u * u * u * from.x + 3 * u * u * t * c1.x + 3 * u * t * t * c2.x + t * t * t * to.x,
      y: u * u * u * from.y + 3 * u * u * t * c1.y + 3 * u * t * t * c2.y + t * t * t * to.y,
    });
  }
  return out;
}

function distanceToSegment(p: Point, a: Point, b: Point): number {
  const vx = b.x - a.x;
  const vy = b.y - a.y;
  const len2 = vx * vx + vy * vy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((p.x - a.x) * vx + (p.y - a.y) * vy) / len2));
  const qx = a.x + t * vx;
  const qy = a.y + t * vy;
  return Math.hypot(p.x - qx, p.y - qy);
}

/** Is `p` within `tolerance` of the cubic from → to? */
export function hitEdge(p: Point, from: Point, to: Point, tolerance: number): boolean {
  const pts = sampleEdge(from, to);
  for (let i = 1; i < pts.length; i += 1) {
    const a = pts[i - 1] as Point;
    const b = pts[i] as Point;
    if (distanceToSegment(p, a, b) <= tolerance) {
      return true;
    }
  }
  return false;
}

/** Normalise a drag rectangle from two corners. */
export function rectFrom(a: Point, b: Point): Rect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    width: Math.abs(a.x - b.x),
    height: Math.abs(a.y - b.y),
  };
}

export function intersects(a: Rect, b: Rect): boolean {
  return !(
    a.x + a.width < b.x ||
    b.x + b.width < a.x ||
    a.y + a.height < b.y ||
    b.y + b.height < a.y
  );
}
