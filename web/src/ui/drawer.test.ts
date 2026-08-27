// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { userEvent } from "vitest/browser";
import { SwathDrawer } from "./drawer.js";

beforeAll(() => {
  SwathDrawer.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

/** A positioned container of `width` px; the drawer docks inside it. */
async function mount(
  width: number,
  html: string,
): Promise<{ container: HTMLElement; drawer: SwathDrawer }> {
  const container = document.createElement("div");
  container.style.cssText = `position:relative;width:${width}px;height:600px`;
  container.innerHTML = html;
  document.body.append(container);
  const drawer = container.querySelector("swath-drawer");
  if (!drawer) {
    throw new Error("no drawer");
  }
  await drawer.updateComplete;
  return { container, drawer };
}

const nextFrame = () => new Promise((r) => requestAnimationFrame(() => r(undefined)));
const base = (drawer: SwathDrawer): HTMLElement =>
  drawer.shadowRoot?.querySelector('[part="base"]') as HTMLElement;

test("closed: not displayed; open: a dialog with the parts, docked right at the size", async () => {
  const { drawer } = await mount(1200, '<swath-drawer size="280px"><p>hi</p></swath-drawer>');
  expect(getComputedStyle(drawer).display).toBe("none");
  drawer.open = true;
  await drawer.updateComplete;
  for (const part of ["base", "header", "body", "footer", "handle", "scrim"]) {
    expect(drawer.shadowRoot?.querySelector(`[part="${part}"]`), part).not.toBeNull();
  }
  expect(drawer.getAttribute("presentation")).toBe("right");
  expect(base(drawer).getAttribute("role")).toBe("dialog");
  expect(base(drawer).getAttribute("aria-modal")).toBe("false");
  expect(base(drawer).getBoundingClientRect().width).toBe(280);
  expect(
    getComputedStyle(drawer.shadowRoot?.querySelector('[part="scrim"]') as Element).display,
  ).toBe("none");
});

test("presentation switches to a bottom sheet when the CONTAINER narrows, and back", async () => {
  const { container, drawer } = await mount(
    1000,
    "<swath-drawer open edge='right'></swath-drawer>",
  );
  expect(drawer.getAttribute("presentation")).toBe("right");
  container.style.width = "500px";
  await nextFrame();
  await nextFrame();
  await drawer.updateComplete;
  expect(drawer.getAttribute("presentation")).toBe("bottom");
  container.style.width = "900px";
  await nextFrame();
  await nextFrame();
  await drawer.updateComplete;
  expect(drawer.getAttribute("presentation")).toBe("right");
});

test("snap points size the sheet; the handle drag lands on the nearest one and swipes down to close", async () => {
  const { drawer } = await mount(500, '<swath-drawer open snap="40,90"></swath-drawer>');
  expect(drawer.getAttribute("presentation")).toBe("bottom");
  expect(drawer.snapPoints).toEqual([40, 90]);
  expect(Math.round(base(drawer).getBoundingClientRect().height)).toBe(240); // 40% of 600
  const snaps: number[] = [];
  const closes: string[] = [];
  drawer.addEventListener("swath-change", (e) => snaps.push(Number(e.detail.value)));
  drawer.addEventListener("swath-drawer-close", (e) => closes.push(e.detail.reason));
  const handle = drawer.shadowRoot?.querySelector('[part="handle"]') as HTMLElement;
  const drag = (dy: number) => {
    handle.dispatchEvent(
      new PointerEvent("pointerdown", { bubbles: true, clientX: 0, clientY: 400, pointerId: 1 }),
    );
    handle.dispatchEvent(
      new PointerEvent("pointermove", {
        bubbles: true,
        clientX: 0,
        clientY: 400 + dy,
        pointerId: 1,
      }),
    );
    handle.dispatchEvent(
      new PointerEvent("pointerup", { bubbles: true, clientX: 0, clientY: 400 + dy, pointerId: 1 }),
    );
  };
  drag(-250); // 240 + 250 = 490 of 600 ≈ 82% → nearest 90
  await drawer.updateComplete;
  expect(drawer.snapIndex).toBe(1);
  expect(snaps).toEqual([1]);
  expect(Math.round(base(drawer).getBoundingClientRect().height)).toBe(540);
  drag(200); // 540 - 200 = 340 ≈ 57% → nearest 40
  await drawer.updateComplete;
  expect(drawer.snapIndex).toBe(0);
  drag(200); // 240 - 200 = 40 → below half the lowest snap: close
  expect(closes).toEqual(["swipe"]);
  expect(drawer.open).toBe(true); // the host decides
});

test("modal: scrim click and Esc ask to close; Tab is trapped; focus moves in and returns", async () => {
  const outside = document.createElement("button");
  outside.textContent = "outside";
  document.body.append(outside);
  outside.focus();
  const { drawer } = await mount(
    1200,
    '<swath-drawer modal><button id="a">a</button><button id="b">b</button></swath-drawer>',
  );
  const closes: string[] = [];
  drawer.addEventListener("swath-drawer-close", (e) => closes.push(e.detail.reason));
  drawer.open = true;
  await drawer.updateComplete;
  expect(base(drawer).getAttribute("aria-modal")).toBe("true");
  expect(document.activeElement?.id).toBe("a");
  await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
  expect(document.activeElement?.id).toBe("b");
  await userEvent.keyboard("{Tab}");
  expect(document.activeElement?.id).toBe("a");
  await userEvent.keyboard("{Escape}");
  drawer.shadowRoot?.querySelector<HTMLElement>('[part="scrim"]')?.click();
  expect(closes).toEqual(["esc", "scrim"]);
  drawer.open = false;
  await drawer.updateComplete;
  expect(document.activeElement).toBe(outside);
});
