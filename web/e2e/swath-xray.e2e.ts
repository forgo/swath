// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The x-ray R4 bar (issue #34): the overlay must paint exactly what the
// Trace stream says — one source of truth, verified twice. The test opens
// its OWN EventSource to /traces (before the overlay opens its), drives
// fresh tile renders, then cross-checks every painted badge against the
// traces the test itself received: tile identity, decision, total_ms.
// The ingest-to-pixel readout and the inspector's bytes_read get the same
// treatment. Both streams ride the vite dev proxy (`/traces` →
// swath:8080), so SSE-through-proxy is exercised too.
import { expect, type Page, test } from "@playwright/test";

// Where the demo page lives: /demo/ under vite dev, / when the binary
// serves the embedded production bundle (set by playwright.config.ts).
const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** Waits until the zero-config bounds fit has landed and settled. The
 * fit is async (tileset metadata fetch -> setStyle -> fitBounds): a view
 * jump issued before it lands is silently clobbered back to the fitted
 * view. The binary-served bundle (issue #103) boots fast enough to
 * expose exactly that race. `getZoom() > 5` discriminates the fitted
 * footprint view (~z12) from the zoom-1 boot view. */
async function waitForFittedView(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const el = document.querySelector("swath-map") as {
      map?: { loaded(): boolean; areTilesLoaded(): boolean; getZoom(): number };
    } | null;
    const map = el?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded() && map.getZoom() > 5);
  });
}

/** The trace envelope as swath-api pins it (traces.rs). */
interface Envelope {
  tile: string;
  layer: string;
  trace: {
    decision: string | { overview: { level: number } } | { cache_hit: { key: string } };
    bytes_read: number;
    timings: { total_ms: number };
    ingest_to_pixel_ms: number | null;
    /** The planner's reasoning (#37); null on unplanned traces. */
    plan?: ReceivedPlan | null;
  };
}

declare global {
  interface Window {
    __received?: Envelope[];
  }
}

/** The test's own subscription — opened in the page so it shares the
 * proxy path (and origin) with the overlay's stream. */
async function subscribeToTraces(page: Page): Promise<void> {
  await page.evaluate(() => {
    const received: Envelope[] = [];
    window.__received = received;
    const source = new EventSource("/traces");
    source.addEventListener("trace", (event) => {
      received.push(JSON.parse((event as MessageEvent<string>).data) as Envelope);
    });
  });
}

/** Latest received envelope per `"layer/z/x/y"` key — the same
 * latest-wins reduction the overlay's store performs. */
function latestByKey(received: Envelope[]): Map<string, Envelope> {
  const latest = new Map<string, Envelope>();
  for (const envelope of received) {
    latest.set(`${envelope.layer}/${envelope.tile}`, envelope);
  }
  return latest;
}

