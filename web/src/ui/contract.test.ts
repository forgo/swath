// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The primitive CONTRACT (ui-system.md §8, ADR 0026). M12 changes values,
// glyphs and themes; it does not change what an organism can address. This
// pins that: the part vocabulary and the observed attributes of every
// primitive, and the event-name set of the catalog they all dispatch through.
//
// It replaces the freeze's path fence, which claimed a CI check that did not
// exist and which forbade the very edits four M12 tasks needed — moving a
// literal out of a primitive and into a token. Asserting the contract is
// strictly stronger: it fails wherever a structural change is written, not
// only in an enumerated set of files.
//
// Changing a row here is a contract change. Do it in the same PR as the
// reason, and say what consumes the new part.
import { expect, test } from "vitest";
import { SwathButton } from "./button.js";
import buttonSource from "./button.ts?raw";
import { SwathCard } from "./card.js";
import cardSource from "./card.ts?raw";
import { SwathCommandPalette } from "./command-palette.js";
import commandPaletteSource from "./command-palette.ts?raw";
import { SwathDrawer } from "./drawer.js";
import drawerSource from "./drawer.ts?raw";
import type { SwathElement } from "./element.js";
import eventsSource from "./events.ts?raw";
import { SwathField } from "./field.js";
import fieldSource from "./field.ts?raw";
import { SwathGranuleCard } from "./granule-card.js";
import granuleCardSource from "./granule-card.ts?raw";
import { SwathHudCard } from "./hud-card.js";
import hudCardSource from "./hud-card.ts?raw";
import { SwathHudDock } from "./hud-dock.js";
import hudDockSource from "./hud-dock.ts?raw";
import { SwathIcon } from "./icon.js";
import iconSource from "./icon.ts?raw";
import { SwathLayerItem } from "./layer-item.js";
import layerItemSource from "./layer-item.ts?raw";
import { SwathMenu } from "./menu.js";
import menuSource from "./menu.ts?raw";
import { SwathRail } from "./rail.js";
import railSource from "./rail.ts?raw";
import { SwathSlider } from "./slider.js";
import sliderSource from "./slider.ts?raw";
import { SwathStatusBar, SwathStatusCell } from "./status-bar.js";
import statusBarSource from "./status-bar.ts?raw";
import { SwathToggle } from "./toggle.js";
import toggleSource from "./toggle.ts?raw";

type Primitive = typeof SwathElement & { tagName: string };

/** Every part name a module writes, wherever it writes it — the CSS and the
 * render code are scanned together, so a part that only appears in one state
 * still counts. */
function partsOf(source: string): string[] {
  return [
    ...new Set([...source.matchAll(/part="([a-z0-9-]+)"/g)].map((m) => m[1] as string)),
  ].sort();
}

const CONTRACT: readonly [Primitive, string[], string[]][] = [
  [
    SwathButton,
    ["base", "icon", "label"],
    ["disabled", "href", "icon", "label", "pressed", "size", "variant"],
  ],
  [
    SwathCard,
    ["base", "body", "footer", "header", "media"],
    ["dense", "interactive", "selected", "title"],
  ],
  [
    SwathCommandPalette,
    ["base", "empty", "group", "hint", "input", "item", "list"],
    ["label", "open", "presentation"],
  ],
  [
    SwathDrawer,
    ["base", "body", "footer", "handle", "header", "scrim"],
    ["edge", "label", "modal", "open", "presentation", "resizable", "size", "snap"],
  ],
  [
    SwathField,
    ["base", "control", "error", "help", "label"],
    [
      "disabled",
      "error",
      "help",
      "label",
      "name",
      "placeholder",
      "readonly",
      "required",
      "type",
      "value",
    ],
  ],
  [
    SwathGranuleCard,
    ["media", "meta", "note", "pending", "title"],
    ["dataset-id", "datetime", "granule-id", "kind", "layout", "note", "selected", "thumbnail"],
  ],
  [
    SwathHudCard,
    ["base", "body", "header", "title"],
    ["auto-hide", "collapsed", "collapsible", "dense", "title"],
  ],
  [SwathHudDock, ["base", "corner"], ["collapsed"]],
  [SwathIcon, ["svg"], ["label", "name", "size"]],
  [
    SwathLayerItem,
    ["base", "info", "meta", "opacity", "row", "title"],
    ["active", "expanded", "href", "kind", "layer-id", "meta", "opacity", "title", "visible"],
  ],
  [SwathMenu, ["item", "list", "trigger"], ["label", "open", "presentation"]],
  [
    SwathRail,
    ["base", "brand", "collapse", "content", "footer", "item", "nav"],
    ["collapsed", "mode"],
  ],
  [
    SwathSlider,
    ["base", "control", "value"],
    ["disabled", "label", "max", "min", "name", "step", "value"],
  ],
  [SwathStatusBar, ["base", "label", "value"], ["chip"]],
  [SwathStatusCell, ["base", "label", "value"], ["label", "mono", "value"]],
  [SwathToggle, ["base", "control", "thumb", "track"], ["checked", "disabled", "label", "name"]],
];

