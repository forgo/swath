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
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The guest kit's reference NDVI module (ADR 0020, `examples/udf/ndvi`
 * — the committed fixture the wasmtime adapter's goldens pin): two
 * input planes (nir, red in load order) to one gray plane. */
const NDVI_WASM = readFileSync(
  fileURLToPath(
    new URL("../../crates/adapters/swath-udf-wasmtime/tests/fixtures/ndvi.wasm", import.meta.url),
  ),
);

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

// --- A hand-assembled ABI v1 module, one failure mode (issue #208) ---
// The same no-toolchain posture as crates/swath-api/tests/common/wasm.rs
// (which this mirrors byte for byte): structurally conforming — the four
// v1 exports plus memory — so it registers, with a `swath_udf_run` that
// spins until the per-tile fuel budget (or the epoch backstop) stops it.

function uleb(value: number): number[] {
  const out: number[] = [];
  let v = value;
  for (;;) {
    const byte = v & 0x7f;
    v = Math.floor(v / 128);
    if (v === 0) {
      out.push(byte);
      return out;
    }
    out.push(byte | 0x80);
  }
}

function sleb(value: number): number[] {
  const out: number[] = [];
  let v = value;
  for (;;) {
    const byte = v & 0x7f;
    v >>= 7;
    const sign = (byte & 0x40) !== 0;
    if ((v === 0 && !sign) || (v === -1 && sign)) {
      out.push(byte);
      return out;
    }
    out.push(byte | 0x80);
  }
}

function section(id: number, payload: number[]): number[] {
  return [id, ...uleb(payload.length), ...payload];
}

function counted(items: number[][]): number[] {
  return [...uleb(items.length), ...items.flat()];
}

function name(text: string): number[] {
  const bytes = [...new TextEncoder().encode(text)];
  return [...uleb(bytes.length), ...bytes];
}

/** `i32.const k` then `end`. */
function retI32(k: number): number[] {
  return [0x41, ...sleb(k), 0x0b];
}

/** A structurally conforming ABI v1 module (abi = 1, one output plane,
 * `swath_udf_alloc` answering 8 inside a 4 MiB memory) whose
 * `swath_udf_run` body is `run`. */
function abiModule(run: number[]): Buffer {
  const exportEntry = (n: string, kind: number, index: number): number[] => [
    ...name(n),
    kind,
    ...uleb(index),
  ];
  const body = (code: number[]): number[] => {
    const entry = [0x00, ...code]; // zero locals
    return [...uleb(entry.length), ...entry];
  };
  return Buffer.from([
    0x00,
    0x61,
    0x73,
    0x6d,
    0x01,
    0x00,
    0x00,
    0x00,
    // Types: 0 = () -> i32, 1 = (i32) -> i32, 2 = (i32, i32) -> i64.
    ...section(
      1,
      counted([
        [0x60, 0x00, 0x01, 0x7f],
        [0x60, 0x01, 0x7f, 0x01, 0x7f],
        [0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e],
      ]),
    ),
    // Functions: abi, output_planes, alloc, run.
    ...section(3, counted([[0], [1], [1], [2]])),
    // Memory: 64 pages (4 MiB), no max.
    ...section(5, counted([[0x00, 0x40]])),
    ...section(
      7,
      counted([
        exportEntry("memory", 0x02, 0),
        exportEntry("swath_udf_abi", 0x00, 0),
        exportEntry("swath_udf_output_planes", 0x00, 1),
        exportEntry("swath_udf_alloc", 0x00, 2),
        exportEntry("swath_udf_run", 0x00, 3),
      ]),
    ),
    ...section(10, counted([body(retI32(1)), body(retI32(1)), body(retI32(8)), body(run)])),
  ]);
}

/** The fuel bomb: `swath_udf_run` is `(loop (br 0))`. */
const FUEL_BOMB = abiModule([0x03, 0x40, 0x0c, 0x00, 0x0b, 0x42, 0x00, 0x0b]);

/** Picks `buffer` as a `.wasm` through the UDF card's file input. */
async function uploadModule(page: Page, id: string, fileName: string, buffer: Buffer) {
  await fieldById(page, id).setInputFiles({ name: fileName, mimeType: "application/wasm", buffer });
}