test("overlay paints decisions matching the traces the test received over SSE", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await expect(page.locator("swath-map canvas.maplibregl-canvas")).toBeVisible();

  // Subscribe FIRST: the broadcast bus delivers every event published
  // after subscribe, so opening before the overlay guarantees the test's
  // stream is a superset of what the overlay saw.
  await subscribeToTraces(page);

  // Enable x-ray through the built-in toggle control (the user path).
  const toggle = page.getByRole("button", { name: "Toggle x-ray overlay" });
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("swath-map .swath-xray")).toBeAttached();

  // Force fresh renders both subscribers will see: jump deeper into the
  // fixture footprint (bbox -106.1..-105.9 / 39.2..39.4; style zoom 13
  // displays z14 tiles from the 256px source — well inside the served
  // 0..24 matrix range). "Fresh" matters twice over: cache hits carry no
  // ingest_to_pixel_ms (the north-star number belongs to the FIRST
  // render after ingest), and `just e2e-web` runs both modes (issue
  // #103) against ONE stack — so the against-binary pass dives one zoom
  // deeper (z15 tiles) than the vite-dev pass to keep its tiles unseen.
  await waitForFittedView(page);
  const fresh = process.env.SWATH_E2E_MODE === "binary" ? "14" : "13";
  await page.evaluate((zoom) => {
    const el = document.querySelector("swath-map");
    el?.setAttribute("center", "-106.0,39.3");
    el?.setAttribute("zoom", zoom);
  }, fresh);

  // Badges appear once traces flow.
  await page.waitForFunction(() => document.querySelectorAll(".swath-xray-badge").length > 0);

  // THE agreement check, polled until both async streams settle: every
  // painted badge must correspond to a trace the test received — same
  // tile key, same decision kind, same total_ms (in data AND label text).
  const agreementHandle = await page.waitForFunction(() => {
    const received = window.__received ?? [];
    const latest = new Map<string, Envelope>();
    for (const envelope of received) {
      latest.set(`${envelope.layer}/${envelope.tile}`, envelope);
    }
    const badges = [...document.querySelectorAll<HTMLElement>(".swath-xray-badge")];
    if (badges.length === 0) {
      return null;
    }
    for (const badge of badges) {
      const envelope = latest.get(badge.dataset.key ?? "");
      if (!envelope) {
        return null;
      }
      const { decision } = envelope.trace;
      // Mirror the overlay's decisionKind: keyed cache hits (#36) and
      // overviews collapse to their flat kind.
      const kind =
        typeof decision === "string"
          ? decision
          : "cache_hit" in decision
            ? "cache_hit"
            : "overview";
      const totalMs = envelope.trace.timings.total_ms;
      if (badge.dataset.decision !== kind) {
        return null;
      }
      if (badge.dataset.totalMs !== String(totalMs)) {
        return null;
      }
      if (!(badge.textContent ?? "").includes(`${totalMs} ms`)) {
        return null;
      }
    }
    return { badges: badges.length, received: received.length };
  });
  const agreement = (await agreementHandle.jsonValue()) as {
    badges: number;
    received: number;
  };
  expect(agreement.badges).toBeGreaterThan(0);
  expect(agreement.received).toBeGreaterThanOrEqual(agreement.badges);

  // The ingest-to-pixel readout (THE demo number): catalog-backed layers
  // stamp ingest_to_pixel_ms on every render of the dropped granule, so
  // the readout must show a value the test also received.
  await page.waitForFunction(() => {
    const received = window.__received ?? [];
    const values = received
      .map((envelope) => envelope.trace.ingest_to_pixel_ms)
      .filter((value): value is number => value !== null);
    const text = document.querySelector(".swath-xray-ingest")?.textContent ?? "";
    return values.some((value) => text === `ingest→pixel: ${value} ms`);
  });

  // Inspector: clicking a badge opens the trace popover, and its
  // bytes_read matches the SSE-received trace for that same tile.
  const key = await page.locator(".swath-xray-badge").first().getAttribute("data-key");
  expect(key).not.toBeNull();
  await page.locator(`.swath-xray-badge[data-key="${key}"]`).first().click();
  const inspector = page.locator(".swath-xray-inspector");
  await expect(inspector).toBeVisible();
  await expect(inspector).toContainText(String(key));
  const received = await page.evaluate(() => window.__received ?? []);
  const envelope = latestByKey(received).get(key ?? "");
  expect(envelope).toBeDefined();
  await expect(inspector).toContainText(`${envelope?.trace.bytes_read}`);

  // Toggle off: overlay DOM (badges, readout, inspector) is fully removed.
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator("swath-map .swath-xray")).toHaveCount(0);
});

// --- x-ray v1 (issue #42): the same agreement bar for the new surfaces.
// One source of truth (the test's own SSE subscription), verified twice:
// the bytes heatmap buckets, the feed lines, and the why-view table must
// all match what the stream itself delivered.

/** The plan payload as swath-core pins it (subset the assertions read). */
interface ReceivedPlan {
  chosen: string | { overview: { factor: number } };
  considered: {
    strategy: string | { overview: { factor: number } };
    estimated_cost_bytes: number;
    admissible: boolean;
    reason: string;
  }[];
}

