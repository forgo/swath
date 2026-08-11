// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The docs screenshot suite (issue #112): every committed image under
// docs/media/screenshots/ is captured HERE, by `just screenshots`, from
// the fixture compose stack — never by hand, so the shots cannot silently
// go stale. Design constraints:
//
// - Fixture data only. The stack is tests/e2e/stack-up.sh (committed HLS
//   granule, dropped and polled live before this suite starts). No
//   basemap: `basemap=demo` would fetch third-party world tiles — off.
// - Deterministic by construction. Filenames, viewport, and DPR are
//   fixed; the recipe clears the tile cache before EACH capture run, so
//   the whole run replays the same request history from the same cold
//   state — x-ray decisions, plan mix, and cache counters reproduce, and
//   only timings/timestamps may differ. Each shot declares the pdiff
//   policy (tolerance, max bad-pixel fraction) that budget needs; the
//   recipe's second capture run is diffed shot-by-shot against the first
//   (tests/screenshots/verify_stable.py).
// - Provenance. After the last shot this suite writes shots.json (the
//   machine-readable manifest: caption, policy, sha256 per shot) and
//   index.md (the human index), both stamped with the capture git sha
//   (SWATH_CAPTURE_SHA, from the recipe).
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = "/demo/";

/** The demo viewpoint (same center `just demo` opens on) and the proven
 * fixture tile (z/y/x) the stack polls live before capture starts. */
const CENTER = "-105.4475,39.2650";
const TILE = "12/1561/848";

const OUT_DIR = process.env.SWATH_SHOTS_DIR ?? "";
if (OUT_DIR === "") {
  throw new Error("SWATH_SHOTS_DIR is required (run via `just screenshots`)");
}
const CAPTURE_SHA = process.env.SWATH_CAPTURE_SHA ?? "uncommitted";

/** One captured shot: filename, the one-line caption (what the shot
 * evidences), and the stability policy the second-run pdiff enforces. */
interface Shot {
  file: string;
  caption: string;
  /** pdiff per-channel tolerance (swath-testkit DiffPolicy). */
  tolerance: number;
  /** pdiff max fraction of pixels allowed past the tolerance. Shots that
   * show wall-clock timestamps or per-run timings budget for exactly that
   * text; pixel content everywhere else must reproduce. */
  maxBadFrac: number;
}

const captured: Shot[] = [];

async function capture(
  page: Page,
  file: string,
  caption: string,
  policy?: { tolerance?: number; maxBadFrac?: number },
): Promise<void> {
  // Let the last paint (overlay text, badge layout) settle before freezing.
  await page.waitForTimeout(400);
  mkdirSync(OUT_DIR, { recursive: true });
  await page.screenshot({
    path: path.join(OUT_DIR, file),
    animations: "disabled",
    caret: "hide",
  });
  captured.push({
    file,
    caption,
    tolerance: policy?.tolerance ?? 2,
    maxBadFrac: policy?.maxBadFrac ?? 0.005,
  });
}

/** Structural view of <swath-map> for in-page evaluation (same shape the
 * e2e suites use). */
interface SwathMapLike {
  map?: {
    loaded(): boolean;
    areTilesLoaded(): boolean;
    getZoom(): number;
    jumpTo(options: { zoom: number }): void;
  };
}

/** Waits until the map is up and settled on a real view: style loaded,
 * every needed tile loaded, and past the boot view (zoom 1). */
async function waitForFittedView(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const map = (document.querySelector("swath-map") as SwathMapLike | null)?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded() && map.getZoom() > 5);
  });
}

async function waitForMapIdle(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const map = (document.querySelector("swath-map") as SwathMapLike | null)?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded());
  });
}

/** Waits for the x-ray overlay to be painting: attached, at least one
 * badge, and (best effort) the ingest→pixel readout populated. */
async function waitForXRay(page: Page): Promise<void> {
  await expect(page.locator("swath-map .swath-xray")).toBeAttached();
  await page.waitForFunction(() => document.querySelectorAll(".swath-xray-badge").length > 0);
  await page
    .waitForFunction(
      () => {
        const readout = document.querySelector(".swath-xray-ingest");
        return readout?.textContent !== null && readout?.textContent.includes("ms") === true;
      },
      undefined,
      { timeout: 10_000 },
    )
    .catch(() => undefined); // cache-hit-only views may never learn i2p
}

