// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Runs in a REAL browser (Vitest Browser Mode + Playwright): actual Custom
// Elements registry, actual shadow roots — the things jsdom fakes.
import { beforeAll, expect, test } from "vitest";
import { defineSwathBadge, SwathBadge } from "./swath-badge.js";

beforeAll(() => {
  defineSwathBadge();
});

test("registers exactly once, upgrades, and renders its label in its shadow root", async () => {
  defineSwathBadge(); // second call must be a no-op, not a registry error
  expect(customElements.get(SwathBadge.tagName)).toBe(SwathBadge);

  const el = document.createElement("swath-badge");
  el.setAttribute("label", "ingest-to-pixel");
  document.body.append(el);
  await el.updateComplete;

  expect(el).toBeInstanceOf(SwathBadge);
  expect(el.shadowRoot?.textContent).toBe("ingest-to-pixel");
  expect(el.getAttribute("role")).toBe("status");
  // Styled through tokens only: the pill radius resolves from the document sheet.
  expect(getComputedStyle(el).borderTopLeftRadius).toBe("999px");
  el.remove();
});

test("reacts to attribute and property changes while connected", async () => {
  const el = document.createElement("swath-badge");
  document.body.append(el);
  await el.updateComplete;
  expect(el.shadowRoot?.textContent).toBe("swath");

  el.setAttribute("label", "live");
  await el.updateComplete;
  expect(el.shadowRoot?.textContent).toBe("live");
  el.label = "cache";
  await el.updateComplete;
  expect(el.shadowRoot?.textContent).toBe("cache");
  el.remove();
});
