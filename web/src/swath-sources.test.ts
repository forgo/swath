// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { SwathApi } from "./api.js";
import { defineSwathSources, type SwathSources } from "./swath-sources.js";

const SERVER = "https://swath.test";
const NOW = Date.parse("2026-09-04T10:05:00Z");

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

const WATCHING = {
  id: "fire",
  title: "Fire drops",
  kind: "filedrop",
  scheme: "file",
  origin: "config",
  datasets: ["hls-s30-fire"],
  status: {
    state: "watching",
    reachable: true,
    lastEvent: "2026-09-04T10:01:00Z",
    ingested: 6,
    failures: 0,
  },
};

const BROKEN = {
  id: "archive",
  title: "Archive",
  kind: "filedrop",
  scheme: "s3",
  origin: "api",
  datasets: [],
  status: {
    state: "failing",
    reachable: false,
    lastEvent: "2026-09-04T09:00:00Z",
    lastError: "permission denied",
    ingested: 0,
    failures: 4,
  },
};

function stub(options: { sources?: unknown; sourcesStatus?: number; post?: () => Response } = {}) {
  const requests: string[] = [];
  const impl = (async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = String(input);
    const method = init?.method ?? "GET";
    requests.push(`${method} ${new URL(url).pathname}`);
    if (method === "POST") {
      return options.post ? options.post() : json({ ok: true }, 201);
    }
    if (url.endsWith("/sources")) {
      return options.sourcesStatus !== undefined
        ? json({ detail: "no" }, options.sourcesStatus)
        : json(options.sources ?? { sources: [] });
    }
    return new Response("not scripted", { status: 404 });
  }) as typeof fetch;
  return { impl, requests };
}

beforeAll(() => {
  defineSwathSources();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(s: ReturnType<typeof stub>): Promise<SwathSources> {
  const element = document.createElement("swath-sources");
  element.api = new SwathApi({ base: SERVER, fetch: s.impl });
  element.now = () => NOW;
  document.body.append(element);
  await element.updateComplete;
  return element;
}

const settle = () => new Promise((r) => setTimeout(r, 30));
const rows = (element: SwathSources) =>
  [...(element.shadowRoot?.querySelectorAll('[part="row"]') ?? [])] as HTMLElement[];

test("lazy by contract: nothing is fetched until the mode is entered", async () => {
  const s = stub({ sources: { sources: [WATCHING] } });
  const element = await mount(s);
  await settle();
  expect(s.requests).toEqual([]);
  element.active = true;
  await element.updateComplete;
  await settle();
  expect(s.requests).toEqual(["GET /sources"]);
});

test("every state on the screen traces to a served field", async () => {
  const s = stub({ sources: { sources: [WATCHING, BROKEN] } });
  const element = await mount(s);
  element.active = true;
  await element.updateComplete;
  await settle();
  await element.updateComplete;

  const [watching, broken] = rows(element);
  expect(watching?.textContent).toContain("watching");
  // Freshness from the server's instant against the injected clock.
  expect(watching?.textContent).toContain("4 min ago");
  expect(watching?.textContent).toContain("6 ingested");
  expect(watching?.textContent).toContain("feeds hls-s30-fire");
  expect(watching?.dataset["tone"]).toBe("ok");

  // A broken source reads as broken at a glance: the tone is on the row,
  // not only in the failure count.
  expect(broken?.dataset["tone"]).toBe("danger");
  expect(broken?.textContent).toContain("not answering");
  expect(broken?.textContent).toContain("permission denied");
  expect(broken?.textContent).toContain("4 failed");
  // Origin and scheme are shown, and the path never is.
  expect(broken?.textContent).toContain("api");
  expect(broken?.textContent).toContain("s3");
});

test("the first-run action registers the fixtures and says what it registered", async () => {
  let listed = 0;
  const s = stub();
  const impl = (async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = String(input);
    if ((init?.method ?? "GET") === "GET" && url.endsWith("/sources")) {
      listed += 1;
      s.requests.push("GET /sources");
      return json(listed === 1 ? { sources: [] } : { sources: [WATCHING] });
    }
    return s.impl(input, init);
  }) as typeof fetch;
  const element = await mount({ impl, requests: s.requests });
  element.active = true;
  await element.updateComplete;
  await settle();
  await element.updateComplete;

  // The empty state offers work rather than apologising.
  const action = element.shadowRoot?.querySelector('[part="action"]') as HTMLElement | null;
  expect(action?.textContent).toContain("Load the fixture stack");

  await element.loadFixtures();
  await element.updateComplete;
  // Two datasets and seven granules — and the report names them.
  const report = element.shadowRoot?.querySelector('[part="report"]')?.textContent ?? "";
  expect(report).toContain("hls-s30: 1 granule");
  expect(report).toContain("hls-s30-fire: 6 granules");
  expect(s.requests.filter((r) => r === "POST /datasets")).toHaveLength(2);
  expect(s.requests.filter((r) => r.startsWith("POST /datasets/"))).toHaveLength(7);
  // And the screen re-reads, so what it shows is what the server says.
  expect(rows(element)).toHaveLength(1);
});

test("re-running is idempotent: a 409 is 'already there', not a failure", async () => {
  const s = stub({
    sources: { sources: [] },
    post: () => json({ detail: "dataset `hls-s30` exists" }, 409),
  });
  const element = await mount(s);
  element.active = true;
  await element.updateComplete;
  await settle();
  await element.loadFixtures();
  await element.updateComplete;

  const report = element.shadowRoot?.querySelector('[part="report"]')?.textContent ?? "";
  expect(report).toContain("already there");
  expect(report).not.toContain("Could not");
});

test("a failure is the server's words, and no state is invented to fill the gap", async () => {
  const s = stub({ sourcesStatus: 500 });
  const element = await mount(s);
  element.active = true;
  await element.updateComplete;
  await settle();
  await element.updateComplete;
  expect(element.shadowRoot?.querySelector('[part="error"]')?.textContent).toContain(
    "Could not list sources",
  );
  expect(rows(element)).toEqual([]);
});