function paletteButton(page: Page, processId: string) {
  return page.locator(
    `swath-authoring-panel .swath-authoring-palette button[data-process="${processId}"]`,
  );
}

function fieldById(page: Page, id: string) {
  return page.locator(`#swath-authoring-${id}`);
}

async function openAuthoringPanel(page: Page): Promise<void> {
  await page.locator("swath-authoring-panel .swath-authoring-toggle").click();
  await expect(paletteButton(page, "load_collection")).toBeVisible();
}

test("landing page: layer rail + default layer on a fitted view", async ({ page }) => {
  const tile = page.waitForResponse(
    (response) => response.url().includes("/tilesets/ndvi/tiles/") && response.status() === 200,
  );
  await page.goto(DEMO_PATH);
  await tile;
  await waitForFittedView(page);
  await capture(
    page,
    "01-landing-layer-rail.png",
    "Zero-config landing page: the layer rail beside the map, default NDVI layer fitted to the fixture granule's footprint.",
  );
});

test("colormapped NDVI at z12", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=12`);
  await waitForFittedView(page);
  await capture(
    page,
    "02-ndvi-colormapped.png",
    "Colormapped NDVI (rdylgn) rendered live at z12 from the fixture granule — nothing pre-baked.",
  );
});

test("true color at the same view", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?layer=truecolor&center=${CENTER}&zoom=12`);
  await waitForFittedView(page);
  await capture(
    page,
    "03-truecolor-live.png",
    "HLS true color at the same viewpoint: selecting a layer in the rail re-points the raster source.",
  );
});

test("x-ray decisions + why-view inspector", async ({ page }) => {
  // z13 tiles are untouched at this point in the run, so with the cold
  // per-run cache every badge shows a real live-render decision.
  await page.goto(`${DEMO_PATH}?xray&layer=truecolor&center=${CENTER}&zoom=13`);
  await waitForFittedView(page);
  await waitForXRay(page);
  await capture(
    page,
    "04-xray-decisions.png",
    "X-ray decision overlay: every tile badged with its render decision and timing; ingest→pixel readout top-left.",
    { maxBadFrac: 0.03 }, // per-tile ms text differs between runs
  );

  // "First badge" is arrival-order nondeterministic; inspect the
  // lexicographically smallest key so both capture runs open the SAME tile.
  const key = await page.evaluate(() => {
    const keys = [...document.querySelectorAll<HTMLElement>(".swath-xray-badge")]
      .map((badge) => badge.dataset.key ?? "")
      .filter((k) => k !== "");
    return keys.sort()[0];
  });
  if (key === undefined) {
    throw new Error("no x-ray badge carries a data-key");
  }
  await page.locator(`.swath-xray-badge[data-key="${key}"]`).first().click();
  const inspector = page.locator(".swath-xray-inspector");
  await expect(inspector).toBeVisible();
  await expect(inspector.locator(".swath-xray-plan tbody tr").first()).toBeVisible();
  await capture(
    page,
    "05-xray-why-view.png",
    "Why-view for one tile: the planner's candidate table — chosen plan, rejected candidates, and the reason for each.",
    { maxBadFrac: 0.03 },
  );
});

test("x-ray bytes heatmap + trace feed", async ({ page }) => {
  // NDVI at z13 is also untouched in this run: live renders with real
  // bytes_read, so the log-scale heatmap has an actual range to show.
  await page.goto(`${DEMO_PATH}?xray&layer=ndvi&center=${CENTER}&zoom=13`);
  await waitForFittedView(page);
  await waitForXRay(page);

  const modes = page.locator(".swath-xray-modes");
  await modes.getByRole("button", { name: "bytes", exact: true }).click();
  await expect(page.locator(".swath-xray-scale")).toBeVisible();
  await capture(
    page,
    "06-xray-heatmap.png",
    "Bytes-read heatmap mode: badges bucketed on a log scale, with the legend the overlay itself publishes.",
    { maxBadFrac: 0.03 },
  );

  await page.getByRole("button", { name: "trace feed" }).click();
  await expect(page.locator(".swath-xray-feed-lines")).toBeVisible();
  await expect(page.locator(".swath-xray-feed-lines li").first()).toBeVisible();
  await capture(
    page,
    "07-xray-trace-feed.png",
    "Trace feed: the /traces SSE stream as scrollback — every line a render decision, clickable back to its tile.",
    { maxBadFrac: 0.06 }, // feed lines carry wall-clock timestamps
  );
});

