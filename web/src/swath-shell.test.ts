// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { page } from "vitest/browser";
import { defineSwathShell, SwathShell } from "./swath-shell.js";
import { SwathElement } from "./ui/element.js";
import { css } from "./ui/styles.js";

beforeAll(async () => {
  // The shell reflows by VIEWPORT width (#293): pin the desktop tier.
  await page.viewport(1528, 928);
  defineSwathShell();
});

afterEach(() => {
  document.body.replaceChildren();
});

/** Stands in for a `slot="main"` child, which is really a `swath-drawer`.
 *
 * The shape that matters is where its `position: absolute; inset: 0` comes
 * from: its OWN shadow root's `:host`, which the outer tree overrides — that
 * is how the shell reserves the preview column. A plain `<div>` styled
 * inline, or by a document rule, cannot model it: both beat `::slotted()`,
 * which is exactly the cascade that made #463 ship broken while a test
 * using a `<div>` passed. */
class MainProbe extends SwathElement {
  static override tagName = "main-probe";
  static override styles = [css`:host { position: absolute; inset: 0; display: block; }`];
  protected render(): void {
    // Nothing to draw: the fixture exists for its :host geometry alone.
  }
}
MainProbe.define();

async function mount(attrs = ""): Promise<SwathShell> {
  const host = document.createElement("div");
  host.style.cssText = "width:1528px;height:928px";
  host.innerHTML = `<swath-shell ${attrs}>
    <div slot="rail" id="rail" style="width:248px;height:100%">rail</div>
    <span slot="topbar" id="top">Layers</span>
    <div slot="map" id="map">map</div>
    <main-probe slot="main" id="main"></main-probe>
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

test("composing reserves the preview column out of the main slot (#400)", async () => {
  const shell = await mount();
  const main = document.querySelector("#main") as HTMLElement | null;
  expect(main, "the fixture must slot a main child").not.toBeNull();
  const full = Math.round((main as HTMLElement).getBoundingClientRect().width);

  shell.compose = true;
  await shell.updateComplete;
  const reserved = Math.round((main as HTMLElement).getBoundingClientRect().width);
  expect(full - reserved).toBe(320);

  shell.compose = false;
  await shell.updateComplete;
  expect(Math.round((main as HTMLElement).getBoundingClientRect().width)).toBe(full);
});

// NOTE: the shell reserves the column; it does NOT position the map. It
// cannot — the map is light DOM and document styles beat `::slotted()`, so
// the consumer's own `swath-map[slot="map"]` rule wins (#463). An earlier
// version of this test asserted the map's geometry against a plain slotted
// `<div>`, which has no document rule, so it passed while the real page
// rendered the column at the wrong edge and empty. The map's placement is
// asserted where it is real: `modes.e2e.ts`, against the demo page.
