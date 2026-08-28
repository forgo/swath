// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { userEvent } from "vitest/browser";
import { SwathField } from "./field.js";

beforeAll(() => {
  SwathField.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(html: string): Promise<{ form: HTMLFormElement; field: SwathField }> {
  const form = document.createElement("form");
  form.innerHTML = html;
  document.body.append(form);
  const field = form.querySelector("swath-field");
  if (!field) {
    throw new Error("no field");
  }
  await field.updateComplete;
  return { form, field };
}

const control = <T extends HTMLElement>(field: SwathField): T =>
  field.shadowRoot?.querySelector('[part="control"]') as T;

test("text: label, help, placeholder; typing emits swath-input live, blur commits swath-change", async () => {
  const { form, field } = await mount(
    '<swath-field name="id" label="Dataset id" help="letters, digits, - and _" placeholder="hls-demo"></swath-field>',
  );
  const input = control<HTMLInputElement>(field);
  expect(input.type).toBe("text");
  expect(input.placeholder).toBe("hls-demo");
  expect(field.shadowRoot?.querySelector('[part="label"]')?.textContent).toBe("Dataset id");
  expect(field.shadowRoot?.querySelector('[part="help"]')?.textContent).toContain("letters");
  expect(input.getAttribute("aria-describedby")).toBe("help");
  const live: string[] = [];
  const committed: string[] = [];
  field.addEventListener("swath-input", (e) => live.push(String(e.detail.value)));
  field.addEventListener("swath-change", (e) => committed.push(String(e.detail.value)));
  field.focus();
  await userEvent.keyboard("ab");
  expect(live).toEqual(["a", "ab"]);
  expect(field.value).toBe("ab");
  input.blur();
  expect(committed).toEqual(["ab"]);
  expect(new FormData(form).get("id")).toBe("ab");
});

test("native validity flows to the host; a server `error` is a custom error that clears on input", async () => {
  const { form, field } = await mount(
    '<swath-field name="n" type="number" required></swath-field>',
  );
  expect(field.checkValidity()).toBe(false);
  expect(field.validity.valueMissing).toBe(true);
  expect(form.checkValidity()).toBe(false);
  field.value = "3";
  await field.updateComplete;
  expect(field.checkValidity()).toBe(true);
  field.error = "declared bands differ";
  await field.updateComplete;
  expect(field.validity.customError).toBe(true);
  expect(field.validationMessage).toBe("declared bands differ");
  expect(field.shadowRoot?.querySelector('[part="error"]')?.textContent).toBe(
    "declared bands differ",
  );
  expect(control(field).getAttribute("aria-describedby")).toBe("error help");
  field.focus();
  await userEvent.keyboard("4");
  await field.updateComplete;
  expect(field.error).toBeUndefined();
  expect(field.checkValidity()).toBe(true);
});

test("select renders options from the property and commits the chosen value", async () => {
  const { form, field } = await mount(
    '<swath-field name="band" type="select" label="Band"></swath-field>',
  );
  field.options = [
    { value: "b04", label: "Red" },
    { value: "b08", label: "NIR" },
  ];
  field.value = "b08";
  await field.updateComplete;
  const select = control<HTMLSelectElement>(field);
  expect([...select.options].map((o) => o.value)).toEqual(["b04", "b08"]);
  expect(select.value).toBe("b08");
  expect(new FormData(form).get("band")).toBe("b08");
  await userEvent.selectOptions(select, "b04");
  expect(field.value).toBe("b04");
});

test("checkbox: checked ↔ form value 'on'; textarea and file take their native controls", async () => {
  const { form, field } = await mount(
    '<swath-field name="live" type="checkbox" label="Live"></swath-field>',
  );
  expect(new FormData(form).get("live")).toBeNull();
  control<HTMLInputElement>(field).click();
  await field.updateComplete;
  expect(field.checked).toBe(true);
  expect(new FormData(form).get("live")).toBe("on");

  const { field: area } = await mount('<swath-field name="notes" type="textarea"></swath-field>');
  expect(control(area).tagName).toBe("TEXTAREA");
  const { field: file } = await mount('<swath-field name="cog" type="file"></swath-field>');
  expect(control<HTMLInputElement>(file).type).toBe("file");
  expect(file.files?.length).toBe(0);
});

test("form reset clears value, checked and error; a disabled fieldset disables the field", async () => {
  const { form, field } = await mount(
    '<fieldset><swath-field name="id" value="x" error="bad"></swath-field></fieldset>',
  );
  form.reset();
  await field.updateComplete;
  expect(field.value).toBeUndefined();
  expect(field.error).toBeUndefined();
  expect(control<HTMLInputElement>(field).value).toBe("");
  form.querySelector("fieldset")?.setAttribute("disabled", "");
  await field.updateComplete;
  expect(field.disabled).toBe(true);
  expect(control<HTMLInputElement>(field).disabled).toBe(true);
});

test("readonly reflects to the control (a STAC-named dataset id is not editable)", async () => {
  const { field } = await mount('<swath-field name="id" value="hls-demo" readonly></swath-field>');
  expect(control<HTMLInputElement>(field).readOnly).toBe(true);
  field.readonly = false;
  await field.updateComplete;
  expect(control<HTMLInputElement>(field).readOnly).toBe(false);
});
