// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Smoke for the canvas fixture (issue #290): a real browser drives the
// pan / zoom / drag / connect gestures on desktop and on the `mobile`
// project (touch). The fixture is dev-only: vite mode only.
import { expect, test } from "@playwright/test";

test.skip(process.env.SWATH_E2E_MODE === "binary", "the canvas fixture is not part of dist");

const PAGE = "/demo/canvas.html";

test("desktop: drag a node, wheel-zoom, connect by drag, delete by key", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "mouse gestures run on the desktop project");
  await page.goto(PAGE);
  const canvas = page.locator("swath-canvas");
  await expect(canvas).toHaveAttribute("data-view", /.+/);
  const header = page.locator('swath-canvas-node[node-id="ndvi"] [part="header"]');
  const before = await header.boundingBox();
  if (!before) {
    throw new Error("no node");
  }
  await page.mouse.move(before.x + 10, before.y + 10);
  await page.mouse.down();
  await page.mouse.move(before.x + 90, before.y + 60, { steps: 6 });
  await page.mouse.up();
  await expect(page.locator("#log")).toContainText("move ndvi");
  await page.mouse.wheel(0, -300);
  await expect
    .poll(async () => Number((await canvas.getAttribute("data-view"))?.split(",")[2]))
    .toBeGreaterThan(0.5);
  const from = page.locator(
    'swath-canvas-node[node-id="ndvi"] swath-canvas-port[name="out"] button',
  );
  const to = page.locator(
    'swath-canvas-node[node-id="save"] swath-canvas-port[name="data"] button',
  );
  const a = await from.boundingBox();
  const b = await to.boundingBox();
  if (!a || !b) {
    throw new Error("no ports");
  }
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 8 });
  await page.mouse.up();
  await expect(page.locator("#log")).toContainText("connect ndvi.out → save.data");
  await expect(page.locator('swath-canvas [part="edges"] path.edge')).toHaveCount(2);
  await page
    .locator('swath-canvas [part="edges"] path.hit')
    .last()
    .dispatchEvent("pointerdown", { button: 0, bubbles: true });
  await canvas.focus();
  await page.keyboard.press("Delete");
  await expect(page.locator("#log")).toContainText('delete {"nodes":[],"edges":["e2"]}');
});

test("keyboard: Tab roves nodes, Enter on ports connects, Esc cancels", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "a keyboard is the desktop project's");
  await page.goto(PAGE);
  const canvas = page.locator("swath-canvas");
  await canvas.focus();
  await page.keyboard.press("Tab");
  await expect(page.locator('swath-canvas-node[node-id="load"] div[part="base"]')).toBeFocused();
  await page.locator('swath-canvas-node[node-id="load"] swath-canvas-port button').focus();
  await page.keyboard.press("Enter");
  await expect(page.locator('swath-canvas-node[node-id="load"] swath-canvas-port')).toHaveAttribute(
    "armed",
    "",
  );
  await canvas.focus();
  await page.keyboard.press("Escape");
  await expect(
    page.locator('swath-canvas-node[node-id="load"] swath-canvas-port'),
  ).not.toHaveAttribute("armed", "");
});

test("touch (mobile project): one finger pans, tap-to-connect completes", async ({
  page,
  isMobile,
}) => {
  test.skip(!isMobile, "touch gestures run on the mobile project");
  await page.goto(PAGE);
  const canvas = page.locator("swath-canvas");
  const box = await canvas.boundingBox();
  if (!box) {
    throw new Error("no canvas");
  }
  const before = await canvas.getAttribute("data-view");
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x: box.x + 300, y: box.y + 300 }],
  });
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchMove",
    touchPoints: [{ x: box.x + 360, y: box.y + 340 }],
  });
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await expect.poll(() => canvas.getAttribute("data-view")).not.toBe(before);
  await page
    .locator('swath-canvas-node[node-id="ndvi"] swath-canvas-port[name="out"] button')
    .tap();
  await page
    .locator('swath-canvas-node[node-id="save"] swath-canvas-port[name="data"] button')
    .tap();
  await expect(page.locator("#log")).toContainText("connect ndvi.out → save.data");
});
