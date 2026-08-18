// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// <swath-add-data-panel> (issue #197): capabilities-gated (read-only
// renders no form), lazy (zero requests until opened), and the whole
// paste → register → quick-look flow with recorded POST bodies, the 409
// continue, server problems routed to fields, and the local-mode upload.

import { afterEach, beforeAll, expect, test } from "vitest";
import {
  defineSwathAddDataPanel,
  READ_ONLY_NOTE,
  SwathAddDataPanel,
} from "./swath-add-data-panel.js";

const SERVER = "https://swath.test";

const WRITABLE_CAPABILITIES = {
  endpoints: [
    { path: "/datasets", methods: ["POST"] },
    { path: "/datasets/{dataset_id}/granules", methods: ["GET", "POST"] },
    { path: "/services", methods: ["GET", "POST"] },
    { path: "/uploads/{filename}", methods: ["PUT"] },
  ],
};

const READ_ONLY_CAPABILITIES = {
  endpoints: [
    { path: "/datasets/{dataset_id}/granules", methods: ["GET"] },
    { path: "/services", methods: ["GET"] },
    { path: "/result", methods: ["POST"] },
  ],
};

const ITEM_URL = "https://data.test/scene-1/item.json";
const ITEM = {
  type: "Feature",
  stac_version: "1.1.0",
  id: "scene-1",
  collection: "hls-demo",
  bbox: [-105.5, 39.2, -105.4, 39.3],
  properties: { datetime: "2024-06-06T17:54:00Z" },
  assets: { b04: { href: "scene-1-b04.tif" } },
};

interface Recorded {
  method: string;
  url: string;
  body: unknown;
}

/** A scripted fetch keyed `"METHOD url"`, recording every call. */
function fetchStub(routes: Record<string, () => Response>): {
  impl: typeof fetch;
  requests: Recorded[];
} {
  const requests: Recorded[] = [];
  const impl = (async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = input instanceof Request ? input.url : String(input);
    const method = init?.method ?? "GET";
    let body: unknown;
    if (typeof init?.body === "string") {
      body = JSON.parse(init.body);
    } else if (init?.body instanceof File) {
      body = await init.body.text();
    }
    requests.push({ method, url, body });
    const route = routes[`${method} ${url}`];
    if (route === undefined) {
      return new Response("not scripted", { status: 404 });
    }
    return route();
  }) as typeof fetch;
  return { impl, requests };
}

const json = (body: unknown, status = 200, headers: Record<string, string> = {}): Response =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });

function mount(
  stub: { impl: typeof fetch },
  attributes: Record<string, string> = {},
): SwathAddDataPanel {
  const panel = document.createElement("swath-add-data-panel") as SwathAddDataPanel;
  panel.setAttribute("server", SERVER);
  for (const [name, value] of Object.entries(attributes)) {
    panel.setAttribute(name, value);
  }
  panel.fetchImpl = stub.impl;
  document.body.append(panel);
  return panel;
}

function open(panel: SwathAddDataPanel): void {
  panel.querySelector<HTMLButtonElement>(".swath-add-data-toggle")?.click();
}

async function paste(panel: SwathAddDataPanel, link: string): Promise<void> {
  const input = panel.querySelector<HTMLInputElement>("#swath-add-data-link");
  if (input === null) {
    throw new Error("link input missing");
  }
  input.value = link;
  input.dispatchEvent(new Event("input"));
  input.dispatchEvent(new Event("change"));
  await panel.ready;
}

async function submit(panel: SwathAddDataPanel): Promise<void> {
  const button = panel.querySelector<HTMLButtonElement>(".swath-add-data-submit");
  if (button === null) {
    throw new Error("submit missing");
  }
  expect(button.disabled, "the flow must be submittable").toBe(false);
  button.click();
  await panel.ready;
}

beforeAll(() => {
  defineSwathAddDataPanel();
});

afterEach(() => {
  document.body.replaceChildren();
});

