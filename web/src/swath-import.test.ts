// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { SwathApi } from "./api.js";
import { defineSwathImport, type SwathImport } from "./swath-import.js";
import { createSwathEvent } from "./ui/events.js";

const SERVER = "https://swath.test";

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

const REGISTER = {
  federationOff: false,
  register: [
    {
      id: "earth-search",
      title: "Earth Search",
      url: "https://stac.example.org/v1",
      host: "stac.example.org",
      allowed: true,
    },
    {
      id: "blocked",
      title: "Somewhere Else",
      url: "https://no.example.net/v1",
      host: "no.example.net",
      allowed: false,
    },
  ],
};

function stub(body: unknown = REGISTER) {
  const requests: string[] = [];
  const impl = (async (input: RequestInfo | URL): Promise<Response> => {
    requests.push(new URL(String(input)).pathname);
    return json(body);
  }) as typeof fetch;
  return { impl, requests };
}

beforeAll(() => {
  defineSwathImport();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(s: ReturnType<typeof stub>): Promise<SwathImport> {
  const element = document.createElement("swath-import");
  element.api = new SwathApi({ base: SERVER, fetch: s.impl });
  document.body.append(element);
  await element.updateComplete;
  return element;
}

const settle = () => new Promise((r) => setTimeout(r, 30));
const entries = (element: SwathImport) =>
  [...(element.shadowRoot?.querySelectorAll('[part="entry"]') ?? [])] as HTMLElement[];

async function type(element: SwathImport, text: string): Promise<void> {
  element.shadowRoot
    ?.querySelector('swath-field[name="import"]')
    ?.dispatchEvent(createSwathEvent("swath-change", { name: "import", value: text }));
  await element.updateComplete;
}

test("the register comes from the server, and a blocked entry is shown with the reason", async () => {
  const s = stub();
  const element = await mount(s);
  element.active = true;
  await element.updateComplete;
  await settle();
  await element.updateComplete;

  expect(s.requests).toEqual(["/sources/register"]);
  const rows = entries(element);
  expect(rows).toHaveLength(2);
  // Offered and reachable: it has an action.
  expect(rows[0]?.dataset["allowed"]).toBe("true");
  expect(rows[0]?.querySelector("swath-button")).not.toBeNull();
  // Offered and not reachable: listed, with the host to allowlist, and
  // no action that would fail.
  expect(rows[1]?.dataset["allowed"]).toBe("false");
  expect(rows[1]?.textContent).toContain("no.example.net is not on this deployment's allowlist");
  expect(rows[1]?.querySelector("swath-button")).toBeNull();
});

test("one input detects the method; picking from the register fills the same input", async () => {
  const s = stub();
  const element = await mount(s);
  element.active = true;
  await element.updateComplete;
  await settle();
  await element.updateComplete;

  await type(element, JSON.stringify({ type: "Collection", id: "sentinel-2-l2a" }));
  expect(element.detection).toMatchObject({ ok: true, method: "stac-collection" });
  expect(element.shadowRoot?.querySelector('[part="detected"]')?.textContent).toContain(
    "a STAC collection — sentinel-2-l2a",
  );

  // The register's action goes through the same path, so there is one
  // flow whichever way you started.
  const steps: string[] = [];
  element.addEventListener("swath-import-step", (event) => steps.push(event.detail.step));
  entries(element)[0]?.querySelector("swath-button")?.dispatchEvent(new Event("click"));
  await element.updateComplete;
  expect(element.detection).toMatchObject({ ok: true, url: "https://stac.example.org/v1" });
  expect(steps).toEqual(["review"]);
});

test("detection failure says what it tried and offers the explicit choice", async () => {
  const s = stub();
  const element = await mount(s);
  element.active = true;
  await element.updateComplete;
  await settle();

  await type(element, "denver");
  const note = element.shadowRoot?.querySelector('[part="undetected"]')?.textContent ?? "";
  expect(note).toContain("We tried a STAC document and a link to a STAC endpoint");
  expect(note).toContain("Choose the method yourself");

  // Four explicit choices, and picking one is what the flow imports as.
  const choices = [...(element.shadowRoot?.querySelectorAll("[data-method]") ?? [])];
  expect(choices).toHaveLength(4);
  expect(element.method).toBeUndefined();
  (
    choices.find((c) => (c as HTMLElement).dataset["method"] === "stac-item") as HTMLElement
  )?.dispatchEvent(new Event("click"));
  await element.updateComplete;
  expect(element.method).toBe("stac-item");
});

test("every step is nameable, and an unknown one is the beginning", async () => {
  const element = await mount(stub());
  expect(element.current).toBe("source");

  // A deep link into the middle of a flow resumes there.
  element.step = "review";
  await element.updateComplete;
  expect(element.current).toBe("review");
  expect(element.shadowRoot?.querySelector('[aria-current="step"]')?.textContent).toBe(
    "What we found",
  );
  // Nothing chosen: it says so rather than pretending to resume.
  expect(element.shadowRoot?.querySelector('[part="detected"]')?.textContent).toContain(
    "Nothing chosen yet",
  );

  // A link naming a step nobody has is a link to the start.
  element.step = "nonsense";
  await element.updateComplete;
  expect(element.current).toBe("source");
});

test("a register that cannot be read claims nothing reachable", async () => {
  const failing = {
    impl: (async () => new Response("nope", { status: 500 })) as typeof fetch,
    requests: [] as string[],
  };
  const element = await mount(failing);
  element.active = true;
  await element.updateComplete;
  await settle();
  await element.updateComplete;
  expect(element.register.rows).toEqual([]);
  expect(element.register.federationOff).toBe(true);
  expect(entries(element)).toEqual([]);
});
