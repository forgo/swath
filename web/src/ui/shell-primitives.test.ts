// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The shell's primitives (#284): dock slots, card fold, status cells, rail.
import { afterEach, beforeAll, expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { SwathHudCard } from "./hud-card.js";
import { DOCK_SLOTS, SwathHudDock } from "./hud-dock.js";
import { SwathRail } from "./rail.js";
import { SwathStatusBar, SwathStatusCell } from "./status-bar.js";

beforeAll(async () => {
  // The shell reflows by VIEWPORT width (#293): pin the desktop tier.
  await page.viewport(1528, 928);
  SwathHudDock.define();
  SwathHudCard.define();
  SwathStatusBar.define();
  SwathStatusCell.define();
  SwathRail.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount<K extends keyof HTMLElementTagNameMap>(html: string, tag: K) {
  const host = document.createElement("div");
  host.style.cssText = "position:relative;width:800px;height:600px";
  host.innerHTML = html;
  document.body.append(host);
  const element = host.querySelector(tag);
  if (!element) {
    throw new Error(`no ${tag}`);
  }
  await (element as unknown as { updateComplete: Promise<void> }).updateComplete;
  return element;
}

test("hud-dock: eight named slots assign children to their corners; the dock ignores pointers", async () => {
  const dock = await mount(
    `<swath-hud-dock>${DOCK_SLOTS.map((s) => `<span slot="${s}" id="c-${s}">${s}</span>`).join("")}</swath-hud-dock>`,
    "swath-hud-dock",
  );
  for (const name of DOCK_SLOTS) {
    const slot = dock.shadowRoot?.querySelector<HTMLSlotElement>(`slot[name="${name}"]`);
    expect(
      slot?.assignedElements().map((e) => e.id),
      name,
    ).toEqual([`c-${name}`]);
  }
  expect(getComputedStyle(dock).pointerEvents).toBe("none");
  expect(getComputedStyle(dock.querySelector("#c-top-left") as Element).pointerEvents).toBe("auto");
});

test("hud-dock geometry: centre slots centre their card, sides hug their edges (the 3 × 3 placement)", async () => {
  const dock = await mount(
    '<swath-hud-dock><div slot="top-center" style="width:200px;height:20px"></div><div slot="bottom-center" style="width:200px;height:20px"></div><div slot="left" style="width:100px;height:40px"></div><div slot="bottom-right" style="width:100px;height:20px"></div></swath-hud-dock>',
    "swath-hud-dock",
  );
  const box = (slot: string) => {
    const b = (dock.querySelector(`[slot="${slot}"]`) as HTMLElement).getBoundingClientRect();
    const d = dock.getBoundingClientRect();
    return {
      cx: Math.round(b.left + b.width / 2 - d.left),
      top: Math.round(b.top - d.top),
      right: Math.round(d.right - b.right),
    };
  };
  expect(box("top-center").cx).toBe(400); // centred in the 800 px host
  expect(box("top-center").top).toBe(8);
  expect(box("bottom-center").cx).toBe(400);
  expect(box("bottom-center").top).toBe(600 - 8 - 20);
  expect(box("left").top).toBeGreaterThan(200); // the middle row, vertically centred
  expect(box("bottom-right").right).toBe(8);
});

test("hud-card: title + actions; collapsible folds the body and emits swath-toggle", async () => {
  const card = await mount(
    '<swath-hud-card title="Ingest" collapsible><button slot="actions">x</button><p>body</p></swath-hud-card>',
    "swath-hud-card",
  );
  const header = card.shadowRoot?.querySelector<HTMLButtonElement>('button[part="header"]');
  expect(header?.querySelector('[part="title"]')?.textContent).toBe("Ingest");
  expect(header?.getAttribute("aria-expanded")).toBe("true");
  const toggles: boolean[] = [];
  card.addEventListener("swath-toggle", (e) => toggles.push(e.detail.pressed));
  header?.click();
  await card.updateComplete;
  expect(card.collapsed).toBe(true);
  expect(header?.getAttribute("aria-expanded")).toBe("false");
  expect(getComputedStyle(card.shadowRoot?.querySelector('[part="body"]') as Element).display).toBe(
    "none",
  );
  expect(toggles).toEqual([false]);
  const plain = await mount('<swath-hud-card title="Feed"></swath-hud-card>', "swath-hud-card");
  expect(plain.shadowRoot?.querySelector('button[part="header"]')).toBeNull();
});

test("hud-card auto-hide: hidden while the default slot is empty, shown once content arrives", async () => {
  const card = await mount(
    '<swath-hud-card auto-hide title="Inspector"></swath-hud-card>',
    "swath-hud-card",
  );
  expect(card.hidden).toBe(true);
  card.append(document.createElement("p"));
  await new Promise((r) => setTimeout(r, 0)); // slotchange is async
  expect(card.hidden).toBe(false);
  card.replaceChildren();
  await new Promise((r) => setTimeout(r, 0));
  expect(card.hidden).toBe(true);
});

test("status bar: cells render label/value, mono reflects, an empty value reads as —", async () => {
  const bar = await mount(
    '<swath-status-bar><swath-status-cell label="zoom" value="8.25" mono></swath-status-cell><swath-status-cell label="crs"></swath-status-cell></swath-status-bar>',
    "swath-status-bar",
  );
  const [zoom, crs] = [...bar.querySelectorAll("swath-status-cell")];
  await zoom?.updateComplete;
  await crs?.updateComplete;
  expect(zoom?.shadowRoot?.querySelector('[part="label"]')?.textContent).toBe("zoom");
  expect(zoom?.shadowRoot?.querySelector('[part="value"]')?.textContent).toBe("8.25");
  expect(zoom?.hasAttribute("mono")).toBe(true);
  const empty = crs?.shadowRoot?.querySelector('[part="value"]') as Element;
  expect(getComputedStyle(empty, "::before").content).toBe('"—"');
  expect(Math.round(bar.getBoundingClientRect().height)).toBe(24);
});

test("rail: items + mode → aria-current; a pick emits swath-mode-change once; ↑↓ rove; collapse toggles width", async () => {
  const rail = await mount(
    '<swath-rail mode="layers"><b slot="brand">Swath</b><p>content</p><i slot="footer">f</i></swath-rail>',
    "swath-rail",
  );
  rail.items = [
    { id: "layers", label: "Layers", icon: "layers" },
    { id: "data", label: "Data", icon: "data" },
    { id: "author", label: "Author", icon: "author" },
  ];
  await rail.updateComplete;
  const items = () => [
    ...(rail.shadowRoot?.querySelectorAll<HTMLButtonElement>('[part="item"]') ?? []),
  ];
  expect(items().map((b) => b.getAttribute("aria-current"))).toEqual(["page", null, null]);
  const modes: string[] = [];
  rail.addEventListener("swath-mode-change", (e) => modes.push(e.detail.mode));
  items()[1]?.click();
  await rail.updateComplete;
  expect(rail.mode).toBe("data");
  expect(items().map((b) => b.getAttribute("aria-current"))).toEqual([null, "page", null]);
  items()[1]?.click(); // already the mode: silent
  expect(modes).toEqual(["data"]);
  items()[1]?.focus();
  await userEvent.keyboard("{ArrowDown}");
  expect(rail.shadowRoot?.activeElement).toBe(items()[2]);
  await userEvent.keyboard("{ArrowDown}");
  expect(rail.shadowRoot?.activeElement).toBe(items()[0]);
  // 56px icon strip + 248px panel (#398): the strip sits BESIDE the panel
  // rather than inside it, so the panel keeps its full width and layer
  // titles stop truncating. Collapsed, the strip alone remains.
  expect(Math.round(rail.getBoundingClientRect().width)).toBe(304);
  rail.style.transition = "none"; // the width animates (motion tokens); read the end state
  const collapsed: boolean[] = [];
  rail.addEventListener("swath-toggle", (e) => collapsed.push(e.detail.pressed));
  rail.shadowRoot
    ?.querySelector<HTMLElement>('[part="collapse"]')
    ?.shadowRoot?.querySelector("button")
    ?.click();
  await rail.updateComplete;
  expect(rail.collapsed).toBe(true);
  expect(collapsed).toEqual([true]);
  expect(Math.round(rail.getBoundingClientRect().width)).toBe(56);
});
