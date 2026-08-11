// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The authoring loop through the UI (issue #109, ADR 0010), against the
// real stack in both modes: the panel's forms — generated from the
// server's own GET /processes, the collection picker fed by GET
// /collections — compose the NDVI graph, publish it, and the served
// tiles are BYTE-identical to the built-in NDVI layer (the existing
// openeo_services.rs assertion, now UI-driven). Validation gates submit
// until the graph is structurally valid; a graph the server still
// rejects renders its diagnostic on the offending field; deleting a
// published service 404s its tile URL; and the NDVI template publishes
// a working layer from one click.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The proven-live fixture tile (z/y/x — OGC tileMatrix/tileRow/tileCol):
 * the granule tests/e2e/stack-up.sh drops, same tile the Rust
 * byte-identity test uses (crates/swath-api/tests/openeo_services.rs). */
const TILE = "12/1561/848";

function paletteButton(page: Page, processId: string) {
  return page.locator(
    `swath-authoring-panel .swath-authoring-palette button[data-process="${processId}"]`,
  );
}

function fieldById(page: Page, id: string) {
  return page.locator(`#swath-authoring-${id}`);
}

/** The panel is collapsed and lazy (fetches nothing until opened, like
 * the dataset browser): every flow starts by toggling it open. */
async function openPanel(page: Page): Promise<void> {
  await page.locator("swath-authoring-panel .swath-authoring-toggle").click();
}

function submitButton(page: Page) {
  return page.locator("swath-authoring-panel .swath-authoring-submit");
}

/** Composes the NDVI pipeline through the generated forms. The data-flow
 * selects need no touch: cube parameters (and each step's first required
 * parameter) wire to the previous step by schema-derived default. */
async function authorNdvi(page: Page, outputMax: string, colormap: string): Promise<void> {
  await paletteButton(page, "load_collection").click();
  await paletteButton(page, "ndvi").click();
  await paletteButton(page, "linear_scale_range").click();
  await paletteButton(page, "save_result").click();
  // The collection is a picker fed by GET /collections — no free text.
  await fieldById(page, "s1-id").selectOption("hls-s30");
  await fieldById(page, "s1-bands").fill("b8a,b04");
  await fieldById(page, "s2-nir").fill("b8a");
  await fieldById(page, "s2-red").fill("b04");
  await fieldById(page, "s3-inputMin").fill("-1");
  await fieldById(page, "s3-inputMax").fill("1");
  await fieldById(page, "s4-options").selectOption(colormap);
  // Extents, output range, and format ride their smart defaults
  // (null / 0..255 / png) under the advanced toggles; only a non-default
  // output range needs the s3 advanced section opened.
  if (outputMax !== "255") {
    await page.locator('[data-step="s3"] .swath-authoring-advanced-toggle').click();
    await fieldById(page, "s3-outputMax").fill(outputMax);
  }
}

/** Publishes the composed graph and returns the openEO service id from
 * the creation response. */
async function publish(page: Page): Promise<string> {
  const created = page.waitForResponse(
    (response) => response.url().includes("/services") && response.request().method() === "POST",
  );
  await submitButton(page).click();
  const response = await created;
  expect(response.status()).toBe(201);
  const id = response.headers()["openeo-identifier"];
  if (!id) {
    throw new Error("creation response carried no OpenEO-Identifier header");
  }
  return id;
}

test("UI-authored NDVI serves tiles byte-identical to the built-in layer, no reload", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);
  await expect(paletteButton(page, "load_collection")).toBeVisible();

  await authorNdvi(page, "255", "rdylgn");
  await fieldById(page, "title").fill("NDVI (authored)");
  const id = await publish(page);

  // The authored layer appears in the layer browser immediately — same
  // page, no reload — and becomes the viewed layer.
  const layerButton = page.locator(`swath-layer-panel button[data-layer="${id}"]`);
  await expect(layerButton).toBeVisible();
  await expect(layerButton).toContainText("NDVI (authored)");
  await expect(layerButton).toHaveAttribute("aria-pressed", "true");

  // THE assertion, UI-driven: the authored service's tile bytes equal
  // the built-in NDVI layer's — same compiler, same serve path.
  const authored = await page.request.get(`/tilesets/${id}/tiles/${TILE}`);
  const builtin = await page.request.get(`/tilesets/ndvi/tiles/${TILE}`);
  expect(authored.status()).toBe(200);
  expect(builtin.status()).toBe(200);
  const authoredBytes = await authored.body();
  const builtinBytes = await builtin.body();
  expect(authoredBytes.length).toBeGreaterThan(0);
  expect(authoredBytes.equals(builtinBytes)).toBe(true);
});

