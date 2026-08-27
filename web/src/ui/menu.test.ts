// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { userEvent } from "vitest/browser";
import { type MenuItem, SwathMenu } from "./menu.js";

const ITEMS: MenuItem[] = [
  { id: "zoom", label: "Zoom to" },
  { id: "rename", label: "Rename", disabled: true },
  { id: "raise", label: "Raise" },
  { id: "remove", label: "Remove", icon: "trash", danger: true },
];

beforeAll(() => {
  SwathMenu.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(): Promise<SwathMenu> {
  const host = document.createElement("div");
  host.innerHTML =
    '<swath-menu label="Layer actions"><button slot="trigger" type="button">more</button></swath-menu>';
  document.body.append(host);
  const menu = host.querySelector("swath-menu");
  if (!menu) {
    throw new Error("no menu");
  }
  menu.items = ITEMS;
  await menu.updateComplete;
  return menu;
}

const items = (menu: SwathMenu): HTMLButtonElement[] => [
  ...(menu.shadowRoot?.querySelectorAll<HTMLButtonElement>('[part="item"]') ?? []),
];
const focused = (menu: SwathMenu): string | undefined =>
  (menu.shadowRoot?.activeElement as HTMLElement | null)?.dataset["id"];

test("closed by default; the trigger opens it in the viewport's presentation, with roles and parts", async () => {
  const menu = await mount();
  const list = menu.shadowRoot?.querySelector('[part="list"]') as HTMLElement;
  expect(list.hidden).toBe(true);
  menu.querySelector("button")?.click();
  await menu.updateComplete;
  expect(menu.open).toBe(true);
  // Popover at ≥ 640 px, a bottom sheet below (the test viewport decides).
  expect(menu.getAttribute("presentation")).toBe(window.innerWidth >= 640 ? "popover" : "sheet");
  expect(list.hidden).toBe(false);
  expect(list.getAttribute("role")).toBe("menu");
  expect(list.getAttribute("aria-label")).toBe("Layer actions");
  expect(items(menu).map((b) => b.getAttribute("role"))).toEqual([
    "menuitem",
    "menuitem",
    "menuitem",
    "menuitem",
  ]);
  expect(items(menu)[1]?.disabled).toBe(true);
  expect(items(menu)[3]?.hasAttribute("data-danger")).toBe(true);
  expect(items(menu)[3]?.querySelector("swath-icon")?.name).toBe("trash");
  expect(focused(menu)).toBe("zoom"); // first enabled item takes focus
});

test("↑↓ skip disabled items and wrap, Home/End, typeahead, Enter selects and closes", async () => {
  const menu = await mount();
  const selected: string[] = [];
  const closed: string[] = [];
  menu.addEventListener("swath-menu-select", (e) => selected.push(e.detail.id));
  menu.addEventListener("swath-drawer-close", (e) => closed.push(e.detail.reason));
  menu.show();
  await menu.updateComplete;
  await userEvent.keyboard("{ArrowDown}");
  expect(focused(menu)).toBe("raise"); // "rename" is disabled
  await userEvent.keyboard("{ArrowDown}{ArrowDown}");
  expect(focused(menu)).toBe("zoom"); // wrapped
  await userEvent.keyboard("{End}");
  expect(focused(menu)).toBe("remove");
  await userEvent.keyboard("{Home}");
  expect(focused(menu)).toBe("zoom");
  await userEvent.keyboard("r");
  expect(focused(menu)).toBe("raise");
  await userEvent.keyboard("{Enter}");
  expect(selected).toEqual(["raise"]);
  expect(menu.open).toBe(false);
  expect(closed).toEqual(["select"]);
});

test("Esc and an outside pointerdown close with their reasons; inside clicks do not", async () => {
  const menu = await mount();
  const closed: string[] = [];
  menu.addEventListener("swath-drawer-close", (e) => closed.push(e.detail.reason));
  menu.show();
  await menu.updateComplete;
  await userEvent.keyboard("{Escape}");
  expect(menu.open).toBe(false);
  menu.show();
  await menu.updateComplete;
  items(menu)[0]?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, composed: true }));
  expect(menu.open).toBe(true);
  document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  expect(menu.open).toBe(false);
  expect(closed).toEqual(["esc", "outside"]);
});

test("a long-press on the trigger (touch) opens; a short one or a drift does not", async () => {
  const menu = await mount();
  const trigger = menu.shadowRoot?.querySelector('[part="trigger"]') as HTMLElement;
  const press = (type: string, x = 10, y = 10) =>
    trigger.dispatchEvent(
      new PointerEvent(type, { bubbles: true, pointerType: "touch", clientX: x, clientY: y }),
    );
  press("pointerdown");
  press("pointermove", 30, 10); // drifted: cancelled
  await new Promise((r) => setTimeout(r, 560));
  expect(menu.open).toBe(false);
  press("pointerdown");
  press("pointerup");
  await new Promise((r) => setTimeout(r, 560));
  expect(menu.open).toBe(false);
  press("pointerdown");
  await new Promise((r) => setTimeout(r, 560));
  expect(menu.open).toBe(true);
});
