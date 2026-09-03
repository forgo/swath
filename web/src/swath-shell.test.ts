// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { page } from "vitest/browser";
import { defineSwathShell, SwathShell } from "./swath-shell.js";

beforeAll(async () => {
  // The shell reflows by VIEWPORT width (#293): pin the desktop tier.
  await page.viewport(1528, 928);
  defineSwathShell();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(attrs = ""): Promise<SwathShell> {
  const host = document.createElement("div");
  host.style.cssText = "width:1528px;height:928px";
  host.innerHTML = `<swath-shell ${attrs}>
    <div slot="rail" id="rail" style="width:248px;height:100%">rail</div>
    <span slot="topbar" id="top">Layers</span>
    <div slot="map" id="map">map</div>
    <div slot="hud" id="hud">hud</div>
    <div slot="inspector" id="insp">inspector</div>
    <div slot="statusbar" id="status">status</div>
  </swath-shell>`;
  document.body.append(host);
  const shell = host.querySelector("swath-shell");
  if (!shell) {
    throw new Error("no shell");
  }
  await shell.updateComplete;
  return shell;
}

const assigned = (shell: SwathShell, name: string): string[] =>
  shell.shadowRoot
    ?.querySelector<HTMLSlotElement>(`slot[name="${name}"]`)
    ?.assignedElements()
    .map((e) => e.id) ?? [];

test("registers once; every region slot takes its child; the map stays light DOM", async () => {
  defineSwathShell();
  expect(customElements.get(SwathShell.tagName)).toBe(SwathShell);
  const shell = await mount();
  for (const [slot, id] of [
    ["rail", "rail"],
    ["topbar", "top"],
    ["map", "map"],
    ["hud", "hud"],
    ["inspector", "insp"],
    ["statusbar", "status"],
  ] as const) {
    expect(assigned(shell, slot), slot).toEqual([id]);
  }
  expect(document.querySelector("#map")?.closest("swath-shell")).toBe(shell);
});

test("geometry at the pinned 1528 × 928: rail 248, top bar 44, status 24 → a 1280 × 860 canvas", async () => {
  const shell = await mount();
  const map = document.querySelector("#map") as HTMLElement;
  const box = map.getBoundingClientRect();
  expect(Math.round(box.left)).toBe(248);
  expect(Math.round(box.top)).toBe(44);
  expect(Math.round(box.width)).toBe(1280);
  expect(Math.round(box.height)).toBe(860);
  const inspector = shell.shadowRoot?.querySelector('[part="inspector"]') as HTMLElement;
  expect(getComputedStyle(inspector).display).toBe("none");
  shell.inspector = true;
  await shell.updateComplete;
  expect(Math.round(inspector.getBoundingClientRect().width)).toBe(320);
  expect(Math.round(map.getBoundingClientRect().width)).toBe(960);
});

test("view reflects the mode; exactly one role=status live region, fed by announce()", async () => {
  const shell = await mount('view="data"');
  expect(shell.getAttribute("view")).toBe("data");
  const regions = shell.shadowRoot?.querySelectorAll('[role="status"]') ?? [];
  expect(regions).toHaveLength(1);
  const live = regions[0] as HTMLElement;
  expect(live.getAttribute("aria-live")).toBe("polite");
  shell.announce("Layer ndvi is on the map");
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  expect(live.textContent).toBe("Layer ndvi is on the map");
});

test("slots exist synchronously on upgrade: a slotted child has its size before any microtask", () => {
  const host = document.createElement("div");
  host.style.cssText = "width:1528px;height:928px";
  host.innerHTML =
    '<swath-shell><div slot="rail" style="width:248px;height:100%"></div><div slot="map" id="sync-map" style="position:absolute;inset:0"></div></swath-shell>';
  document.body.append(host);
  // No await: MapLibre-style consumers measure their container right here.
  const map = document.querySelector("#sync-map") as HTMLElement;
  expect(Math.round(map.getBoundingClientRect().width)).toBe(1280);
  expect(Math.round(map.getBoundingClientRect().height)).toBe(860);
});

test("composing gives the canvas the region and the map a preview column (#400)", async () => {
  const shell = await mount();
  const map = document.querySelector("#map") as HTMLElement;
  const full = Math.round(map.getBoundingClientRect().width);

  shell.compose = true;
  await shell.updateComplete;
  const preview = Math.round(map.getBoundingClientRect().width);

  // ADR 0028's amended rule: the map is always present AND never smaller
  // than a live preview. It shrinks to the preview column — it does not
  // disappear, and nothing is drawn over it.
  expect(preview).toBe(320);
  expect(preview).toBeLessThan(full);
  expect(getComputedStyle(map).display).not.toBe("none");
  // It is on the far side: the canvas takes everything left of it.
  const box = map.getBoundingClientRect();
  expect(Math.round(box.right)).toBe(Math.round(window.innerWidth));

  shell.compose = false;
  await shell.updateComplete;
  expect(Math.round(map.getBoundingClientRect().width)).toBe(full);
});