test("registers exactly once and upgrades with an accessible role", () => {
  defineSwathAddDataPanel(); // second call must be a no-op
  expect(customElements.get(SwathAddDataPanel.tagName)).toBe(SwathAddDataPanel);
  const panel = mount(fetchStub({}));
  expect(panel.getAttribute("role")).toBe("group");
  expect(panel.getAttribute("aria-label")).toBe("Add data");
});

test("lazy by contract: a closed panel issues zero requests", async () => {
  const stub = fetchStub({ [`GET ${SERVER}/`]: () => json(WRITABLE_CAPABILITIES) });
  const panel = mount(stub);
  expect(stub.requests).toEqual([]);
  open(panel);
  await panel.ready;
  expect(stub.requests.map((r) => `${r.method} ${r.url}`)).toEqual([`GET ${SERVER}/`]);
});

test("read-only capabilities render the note, never the form", async () => {
  const stub = fetchStub({ [`GET ${SERVER}/`]: () => json(READ_ONLY_CAPABILITIES) });
  const panel = mount(stub);
  open(panel);
  await panel.ready;
  expect(panel.querySelector(".swath-add-data-readonly")?.textContent).toBe(READ_ONLY_NOTE);
  expect(panel.querySelector("form")).toBeNull();
  expect(panel.querySelector("#swath-add-data-link")).toBeNull();
});

test("paste a STAC item: prefilled, registered inline, quick look announced", async () => {
  const stub = fetchStub({
    [`GET ${SERVER}/`]: () => json(WRITABLE_CAPABILITIES),
    [`GET ${ITEM_URL}`]: () => json(ITEM),
    [`POST ${SERVER}/datasets`]: () => json({ id: "hls-demo" }, 201),
    [`POST ${SERVER}/datasets/hls-demo/granules`]: () =>
      json({ id: "scene-1", dataset: "hls-demo" }, 201),
    [`POST ${SERVER}/services`]: () => json({}, 201, { "openeo-identifier": "xyz-abc123def456" }),
  });
  const panel = mount(stub);
  open(panel);
  await panel.ready;
  await paste(panel, ITEM_URL);

  // Pre-filled from the fetched item, editable before anything is sent.
  expect(panel.querySelector<HTMLInputElement>("#swath-add-data-dataset")?.value).toBe("hls-demo");
  expect(stub.requests.filter((r) => r.method === "POST")).toEqual([]);

  const announced = new Promise<{ dataset: string; layer: string }>((resolve) => {
    panel.addEventListener(
      "swath-data-added",
      (event) => {
        resolve((event as CustomEvent<{ dataset: string; layer: string }>).detail);
      },
      { once: true },
    );
  });
  await submit(panel);

  const posts = stub.requests.filter((r) => r.method === "POST");
  expect(posts.map((r) => r.url)).toEqual([
    `${SERVER}/datasets`,
    `${SERVER}/datasets/hls-demo/granules`,
    `${SERVER}/services`,
  ]);
  // The dataset body derives from the item; the granule is the inline
  // document itself (the server never fetches URLs — this panel did).
  expect(posts[0]?.body).toMatchObject({ id: "hls-demo", bands: ["b04"] });
  expect(posts[1]?.body).toEqual({ stac_item: ITEM });
  // The quick look goes through the engine: an xyz service graph.
  expect(posts[2]?.body).toMatchObject({ type: "xyz" });
  expect(await announced).toEqual({ dataset: "hls-demo", layer: "xyz-abc123def456" });
  expect(panel.querySelector(".swath-add-data-status")?.textContent).toContain("Serving");
});

test("an existing dataset (409) is added to, not an error", async () => {
  const stub = fetchStub({
    [`GET ${SERVER}/`]: () => json(WRITABLE_CAPABILITIES),
    [`GET ${ITEM_URL}`]: () => json(ITEM),
    [`POST ${SERVER}/datasets`]: () =>
      json({ type: "about:blank", title: "Conflict", status: 409, detail: "exists" }, 409),
    [`POST ${SERVER}/datasets/hls-demo/granules`]: () =>
      json({ id: "scene-1", dataset: "hls-demo" }, 201),
    [`POST ${SERVER}/services`]: () => json({}, 201, { "openeo-identifier": "xyz-1" }),
  });
  const panel = mount(stub);
  open(panel);
  await panel.ready;
  await paste(panel, ITEM_URL);
  await submit(panel);
  expect(stub.requests.filter((r) => r.method === "POST")).toHaveLength(3);
  expect(panel.querySelector(".swath-add-data-status")?.textContent).toContain("Serving");
});