test("validation gates submit until the graph is structurally valid", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);
  await expect(paletteButton(page, "load_collection")).toBeVisible();

  // A lone load_collection step with no collection chosen: submit is
  // disabled and the reasons are spelled out — the server's "no
  // load_collection node names a collection" rejection is unreachable.
  await paletteButton(page, "load_collection").click();
  await expect(submitButton(page)).toBeDisabled();
  const reason = page.locator("#swath-authoring-submit-reason");
  await expect(reason).toContainText("field needs a value");
  await expect(reason).toContainText("no step loads a collection");

  // The advanced (defaulted) fields are out of the default view: the
  // nullable extents hide until the step's advanced toggle opens them,
  // each with a visible plain-language explainer.
  await expect(fieldById(page, "s1-spatial_extent")).toHaveCount(0);
  await page.locator('[data-step="s1"] .swath-authoring-advanced-toggle').click();
  await expect(fieldById(page, "s1-spatial_extent")).toBeVisible();
  await expect(
    page.locator('label[for="swath-authoring-s1-spatial_extent"] .swath-authoring-field-help'),
  ).toContainText("leave as is to use the whole collection");

  // Choosing from the /collections-fed picker satisfies both reasons.
  await fieldById(page, "s1-id").selectOption("hls-s30");
  await expect(submitButton(page)).toBeEnabled();
  await expect(reason).toBeEmpty();
});

test("a graph the server rejects renders its diagnostic on the offending field", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);
  await expect(paletteButton(page, "load_collection")).toBeVisible();

  // Client-side valid, semantically wrong: the unsupported 0..1 output
  // range. The compiler's diagnostic names node and argument, so it
  // lands inline on exactly that field.
  await authorNdvi(page, "1", "rdylgn");
  await expect(submitButton(page)).toBeEnabled();
  await submitButton(page).click();
  const note = page.locator("#swath-authoring-s3-outputMin-note");
  await expect(note).toContainText("the output range must be exactly 0..255");
  await expect(page.locator("swath-authoring-panel .swath-authoring-error")).toHaveCount(0);
});

test("the NDVI template publishes a working layer from one click", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);
  const template = page.locator("swath-authoring-panel .swath-authoring-template");
  await expect(template).toBeVisible();
  await template.click();

  // A start-from-working-graph: collection, bands, scale, and colormap
  // prefilled from the server's own metadata — immediately submittable,
  // and narrated in plain words.
  await expect(submitButton(page)).toBeEnabled();
  await expect(page.locator("#swath-authoring-narrative")).toContainText("compute NDVI");
  await expect(page.locator("#swath-authoring-narrative")).toContainText("colored with rdylgn");
  const id = await publish(page);
  const tile = await page.request.get(`/tilesets/${id}/tiles/${TILE}`);
  expect(tile.status()).toBe(200);
});

test("deleting a published service 404s its tile URL and drops it from the browser", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);
  await expect(paletteButton(page, "load_collection")).toBeVisible();

  // A distinct graph (grayscale) so this test owns its service id.
  await authorNdvi(page, "255", "grayscale");
  await fieldById(page, "title").fill("NDVI (deletable)");
  const id = await publish(page);
  await expect(page.locator(`swath-layer-panel button[data-layer="${id}"]`)).toBeVisible();
  const live = await page.request.get(`/tilesets/${id}/tiles/${TILE}`);
  expect(live.status()).toBe(200);

  // Delete from the panel's published-services list.
  const deleted = page.waitForResponse(
    (response) =>
      response.url().includes(`/services/${id}`) && response.request().method() === "DELETE",
  );
  await page.getByRole("button", { name: `Delete ${id}` }).click();
  expect((await deleted).status()).toBe(204);

  // Gone from serving (the honest 404) and from the layer browser.
  await expect
    .poll(async () => (await page.request.get(`/tilesets/${id}/tiles/${TILE}`)).status())
    .toBe(404);
  await expect(page.locator(`swath-layer-panel button[data-layer="${id}"]`)).toHaveCount(0);
});
