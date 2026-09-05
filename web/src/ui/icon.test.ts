// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The icon NAME list is the M10 contract (ui-system.md §4.6 / §8): M12 may
// redraw every glyph; renaming or dropping one fails here.
import { afterEach, beforeAll, expect, test } from "vitest";
import { iconNames, SwathIcon } from "./icon.js";
import iconSource from "./icon.ts?raw";

const NAMES = [
  "layers",
  "data",
  "author",
  "xray",
  "sources",
  "eye",
  "eye-off",
  "more",
  "share",
  "link",
  "close",
  "chevron-down",
  "chevron-right",
  "chevron-left",
  "search",
  "plus",
  "minus",
  "play",
  "pause",
  "compare",
  "fit",
  "check",
  "warning",
  "info",
  "menu",
  "upload",
  "trash",
  "drag",
  "clock",
  "command",
];

beforeAll(() => {
  SwathIcon.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

test("the sheet defines exactly the contract's icon names", () => {
  expect([...iconNames()].sort()).toEqual([...NAMES].sort());
});

// Two names for one drawing is a UI that cannot say two things (#384):
// `share` and `link` shipped byte-identical path data, so the rail's
// "copy a deep link to this view" and a tileset's "here is a reference"
// were the same chain glyph. A closed set of 30 cannot afford a duplicate.
test("no two icons are the same drawing", async () => {
  const drawings = new Map<string, string[]>();
  for (const name of iconNames()) {
    const icon = document.createElement("swath-icon");
    icon.name = name;
    document.body.append(icon);
    await icon.updateComplete;
    const drawn = icon.shadowRoot?.querySelector("svg")?.innerHTML ?? "";
    const key = drawn.replace(/\s+/g, " ").trim();
    expect(key).not.toBe("");
    drawings.set(key, [...(drawings.get(key) ?? []), name]);
  }
  const shared = [...drawings.values()].filter((names) => names.length > 1);
  expect(shared).toEqual([]);
});

test("clones the symbol into its own root, drawn in currentColor at the icon size", async () => {
  const icon = document.createElement("swath-icon");
  icon.name = "eye";
  icon.style.color = "rgb(1, 2, 3)";
  document.body.append(icon);
  await icon.updateComplete;
  const svg = icon.shadowRoot?.querySelector("svg");
  expect(svg?.getAttribute("part")).toBe("svg");
  expect(svg?.getAttribute("stroke")).toBe("currentColor");
  expect(svg?.childElementCount).toBeGreaterThan(0);
  expect(getComputedStyle(icon).width).toBe("16px");
  expect(svg && getComputedStyle(svg).stroke).toBe("rgb(1, 2, 3)");
});

test("decorative unless labelled: aria-hidden ↔ role=img + aria-label", async () => {
  const icon = document.createElement("swath-icon");
  icon.setAttribute("name", "warning");
  document.body.append(icon);
  await icon.updateComplete;
  const svg = () => icon.shadowRoot?.querySelector("svg");
  expect(svg()?.getAttribute("aria-hidden")).toBe("true");
  expect(svg()?.hasAttribute("role")).toBe(false);
  icon.label = "Warning";
  await icon.updateComplete;
  expect(svg()?.getAttribute("role")).toBe("img");
  expect(svg()?.getAttribute("aria-label")).toBe("Warning");
  expect(svg()?.hasAttribute("aria-hidden")).toBe(false);
});

test("an unknown name renders an empty frame, never throws", async () => {
  const icon = document.createElement("swath-icon");
  icon.name = "no-such-icon";
  document.body.append(icon);
  await icon.updateComplete;
  expect(icon.shadowRoot?.querySelector("svg")?.childElementCount).toBe(0);
});

test("no length literal survives in icon.ts — the sizes are tokens", () => {
  // The 12px/24px steps lived here, where no gate could see them: the DRY
  // check polices colour and font literals, not lengths (#386). Until it
  // covers lengths too (#378), this pins the one file that had them.
  const styles = /static override styles = \[([\s\S]*?)\n {2}\];/.exec(iconSource)?.[1] ?? "";
  expect(styles, "icon.ts must still declare styles").not.toBe("");
  expect(styles.match(/\b\d+(?:\.\d+)?(?:px|rem|em)\b/g)).toBeNull();
});
