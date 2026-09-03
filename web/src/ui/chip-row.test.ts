// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The breadcrumb chip row (issue #393): the URL rendered as the things the
// view is of, each droppable.
import { afterEach, beforeAll, expect, test } from "vitest";
import { SwathChipRow } from "./chip-row.js";
import { SwathIcon } from "./icon.js";

beforeAll(() => {
  SwathIcon.define();
  SwathChipRow.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(chips: Parameters<typeof rowChips>[0] = []): Promise<SwathChipRow> {
  const row = document.createElement("swath-chip-row") as SwathChipRow;
  row.chips = chips;
  document.body.append(row);
  await row.updateComplete;
  return row;
}

const rowChips = (chips: SwathChipRow["chips"]): SwathChipRow["chips"] => chips;

const parts = (row: SwathChipRow, part: string): Element[] => [
  ...(row.shadowRoot?.querySelectorAll(`[part="${part}"]`) ?? []),
];

test("renders one chip per thing the view is of, separated, in order", async () => {
  const row = await mount([
    { id: "layer", label: "layer", value: "park-fire-ndvi" },
    { id: "time", label: "date", value: "2024-10-15T19:03:00Z", removable: true },
  ]);
  const chips = parts(row, "chip");
  expect(chips).toHaveLength(2);
  expect(chips[0]?.getAttribute("data-chip")).toBe("layer");
  expect(chips[1]?.textContent).toContain("2024-10-15T19:03:00Z");
  // One separator between two chips, never a leading or trailing one.
  expect(parts(row, "separator")).toHaveLength(1);
});

test("an empty row renders nothing — chrome does not charge rent for a view it has not got", async () => {
  const row = await mount([]);
  expect(parts(row, "chip")).toHaveLength(0);
  const base = row.shadowRoot?.querySelector('[part="base"]');
  expect(base?.children).toHaveLength(0);
  expect(getComputedStyle(base as Element).display).toBe("none");
});

test("only a removable chip offers a remove control, and it says what it drops", async () => {
  const row = await mount([
    { id: "layer", label: "layer", value: "ndvi" },
    { id: "xray", label: "x-ray", value: "on", removable: true },
  ]);
  const removes = parts(row, "remove");
  expect(removes).toHaveLength(1);
  // The accessible name names the thing, not the glyph.
  expect(removes[0]?.getAttribute("aria-label")).toBe("remove the x-ray on");
});

test("removing a chip reports which one, and lets the host decide what that means", async () => {
  const row = await mount([
    { id: "time", label: "date", value: "2024-06-06T17:54:00Z", removable: true },
  ]);
  const seen: string[] = [];
  row.addEventListener("swath-chip-remove", (event) => {
    seen.push(event.detail.chip);
  });
  (parts(row, "remove")[0] as HTMLButtonElement).click();
  expect(seen).toEqual(["time"]);
  // The row does not remove it itself: the URL is the truth, and the host
  // owns the write. A row that mutated itself would be a second source.
  expect(parts(row, "chip")).toHaveLength(1);
});

test("assigning chips replaces them rather than accumulating", async () => {
  const row = await mount([{ id: "layer", label: "layer", value: "a" }]);
  row.chips = [{ id: "layer", label: "layer", value: "b" }];
  await row.updateComplete;
  expect(parts(row, "chip")).toHaveLength(1);
  expect(parts(row, "value")[0]?.textContent).toBe("b");
});
