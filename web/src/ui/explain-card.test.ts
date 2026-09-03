// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The tethered explain card (issue #394): one component, three densities,
// one dismissal rule.
//
// The content is built literally here rather than imported from
// `explain-model.ts`: a primitive must not depend on the organism layer (the
// DRY gate's `ui-reaches-out` rule), and this test is about what the card
// RENDERS given a content object. What the model derives from a trace is
// pinned by `explain-model.test.ts`, where the arithmetic lives.
import { afterEach, beforeAll, expect, test } from "vitest";
import type { ExplainContent } from "../explain-model.js";
import { SwathExplainCard } from "./explain-card.js";
import { SwathIcon } from "./icon.js";

beforeAll(() => {
  SwathIcon.define();
  SwathExplainCard.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

const CONCEPT: ExplainContent = {
  density: "concept",
  title: "granule",
  definition: "One acquisition: the imagery a satellite captured over one area at one moment.",
  rows: [],
  candidates: [],
};

const MEASURED: ExplainContent = {
  density: "measured",
  title: "park-fire-ndvi",
  rows: [
    { label: "decision", value: "live", mono: true },
    { label: "bytes read", value: "78.8 KB", mono: true },
    { label: "total", value: "545 ms", mono: true },
    { label: "ingest→pixel", value: "—", mono: true },
  ],
  candidates: [
    {
      strategy: "overview (factor 0)",
      cost: "0 B",
      admissible: false,
      reason: "source has no overviews",
    },
    { strategy: "live", cost: "78.8 KB", admissible: true, reason: "full-resolution read" },
  ],
  fix: "This dataset has no overviews. Building them with `swath materialize` helps.",
};

async function mount(content: ExplainContent): Promise<SwathExplainCard> {
  const card = document.createElement("swath-explain-card") as SwathExplainCard;
  card.content = content;
  card.open = true;
  document.body.append(card);
  await card.updateComplete;
  return card;
}

const text = (card: SwathExplainCard, part: string): string =>
  card.shadowRoot?.querySelector(`[part="${part}"]`)?.textContent ?? "";

test("concept: a definition, and no figures at all", async () => {
  const card = await mount(CONCEPT);
  expect(card.getAttribute("density")).toBe("concept");
  expect(text(card, "definition")).toContain("acquisition");
  expect(card.shadowRoot?.querySelector('[part="rows"]')).toBeNull();
  expect(card.shadowRoot?.querySelector('[part="candidates"]')).toBeNull();
  expect(card.shadowRoot?.querySelector('[part="fix"]')).toBeNull();
});

test("measured: the rows render, and an unmeasured figure stays an em dash", async () => {
  const card = await mount(MEASURED);
  expect(card.getAttribute("density")).toBe("measured");
  const rows = text(card, "rows");
  expect(rows).toContain("78.8 KB");
  expect(rows).toContain("545 ms");
  // The card must not helpfully turn a dash into a zero.
  expect(rows).toContain("—");
  expect(rows).not.toContain("0 ms");
});

test("measured: the planner's reasons are rendered verbatim, in its own order", async () => {
  const card = await mount(MEASURED);
  const reasons = [...(card.shadowRoot?.querySelectorAll('[part="candidate-reason"]') ?? [])].map(
    (n) => n.textContent,
  );
  expect(reasons).toEqual(["source has no overviews", "full-resolution read"]);
  // An inadmissible candidate is marked in form, not only in words.
  const marks = [...(card.shadowRoot?.querySelectorAll('[part="candidate"]') ?? [])].map((n) =>
    n.getAttribute("data-admissible"),
  );
  expect(marks).toEqual(["false", "true"]);
});

test("the fix appears only when the content carries one", async () => {
  const card = await mount(MEASURED);
  expect(text(card, "fix")).toContain("swath materialize");
  const { fix: _dropped, ...withoutFix } = MEASURED;
  card.content = withoutFix;
  await card.updateComplete;
  expect(card.shadowRoot?.querySelector('[part="fix"]')).toBeNull();
});

test("Escape and an outside click dismiss it; a click inside does not", async () => {
  const card = await mount(MEASURED);
  const dismissed: number[] = [];
  card.addEventListener("swath-explain-dismiss", () => dismissed.push(1));

  card.shadowRoot
    ?.querySelector('[part="base"]')
    ?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, composed: true }));
  expect(card.open).toBe(true);

  document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, composed: true }));
  expect(card.open).toBe(false);
  expect(dismissed).toHaveLength(1);

  card.open = true;
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  expect(card.open).toBe(false);
  expect(dismissed).toHaveLength(2);
});

test("it is hidden until opened, and empty content renders nothing", async () => {
  const card = document.createElement("swath-explain-card") as SwathExplainCard;
  document.body.append(card);
  await card.updateComplete;
  expect(getComputedStyle(card).display).toBe("none");
  expect(card.shadowRoot?.children).toHaveLength(0);
});

test("tethering puts the card beside its anchor, never off-screen", async () => {
  const card = await mount(MEASURED);
  const anchor = document.createElement("div");
  anchor.style.cssText = "position:fixed; left:40px; top:60px; width:20px; height:20px;";
  document.body.append(anchor);
  card.tetherTo(anchor);
  expect(Number.parseInt(card.style.left, 10)).toBeGreaterThanOrEqual(8);
  expect(Number.parseInt(card.style.top, 10)).toBeGreaterThanOrEqual(8);

  // An anchor at the right edge flips the card to the other side rather
  // than letting it hang off the viewport.
  anchor.style.left = `${window.innerWidth - 24}px`;
  card.tetherTo(anchor);
  const left = Number.parseInt(card.style.left, 10);
  expect(left + card.getBoundingClientRect().width).toBeLessThanOrEqual(window.innerWidth);
});