test("authoring panel: schema-driven form with field help", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=12`);
  await waitForFittedView(page);
  await openAuthoringPanel(page);

  // Compose the first two NDVI steps by hand so the generated forms (from
  // the server's own GET /processes) are on screen with their help text.
  await paletteButton(page, "load_collection").click();
  await paletteButton(page, "ndvi").click();
  await fieldById(page, "s1-id").selectOption("hls-s30");
  await fieldById(page, "s1-bands").fill("b8a,b04");
  await expect(page.locator(".swath-authoring-field-help").first()).toBeVisible();
  await capture(
    page,
    "08-authoring-form.png",
    "openEO authoring panel: forms generated from the server's own GET /processes, with plain-language field help.",
  );
});

test("authoring panel: template narrative + advanced fields open", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=12`);
  await waitForFittedView(page);
  await openAuthoringPanel(page);

  await page.locator("swath-authoring-panel .swath-authoring-template").click();
  await expect(page.locator("#swath-authoring-narrative")).toContainText("compute NDVI");
  await page.locator('[data-step="s1"] .swath-authoring-advanced-toggle').click();
  await expect(fieldById(page, "s1-spatial_extent")).toBeVisible();
  await capture(
    page,
    "09-authoring-narrative-advanced.png",
    "NDVI template loaded: the plain-words narrative of the graph, with a step's advanced (defaulted) fields opened.",
  );
});

test("authoring publish: the authored layer serves immediately", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=12`);
  await waitForFittedView(page);
  await openAuthoringPanel(page);
  await page.locator("swath-authoring-panel .swath-authoring-template").click();
  await expect(page.locator("swath-authoring-panel .swath-authoring-submit")).toBeEnabled();

  const created = page.waitForResponse(
    (response) => response.url().includes("/services") && response.request().method() === "POST",
  );
  await page.locator("swath-authoring-panel .swath-authoring-submit").click();
  const response = await created;
  expect(response.status()).toBe(201);
  const id = response.headers()["openeo-identifier"];
  if (!id) {
    throw new Error("creation response carried no OpenEO-Identifier header");
  }

  try {
    // The authored layer lands in the rail and becomes the viewed layer;
    // wait for its tiles before freezing the frame.
    const layerButton = page.locator(`swath-layer-panel button[data-layer="${id}"]`);
    await expect(layerButton).toHaveAttribute("aria-pressed", "true");
    await page.waitForResponse(
      (r) => r.url().includes(`/tilesets/${id}/tiles/`) && r.status() === 200,
    );
    await waitForMapIdle(page);
    // The publish click left the rail scrolled to the submit button;
    // scroll back to the top so the shot shows the authored layer's new
    // entry in the layer rail (selected), above the open panel.
    await page.locator(".swath-rail").evaluate((el) => {
      el.scrollTo({ top: 0 });
    });
    await expect(layerButton).toBeVisible();
    await capture(
      page,
      "10-authoring-published.png",
      "Publish flow result: the authored NDVI service appears in the layer rail and serves on the map immediately — no reload.",
      { maxBadFrac: 0.02 }, // the service id in the rail differs per run
    );
  } finally {
    // Determinism across capture runs: leave no published service behind,
    // or the next run's layer rail would not match this one's.
    const deleted = await page.request.delete(`/services/${id}`);
    expect(deleted.status()).toBe(204);
    await expect
      .poll(async () => (await page.request.get(`/tilesets/${id}/tiles/${TILE}`)).status())
      .toBe(404);
  }
});

test("dataset browser: granule footprints on the map", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=12`);
  await waitForFittedView(page);
  await page.locator("swath-dataset-panel .swath-dataset-panel-toggle").click();
  await page.locator('swath-dataset-panel button[data-dataset="hls-s30"]').click();
  await expect(page.locator("swath-dataset-panel button[data-granule]").first()).toBeVisible();
  // The footprint layer is a real MapLibre line layer over a GeoJSON
  // source; wait for it to carry the granule polygon.
  await page.waitForFunction((id) => {
    const el = document.querySelector("swath-map") as {
      map?: {
        getLayer(id: string): { type?: string } | undefined;
        getSource(id: string): { serialize(): { data?: unknown } } | undefined;
      };
    } | null;
    const map = el?.map;
    if (!map?.getLayer(id) || map.getLayer(id)?.type !== "line") {
      return false;
    }
    const data = map.getSource(id)?.serialize().data as
      | { features?: { geometry?: { type?: string } }[] }
      | undefined;
    const features = data?.features ?? [];
    return features.length > 0 && features[0]?.geometry?.type === "Polygon";
  }, "swath-granule-footprints");
  await capture(
    page,
    "11-dataset-footprints.png",
    "Dataset browser expanded: the catalog's granules listed in the rail, footprints outlined live on the map.",
  );
});