/** The `X-Swath-Trace` debug summary of a tile/preview response. */
function traceHeader(headers: Record<string, string>): {
  decision?: string;
  udf_fuel_used?: number;
} {
  return JSON.parse(headers["x-swath-trace"] ?? "{}") as {
    decision?: string;
    udf_fuel_used?: number;
  };
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
  const layerButton = page.locator(`swath-layer-item[data-layer="${id}"] [part="row"]`);
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

test("a complete draft shows its live preview image before anything is published", async ({
  page,
}) => {
  // B11's countermeasure against the real stack (issue #169, ADR 0014):
  // the moment the NDVI template completes the draft, the panel
  // previews it through POST /result — a real PNG rendered by the same
  // compiler path publishing would use — with nothing published yet.
  await page.goto(DEMO_PATH);
  await openPanel(page);
  const published: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/services")) {
      published.push(request.url());
    }
  });
  const preview = page.waitForResponse(
    (response) => response.url().includes("/result") && response.request().method() === "POST",
  );
  await page.locator("swath-authoring-panel .swath-authoring-template").click();
  const response = await preview;
  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toBe("image/png");
  const image = page.locator("#swath-authoring-preview-image");
  await expect(image).toBeVisible();
  await expect(image).toHaveAttribute("src", /^blob:/);
  // The blob decodes to real pixels.
  await expect
    .poll(() => image.evaluate((element) => (element as HTMLImageElement).naturalWidth))
    .toBeGreaterThan(0);
  await expect(page.locator("#swath-authoring-preview-note")).toContainText("Preview");
  // Seeing the draft published nothing: this page never POSTed a
  // service (previewing is side-effect-free by construction).
  expect(published).toEqual([]);
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
  await expect(page.locator(`swath-layer-item[data-layer="${id}"] [part="row"]`)).toBeVisible();
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
  await expect(page.locator(`swath-layer-item[data-layer="${id}"] [part="row"]`)).toHaveCount(0);
});

test("the layer row's kebab deletes a published service (issue #282)", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);
  await authorNdvi(page, "255", "grayscale");
  await fieldById(page, "title").fill("NDVI (kebab-deletable)");
  const id = await publish(page);
  const item = page.locator(`swath-layer-item[data-layer="${id}"]`);
  await expect(item).toBeVisible();

  // The delete action exists only on rows the server lists as services.
  await item.locator('[part="menu"] swath-button button').click();
  const deleted = page.waitForResponse(
    (response) =>
      response.url().includes(`/services/${id}`) && response.request().method() === "DELETE",
  );
  await item.locator('[part="menu"] [part="item"][data-id="delete"]').click();
  expect((await deleted).status()).toBe(204);
  await expect
    .poll(async () => (await page.request.get(`/tilesets/${id}/tiles/${TILE}`)).status())
    .toBe(404);
  await expect(item).toHaveCount(0);
});

// --- The UDF stage (issue #208, ADR 0018): the run_udf loop end to end ---
// The compose stack is wired with `udf-store` (tests/e2e/swath-catalog.toml),
// so GET /processes lists run_udf and the chip exists; the guest kit's
// reference NDVI module goes upload → preview (with fuel on the trace
// header) → publish → x-ray (fuel per tile from the SSE stream, in the
// inspector facts and the analytics line) → delete → 404.

