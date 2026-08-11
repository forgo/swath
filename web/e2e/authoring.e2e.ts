// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The authoring loop through the Model B canvas (issues #109/#151, ADR
// 0010), against the real stack in both modes: the always-valid
// pipeline — permanent Load → Output frame, stage-typed insert chips,
// vocabulary-only band widgets — composes the NDVI graph and the served
// tiles are BYTE-identical to the built-in NDVI layer (the existing
// openeo_services.rs assertion, UI-driven). The formula builder's
// reduce_dimension child graph publishes and matches the same bytes
// (the design note's "e2e-prove every palette-offered insertion
// actually publishes"); an RGB composite publishes too. Validation
// gates submit with plain-words reasons; a graph the server still
// rejects renders its diagnostic on the offending field (the safety
// net); deleting a published service 404s its tile URL; the NDVI
// template publishes a working layer from one click.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The proven-live fixture tile (z/y/x — OGC tileMatrix/tileRow/tileCol):
 * the granule tests/e2e/stack-up.sh drops, same tile the Rust
 * byte-identity test uses (crates/swath-api/tests/openeo_services.rs). */
const TILE = "12/1561/848";

function fieldById(page: Page, id: string) {
  return page.locator(`#swath-authoring-${id}`);
}

/** The stage-typed insert chip for `processId` at `gap` (0 = right
 * after the Load card). */
function chip(page: Page, gap: number, processId: string) {
  return page.locator(
    `swath-authoring-panel .swath-authoring-insert[data-gap="${gap}"] ` +
      `button[data-process="${processId}"]`,
  );
}

/** The panel is collapsed and lazy (fetches nothing until opened, like
 * the dataset browser): every flow starts by toggling it open. The
 * permanent Load card (s1) rendering means the canvas is ready. */
async function openPanel(page: Page): Promise<void> {
  await page.locator("swath-authoring-panel .swath-authoring-toggle").click();
  await expect(page.locator('swath-authoring-panel [data-step="s1"]')).toBeVisible();
}

function submitButton(page: Page) {
  return page.locator("swath-authoring-panel .swath-authoring-submit");
}

/** Ticks a Load-card band checkbox (tick order = loaded order). */
async function tickBand(page: Page, band: string): Promise<void> {
  await fieldById(page, `s1-bands-${band}`).check();
}

/** Composes the NDVI pipeline on the Model B canvas: collection from
 * the /collections-fed select, bands ticked from the vocabulary
 * checkboxes, NDVI and the stretch step inserted through their
 * stage-typed chips (nir/red arrive prefilled by the band heuristics —
 * asserted, not set). Extents and format ride their smart defaults. */
async function authorNdvi(page: Page, outputMax: string, colormap: string): Promise<void> {
  await fieldById(page, "s1-id").selectOption("hls-s30");
  await tickBand(page, "b8a");
  await tickBand(page, "b04");
  await chip(page, 0, "ndvi").click();
  // The band selects (vocabulary-only, B7) prefilled nir/red correctly.
  await expect(fieldById(page, "s2-nir")).toHaveValue("b8a");
  await expect(fieldById(page, "s2-red")).toHaveValue("b04");
  await chip(page, 1, "linear_scale_range").click();
  await fieldById(page, "s3-inputMin").fill("-1");
  await fieldById(page, "s3-inputMax").fill("1");
  await fieldById(page, "s4-options").selectOption(colormap);
  // Only a non-default output range needs the s3 advanced section.
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

test("the formula builder's reducer child graph compiles to the same NDVI bytes", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);

  // NDVI written by hand in the formula builder — the only place
  // arithmetic exists on the canvas (B2/B3): line1 = b8a − b04,
  // line2 = b8a + b04, line3 = line1 ÷ line2.
  await fieldById(page, "s1-id").selectOption("hls-s30");
  await tickBand(page, "b8a");
  await tickBand(page, "b04");
  await chip(page, 0, "reduce_dimension").click();
  await fieldById(page, "s2-row1-left").selectOption("band:b8a");
  await fieldById(page, "s2-row1-op").selectOption("subtract");
  await fieldById(page, "s2-row1-right").selectOption("band:b04");
  await page.locator("swath-authoring-panel .swath-authoring-formula-add").click();
  await fieldById(page, "s2-row2-left").selectOption("band:b8a");
  await fieldById(page, "s2-row2-op").selectOption("add");
  await fieldById(page, "s2-row2-right").selectOption("band:b04");
  await page.locator("swath-authoring-panel .swath-authoring-formula-add").click();
  await fieldById(page, "s2-row3-left").selectOption("row:0");
  await fieldById(page, "s2-row3-op").selectOption("divide");
  await fieldById(page, "s2-row3-right").selectOption("row:1");
  // The narrative reads the formula back as plain math.
  await expect(page.locator("#swath-authoring-narrative")).toContainText(
    "(b8a − b04) ÷ (b8a + b04)",
  );
  await chip(page, 1, "linear_scale_range").click();
  await fieldById(page, "s3-inputMin").fill("-1");
  await fieldById(page, "s3-inputMax").fill("1");
  await fieldById(page, "s4-options").selectOption("rdylgn");
  await fieldById(page, "title").fill("NDVI (formula)");
  const id = await publish(page);

  // Same expression, same plan, same bytes as the built-in NDVI layer:
  // the composed reduce_dimension child graph is the real thing.
  const authored = await page.request.get(`/tilesets/${id}/tiles/${TILE}`);
  const builtin = await page.request.get(`/tilesets/ndvi/tiles/${TILE}`);
  expect(authored.status()).toBe(200);
  const authoredBytes = await authored.body();
  expect(authoredBytes.length).toBeGreaterThan(0);
  expect(authoredBytes.equals(await builtin.body())).toBe(true);
});

