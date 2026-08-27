// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { userEvent } from "vitest/browser";
import { SwathSlider } from "./slider.js";

beforeAll(() => {
  SwathSlider.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(html: string): Promise<SwathSlider> {
  const host = document.createElement("div");
  host.innerHTML = html;
  document.body.append(host);
  const slider = host.querySelector("swath-slider");
  if (!slider) {
    throw new Error("no slider");
  }
  await slider.updateComplete;
  return slider;
}

const control = (slider: SwathSlider): HTMLInputElement | null =>
  slider.shadowRoot?.querySelector('input[type="range"]') ?? null;

test("a native range with the attributes applied and a formatted readout", async () => {
  const slider = await mount(
    '<swath-slider name="opacity" label="Opacity" min="0" max="1" step="0.05" value="0.6"></swath-slider>',
  );
  slider.format = (v) => `${Math.round(v * 100)}%`;
  slider.requestUpdate();
  await slider.updateComplete;
  const input = control(slider);
  expect(input?.getAttribute("part")).toBe("control");
  expect(input?.min).toBe("0");
  expect(input?.max).toBe("1");
  expect(input?.step).toBe("0.05");
  expect(input?.valueAsNumber).toBeCloseTo(0.6);
  expect(input?.getAttribute("aria-label")).toBe("Opacity");
  expect(slider.shadowRoot?.querySelector("output")?.value).toBe("60%");
  expect(input && getComputedStyle(input).touchAction).toBe("pan-y");
});

test("keyboard: arrows step, Home/End clamp; swath-input live and swath-change on commit", async () => {
  const slider = await mount(
    '<swath-slider name="o" min="0" max="10" step="1" value="5"></swath-slider>',
  );
  const live: number[] = [];
  const committed: number[] = [];
  slider.addEventListener("swath-input", (e) => live.push(Number(e.detail.value)));
  slider.addEventListener("swath-change", (e) => committed.push(Number(e.detail.value)));
  slider.focus();
  await userEvent.keyboard("{ArrowRight}{ArrowRight}{ArrowLeft}");
  expect(slider.value).toBe(6);
  expect(live).toEqual([6, 7, 6]);
  expect(committed).toEqual([6, 7, 6]); // keyboard steps commit as they go (native)
  await userEvent.keyboard("{End}");
  expect(slider.value).toBe(10);
  await userEvent.keyboard("{Home}");
  expect(slider.value).toBe(0);
  expect(slider.getAttribute("value")).toBe("0");
});

test("a programmatic value moves the control and the readout, without events", async () => {
  const slider = await mount('<swath-slider min="0" max="100"></swath-slider>');
  let events = 0;
  slider.addEventListener("swath-input", () => events++);
  slider.value = 42;
  await slider.updateComplete;
  expect(control(slider)?.valueAsNumber).toBe(42);
  expect(slider.shadowRoot?.querySelector("output")?.value).toBe("42");
  expect(events).toBe(0);
});
