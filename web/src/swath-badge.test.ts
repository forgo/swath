// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Runs in a REAL browser (Vitest Browser Mode + Playwright): actual Custom
// Elements registry, actual lifecycle callbacks — the things jsdom fakes.
import { beforeAll, expect, test } from "vitest";
import { defineSwathBadge, SwathBadge } from "./swath-badge.js";

beforeAll(() => {
  defineSwathBadge();
});

test("registers exactly once, upgrades, and renders its label", () => {
  defineSwathBadge(); // second call must be a no-op, not a registry error
  expect(customElements.get(SwathBadge.tagName)).toBe(SwathBadge);

  const el = document.createElement("swath-badge");
  el.setAttribute("label", "ingest-to-pixel");
  document.body.append(el);

  expect(el).toBeInstanceOf(SwathBadge);
  expect(el.textContent).toBe("ingest-to-pixel");
  expect(el.getAttribute("role")).toBe("status");
  el.remove();
});

test("reacts to attribute changes while connected", () => {
  const el = document.createElement("swath-badge");
  document.body.append(el);
  expect(el.textContent).toBe("swath");

  el.setAttribute("label", "live");
  expect(el.textContent).toBe("live");
  el.remove();
});
