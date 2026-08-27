// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import { el } from "./dom.js";

test("el() sets attributes by kind and appends only real children", () => {
  const node = el(
    "button",
    { type: "button", part: "base", disabled: true, hidden: false, title: null, tabindex: 0 },
    "label ",
    el("span", {}, "inner"),
    null,
    undefined,
    false,
  );
  expect(node.tagName).toBe("BUTTON");
  expect(node.getAttribute("type")).toBe("button");
  expect(node.getAttribute("part")).toBe("base");
  expect(node.hasAttribute("disabled")).toBe(true);
  expect(node.hasAttribute("hidden")).toBe(false);
  expect(node.hasAttribute("title")).toBe(false);
  expect(node.getAttribute("tabindex")).toBe("0");
  expect(node.childNodes).toHaveLength(2);
  expect(node.textContent).toBe("label inner");
});
