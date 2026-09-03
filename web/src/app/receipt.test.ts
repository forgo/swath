// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The publish receipt's flow (issue #395): two server answers, and a card
// that stays shut when either is missing.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import type { SwathApi } from "../api.js";
import { SwathExplainCard } from "../ui/explain-card.js";
import { SwathIcon } from "../ui/icon.js";
import { showReceipt } from "./receipt.js";

beforeAll(() => {
  SwathIcon.define();
  SwathExplainCard.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

const ORIGIN = "http://localhost:8080";
const TRACE = '{"decision":"live","bytes_read":546497,"total_ms":327,"ingest_to_pixel_ms":106175}';

const TILESET = {
  boundingBox: { lowerLeft: [-121.74, 39.98], upperRight: [-121.64, 40.06] },
  links: [
    { rel: "self", href: `${ORIGIN}/tilesets/xyz-fire` },
    { rel: "item", href: `${ORIGIN}/tilesets/xyz-fire/tiles/{z}/{y}/{x}` },
  ],
  "swath:window": ["2024-06-01T00:00:00Z", "2024-09-01T00:00:00Z"],
  "swath:sources": 1,
};

function card(): SwathExplainCard {
  const el = document.createElement("swath-explain-card") as SwathExplainCard;
  document.body.append(el);
  return el;
}

/** A stub standing in for the API client's one seam. Cast at the call site
 * rather than declaring a local call signature for it: the DRY gate reserves
 * that spelling for api.ts, and is right to — a second declaration is how a
 * second network path starts. */
function api(handlers: { tileset?: Response; tile?: Response }) {
  const calls: string[] = [];
  return {
    calls,
    fetch: vi.fn((input: string) => {
      calls.push(input);
      if (input.includes("/tiles/")) {
        return Promise.resolve(
          handlers.tile ?? new Response(null, { headers: { "x-swath-trace": TRACE } }),
        );
      }
      return Promise.resolve(
        handlers.tileset ??
          new Response(JSON.stringify(TILESET), {
            headers: { "content-type": "application/json" },
          }),
      );
    }),
  };
}

test("the receipt renders one tile of what was published and opens the card", async () => {
  const el = card();
  const client = api({});
  expect(await showReceipt(client as unknown as SwathApi, el, "xyz-fire", ORIGIN)).toBe(true);
  await el.updateComplete;

  expect(el.open).toBe(true);
  expect(el.getAttribute("density")).toBe("published");
  const rows = el.shadowRoot?.querySelector('[part="rows"]')?.textContent ?? "";
  expect(rows).toContain(`${ORIGIN}/tilesets/xyz-fire`);
  expect(rows).toContain("2024-06-01T00:00:00Z/2024-09-01T00:00:00Z");
  expect(rows).toContain("327 ms");
  expect(rows).toContain("1 source");

  // The tile is fetched in OGC path order, z/row/col — not the XYZ order
  // the tile address is written in.
  const tileCall = client.calls.find((c) => c.includes("/tiles/")) ?? "";
  expect(tileCall).toMatch(/\/tilesets\/xyz-fire\/tiles\/\d+\/\d+\/\d+$/);
});

test("a tileset that 404s leaves the card shut", async () => {
  const el = card();
  const ok = await showReceipt(
    api({ tileset: new Response("", { status: 404 }) }) as unknown as SwathApi,
    el,
    "xyz-fire",
    ORIGIN,
  );
  expect(ok).toBe(false);
  expect(el.open).toBeFalsy();
});

test("a tile with no trace header leaves the card shut rather than inventing one", async () => {
  const el = card();
  const ok = await showReceipt(
    api({ tile: new Response(null) }) as unknown as SwathApi,
    el,
    "xyz-fire",
    ORIGIN,
  );
  expect(ok).toBe(false);
  expect(el.open).toBeFalsy();
});

test("a network failure is not a publish failure", async () => {
  const el = card();
  const client = { fetch: vi.fn(() => Promise.reject(new Error("offline"))) };
  // Publishing already succeeded; the receipt is proof, not a step.
  await expect(showReceipt(client as unknown as SwathApi, el, "xyz-fire", ORIGIN)).resolves.toBe(
    false,
  );
  expect(el.open).toBeFalsy();
});
