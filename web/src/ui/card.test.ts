// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { userEvent } from "vitest/browser";
import { SwathCard } from "./card.js";

beforeAll(() => {
  SwathCard.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(html: string): Promise<SwathCard> {
  const host = document.createElement("div");
  host.innerHTML = html;
  document.body.append(host);
  const card = host.querySelector("swath-card");
  if (!card) {
    throw new Error("no card");
  }
  await card.updateComplete;
  return card;
}

const base = (card: SwathCard): HTMLElement =>
  card.shadowRoot?.querySelector('[part="base"]') as HTMLElement;

test("slots and parts: media, header (title or slot), body, footer", async () => {
  const card = await mount(
    '<swath-card title="Granule 1"><img slot="media" alt=""><p>body</p><span slot="footer">f</span></swath-card>',
  );
  for (const part of ["base", "media", "header", "body", "footer"]) {
    expect(card.shadowRoot?.querySelector(`[part="${part}"]`), part).not.toBeNull();
  }
  expect(card.shadowRoot?.querySelector('slot[name="header"]')?.textContent).toBe("Granule 1");
  expect(base(card).hasAttribute("role")).toBe(false);
  expect(base(card).hasAttribute("tabindex")).toBe(false);
});

test("interactive: button role, Enter/Space/click activate, selected → aria-pressed", async () => {
  const card = await mount('<swath-card id="g1" interactive selected>x</swath-card>');
  expect(base(card).getAttribute("role")).toBe("button");
  expect(base(card).tabIndex).toBe(0);
  expect(base(card).getAttribute("aria-pressed")).toBe("true");
  const seen: { id: string; long: boolean }[] = [];
  card.addEventListener("swath-activate", (e) => seen.push(e.detail));
  card.focus();
  expect(card.shadowRoot?.activeElement).toBe(base(card));
  await userEvent.keyboard("{Enter}");
  await userEvent.keyboard(" ");
  base(card).click();
  expect(seen).toEqual([
    { id: "g1", long: false },
    { id: "g1", long: false },
    { id: "g1", long: false },
  ]);
});

test("a touch long-press activates with long: true and swallows the following click", async () => {
  const card = await mount('<swath-card id="g2" interactive>x</swath-card>');
  const seen: boolean[] = [];
  card.addEventListener("swath-activate", (e) => seen.push(e.detail.long));
  const press = (type: string, x = 10, y = 10) =>
    base(card).dispatchEvent(
      new PointerEvent(type, { bubbles: true, pointerType: "touch", clientX: x, clientY: y }),
    );
  press("pointerdown");
  await new Promise((r) => setTimeout(r, 560));
  press("pointerup");
  base(card).click();
  expect(seen).toEqual([true]);
  base(card).click();
  expect(seen).toEqual([true, false]);
});
