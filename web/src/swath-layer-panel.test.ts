// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The layer browser's contract (issue #108): presentational only — it
// renders what `update()` feeds it (title + id, aria-pressed on the
// viewed layer) and announces selection as a bubbling event. Real Custom
// Elements in a real browser, same as the rest of the suite.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { defineSwathLayerPanel, SwathLayerPanel } from "./swath-layer-panel.js";

const LAYERS = [
  { id: "truecolor", title: "HLS true color" },
  { id: "ndvi", title: "HLS NDVI" },
];

beforeAll(() => {
  defineSwathLayerPanel();
});

beforeEach(() => {
  document.body.replaceChildren();
});

function mount(): SwathLayerPanel {
  const panel = document.createElement("swath-layer-panel") as SwathLayerPanel;
  document.body.append(panel);
  return panel;
}

test("registers exactly once and upgrades with an accessible role", () => {
  defineSwathLayerPanel(); // second call must be a no-op
  expect(customElements.get(SwathLayerPanel.tagName)).toBe(SwathLayerPanel);
  const panel = mount();
  expect(panel.getAttribute("role")).toBe("group");
  expect(panel.getAttribute("aria-label")).toBe("Layers");
});

test("empty state invites patience, not a blank panel", () => {
  const panel = mount();
  expect(panel.querySelector(".swath-layer-panel-empty")?.textContent).toContain("Waiting");
});

test("update renders every layer with title, id, and aria-pressed", () => {
  const panel = mount();
  panel.update(LAYERS, "ndvi");
  const buttons = [...panel.querySelectorAll<HTMLButtonElement>("button")];
  expect(buttons.map((b) => b.querySelector(".swath-layer-panel-title")?.textContent)).toEqual([
    "HLS true color",
    "HLS NDVI",
  ]);
  expect(buttons.map((b) => b.querySelector(".swath-layer-panel-id")?.textContent)).toEqual([
    "truecolor",
    "ndvi",
  ]);
  expect(buttons.map((b) => b.getAttribute("aria-pressed"))).toEqual(["false", "true"]);

  panel.update(LAYERS, "truecolor");
  const pressed = [...panel.querySelectorAll<HTMLButtonElement>("button")].map((b) =>
    b.getAttribute("aria-pressed"),
  );
  expect(pressed).toEqual(["true", "false"]);
});

test("clicking a layer dispatches a bubbling swath-layer-select", async () => {
  const panel = mount();
  panel.update(LAYERS, "truecolor");
  const selected = new Promise<string>((resolve) => {
    document.body.addEventListener(
      "swath-layer-select",
      (event) => resolve((event as CustomEvent<{ layer: string }>).detail.layer),
      { once: true },
    );
  });
  panel.querySelector<HTMLButtonElement>('button[data-layer="ndvi"]')?.click();
  expect(await selected).toBe("ndvi");
});