test("an RGB composite (3 ticked bands, stretch, no reduce) publishes and serves", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);

  // Tick order is loaded order — red, green, blue.
  await fieldById(page, "s1-id").selectOption("hls-s30");
  await tickBand(page, "b04");
  await tickBand(page, "b03");
  await tickBand(page, "b02");
  // The colormap greys out on a multi-band result, with the plain-words
  // note (B6) — and the composite path still publishes fine.
  // (No middle steps yet, so the Output card is s2.)
  await expect(fieldById(page, "s2-options")).toBeDisabled();
  await expect(fieldById(page, "s2-composite-note")).toContainText("one gray value per pixel");
  await chip(page, 0, "linear_scale_range").click();
  await fieldById(page, "s2-inputMin").fill("0");
  await fieldById(page, "s2-inputMax").fill("3000");
  await fieldById(page, "title").fill("True color (authored)");
  const id = await publish(page);
  const tile = await page.request.get(`/tilesets/${id}/tiles/${TILE}`);
  expect(tile.status()).toBe(200);
});

test("the canvas keeps the pipeline valid: permanent frame, typed chips, plain reasons", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);

  // The empty canvas is already the whole frame: Load (s1) → Output
  // (s2), neither removable — "the graph must end in save_result" (B1)
  // is unconstructible, and no chip anywhere offers arithmetic (B2).
  await expect(page.locator('swath-authoring-panel [data-process="save_result"]')).toBeVisible();
  await expect(
    page.locator('swath-authoring-panel [data-step="s2"] [aria-label^="Remove step"]'),
  ).toHaveCount(0);
  for (const forbidden of ["divide", "add", "subtract", "multiply", "array_element"]) {
    await expect(
      page.locator(
        `swath-authoring-panel .swath-authoring-insert button[data-process="${forbidden}"]`,
      ),
    ).toHaveCount(0);
  }

  // Submit is gated with the reasons in the user's words.
  await expect(submitButton(page)).toBeDisabled();
  const reason = page.locator("#swath-authoring-submit-reason");
  await expect(reason).toContainText("no collection chosen yet");
  await fieldById(page, "s1-id").selectOption("hls-s30");
  await expect(reason).toContainText("no bands ticked yet");

  // Two bands, no reduce: B5's explanation, still gated.
  await tickBand(page, "b8a");
  await tickBand(page, "b04");
  await expect(reason).toContainText("produces 2 channels");

  // The extents stay expert fields under advanced, plain-worded on the
  // card itself.
  await expect(fieldById(page, "s1-extent-summary")).toContainText(
    "everywhere the collection covers",
  );
  await expect(fieldById(page, "s1-spatial_extent")).toHaveCount(0);
  await page.locator('[data-step="s1"] .swath-authoring-advanced-toggle').click();
  await expect(fieldById(page, "s1-spatial_extent")).toBeVisible();
  await expect(
    page.locator('label[for="swath-authoring-s1-spatial_extent"] .swath-authoring-field-help'),
  ).toContainText("leave as is to use the whole collection");

  // Adding NDVI (stage-typed chip) completes a lawful pipeline.
  await chip(page, 0, "ndvi").click();
  await expect(submitButton(page)).toBeEnabled();
  await expect(reason).toBeEmpty();
  // After the scale step, the saturated NDVI pipeline offers no further
  // insertions anywhere (B4 and friends: nothing else fits).
  await chip(page, 1, "linear_scale_range").click();
  await fieldById(page, "s3-inputMin").fill("-1");
  await fieldById(page, "s3-inputMax").fill("1");
  await expect(page.locator("swath-authoring-panel .swath-authoring-insert")).toHaveCount(0);
});

test("a graph the server rejects renders its diagnostic on the offending field", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);

  // Client-side valid, semantically wrong: the unsupported 0..1 output
  // range (an expert-advanced field — the one server rejection Model B
  // leaves reachable here). The compiler's diagnostic names node and
  // argument, so it lands inline on exactly that field: the safety net.
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