test("a server refusal lands under the field that caused it", async () => {
  const stub = fetchStub({
    [`GET ${SERVER}/`]: () => json(WRITABLE_CAPABILITIES),
    [`POST ${SERVER}/datasets`]: () => json({ id: "no-such-scene" }, 201),
    [`POST ${SERVER}/datasets/no-such-scene/granules`]: () =>
      json(
        {
          type: "about:blank",
          title: "Bad Request",
          status: 400,
          detail: "asset `data` (no-such-scene.tif) failed header validation: not found",
        },
        400,
      ),
  });
  const panel = mount(stub);
  open(panel);
  await panel.ready;
  await paste(panel, "no-such-scene.tif");
  // Complete the direct form (plain words gate the button until then).
  const set = (id: string, value: string): void => {
    const input = panel.querySelector<HTMLInputElement>(`#swath-add-data-${id}`);
    if (input === null) {
      throw new Error(`missing field ${id}`);
    }
    input.value = value;
    input.dispatchEvent(new Event("input"));
  };
  set("datetime", "2024-06-06T17:54:00Z");
  await submit(panel);

  // The refusal names the link, in the link's own note — and the quick
  // look never fires.
  expect(panel.querySelector("#swath-add-data-link-note")?.textContent).toContain(
    "could not read that file",
  );
  expect(stub.requests.some((r) => r.url === `${SERVER}/services`)).toBe(false);
});

test("local-mode drop: upload lands in the store, then the form continues", async () => {
  const stub = fetchStub({
    [`GET ${SERVER}/`]: () => json(WRITABLE_CAPABILITIES),
    [`PUT ${SERVER}/uploads/My-Scene.tif`]: () => json({ href: "uploads/My-Scene.tif" }, 201),
  });
  const panel = mount(stub);
  open(panel);
  await panel.ready;

  const picker = panel.querySelector<HTMLInputElement>("#swath-add-data-file");
  expect(picker, "the drop zone renders where uploads are mounted").not.toBeNull();
  if (picker === null) {
    return;
  }
  const transfer = new DataTransfer();
  transfer.items.add(new File(["tiff bytes"], "My Scene.tif"));
  picker.files = transfer.files;
  picker.dispatchEvent(new Event("change"));
  await panel.ready;

  const put = stub.requests.find((r) => r.method === "PUT");
  expect(put?.url).toBe(`${SERVER}/uploads/My-Scene.tif`);
  expect(put?.body).toBe("tiff bytes");
  // The returned store key is now the link; the direct form is open.
  expect(panel.querySelector<HTMLInputElement>("#swath-add-data-link")?.value).toBe(
    "uploads/My-Scene.tif",
  );
  expect(panel.querySelector("#swath-add-data-band")).not.toBeNull();
});

test("no upload capability, no drop zone", async () => {
  const stub = fetchStub({
    [`GET ${SERVER}/`]: () => json({ endpoints: [{ path: "/datasets", methods: ["POST"] }] }),
  });
  const panel = mount(stub);
  open(panel);
  await panel.ready;
  expect(panel.querySelector("form")).not.toBeNull();
  expect(panel.querySelector("#swath-add-data-file")).toBeNull();
});

test("a transient capabilities failure retries on re-open (review round 1, finding 1)", async () => {
  // First GET / fails; the panel must not stay bricked — the error note
  // says "close and re-open to retry", and that retry must really run.
  let calls = 0;
  const stub = fetchStub({
    [`GET ${SERVER}/`]: () => {
      calls += 1;
      return calls === 1 ? new Response("boom", { status: 500 }) : json(WRITABLE_CAPABILITIES);
    },
  });
  const panel = mount(stub);
  open(panel);
  await panel.ready;
  expect(panel.querySelector(".swath-add-data-error")?.textContent).toContain(
    "Cannot reach the server",
  );

  open(panel); // close…
  open(panel); // …and re-open: the promised retry
  await panel.ready;
  expect(panel.querySelector(".swath-add-data-error")).toBeNull();
  expect(panel.querySelector("#swath-add-data-link")).not.toBeNull();
  expect(calls).toBe(2);
});

