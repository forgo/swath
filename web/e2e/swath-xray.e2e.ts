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

/** The trace envelope as swath-api pins it (traces.rs). */
interface Envelope {
  tile: string;
  layer: string;
  trace: {
    decision: string | { overview: { level: number } };
    bytes_read: number;
    timings: { total_ms: number };
    ingest_to_pixel_ms: number | null;
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
  await page.goto("/demo/");
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
  // 0..24 matrix range).
  await page.evaluate(() => {
    const el = document.querySelector("swath-map");
    el?.setAttribute("center", "-106.0,39.3");
    el?.setAttribute("zoom", "13");
  });

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
      const kind = typeof decision === "string" ? decision : "overview";
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
