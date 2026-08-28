// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { SwathGranuleCard } from "./granule-card.js";

beforeAll(() => {
  SwathGranuleCard.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(attrs: Record<string, string>): Promise<SwathGranuleCard> {
  const card = document.createElement("swath-granule-card");
  for (const [name, value] of Object.entries(attrs)) {
    card.setAttribute(name, value);
  }
  document.body.append(card);
  await card.updateComplete;
  return card;
}

const part = (card: SwathGranuleCard, name: string) =>
  card.shadowRoot?.querySelector(`[part="${name}"]`);

test("pending → thumbnail → refusal note; data-granule mirrors the id", async () => {
  const card = await mount({
    "granule-id": "FIX.A.2026",
    dataset: "hls-s30",
    datetime: "2026-06-01T10:00:00Z",
    kind: "rgb",
  });
  expect(card.dataset["granule"]).toBe("FIX.A.2026");
  expect(part(card, "pending")?.getAttribute("aria-busy")).toBe("true");
  expect(part(card, "title")?.textContent).toBe("2026-06-01 10:00Z");
  expect(part(card, "meta")?.textContent).toBe("FIX.A.2026 · rgb preview · current frame");
  card.thumbnail = "blob:fake";
  await card.updateComplete;
  expect((part(card, "media") as HTMLImageElement).getAttribute("src")).toBe("blob:fake");
  expect((part(card, "media") as HTMLImageElement).alt).toContain("FIX.A.2026");
  card.thumbnail = undefined;
  card.note = "the preview exceeds the pixel budget";
  await card.updateComplete;
  expect(part(card, "note")?.textContent).toContain("pixel budget");
  expect(part(card, "media")).toBeNull();
});

test("activation emits swath-activate with the granule id", async () => {
  const card = await mount({ "granule-id": "FIX.B.2026" });
  const seen: string[] = [];
  document.body.addEventListener("swath-activate", (e) => seen.push(e.detail.id));
  const inner = card.shadowRoot
    ?.querySelector("swath-card")
    ?.shadowRoot?.querySelector<HTMLElement>('[part="base"]');
  inner?.click();
  expect(seen).toEqual(["FIX.B.2026"]);
});