test("the dataset id is read-only in STAC mode — a mismatch is unconstructible (finding 2)", async () => {
  const stub = fetchStub({
    [`GET ${SERVER}/`]: () => json(WRITABLE_CAPABILITIES),
    [`GET ${ITEM_URL}`]: () => json(ITEM),
  });
  const panel = mount(stub);
  open(panel);
  await panel.ready;
  await paste(panel, ITEM_URL);

  // The item names its collection; registration must match it (the
  // server refuses otherwise), so the field shows but does not edit —
  // with the help text saying why in plain words.
  const dataset = panel.querySelector<HTMLInputElement>("#swath-add-data-dataset");
  expect(dataset?.readOnly).toBe(true);
  expect(dataset?.value).toBe("hls-demo");
  expect(dataset?.closest("label")?.textContent).toContain("Named by the item's collection");

  // The direct (COG) form keeps the field editable.
  await paste(panel, "scene.tif");
  expect(panel.querySelector<HTMLInputElement>("#swath-add-data-dataset")?.readOnly).toBe(false);
});

test("a slow item fetch resolving late never clobbers a newer paste (finding 3)", async () => {
  let releaseSlow: (() => void) | undefined;
  const slow = new Promise<void>((resolve) => {
    releaseSlow = resolve;
  });
  const base = fetchStub({ [`GET ${SERVER}/`]: () => json(WRITABLE_CAPABILITIES) });
  const impl = (async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    if (String(input) === ITEM_URL) {
      await slow; // the stale fetch: held open until after the next paste
      return json(ITEM);
    }
    return base.impl(input, init);
  }) as typeof fetch;
  const panel = mount({ impl });
  open(panel);
  await panel.ready;

  // Paste the slow item…
  const input = panel.querySelector<HTMLInputElement>("#swath-add-data-link");
  if (input === null) {
    throw new Error("link input missing");
  }
  input.value = ITEM_URL;
  input.dispatchEvent(new Event("input"));
  input.dispatchEvent(new Event("change"));
  const stale = panel.ready;

  // …then move on to a raster link before the item arrives.
  await paste(panel, "newer-scene.tif");
  expect(panel.querySelector<HTMLInputElement>("#swath-add-data-dataset")?.value).toBe(
    "newer-scene",
  );

  // The stale response lands — and must change nothing: the newer draft
  // stays, and no leftover "Reading the item…" spinner survives.
  releaseSlow?.();
  await stale;
  expect(panel.querySelector<HTMLInputElement>("#swath-add-data-dataset")?.value).toBe(
    "newer-scene",
  );
  expect(panel.querySelector<HTMLInputElement>("#swath-add-data-band")).not.toBeNull();
  expect(panel.textContent).not.toContain("Reading the item…");
});

test("the stac attribute (the ?stac= deep link) opens pre-filled, sends nothing", async () => {
  const stub = fetchStub({
    [`GET ${SERVER}/`]: () => json(WRITABLE_CAPABILITIES),
    [`GET ${ITEM_URL}`]: () => json(ITEM),
  });
  const panel = mount(stub, { stac: ITEM_URL });
  await panel.ready;
  // Open, link pre-filled, draft derived — and nothing was registered.
  expect(panel.querySelector(".swath-add-data-toggle")?.getAttribute("aria-expanded")).toBe("true");
  expect(panel.querySelector<HTMLInputElement>("#swath-add-data-link")?.value).toBe(ITEM_URL);
  expect(panel.querySelector<HTMLInputElement>("#swath-add-data-dataset")?.value).toBe("hls-demo");
  expect(stub.requests.every((r) => r.method === "GET")).toBe(true);
});