test("UDF stage: upload → preview → publish → x-ray shows fuel → delete → 404", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);
  // X-ray on FIRST: the overlay subscribes to /traces on toggle, and
  // the published layer's first renders (fuel-bearing, uncached) happen
  // the moment publishing switches the map to it.
  const toggle = page.getByRole("button", { name: "Toggle x-ray overlay" });
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");

  await fieldById(page, "s1-id").selectOption("hls-s30");
  await tickBand(page, "b8a");
  await tickBand(page, "b04");
  // The chip exists only because the server serves run_udf (--udf-store).
  await chip(page, 0, "run_udf").click();
  // The module's context is part of the stage's cache identity (#265),
  // so a per-run value keeps this run's tiles unseen by the write-through
  // cache — `just e2e-web` runs both modes against ONE stack, and a
  // cache hit never runs the module (no fuel to show). The reference
  // module ignores it; the JSON field itself is exercised on the way.
  await fieldById(page, "s2-context").fill(
    JSON.stringify({ run: `${process.env.SWATH_E2E_MODE ?? "vite"}-${Date.now()}` }),
  );
  const preview = page.waitForResponse(
    (response) => response.url().includes("/result") && response.request().method() === "POST",
  );
  await uploadModule(page, "s2-udf", "ndvi.wasm", NDVI_WASM);
  await expect(page.locator("#swath-authoring-s2-udf-module")).toContainText("ndvi.wasm");
  // The preview renders the module through POST /result — the ADR 0014
  // loop as the UDF validation loop (#206) — and meters its fuel.
  const previewed = await preview;
  expect(previewed.status()).toBe(200);
  expect(previewed.headers()["content-type"]).toBe("image/png");
  expect(traceHeader(previewed.headers()).udf_fuel_used).toBeGreaterThan(0);
  const image = page.locator("#swath-authoring-preview-image");
  await expect(image).toBeVisible();
  await expect(image).toHaveAttribute("src", /^blob:/);

  // Stage-typed: only the stretch step fits, after the module; the
  // colormap greys out with the UDF reason (its output renders directly).
  await expect(page.locator('.swath-authoring-insert[data-gap="0"]')).toHaveCount(0);
  await chip(page, 1, "linear_scale_range").click();
  await fieldById(page, "s3-inputMin").fill("-1");
  await fieldById(page, "s3-inputMax").fill("1");
  await expect(fieldById(page, "s4-options")).toBeDisabled();
  await expect(fieldById(page, "s4-composite-note")).toContainText("renders directly");
  await expect(page.locator("#swath-authoring-narrative")).toContainText(
    "run ndvi.wasm on the bands",
  );
  await fieldById(page, "title").fill("NDVI (module)");
  const id = await publish(page);

  // The published layer is the viewed layer, rendering live through
  // the same executor: the x-ray's analytics line narrates the latest
  // UDF tile's fuel and udf_ms from the trace stream…
  const udfLine = page.locator(".swath-xray-analytics-udf");
  await expect(udfLine).toBeVisible({ timeout: 60_000 });
  // Exact values ride the card's data-* attributes (like p50/p95).
  const card = page.locator(".swath-xray-analytics");
  await expect(card).toHaveAttribute("data-udf-tile", new RegExp(`^${id}/`));
  const fuel = Number(await card.getAttribute("data-udf-fuel"));
  expect(fuel).toBeGreaterThan(0);
  await expect(udfLine).toContainText(/fuel \d+/);
  // …and a live badge's inspector carries the same facts per tile.
  const badge = page.locator(`.swath-xray-badge[data-key^="${id}/"][data-decision="live"]`);
  await expect(badge.first()).toBeAttached({ timeout: 60_000 });
  await badge.first().click();
  const inspector = page.locator(".swath-xray-inspector");
  await expect(inspector).toBeVisible();
  await expect(inspector).toContainText("udf fuel");
  await expect(inspector).toContainText(/udf \d+/);
  await page.keyboard.press("Escape");

  // The served tile meters fuel on its trace header like the preview
  // did — unless the map already rendered it and the write-through
  // cache answers: a cache hit never runs the module, so it carries no
  // fuel, honestly (the tiler's contract).
  const tile = await page.request.get(`/tilesets/${id}/tiles/${TILE}`);
  expect(tile.status()).toBe(200);
  expect(tile.headers()["content-type"]).toBe("image/png");
  const served = traceHeader(tile.headers());
  if (served.decision === "cache_hit") {
    expect(served.udf_fuel_used).toBeUndefined();
  } else {
    expect(served.udf_fuel_used).toBeGreaterThan(0);
  }

  // Delete from the panel: gone from serving (the honest 404).
  const deleted = page.waitForResponse(
    (response) =>
      response.url().includes(`/services/${id}`) && response.request().method() === "DELETE",
  );
  await page.getByRole("button", { name: `Delete ${id}` }).click();
  expect((await deleted).status()).toBe(204);
  await expect
    .poll(async () => (await page.request.get(`/tilesets/${id}/tiles/${TILE}`)).status())
    .toBe(404);
});

test("UDF stage: a fuel bomb's refusal reads in plain words on the module and never gates a different valid draft", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await openPanel(page);
  await fieldById(page, "s1-id").selectOption("hls-s30");
  await tickBand(page, "b8a");
  await tickBand(page, "b04");
  await chip(page, 0, "run_udf").click();
  const preview = page.waitForResponse(
    (response) => response.url().includes("/result") && response.request().method() === "POST",
  );
  await uploadModule(page, "s2-udf", "bomb.wasm", FUEL_BOMB);
  // The server refuses under the same per-tile budget publishing would
  // enforce: ProcessGraphComplexity (#206), mapped onto the module field
  // in the user's words — the fuel meter, or its wall-clock backstop.
  const refused = await preview;
  expect(refused.status()).toBe(400);
  expect(((await refused.json()) as { code: string }).code).toBe("ProcessGraphComplexity");
  const note = page.locator("#swath-authoring-s2-udf-note");
  await expect(note).toContainText("per-tile budget");
  await expect(note).toContainText("Publishing is not blocked");
  await expect(page.locator("#swath-authoring-preview-note")).toContainText(
    "see the note on step s2",
  );
  await expect(page.locator("#swath-authoring-preview-image")).toBeHidden();
  await expect(submitButton(page)).toBeEnabled();
  await expect(page.locator("swath-authoring-panel .swath-authoring-error")).toHaveCount(0);

  // A different, valid draft on the same canvas publishes and serves:
  // drop the module, author NDVI in its place.
  await page.getByRole("button", { name: "Remove step s2" }).click();
  await chip(page, 0, "ndvi").click();
  await chip(page, 1, "linear_scale_range").click();
  await fieldById(page, "s3-inputMin").fill("-1");
  await fieldById(page, "s3-inputMax").fill("1");
  await fieldById(page, "title").fill("NDVI (after the bomb)");
  const id = await publish(page);
  const tile = await page.request.get(`/tilesets/${id}/tiles/${TILE}`);
  expect(tile.status()).toBe(200);
});
