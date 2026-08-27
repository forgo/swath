// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { beforeAll, beforeEach, expect, test } from "vitest";
import { defineSwathLayerList, SwathLayerList } from "./swath-layer-list.js";
import type { SwathLayerItem } from "./ui/layer-item.js";

const LAYERS = [
  { id: "truecolor", title: "HLS true color" },
  { id: "ndvi", title: "HLS NDVI" },
];

beforeAll(() => {
  defineSwathLayerList();
});

beforeEach(() => {
  document.body.replaceChildren();
});

async function mount(): Promise<SwathLayerList> {
  const list = document.createElement("swath-layer-list");
  document.body.append(list);
  await list.updateComplete;
  return list;
}

const items = (list: SwathLayerList): SwathLayerItem[] => [
  ...(list.shadowRoot?.querySelectorAll("swath-layer-item") ?? []),
];

test("registers exactly once and upgrades with an accessible role", async () => {
  defineSwathLayerList(); // second call must be a no-op
  expect(customElements.get(SwathLayerList.tagName)).toBe(SwathLayerList);
  const list = await mount();
  expect(list.getAttribute("role")).toBe("group");
  expect(list.getAttribute("aria-label")).toBe("Layers");
});

test("empty state invites patience, not a blank panel", async () => {
  const list = await mount();
  expect(list.shadowRoot?.querySelector('[part="empty"]')?.textContent).toContain("Waiting");
});

test("update renders one item per layer, active carries the view, data-layer on each host", async () => {
  const list = await mount();
  list.update(LAYERS, "ndvi", { visible: false, opacity: 0.5 });
  await list.updateComplete;
  await Promise.all(items(list).map((item) => item.updateComplete));
  expect(items(list).map((i) => i.dataset["layer"])).toEqual(["truecolor", "ndvi"]);
  expect(items(list).map((i) => i.title)).toEqual(["HLS true color", "HLS NDVI"]);
  expect(items(list).map((i) => i.active)).toEqual([false, true]);
  expect(items(list).map((i) => i.visible)).toEqual([true, false]);
  expect(items(list).map((i) => i.opacity)).toEqual([1, 0.5]);
  expect(items(list)[1]?.href).toBe("/tilesets/ndvi");

  const before = items(list)[0];
  list.update(LAYERS, "truecolor");
  await list.updateComplete;
  expect(items(list)[0]).toBe(before); // rows are reused, so focus survives an update
  expect(items(list).map((i) => i.active)).toEqual([true, false]);
});

test("services get kind=service (the delete action); the rest are datasets", async () => {
  const list = await mount();
  list.update(LAYERS, "ndvi");
  list.services = ["ndvi"];
  await list.updateComplete;
  expect(items(list).map((i) => i.kind)).toEqual(["dataset", "service"]);
});

test("a row's select bubbles out of the list as swath-layer-select", async () => {
  const list = await mount();
  list.update(LAYERS, "truecolor");
  await list.updateComplete;
  await Promise.all(items(list).map((item) => item.updateComplete));
  const selected = new Promise<string>((resolve) => {
    document.body.addEventListener("swath-layer-select", (event) => resolve(event.detail.layer), {
      once: true,
    });
  });
  items(list)[1]?.shadowRoot?.querySelector<HTMLButtonElement>('[part="row"]')?.click();
  expect(await selected).toBe("ndvi");
});