test("every primitive is in the contract table", () => {
  // A new primitive with no row is a structural change nobody reviewed.
  const tags = CONTRACT.map(([cls]) => cls.tagName);
  expect(new Set(tags).size, "duplicate rows").toBe(tags.length);
  expect(tags.length).toBeGreaterThanOrEqual(15);
});

const SOURCES: Record<string, string> = {
  "swath-button": buttonSource,
  "swath-card": cardSource,
  "swath-command-palette": commandPaletteSource,
  "swath-drawer": drawerSource,
  "swath-field": fieldSource,
  "swath-granule-card": granuleCardSource,
  "swath-hud-card": hudCardSource,
  "swath-hud-dock": hudDockSource,
  "swath-icon": iconSource,
  "swath-layer-item": layerItemSource,
  "swath-menu": menuSource,
  "swath-rail": railSource,
  "swath-slider": sliderSource,
  // One module, two elements: the bar and its cell.
  "swath-status-bar": statusBarSource,
  "swath-status-cell": statusBarSource,
  "swath-toggle": toggleSource,
};

test("every primitive in the contract has a source to check", () => {
  // A row whose module is missing would make the next test vacuous.
  for (const [cls] of CONTRACT) {
    expect(SOURCES[cls.tagName], `no source registered for ${cls.tagName}`).toBeTruthy();
  }
});

test("the part vocabulary of each primitive is what the contract says", () => {
  for (const [cls, parts] of CONTRACT) {
    const found = partsOf(SOURCES[cls.tagName] as string);
    expect(found.length, `${cls.tagName} exposes no parts`).toBeGreaterThan(0);
    for (const part of parts) {
      expect(found, `${cls.tagName} must still expose part="${part}"`).toContain(part);
    }
  }
});

test("observed attributes are the contract's, where the contract names them", () => {
  for (const [cls, , attributes] of CONTRACT) {
    if (attributes.length === 0) {
      continue;
    }
    expect([...cls.observedAttributes].sort(), cls.tagName).toEqual([...attributes].sort());
  }
});

test("the event catalog's names are the contract", () => {
  // Every Swath event is dispatched through events.ts (the DRY gate keeps custom-event
  // construction there), so SwathEventMap's key set IS the event
  // surface. Read at brace depth 1 inside the interface, so a nested detail
  // field is not mistaken for an event name.
  const open = eventsSource.indexOf("export interface SwathEventMap {");
  expect(open, "SwathEventMap must exist").toBeGreaterThan(-1);
  const names: string[] = [];
  let depth = 0;
  for (const line of eventsSource.slice(open).split("\n")) {
    const key = /^\s{2}"?([a-z][a-z0-9-]*)"?\??:/.exec(line);
    if (depth === 1 && key?.[1]) {
      names.push(key[1]);
    }
    depth += (line.match(/\{/g)?.length ?? 0) - (line.match(/\}/g)?.length ?? 0);
    if (depth === 0 && names.length > 0) {
      break;
    }
  }
  const unique = [...new Set(names)].sort();
  expect(unique.length, "the catalog must not shrink").toBeGreaterThanOrEqual(20);
  // Names are `swath-<subject>-<verb>` going forward; the M5-M9 names are
  // grandfathered by this list and must not grow.
  expect(unique.filter((n) => !n.startsWith("swath-"))).toEqual(["layerchange"]);
});