test("v1: heatmap buckets, feed lines, and why-view match the SSE stream", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await expect(page.locator("swath-map canvas.maplibregl-canvas")).toBeVisible();
  await subscribeToTraces(page);

  const toggle = page.getByRole("button", { name: "Toggle x-ray overlay" });
  await toggle.click();
  await expect(page.locator("swath-map .swath-xray")).toBeAttached();
  // A view of its OWN (different zoom than the v0 test): when this test
  // runs after v0 on the same stack, v0's tiles are all cache hits by
  // now — zero bytes_read across the board. Fresh z13 tiles guarantee
  // live/overview renders so the heatmap has a non-degenerate range too.
  // (Same fit-race guard as v0: jump only after the fitted view landed.)
  await waitForFittedView(page);
  await page.evaluate(() => {
    const el = document.querySelector("swath-map");
    el?.setAttribute("center", "-105.95,39.25");
    el?.setAttribute("zoom", "12");
  });
  await page.waitForFunction(() => document.querySelectorAll(".swath-xray-badge").length > 0);

  // --- Bytes heatmap: switch modes through the overlay's own control.
  const modeGroup = page.getByRole("group", { name: "X-ray display mode" });
  await modeGroup.getByRole("button", { name: "bytes", exact: true }).click();
  await expect(modeGroup.getByRole("button", { name: "bytes", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.locator(".swath-xray-scale")).toBeVisible();

  // Every painted badge must carry (a) the bytes_read the test itself
  // received for that tile and (b) the log-scale bucket recomputed here
  // from that value and the legend's published min/max — the overlay's
  // scale choice, verified independently.
  await page.waitForFunction(() => {
    const BUCKETS = 5;
    const bucketOf = (bytes: number, min: number, max: number): number => {
      if (bytes <= 0) return 0;
      if (!(max > min) || min <= 0) return BUCKETS;
      const t = (Math.log(bytes) - Math.log(min)) / (Math.log(max) - Math.log(min));
      return 1 + Math.max(0, Math.min(BUCKETS - 1, Math.floor(t * BUCKETS)));
    };
    const received = window.__received ?? [];
    const latest = new Map<string, Envelope>();
    for (const envelope of received) {
      latest.set(`${envelope.layer}/${envelope.tile}`, envelope);
    }
    const scale = document.querySelector<HTMLElement>(".swath-xray-scale");
    const min = Number(scale?.dataset.min);
    const max = Number(scale?.dataset.max);
    // No published range is legitimate exactly when the store holds no
    // non-zero bytes (an all-cache-hit view): then every badge must sit
    // in the zero bucket. Any non-zero badge without a range just means
    // the repaint hasn't landed yet — keep polling.
    const hasRange = Number.isFinite(min) && Number.isFinite(max);
    const badges = [...document.querySelectorAll<HTMLElement>(".swath-xray-badge")];
    if (badges.length === 0) {
      return null;
    }
    for (const badge of badges) {
      const envelope = latest.get(badge.dataset.key ?? "");
      if (!envelope) {
        return null;
      }
      if (badge.dataset.bytes !== String(envelope.trace.bytes_read)) {
        return null;
      }
      const bytes = envelope.trace.bytes_read;
      const expected = hasRange ? bucketOf(bytes, min, max) : bytes <= 0 ? 0 : null;
      if (expected === null || badge.dataset.bytesBucket !== String(expected)) {
        return null;
      }
    }
    return badges.length;
  });

  // --- Feed: every line is a trace the test received; the newest line
  // is the newest envelope (polled — both readers chase the same stream).
  await page.getByRole("button", { name: "trace feed" }).click();
  await expect(page.locator(".swath-xray-feed-lines")).toBeVisible();
  await page.waitForFunction(() => {
    const received = window.__received ?? [];
    const lines = [...document.querySelectorAll<HTMLElement>(".swath-xray-feed-lines li > button")];
    if (lines.length === 0 || received.length === 0) {
      return null;
    }
    if (lines.length > received.length) {
      return null; // the test subscribed first: its stream is the superset
    }
    const keys = new Set(received.map((envelope) => `${envelope.layer}/${envelope.tile}`));
    if (!lines.every((line) => keys.has(line.dataset.key ?? ""))) {
      return null;
    }
    const last = received[received.length - 1];
    const lastKey = `${last?.layer}/${last?.tile}`;
    return lines[lines.length - 1]?.dataset.key === lastKey ? lines.length : null;
  });

  // --- Why-view: find a badge whose received trace carries a plan, open
  // its inspector, and check the table against that same payload.
  const planKey = await page.waitForFunction(() => {
    const received = window.__received ?? [];
    const latest = new Map<string, Envelope>();
    for (const envelope of received) {
      latest.set(`${envelope.layer}/${envelope.tile}`, envelope);
    }
    for (const badge of document.querySelectorAll<HTMLElement>(".swath-xray-badge")) {
      const envelope = latest.get(badge.dataset.key ?? "");
      if (envelope?.trace.plan) {
        return badge.dataset.key ?? null;
      }
    }
    return null;
  });
  const key = (await planKey.jsonValue()) as string;
  await page.locator(`.swath-xray-badge[data-key="${key}"]`).first().click();
  const inspector = page.locator(".swath-xray-inspector");
  await expect(inspector).toBeVisible();
  const received = await page.evaluate(() => window.__received ?? []);
  const plan = latestByKey(received).get(key)?.trace.plan;
  expect(plan).toBeTruthy();
  const rows = inspector.locator(".swath-xray-plan tbody tr");
  await expect(rows).toHaveCount(plan?.considered.length ?? -1);
  const chosenLabel =
    typeof plan?.chosen === "string"
      ? plan.chosen
      : `overview (factor ${plan?.chosen.overview.factor})`;
  await expect(inspector).toContainText(`planner — chose ${chosenLabel}`);
  await expect(inspector.locator('.swath-xray-plan tr[data-chosen="true"]')).toContainText(
    "✓ chosen",
  );
  for (const candidate of plan?.considered ?? []) {
    await expect(inspector.locator(".swath-xray-plan")).toContainText(candidate.reason);
  }

  // --- Off mode clears the badges without tearing the overlay down.
  await modeGroup.getByRole("button", { name: "off", exact: true }).click();
  await expect(page.locator(".swath-xray-badge")).toHaveCount(0);
  await expect(page.locator("swath-map .swath-xray")).toBeAttached();
});
