// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, expect, test, vi } from "vitest";
import { ApiProblem, type EventSourceLike, fieldFor, SwathApi } from "./api.js";

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });

/** A scripted fetch: `"METHOD url"` → response factory; records calls. */
function stub(routes: Record<string, () => Response | Promise<Response>>) {
  const calls: { url: string; init: RequestInit | undefined }[] = [];
  const impl = (async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = String(input);
    calls.push({ url, init });
    const route = routes[`${init?.method ?? "GET"} ${url}`];
    return route ? route() : new Response("not scripted", { status: 404 });
  }) as typeof fetch;
  return { impl, calls };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

test("url(): base without a trailing slash, query with undefined dropped, absolute passthrough", () => {
  const api = new SwathApi({ base: "https://swath.test///" });
  expect(api.base).toBe("https://swath.test");
  expect(api.url("/tilesets")).toBe("https://swath.test/tilesets");
  expect(api.url("/traces", { layer: "ndvi", z: 3, live: true, none: undefined })).toBe(
    "https://swath.test/traces?layer=ndvi&z=3&live=true",
  );
  expect(api.url("/t?a=1", { b: "x y" })).toBe("https://swath.test/t?a=1&b=x+y");
  expect(api.url("https://stac.example/item.json")).toBe("https://stac.example/item.json");
  expect(new SwathApi().url("/healthz")).toBe("/healthz");
});

test("the default fetch is globalThis.fetch at call time (vi.stubGlobal keeps working)", async () => {
  const api = new SwathApi({ base: "https://swath.test" });
  vi.stubGlobal("fetch", stub({ "GET https://swath.test/x": () => json({ ok: 1 }) }).impl);
  expect(await api.json("/x")).toEqual({ ok: 1 });
});

test("json() sends accept, merges headers, passes an abort signal through", async () => {
  const { impl, calls } = stub({
    "POST https://swath.test/result": () => json({ done: true }),
  });
  const api = new SwathApi({ base: "https://swath.test", fetch: impl });
  const controller = new AbortController();
  await api.json("/result", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: "{}",
    signal: controller.signal,
  });
  expect(calls[0]?.init?.headers).toEqual({
    accept: "application/json",
    "content-type": "application/json",
  });
  expect(calls[0]?.init?.signal).toBe(controller.signal);
});

test("an aborted request rejects with the AbortError, not an ApiProblem", async () => {
  const api = new SwathApi({
    fetch: ((_input: RequestInfo | URL, init?: RequestInit) =>
      new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener("abort", () => reject(init.signal?.reason));
      })) as typeof fetch,
  });
  const controller = new AbortController();
  const pending = api.json("/slow", { signal: controller.signal });
  controller.abort();
  await expect(pending).rejects.toMatchObject({ name: "AbortError" });
});

test("json()/blob() reject with ApiProblem: RFC 7807, openEO, and non-JSON bodies", async () => {
  const api = new SwathApi({
    base: "https://swath.test",
    fetch: stub({
      "GET https://swath.test/p": () =>
        json(
          { type: "about:blank", title: "Bad Request", status: 400, detail: "no such band" },
          400,
        ),
      "GET https://swath.test/o": () =>
        json({ code: "ProcessGraphInvalid", message: "cycle" }, 422),
      "GET https://swath.test/h": () => new Response("<html>gateway</html>", { status: 502 }),
    }).impl,
  });
  const p = await api.json("/p").catch((e: unknown) => e);
  expect(p).toBeInstanceOf(ApiProblem);
  expect(p).toMatchObject({ status: 400, title: "Bad Request", detail: "no such band" });
  expect((p as ApiProblem).message).toBe("Bad Request: no such band");

  const o = await api.blob("/o").catch((e: unknown) => e);
  expect(o).toMatchObject({ status: 422, title: "ProcessGraphInvalid", detail: "cycle" });

  const h = await api.json("/h").catch((e: unknown) => e);
  expect(h).toMatchObject({ status: 502, title: "", detail: "", body: undefined });
  expect((h as ApiProblem).message).toBe("HTTP 502");
});

test("blob() returns the body on success", async () => {
  const api = new SwathApi({
    fetch: stub({
      "GET /result": () => new Response(new Blob([new Uint8Array([1, 2, 3])]), { status: 200 }),
    }).impl,
  });
  expect((await api.blob("/result")).size).toBe(3);
});

test("capabilities(): one GET / per success, shared; a failure clears so the next call retries", async () => {
  let fail = true;
  const { impl, calls } = stub({
    "GET https://swath.test/": () => (fail ? json({ title: "down" }, 503) : json({ links: [] })),
  });
  const api = new SwathApi({ base: "https://swath.test", fetch: impl });

  await expect(api.capabilities()).rejects.toBeInstanceOf(ApiProblem);
  expect(calls).toHaveLength(1);
  fail = false;
  const [a, b] = await Promise.all([api.capabilities(), api.capabilities()]);
  expect(a).toEqual({ links: [] });
  expect(b).toBe(a);
  expect(calls).toHaveLength(2);
  await api.capabilities();
  expect(calls).toHaveLength(2); // cached after success
});

test("events() opens the SSE stream under the base through the factory", () => {
  const opened: string[] = [];
  const source: EventSourceLike = { addEventListener: () => undefined, close: () => undefined };
  const api = new SwathApi({
    base: "https://swath.test",
    eventSource: (url) => {
      opened.push(url);
      return source;
    },
  });
  expect(api.events("/traces", { layer: "ndvi" })).toBe(source);
  expect(opened).toEqual(["https://swath.test/traces?layer=ndvi"]);
});

test("fieldFor(): first matching rule routes the detail; none → flow line; no detail → status", () => {
  const rules = [
    { includes: "URL-safe", field: "dataset", note: () => "use only letters" },
    { includes: "band", field: "band" },
  ] as const;
  expect(fieldFor(new ApiProblem(400, { detail: "id is not URL-safe" }), rules)).toEqual({
    field: "dataset",
    note: "use only letters",
  });
  expect(fieldFor(new ApiProblem(400, { detail: "declared bands differ" }), rules)).toEqual({
    field: "band",
    note: "declared bands differ",
  });
  expect(fieldFor(new ApiProblem(400, { detail: "something else" }), rules)).toEqual({
    field: "",
    note: "something else",
  });
  expect(fieldFor(new ApiProblem(500, "oops"), rules)).toEqual({
    field: "",
    note: "the server refused with HTTP 500",
  });
});
