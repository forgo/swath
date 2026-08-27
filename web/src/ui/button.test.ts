// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { SwathButton } from "./button.js";

beforeAll(() => {
  SwathButton.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(html: string): Promise<SwathButton> {
  const host = document.createElement("div");
  host.innerHTML = html;
  document.body.append(host);
  const button = host.querySelector("swath-button");
  if (!button) {
    throw new Error("no button");
  }
  await button.updateComplete;
  return button;
}

const inner = (button: SwathButton): HTMLButtonElement | null =>
  button.shadowRoot?.querySelector("button") ?? null;

test("renders a real <button part=base> with label and icon parts; slot carries the text", async () => {
  const button = await mount('<swath-button icon="share" label="Share">share</swath-button>');
  const base = inner(button);
  expect(base?.getAttribute("part")).toBe("base");
  expect(base?.type).toBe("button");
  expect(base?.getAttribute("aria-label")).toBe("Share");
  expect(base?.querySelector('[part="label"] slot')).not.toBeNull();
  expect(base?.querySelector("swath-icon")?.name).toBe("share");
  expect(button.textContent?.trim()).toBe("share");
});

test("focus delegates to the control; Enter/Space activate it like a native button", async () => {
  const button = await mount("<swath-button>go</swath-button>");
  const clicks = vi.fn();
  button.addEventListener("click", clicks);
  button.focus();
  expect(button.shadowRoot?.activeElement).toBe(inner(button));
  inner(button)?.click();
  expect(clicks).toHaveBeenCalledTimes(1);
  expect(button.matches(":focus-within")).toBe(true);
});

test("pressed: aria-pressed reflects, activation flips it and emits swath-toggle", async () => {
  const button = await mount("<swath-button pressed>x-ray</swath-button>");
  expect(inner(button)?.getAttribute("aria-pressed")).toBe("true");
  const toggles: boolean[] = [];
  document.body.addEventListener("swath-toggle", (event) => toggles.push(event.detail.pressed));
  inner(button)?.click();
  await button.updateComplete;
  expect(button.pressed).toBe(false);
  expect(button.hasAttribute("pressed")).toBe(false);
  expect(inner(button)?.getAttribute("aria-pressed")).toBe("false");
  expect(toggles).toEqual([false]);
});

test('pressed="false" is an unpressed toggle button (aria-style), not a plain one', async () => {
  const button = await mount('<swath-button pressed="false">x-ray</swath-button>');
  expect(button.pressed).toBe(false);
  expect(inner(button)?.getAttribute("aria-pressed")).toBe("false");
  inner(button)?.click();
  await button.updateComplete;
  expect(inner(button)?.getAttribute("aria-pressed")).toBe("true");
});

test("a plain button never carries aria-pressed", async () => {
  const button = await mount("<swath-button>plain</swath-button>");
  expect(inner(button)?.hasAttribute("aria-pressed")).toBe(false);
});

test("disabled reflects to the control and blocks activation", async () => {
  const button = await mount("<swath-button disabled>share</swath-button>");
  expect(inner(button)?.disabled).toBe(true);
  button.disabled = false;
  await button.updateComplete;
  expect(inner(button)?.disabled).toBe(false);
  expect(button.hasAttribute("disabled")).toBe(false);
});

test("icon-only without a label is a programming error, loudly", async () => {
  const host = document.createElement("div");
  host.innerHTML = '<swath-button icon="close"></swath-button>';
  document.body.append(host);
  const button = host.querySelector("swath-button");
  await expect(button?.updateComplete).rejects.toThrow(/needs a label/);
});

test("href renders an <a part=base> instead", async () => {
  const button = await mount('<swath-button href="/docs">docs</swath-button>');
  const anchor = button.shadowRoot?.querySelector("a");
  expect(anchor?.getAttribute("part")).toBe("base");
  expect(anchor?.getAttribute("href")).toBe("/docs");
  expect(inner(button)).toBeNull();
});

test("the sheet pins a ≥44px hit target on coarse pointers (the media block cannot be flipped in-page)", () => {
  const rules = [...(SwathButton.styles[0]?.cssRules ?? [])];
  const coarse = rules.find(
    (rule): rule is CSSMediaRule =>
      rule instanceof CSSMediaRule && rule.media.mediaText === "(pointer: coarse)",
  );
  expect(coarse?.cssRules[0]?.cssText).toContain("var(--swath-size-target)");
});
