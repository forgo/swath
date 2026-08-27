// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { SwathToggle } from "./toggle.js";

beforeAll(() => {
  SwathToggle.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(html: string): Promise<{ form: HTMLFormElement; toggle: SwathToggle }> {
  const form = document.createElement("form");
  form.innerHTML = html;
  document.body.append(form);
  const toggle = form.querySelector("swath-toggle");
  if (!toggle) {
    throw new Error("no toggle");
  }
  await toggle.updateComplete;
  return { form, toggle };
}

const control = (toggle: SwathToggle): HTMLButtonElement | null =>
  toggle.shadowRoot?.querySelector('[part="control"]') ?? null;

test("renders role=switch with the parts and label; the parts vocabulary is the contract", async () => {
  const { toggle } = await mount('<swath-toggle name="eye" label="Visible"></swath-toggle>');
  const button = control(toggle);
  expect(button?.getAttribute("role")).toBe("switch");
  expect(button?.getAttribute("aria-checked")).toBe("false");
  expect(button?.getAttribute("aria-label")).toBe("Visible");
  for (const part of ["base", "control", "track", "thumb"]) {
    expect(toggle.shadowRoot?.querySelector(`[part="${part}"]`), part).not.toBeNull();
  }
});

test("a click (what Space does on the native button) flips checked, emits swath-change once", async () => {
  const { toggle } = await mount('<swath-toggle name="eye" label="Visible"></swath-toggle>');
  const seen: { name: string; value: unknown }[] = [];
  document.body.addEventListener("swath-change", (event) => seen.push(event.detail));
  control(toggle)?.click();
  await toggle.updateComplete;
  expect(toggle.checked).toBe(true);
  expect(toggle.hasAttribute("checked")).toBe(true);
  expect(control(toggle)?.getAttribute("aria-checked")).toBe("true");
  expect(seen).toEqual([{ name: "eye", value: true }]);
  // Programmatic changes are not user changes: no event.
  toggle.checked = false;
  await toggle.updateComplete;
  expect(seen).toHaveLength(1);
});

test("form-associated: name/checked become the form value, reset clears, fieldset disables", async () => {
  const { form, toggle } = await mount(
    '<fieldset><swath-toggle name="live" label="Live" checked></swath-toggle></fieldset>',
  );
  expect(new FormData(form).get("live")).toBe("on");
  control(toggle)?.click();
  await toggle.updateComplete;
  expect(new FormData(form).get("live")).toBeNull();
  toggle.checked = true;
  await toggle.updateComplete;
  form.reset();
  await toggle.updateComplete;
  expect(toggle.checked).toBe(false);
  form.querySelector("fieldset")?.setAttribute("disabled", "");
  await toggle.updateComplete;
  expect(toggle.disabled).toBe(true);
  expect(control(toggle)?.disabled).toBe(true);
});

test("focus delegates to the switch control", async () => {
  const { toggle } = await mount('<swath-toggle label="Visible"></swath-toggle>');
  toggle.focus();
  expect(toggle.shadowRoot?.activeElement).toBe(control(toggle));
});

test("the sheet pins a ≥44px hit target on coarse pointers", () => {
  const rules = [...(SwathToggle.styles[0]?.cssRules ?? [])];
  const coarse = rules.find(
    (rule): rule is CSSMediaRule =>
      rule instanceof CSSMediaRule && rule.media.mediaText === "(pointer: coarse)",
  );
  expect(coarse?.cssRules[0]?.cssText).toContain("var(--swath-size-target)");
});
