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
// - Painted, provably. Every shot opens its view through
//   `gotoAndWaitForTiles` (a real tile 200 before the fitted-view gate),
//   and the verifier judges each image alone (`pdiff --content`): a
//   run-vs-run diff cannot tell two identical blanks from two identical
//   scenes, which is exactly how the #211 review caught unpainted shots.
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
  /** pdiff per-channel tolerance (swath-testsupport DiffPolicy). */
  tolerance: number;
  /** pdiff max fraction of pixels allowed past the tolerance. Shots that
   * show wall-clock timestamps or per-run timings budget for exactly that
   * text; pixel content everywhere else must reproduce. */
  maxBadFrac: number;
  /** Left edge of the map region the content gate inspects (0 on phones). */
  railWidth: number;
}

const captured: Shot[] = [];

async function capture(
  page: Page,
  file: string,
  caption: string,
  policy?: { tolerance?: number; maxBadFrac?: number; railWidth?: number },
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
    // The content gate inspects the canvas right of the rail: 248 on the
    // desktop shots, 0 on the phone shots (the rail is a bottom tab bar).
    railWidth: policy?.railWidth ?? 248,
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

/**
 * Opens a view and waits for a REAL tile of `layer` to answer 200 before
 * the fitted-view gate. The gate alone is not enough (the #211 review
 * found blank shots): a layer apply reads tileset metadata and the
 * granule listing before its first tile request, and during that round
 * trip the map is loaded, needs zero tiles (`areTilesLoaded` is true of
 * an empty source), and already sits at the deep link's zoom — every
 * condition met, nothing painted. Registering the response wait BEFORE
 * navigation means a fast tile cannot be missed either.
 */
async function gotoAndWaitForTiles(page: Page, url: string, layer: string): Promise<void> {
  const tile = page.waitForResponse(
    (response) => response.url().includes(`/tilesets/${layer}/tiles/`) && response.status() === 200,
  );
  await page.goto(url);
  await tile;
  await waitForFittedView(page);
}

async function waitForMapIdle(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const map = (document.querySelector("swath-map") as SwathMapLike | null)?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded());
  });
}

/** Waits for the x-ray overlay to be painting AND settled: attached,
 * the map idle (every tile loaded, so every trace has been published),
 * the badge count quiescent (>0 and unchanged for 1.2 s — badges ride
 * the SSE stream, which trails the tile responses), and (best effort)
 * the ingest→pixel readout populated. The quiescence matters: freezing
 * the frame while badges were still streaming in made the badge COUNT
 * a per-run race, which the second-capture pdiff rightly rejected. */
