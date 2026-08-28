// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The add-data panel (issue #197) against the real stack in both modes:
// paste a link to a fixture COG → the #196 registration flow → the
// quick-look layer appears in the rail, serves, and its tiles are traced
// (the x-ray header) — everything through the engine, no client-side
// rendering (ADR 0019). Plus the routed flows kept deterministic by
// page.route fixtures: the server-refusal path (an RFC 7807 problem
// rendered under the link field), the /?stac= "Open in Swath" deep link
// (pre-filled, nothing registered, URL bytes untouched), and read-only
// capabilities hiding the form (#198 — a second server flavor the compose
// stack does not run, so the capabilities document is the fixture).
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The proven-live fixture tile (z/y/x — OGC tileMatrix/tileRow/tileCol)
 * of the granule tests/e2e/stack-up.sh drops. */
const TILE = "12/1561/848";

/** A fixture COG key the stack's store really holds (stack-up drops it). */
const FIXTURE_COG = "hlss30-t13sdd-2024158-b04.tif";

async function openPanel(page: Page): Promise<void> {
  await page.locator("swath-add-data-panel .swath-add-data-toggle").click();
}

/** Fields are <swath-field>s in the panel's shadow root (#289): the
 * native control is one level deeper (Playwright pierces both). */
function field(page: Page, id: string) {
  return page.locator(`#swath-add-data-${id} input`);
}

/** Pastes a link and blurs so the panel inspects it. */
async function paste(page: Page, link: string): Promise<void> {
  await field(page, "link").fill(link);
  await field(page, "link").blur();
}

test("paste a fixture COG: registered, in the rail, serving, traced", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);

  // The writable stack advertises registration; the form renders.
  await paste(page, FIXTURE_COG);
  // Ids are seeded from the file name; the acquisition instant is ours.
  await expect(field(page, "dataset")).toHaveValue("hlss30-t13sdd-2024158-b04");
  await field(page, "datetime").fill("2024-06-06T17:54:00Z");

  const created = page.waitForResponse(
    (response) => response.url().includes("/services") && response.request().method() === "POST",
  );
  await page.locator("swath-add-data-panel .swath-add-data-submit").click();
  const response = await created;
  expect(response.status()).toBe(201);
  const id = response.headers()["openeo-identifier"];
  if (!id) {
    throw new Error("service creation carried no OpenEO-Identifier header");
  }

  // The quick look appears in the layer rail immediately — no reload —
  // and becomes the viewed layer.
  const layerButton = page.locator(`swath-layer-item[data-layer="${id}"] [part="row"]`);
  await expect(layerButton).toBeVisible();
  await expect(layerButton).toContainText("quick look");
  await expect(layerButton).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("swath-add-data-panel .swath-add-data-status")).toContainText(
    "Serving",
  );

  // Served through the engine and traced: the tile carries the x-ray
  // debug header every rendered tile does.
  const tile = await page.request.get(`/tilesets/${id}/tiles/${TILE}`);
  expect(tile.status()).toBe(200);
  expect(tile.headers()["content-type"]).toBe("image/png");
  expect(tile.headers()["x-swath-trace"]).toBeTruthy();
});

test("a file the server cannot read renders its refusal under the link", async ({ page }) => {
  // Deterministic server answers (and no junk datasets left in the real
  // catalog): the registration routes are fixtures; the refusal body is
  // the server's exact RFC 7807 shape.
  await page.route("**/datasets", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 201, json: { id: "junk" } })
      : route.fallback(),
  );
  await page.route("**/datasets/junk/granules", (route) =>
    route.fulfill({
      status: 400,
      json: {
        type: "about:blank",
        title: "Bad Request",
        status: 400,
        detail: "asset `data` (junk.tif) failed header validation: object not found",
      },
    }),
  );
  const services: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/services")) {
      services.push(request.url());
    }
  });

  await page.goto(DEMO_PATH);
  await openPanel(page);
  await paste(page, "junk.tif");
  await field(page, "datetime").fill("2024-06-06T17:54:00Z");
  await page.locator("swath-add-data-panel .swath-add-data-submit").click();

  // The problem lands under the field that caused it, in plain words —
  // and the quick look never fires.
  await expect(page.locator('#swath-add-data-link [part="error"]')).toContainText(
    "could not read that file",
  );
  expect(services).toEqual([]);
});

test("the /?stac= deep link pre-fills the flow and registers nothing", async ({ page }) => {
  const itemUrl = "https://demo.invalid/e2e-item.json";
  await page.route("**/e2e-item.json", (route) =>
    route.fulfill({
      json: {
        type: "Feature",
        stac_version: "1.1.0",
        id: "deep-linked-scene",
        collection: "hls-s30",
        bbox: [-105.5, 39.2, -105.4, 39.3],
        properties: { datetime: "2024-06-06T17:54:00Z" },
        assets: { b04: { href: FIXTURE_COG } },
      },
      headers: { "access-control-allow-origin": "*" },
    }),
  );
  const posts: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST") {
      posts.push(request.url());
    }
  });

  const entry = `${DEMO_PATH}?stac=${encodeURIComponent(itemUrl)}`;
  await page.goto(entry);

  // Open, pre-filled from the fetched item — registering stays a click.
  await expect(page.locator("swath-add-data-panel .swath-add-data-toggle button")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await expect(field(page, "link")).toHaveValue(itemUrl);
  await expect(field(page, "dataset")).toHaveValue("hls-s30");
  expect(posts).toEqual([]);

  // Byte-stability (issue #108's contract): the pasted deep link is
  // never rewritten on load — `stac` is a pass-through param.
  expect(new URL(page.url()).search).toBe(`?stac=${encodeURIComponent(itemUrl)}`);
});

test("read-only capabilities hide the form (capabilities-driven, not hardcoded)", async ({
  page,
}) => {
  // The compose stack is writable; a --read-only server's capabilities
  // document (#198: write methods filtered out) is the routed fixture.
  await page.route(
    (url) => url.pathname === "/",
    (route) =>
      route.request().resourceType() === "fetch"
        ? route.fulfill({
            json: {
              title: "Swath",
              endpoints: [
                { path: "/datasets/{dataset_id}/granules", methods: ["GET"] },
                { path: "/services", methods: ["GET"] },
                { path: "/result", methods: ["POST"] },
              ],
            },
          })
        : route.fallback(),
  );

  await page.goto(DEMO_PATH);
  await openPanel(page);
  await expect(page.locator("swath-add-data-panel .swath-add-data-readonly")).toContainText(
    "read-only",
  );
  await expect(page.locator("swath-add-data-panel form")).toHaveCount(0);
  await expect(field(page, "link")).toHaveCount(0);
});
