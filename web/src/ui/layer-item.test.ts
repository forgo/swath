// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { SwathLayerItem } from "./layer-item.js";
import type { SwathMenu } from "./menu.js";

beforeAll(() => {
  SwathLayerItem.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(attrs: Record<string, string>): Promise<SwathLayerItem> {
  const item = document.createElement("swath-layer-item");
  for (const [name, value] of Object.entries(attrs)) {
    item.setAttribute(name, value);
  }
  document.body.append(item);
  await item.updateComplete;
  return item;
}

const part = <T extends HTMLElement>(item: SwathLayerItem, name: string): T =>
  item.shadowRoot?.querySelector(`[part="${name}"]`) as T;
const innerButton = (host: HTMLElement): HTMLButtonElement =>
  host.shadowRoot?.querySelector("button") as HTMLButtonElement;

test("renders the row with title/meta, mirrors layer-id as data-layer, active → aria-pressed", async () => {
  const item = await mount({ "layer-id": "ndvi", title: "HLS NDVI", active: "" });
  expect(item.dataset["layer"]).toBe("ndvi");
  for (const name of ["base", "row", "eye", "opacity", "menu"]) {
    expect(part(item, name), name).not.toBeNull();
  }
  expect(part(item, "title").textContent).toBe("HLS NDVI");
  expect(part(item, "meta").textContent).toBe("ndvi");
  expect(part<HTMLButtonElement>(item, "row").getAttribute("aria-pressed")).toBe("true");
  item.active = false;
  await item.updateComplete;
  expect(part<HTMLButtonElement>(item, "row").getAttribute("aria-pressed")).toBe("false");
});

test("the row selects; the eye toggles visibility; the slider reports opacity — all row-scoped", async () => {
  const item = await mount({ "layer-id": "ndvi", title: "NDVI", active: "", visible: "" });
  const seen: string[] = [];
  document.body.addEventListener("swath-layer-select", (e) =>
    seen.push(`select:${e.detail.layer}`),
  );
  document.body.addEventListener("swath-layer-visibility", (e) =>
    seen.push(`visible:${e.detail.layer}:${e.detail.visible}`),
  );
  document.body.addEventListener("swath-layer-opacity", (e) =>
    seen.push(`opacity:${e.detail.layer}:${e.detail.opacity}`),
  );
  const generic: string[] = [];
  document.body.addEventListener("swath-toggle", () => generic.push("toggle"));
  document.body.addEventListener("swath-input", () => generic.push("input"));
  part<HTMLButtonElement>(item, "row").click();
  const eye = part<HTMLElement>(item, "eye");
  expect(eye.getAttribute("icon")).toBe("eye");
  innerButton(eye).click();
  await item.updateComplete;
  expect(item.visible).toBe(false);
  expect(eye.getAttribute("icon")).toBe("eye-off");
  expect(innerButton(eye).getAttribute("aria-label")).toBe("Show NDVI");
  const range = part<HTMLElement>(item, "opacity").shadowRoot?.querySelector(
    "input",
  ) as HTMLInputElement;
  range.value = "0.4";
  range.dispatchEvent(new Event("input", { bubbles: true }));
  expect(item.opacity).toBeCloseTo(0.4);
  expect(seen).toEqual(["select:ndvi", "visible:ndvi:false", "opacity:ndvi:0.4"]);
  expect(generic).toEqual([]); // the primitives' own events stay inside the row
});

test("the opacity slider shows only on the active row", async () => {
  const item = await mount({ "layer-id": "a", title: "A" });
  expect(getComputedStyle(part(item, "opacity")).display).toBe("none");
  item.active = true;
  await item.updateComplete;
  expect(getComputedStyle(part(item, "opacity")).display).not.toBe("none");
});

test("kebab: zoom / compare / info always; delete only for services; info expands the row", async () => {
  const item = await mount({
    "layer-id": "svc",
    title: "Svc",
    kind: "dataset",
    href: "/tilesets/svc",
  });
  const menu = part<SwathMenu>(item, "menu");
  const labels = () =>
    [...(menu.shadowRoot?.querySelectorAll('[part="item"]') ?? [])].map((b) =>
      b.textContent?.trim(),
    );
  expect(labels()).toEqual(["Zoom to data", "Compare with this", "Info"]);
  item.kind = "service";
  await item.updateComplete;
  expect(labels()).toEqual(["Zoom to data", "Compare with this", "Info", "Delete service"]);
  const actions: string[] = [];
  document.body.addEventListener("swath-layer-action", (e) =>
    actions.push(`${e.detail.layer}:${e.detail.action}`),
  );
  innerButton(menu.querySelector("swath-button") as HTMLElement).click(); // trigger
  await menu.updateComplete;
  menu.shadowRoot?.querySelector<HTMLButtonElement>('[part="item"][data-id="info"]')?.click();
  await item.updateComplete;
  expect(actions).toEqual(["svc:info"]);
  expect(item.expanded).toBe(true);
  expect(part(item, "info").textContent).toContain("svc · service");
  expect(part(item, "info").querySelector("a")?.getAttribute("href")).toBe("/tilesets/svc");
  innerButton(menu.querySelector("swath-button") as HTMLElement).click();
  await menu.updateComplete;
  menu.shadowRoot?.querySelector<HTMLButtonElement>('[part="item"][data-id="delete"]')?.click();
  expect(actions).toEqual(["svc:info", "svc:delete"]);
});