async function waitForXRay(page: Page): Promise<void> {
  await expect(page.locator("swath-map .swath-xray")).toBeAttached();
  await waitForMapIdle(page);
  await page.waitForFunction(() => {
    const w = window as unknown as { __badgeQuiet?: { n: number; at: number } };
    const n = document.querySelectorAll(".swath-xray-badge").length;
    const now = Date.now();
    if (!w.__badgeQuiet || w.__badgeQuiet.n !== n) {
      w.__badgeQuiet = { n, at: now };
      return false;
    }
    return n > 0 && now - w.__badgeQuiet.at > 1200;
  });
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

function fieldById(page: Page, id: string) {
  return page.locator(`#swath-authoring-${id}`);
}

/** The Model B canvas's stage-typed insert chip for `processId` at
 * `gap` (0 = right after the permanent Load card) — the same driving
 * convention as web/e2e/authoring.e2e.ts. */
function chip(page: Page, gap: number, processId: string) {
  return page.locator(
    `.swath-authoring-insert[data-gap="${gap}"] ` + `button[data-process="${processId}"]`,
  );
}

/** The panel is collapsed and lazy; the permanent Load card (s1)
 * rendering means the canvas is ready (Model B, issue #168). */
async function openAuthoringPanel(page: Page): Promise<void> {
  // Author mode (#291): the strip drawer over the map + the inspector.
  await page.locator('swath-rail [part="item"][data-mode="author"]').click();
  await page.locator("swath-authoring-panel .swath-authoring-toggle").click();
  // The inspector (fields, preview, publish) shows the *selected* step.
  const chip = page.locator('.swath-authoring-chip[data-chip="s1"]');
  if ((await chip.count()) > 0 && (await chip.getAttribute("aria-pressed")) !== "true") {
    await chip.click();
  }
  await expect(page.locator('[data-step="s1"]')).toBeVisible();
}

/** Waits until the canvas's live preview (POST /result, debounced) has
 * arrived AND decoded — the preview image is deterministic pixel
 * content, but its arrival is async, so freezing the frame before it
 * lands made the shot a race between capture runs. */
async function waitForAuthoringPreview(page: Page): Promise<void> {
  const image = page.locator("#swath-authoring-preview-image");
  await expect(image).toBeVisible({ timeout: 15_000 });
  // Quiesce on the blob URL, not just presence: each edit re-previews
  // after a debounce, so a mid-composition preview can land first and a
  // later one replace it — freezing between the two made the shot a
  // per-run coin flip. Stable-for-1.2s (4x the debounce) plus decoded
  // pixels means the FINAL graph's preview is what the shot carries.
  await page.waitForFunction(() => {
    const img = document.querySelector<HTMLImageElement>("#swath-authoring-preview-image");
    if (!img || img.naturalWidth === 0) {
      return false;
    }
    const w = window as unknown as { __previewQuiet?: { src: string; at: number } };
    const now = Date.now();
    if (!w.__previewQuiet || w.__previewQuiet.src !== img.src) {
      w.__previewQuiet = { src: img.src, at: now };
      return false;
    }
    return now - w.__previewQuiet.at > 1200;
  });
}

test.describe("landing", () => {
  // The cinematic landing (issue #211) loops the fire season on load —
  // a moving target no second capture run could reproduce. Under
  // `prefers-reduced-motion` it waits on the latest frame with a play
  // affordance instead: a real, deterministic state of the same page,
  // and the accessibility path evidenced in the process.
  test.use({ contextOptions: { reducedMotion: "reduce" } });

  test("landing page: the fire-season loop, held for reduced motion", async ({ page }) => {
    await gotoAndWaitForTiles(page, DEMO_PATH, "park-fire-ndvi");
    await expect(page.locator(".swath-map-landing")).toHaveAttribute("data-state", "reduced");
    await expect(page.locator("#swath-share button")).toBeEnabled();
    await capture(
      page,
      "01-landing-layer-rail.png",
      "Zero-config landing page: the Park Fire series auto-framed and playable (held on its latest frame here — the reduced-motion state, with its play affordance), the x-ray invitation top-center, the Share button in the rail.",
    );
  });
});

test("colormapped NDVI at z12", async ({ page }) => {
  await gotoAndWaitForTiles(page, `${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=12`, "ndvi");
  await capture(
    page,
    "02-ndvi-colormapped.png",
    "Colormapped NDVI (rdylgn) rendered live at z12 from the fixture granule — nothing pre-baked.",
  );
});

test("true color at the same view", async ({ page }) => {
  await gotoAndWaitForTiles(
    page,
    `${DEMO_PATH}?layer=truecolor&center=${CENTER}&zoom=12`,
    "truecolor",
  );
  await capture(
    page,
    "03-truecolor-live.png",
    "HLS true color at the same viewpoint: selecting a layer in the rail re-points the raster source.",
  );
});

test("x-ray decisions + why-view inspector", async ({ page }) => {
  // z13 tiles are untouched at this point in the run, so with the cold
  // per-run cache every badge shows a real live-render decision.
  await gotoAndWaitForTiles(
    page,
    `${DEMO_PATH}?xray&view=xray&layer=truecolor&center=${CENTER}&zoom=13`,
    "truecolor",
  );
  await waitForXRay(page);
  await capture(
    page,
    "04-xray-decisions.png",
    "X-ray decision overlay: every tile badged with its render decision and timing; ingest→pixel readout top-left.",
    { maxBadFrac: 0.03 }, // per-tile ms text differs between runs
  );

  // "First badge" is arrival-order nondeterministic; inspect the
  // lexicographically smallest key so both capture runs open the SAME tile.
  const key = await page.locator(".swath-xray-badge").evaluateAll((badges: HTMLElement[]) => {
    const keys = badges.map((badge) => badge.dataset.key ?? "").filter((k) => k !== "");
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
  await gotoAndWaitForTiles(
    page,
    `${DEMO_PATH}?xray&view=xray&layer=ndvi&center=${CENTER}&zoom=13`,
    "ndvi",
  );
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

  // Exact: the feed card's header/pause controls are named "… trace feed" too.
  await page.getByRole("button", { name: "trace feed", exact: true }).click();
  await expect(page.locator(".swath-xray-feed-lines")).toBeVisible();
  await expect(page.locator(".swath-xray-feed-lines li").first()).toBeVisible();
  await capture(
    page,
    "07-xray-trace-feed.png",
    "Trace feed: the /traces SSE stream as scrollback — every line a render decision, clickable back to its tile.",
    { maxBadFrac: 0.06 }, // feed lines carry wall-clock timestamps
  );
});

test("authoring panel: the always-valid canvas with field help", async ({ page }) => {
  await gotoAndWaitForTiles(page, `${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=12`, "ndvi");
  await openAuthoringPanel(page);

  // Compose the first NDVI steps on the Model B canvas (issue #168):
  // collection from the /collections-fed select, bands ticked from the
  // vocabulary checkboxes, NDVI inserted through its stage-typed chip —
  // so the generated cards are on screen with their help text.
  await fieldById(page, "s1-id").selectOption("hls-s30");
  await fieldById(page, "s1-bands-b8a").check();
  await fieldById(page, "s1-bands-b04").check();
  await chip(page, 0, "ndvi").click();
  await expect(fieldById(page, "s3-nir")).toHaveValue("b8a");
  await expect(
    page.locator("#swath-author-inspector .swath-authoring-field-help").first(),
  ).toBeVisible();
  await waitForAuthoringPreview(page);
  await capture(
    page,
    "08-authoring-form.png",
    "openEO authoring canvas (Model B): cards generated from the server's own GET /processes, bands from the vocabulary, plain-language field help.",
  );
});

test("authoring panel: template narrative + advanced fields open", async ({ page }) => {
  await gotoAndWaitForTiles(page, `${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=12`, "ndvi");
  await openAuthoringPanel(page);

  await page.locator(".swath-authoring-template").click();
  await expect(page.locator("#swath-authoring-narrative")).toContainText("compute NDVI");
  await page.locator('[data-step="s1"] .swath-authoring-advanced-toggle').click();
  await expect(fieldById(page, "s1-spatial_extent")).toBeVisible();
  await waitForAuthoringPreview(page);
  await capture(
    page,
    "09-authoring-narrative-advanced.png",
    "NDVI template loaded: the plain-words narrative of the graph, with a step's advanced (defaulted) fields opened.",
  );
});

test("authoring publish: the authored layer serves immediately", async ({ page }) => {
  await gotoAndWaitForTiles(page, `${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=12`, "ndvi");
  await openAuthoringPanel(page);
  await page.locator(".swath-authoring-template").click();
  await expect(page.locator(".swath-authoring-submit")).toBeEnabled();

  const created = page.waitForResponse(
    (response) => response.url().includes("/services") && response.request().method() === "POST",
  );
  await page.locator(".swath-authoring-submit").click();
  const response = await created;
  expect(response.status()).toBe(201);
  const id = response.headers()["openeo-identifier"];
  if (!id) {
    throw new Error("creation response carried no OpenEO-Identifier header");
  }

  try {
    // The authored layer lands in the rail and becomes the viewed layer;
    // wait for its tiles before freezing the frame.
    // Author mode hides the layer list (issue #291): back to Layers to see the row.
    await page.locator('swath-rail [part="item"][data-mode="layers"]').click();
    const layerButton = page.locator(`swath-layer-item[data-layer="${id}"] [part="row"]`);
    await expect(layerButton).toHaveAttribute("aria-pressed", "true");
    await page.waitForResponse(
      (r) => r.url().includes(`/tilesets/${id}/tiles/`) && r.status() === 200,
    );
    await waitForMapIdle(page);
    // The publish click left the rail scrolled to the submit button;
    // scroll back to the top so the shot shows the authored layer's new
    // entry in the layer rail (selected), above the open panel.
    await page.locator('swath-rail [part="content"]').evaluate((el) => {
      el.scrollTo({ top: 0 });
    });
    await expect(layerButton).toBeVisible();
    // The frame is Layers mode: the new rail entry (selected) over its
    // tiles. The draft's preview lives in Author mode, which hides the
    // layer list (#291) — shots 08/09 carry it.
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

test("change detection: the first DAG product on the canvas", async ({ page }) => {
  await gotoAndWaitForTiles(
    page,
    `${DEMO_PATH}?layer=park-fire-ndvi&center=-121.6931,40.0208&zoom=13`,
    "park-fire-ndvi",
  );
  await openAuthoringPanel(page);
  await fieldById(page, "s1-id").selectOption("hls-s30-fire");
  await page.locator(".swath-authoring-template-change").click();
  await expect(page.locator("#swath-authoring-narrative")).toContainText("subtract");
  await waitForAuthoringPreview(page);
  await capture(
    page,
    "15-change-detection.png",
    // The join is ADR 0022's; the caption is for readers, not the ADR index.
    "Change detection: two dated branches of one collection joined by a subtract — NDVI(later) − NDVI(earlier), previewed before publishing.",
    { maxBadFrac: 0.03 }, // the preview's live render carries a few seam pixels
  );
});

test("compare swipe: NDVI against true color, one handle", async ({ page }) => {
  const truecolorTile = page.waitForResponse(
    (r) => r.url().includes("/tilesets/truecolor/tiles/") && r.status() === 200,
  );
  await gotoAndWaitForTiles(
    page,
    `${DEMO_PATH}?layer=ndvi&cl=truecolor&center=${CENTER}&zoom=12&swipe=0.5`,
    "ndvi",
  );
  await truecolorTile;
  const handle = page.locator("swath-map .swath-map-compare-handle");
  await expect(handle).toHaveAttribute("data-mode", "layer");
  await expect(page.locator('.swath-map-compare-label[data-side="left"]')).toHaveText("ndvi");
  await expect(page.locator('.swath-map-compare-label[data-side="right"]')).toHaveText("truecolor");
  await waitForMapIdle(page);
  await capture(
    page,
    "16-compare-swipe.png",
    "Compare swipe, layer against layer: NDVI left, true color right of one draggable handle — the same handle works date-vs-date on a time series, and its position rides in the share link.",
  );
});

test("command palette: ⌘K, type, jump", async ({ page }) => {
  await gotoAndWaitForTiles(
    page,
    `${DEMO_PATH}?layer=truecolor&center=${CENTER}&zoom=12`,
    "truecolor",
  );
  await page.keyboard.press("ControlOrMeta+k");
  const palette = page.locator("swath-command-palette");
  await expect(palette).toHaveAttribute("open", "");
  await expect(palette.locator('[part="input"]')).toBeFocused();
  await page.keyboard.type("ndvi");
  await expect(palette.locator('[part="item"]').first()).toHaveAttribute(
    "data-command",
    "layer:ndvi",
  );
  await capture(
    page,
    "17-command-palette.png",
    "Command palette (⌘K / Ctrl-K): type a layer, mode, or action and jump — the whole shell is reachable from the keyboard.",
  );
});

test("dataset browser: granule footprints on the map", async ({ page }) => {
  await gotoAndWaitForTiles(
    page,
    `${DEMO_PATH}?view=data&layer=ndvi&center=${CENTER}&zoom=12`,
    "ndvi",
  );
  await page.locator('swath-catalog [part="dataset"] select').selectOption("hls-s30");
  const firstCard = page.locator("swath-catalog swath-granule-card").first();
  await expect(firstCard).toBeVisible();
  // Every thumbnail is a preview the engine rendered: wait for the first
  // card's <img> to decode before freezing the frame.
  await expect(firstCard.locator('img[part="media"]')).toBeVisible({ timeout: 60_000 });
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
    "Data mode: the catalog's granule cards with engine-rendered thumbnails in the rail, footprints outlined live on the map.",
  );
});

test("trace analytics panel under load", async ({ page }) => {
  await gotoAndWaitForTiles(
    page,
    `${DEMO_PATH}?xray&view=xray&layer=ndvi&center=${CENTER}&zoom=12`,
    "ndvi",
  );
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

test("time slider: first pass live, second pass cached (issue #182)", async ({ page }) => {
  // Four scrubs of ~24 live z13 NDVI renders each: comfortably done in
  // seconds warm, but the shared-machine cold path has been observed to
  // need more than the config's 120 s — give the sequence real headroom.
  test.setTimeout(300_000);
  // The Park Fire series (six granules, dropped by stack-up.sh): scrub
  // the season with the x-ray open. This run's cache is cold, so the
  // scripted request history reproduces exactly: the frames visited
  // below render live the first time and replay as cache hits when
  // revisited — the pair of shots IS the glass-box animation story.
  const listing = await page.request.get("/datasets/hls-s30-fire/granules");
  expect(listing.ok()).toBe(true);
  const body = (await listing.json()) as { granules?: { datetime?: string }[] };
  const frames = (body.granules ?? [])
    .map((granule) => granule.datetime)
    .filter((value): value is string => typeof value === "string")
    .sort((a, b) => Date.parse(a) - Date.parse(b));
  expect(frames.length).toBeGreaterThanOrEqual(3);

  await gotoAndWaitForTiles(
    page,
    `${DEMO_PATH}?xray&view=xray&layer=park-fire-ndvi&center=-121.6932,40.0208&zoom=12`,
    "park-fire-ndvi",
  );
  await waitForXRay(page);
  const slider = page.locator(".swath-map-time");
  await expect(slider).toHaveAttribute("data-frames", String(frames.length));

  /** Scrubs via the control's own range input and waits until every
   * badge shows `kind` and the analytics card narrates the frame. */
  const scrubAndSettle = async (index: number, kind: string): Promise<void> => {
    // Scrub only against a settled map (the e2e suite's discipline):
    // a scrub inside a still-loading style would race the re-point.
    await waitForMapIdle(page);
    await page
      .locator('.swath-map-time input[type="range"]')
      .evaluate((el: HTMLInputElement, value) => {
        el.value = String(value);
        el.dispatchEvent(new Event("input", { bubbles: true }));
      }, index);
    await page.waitForFunction(
      ({ frame, kind }) => {
        const analytics = document.querySelector<HTMLElement>(".swath-xray-analytics");
        if (analytics?.dataset.frame !== frame) {
          return false;
        }
        const badges = [...document.querySelectorAll<HTMLElement>(".swath-xray-badge")];
        return badges.length > 0 && badges.every((badge) => badge.dataset.decision === kind);
      },
      { frame: frames[index] ?? "", kind },
    );
  };

  // First pass: the fresh-burn-scar frame (2024-08-16) renders live.
  await scrubAndSettle(2, "live");
  await capture(
    page,
    "13-time-slider-live.png",
    "Time slider over the Park Fire season, first pass: the scrubbed frame is rendered live — every badge says so, and the analytics card narrates the frame's own plan mix.",
    { maxBadFrac: 0.04 }, // per-tile ms text differs between runs; the analytics summary now sits in the rail with longer lines (#286)
  );

  // Step forward, then revisit the same frame: the season's second pass
  // replays from the tile cache — the badges flip to cache_hit.
  await scrubAndSettle(3, "live");
  await scrubAndSettle(2, "cache_hit");
  await capture(
    page,
    "14-time-slider-cached.png",
    // Frame identity is ADR 0015's rule; the caption stays in the reader's words.
    "The same frame revisited: every tile is a cache hit (same granule, same cache entry), which is why the loop replays smoothly.",
    { maxBadFrac: 0.03 },
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
    viewport: { width: 1528, height: 928, dpr: 1 },
    stack: "tests/e2e/stack-up.sh (fixture granule only), captured via `just screenshots`",
    shots,
  };
  writeFileSync(path.join(OUT_DIR, "shots.json"), `${JSON.stringify(manifest, null, 2)}\n`);

  const lines = [
    "# docs/media/screenshots — captured UI evidence",
    "",
    "Generated by `just screenshots` — never edit or hand-replace a shot; re-run the recipe.",
    "Every image is captured from the fixture compose stack (tests/e2e/stack-up.sh, committed",
    "HLS granule, no third-party basemap) at a pinned viewport (1528x928, DPR 1 — a 1280x860 canvas beside the 248px rail, under the 44px top bar); every shot must",
    "show a painted map (`pdiff --content`: the map region is never near-uniform), and a second",
    "capture run must reproduce each shot within its perceptual-diff policy",
    "(tests/screenshots/verify_stable.py + swath-testsupport pdiff) before the recipe passes.",
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

// --- The phone tier (issue #293): m01–m04 at 393×852 with touch ---
test.describe("phone", () => {
  test.use({
    viewport: { width: 393, height: 852 },
    hasTouch: true,
    isMobile: true,
    deviceScaleFactor: 1,
  });
  const PHONE = { railWidth: 0, maxBadFrac: 0.03 };

  test("m01 landing on a phone: tab bar, dock chip", async ({ page }) => {
    await gotoAndWaitForTiles(page, `${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=11`, "ndvi");
    await expect(page.locator("swath-shell")).toHaveAttribute("tier", "phone");
    await page.locator('swath-rail [part="item"][data-mode="layers"]').tap(); // fold the sheet away
    await expect(page.locator("#swath-rail-drawer")).not.toHaveAttribute("open", "");
    await waitForMapIdle(page);
    await capture(
      page,
      "m01-phone-landing.png",
      "Phone tier: the map with the bottom tab bar, the toggles top-right in the dock, the status chip bottom-left.",
      PHONE,
    );
  });

  test("m02 layers sheet", async ({ page }) => {
    await gotoAndWaitForTiles(page, `${DEMO_PATH}?layer=ndvi&center=${CENTER}&zoom=11`, "ndvi");
    await expect(page.locator("#swath-rail-drawer")).toHaveAttribute("open", "");
    await expect(page.locator('swath-layer-item[data-layer="ndvi"] [part="row"]')).toBeVisible();
    await waitForMapIdle(page);
    await capture(
      page,
      "m02-phone-layers-sheet.png",
      "Phone tier: the layer list as a bottom sheet at its 40% snap over the map.",
      PHONE,
    );
  });

  test("m03 data sheet with the catalog", async ({ page }) => {
    await gotoAndWaitForTiles(
      page,
      `${DEMO_PATH}?view=data&layer=ndvi&center=${CENTER}&zoom=11`,
      "ndvi",
    );
    await page.locator('swath-catalog [part="dataset"] select').selectOption("hls-s30");
    const first = page.locator("swath-catalog swath-granule-card").first();
    await expect(first).toBeVisible();
    await expect(first.locator('img[part="media"]')).toBeVisible({ timeout: 60_000 });
    await waitForMapIdle(page);
    await capture(
      page,
      "m03-phone-data-sheet.png",
      "Phone tier: Data mode's catalog in the sheet, an engine thumbnail on the first card.",
      PHONE,
    );
  });

  test("m04 x-ray on a phone", async ({ page }) => {
    await gotoAndWaitForTiles(
      page,
      `${DEMO_PATH}?xray&view=xray&layer=truecolor&center=${CENTER}&zoom=13`,
      "truecolor",
    );
    await page.locator('swath-rail [part="item"][data-mode="xray"]').tap(); // fold the sheet away
    await expect(page.locator("#swath-rail-drawer")).not.toHaveAttribute("open", "");
    await waitForXRay(page);
    await capture(
      page,
      "m04-phone-xray.png",
      "Phone tier: x-ray badges over the map, the readouts card in the dock strip, the toggles top-right.",
      PHONE,
    );
  });
});