test("trace analytics panel under load", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?xray&layer=ndvi&center=${CENTER}&zoom=12`);
  await waitForFittedView(page);
  await waitForXRay(page);

  // Generate load beyond the initial view: a zoom-out to z11 (fresh
  // overview-path renders in this run's cold cache) and back to z12
  // (cache hits from this same page session) — a real plan mix.
  for (const zoom of [11, 12]) {
    await page.evaluate((z) => {
      (document.querySelector("swath-map") as SwathMapLike | null)?.map?.jumpTo({ zoom: z });
    }, zoom);
    await waitForMapIdle(page);
  }
  const analytics = page.locator(".swath-xray-analytics");
  await expect(analytics).toBeVisible();
  await page.waitForFunction(() => {
    const panel = document.querySelector<HTMLElement>(".swath-xray-analytics");
    return Number(panel?.dataset.samples ?? "0") > 0;
  });
  await capture(
    page,
    "12-analytics-under-load.png",
    "Trace analytics under load: rolling p50/p95 render latency, plan mix, and cache hit rate over the session's tiles.",
    { maxBadFrac: 0.03 }, // latency quantiles differ between runs
  );
});

test.afterAll(() => {
  // Provenance + index (issue #112): shots.json is the machine-readable
  // manifest verify_stable.py replays the pdiff policies from; index.md
  // is the human index with one-line captions. Both are stamped with the
  // git sha the capture ran at.
  if (captured.length === 0) {
    return;
  }
  const shots = captured.map((shot) => ({
    ...shot,
    sha256: createHash("sha256")
      .update(readFileSync(path.join(OUT_DIR, shot.file)))
      .digest("hex"),
  }));
  const manifest = {
    schema: "swath-screenshots/1",
    git_sha: CAPTURE_SHA,
    captured: new Date().toISOString(),
    viewport: { width: 1528, height: 860, dpr: 1 },
    stack: "tests/e2e/stack-up.sh (fixture granule only), captured via `just screenshots`",
    shots,
  };
  writeFileSync(path.join(OUT_DIR, "shots.json"), `${JSON.stringify(manifest, null, 2)}\n`);

  const lines = [
    "# docs/media/screenshots — captured UI evidence",
    "",
    "Generated by `just screenshots` — never edit or hand-replace a shot; re-run the recipe.",
    "Every image is captured from the fixture compose stack (tests/e2e/stack-up.sh, committed",
    "HLS granule, no third-party basemap) at a pinned viewport (1528x860, DPR 1), and a second",
    "capture run must reproduce each shot within its perceptual-diff policy",
    "(tests/screenshots/verify_stable.py + swath-testkit pdiff) before the recipe passes.",
    "",
    `- capture sha: \`${CAPTURE_SHA}\``,
    `- captured: ${manifest.captured}`,
    "- machine-readable manifest (per-shot sha256 + pdiff policy): [`shots.json`](shots.json)",
    "",
    "| shot | evidences |",
    "|---|---|",
    ...shots.map((shot) => `| [\`${shot.file}\`](${shot.file}) | ${shot.caption} |`),
    "",
  ];
  writeFileSync(path.join(OUT_DIR, "index.md"), `${lines.join("\n")}`);
});
