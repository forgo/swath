// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import {
  detect,
  hostOf,
  IMPORT_STEPS,
  nextStep,
  stepFrom,
  type Undetected,
  undetectedNote,
} from "./import-model.js";

test("a pasted STAC document names its own type, so nothing is guessed", () => {
  for (const [type, method] of [
    ["Catalog", "stac-catalog"],
    ["Collection", "stac-collection"],
    ["Feature", "stac-item"],
    ["FeatureCollection", "stac-item-collection"],
  ] as const) {
    const detected = detect(JSON.stringify({ type, id: "demo" }));
    expect(detected).toMatchObject({ ok: true, method, title: "demo" });
  }
});

test("a pasted link is taken as a link, and its host is the title", () => {
  const detected = detect("  https://stac.example.org/v1/catalog.json  ");
  expect(detected).toMatchObject({
    ok: true,
    method: "stac-catalog",
    url: "https://stac.example.org/v1/catalog.json",
    title: "stac.example.org",
  });
  expect(hostOf("https://user@stac.example.org:8080/x")).toBe("stac.example.org");
  expect(hostOf("not a url")).toBeUndefined();
});

test("the title prefers what the document calls itself", () => {
  expect(detect(JSON.stringify({ type: "Catalog", id: "x", title: "Sentinel" }))).toMatchObject({
    title: "Sentinel",
  });
  // No title, no id: the method's own name rather than an empty string.
  expect(detect(JSON.stringify({ type: "Catalog" }))).toMatchObject({ title: "stac-catalog" });
});

test("detection failure says what it tried and offers the explicit choice", () => {
  const notJson = detect("{ this is not json") as Undetected;
  expect(notJson.ok).toBe(false);
  expect(notJson.tried).toEqual(["a STAC document"]);
  expect(notJson.reason).toContain("does not parse");

  const wrongType = detect(JSON.stringify({ type: "Shapefile" })) as Undetected;
  expect(wrongType.reason).toContain('"Shapefile"');

  const noType = detect(JSON.stringify({ id: "x" })) as Undetected;
  expect(noType.reason).toContain("no `type`");

  const neither = detect("denver") as Undetected;
  expect(neither.tried).toEqual(["a STAC document", "a link to a STAC endpoint"]);
  const note = undetectedNote(neither);
  expect(note).toContain("We tried a STAC document and a link to a STAC endpoint");
  expect(note).toContain("Choose the method yourself");
  // Never the unhelpful version.
  expect(note).not.toContain("invalid input");
});

test("an empty input asks for one rather than reporting a failure", () => {
  const empty = detect("   ") as Undetected;
  expect(empty.ok).toBe(false);
  expect(empty.tried).toEqual([]);
  expect(undetectedNote(empty)).toBe("Paste a link or drop a file to start.");
});

test("every step is nameable, and an unknown one is the beginning", () => {
  expect(IMPORT_STEPS).toEqual(["source", "review", "confirm"]);
  expect(stepFrom("review")).toBe("review");
  expect(stepFrom(null)).toBe("source");
  // A link naming a step nobody has goes to the start, not to an error.
  expect(stepFrom("nonsense")).toBe("source");
  expect(nextStep("source")).toBe("review");
  expect(nextStep("confirm")).toBeUndefined();
});
