// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { userEvent } from "vitest/browser";
import { SwathCommandPalette } from "./command-palette.js";

beforeAll(() => {
  SwathCommandPalette.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(): Promise<{ palette: SwathCommandPalette; ran: string[] }> {
  const palette = document.createElement("swath-command-palette");
  const ran: string[] = [];
  palette.commands = [
    {
      id: "layer:ndvi",
      label: "Show layer: HLS NDVI",
      hint: "ndvi",
      group: "Layers",
      run: () => ran.push("layer:ndvi"),
    },
    {
      id: "layer:truecolor",
      label: "Show layer: HLS true color",
      group: "Layers",
      run: () => ran.push("layer:truecolor"),
    },
    { id: "zoom", label: "Zoom to data", group: "Map", run: () => ran.push("zoom") },
  ];
  document.body.append(palette);
  await palette.updateComplete;
  return { palette, ran };
}

const items = (p: SwathCommandPalette) => [
  ...(p.shadowRoot?.querySelectorAll<HTMLButtonElement>('[part="item"]') ?? []),
];
const input = (p: SwathCommandPalette) =>
  p.shadowRoot?.querySelector<HTMLInputElement>('[part="input"]') as HTMLInputElement;

test("closed by default; show() focuses the input, lists everything; typing filters and marks; Enter runs and closes", async () => {
  const { palette, ran } = await mount();
  expect(getComputedStyle(palette).display).toBe("none");
  const chosen: string[] = [];
  palette.addEventListener("swath-command", (e) => chosen.push(e.detail.id));
  palette.show();
  await palette.updateComplete;
  await new Promise((r) => setTimeout(r, 0));
  expect(palette.shadowRoot?.activeElement).toBe(input(palette));
  expect(items(palette)).toHaveLength(3);
  await userEvent.keyboard("ndvi");
  await palette.updateComplete;
  expect(items(palette)[0]?.dataset["command"]).toBe("layer:ndvi");
  expect(items(palette)[0]?.querySelector("mark")?.textContent).toBe("NDVI");
  await userEvent.keyboard("{Enter}");
  expect(chosen).toEqual(["layer:ndvi"]);
  expect(ran).toEqual(["layer:ndvi"]);
  expect(palette.open).toBe(false);
});

test("↑↓ move the selection with wrap; Esc closes and restores focus to the opener", async () => {
  const opener = document.createElement("button");
  opener.textContent = "open";
  document.body.append(opener);
  opener.focus();
  const { palette, ran } = await mount();
  palette.show();
  await palette.updateComplete;
  await new Promise((r) => setTimeout(r, 0));
  await userEvent.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}");
  await palette.updateComplete;
  expect(items(palette).map((i) => i.getAttribute("aria-selected"))).toEqual([
    "true",
    "false",
    "false",
  ]); // wrapped
  await userEvent.keyboard("{ArrowUp}");
  await palette.updateComplete;
  expect(items(palette)[2]?.getAttribute("aria-selected")).toBe("true");
  await userEvent.keyboard("{Escape}");
  expect(palette.open).toBe(false);
  expect(document.activeElement).toBe(opener);
  expect(ran).toEqual([]);
});

test("presentation follows the viewport: dialog at ≥ 640px, sheet below; no match shows an empty note", async () => {
  const { palette } = await mount();
  palette.show();
  await palette.updateComplete;
  expect(palette.getAttribute("presentation")).toBe(window.innerWidth >= 640 ? "dialog" : "sheet");
  await new Promise((r) => setTimeout(r, 0));
  await userEvent.keyboard("zzzz");
  await palette.updateComplete;
  expect(palette.shadowRoot?.querySelector('[part="empty"]')?.textContent).toContain("No command");
  vi.restoreAllMocks();
});
