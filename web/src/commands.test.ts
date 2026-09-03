// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test, vi } from "vitest";
import { buildCommands, type CommandContext } from "./commands.js";
import { matchCommands } from "./ui/command-model.js";

function context(overrides: Partial<CommandContext> = {}): CommandContext {
  return {
    layers: [
      { id: "truecolor", title: "HLS true color" },
      { id: "ndvi", title: "HLS NDVI" },
    ],
    activeLayer: "truecolor",
    mode: "layers",
    xray: false,
    compareAvailable: true,
    setLayer: vi.fn(),
    setMode: vi.fn(),
    toggleXray: vi.fn(),
    toggleCompare: vi.fn(),
    zoomToData: vi.fn(),
    share: vi.fn(),
    zoomToGranule: vi.fn(),
    ...overrides,
  };
}

test("layers, the other modes, map toggles and share; 'ndvi' ranks the NDVI layer first and runs setLayer", () => {
  const ctx = context();
  const commands = buildCommands(ctx);
  expect(commands.map((c) => c.id)).toEqual([
    "layer:truecolor",
    "layer:ndvi",
    "mode:data",
    "mode:author",
    "mode:xray",
    "xray",
    "compare",
    "zoom-to-data",
    "share",
  ]);
  const top = matchCommands(commands, "ndvi")[0];
  expect(top?.command.id).toBe("layer:ndvi");
  top?.command.run();
  expect(ctx.setLayer).toHaveBeenCalledWith("ndvi");
  expect(commands.find((c) => c.id === "layer:truecolor")?.hint).toBe("truecolor · viewing");
});

test("x-ray label follows state; compare hidden when unavailable; granule jumps only in Data mode", () => {
  const off = buildCommands(context({ xray: true, compareAvailable: false }));
  expect(off.find((c) => c.id === "xray")?.label).toBe("Turn the x-ray overlay off");
  expect(off.some((c) => c.id === "compare")).toBe(false);
  const granule = {
    id: "FIX.A.2026",
    bbox: [10, 45, 11, 46] as const,
    datetime: "2026-06-01T10:00:00Z",
  };
  const ctx = context({ mode: "data", granules: [{ dataset: "hls-s30", granule }] });
  const data = buildCommands(ctx);
  expect(data.some((c) => c.id === "mode:data")).toBe(false);
  const jump = data.find((c) => c.id === "granule:hls-s30/FIX.A.2026");
  expect(jump?.hint).toBe("2026-06-01");
  jump?.run();
  expect(ctx.zoomToGranule).toHaveBeenCalledWith("hls-s30", granule);
  expect(
    buildCommands(context({ granules: [{ dataset: "hls-s30", granule }] })).some((c) =>
      c.id.startsWith("granule:"),
    ),
  ).toBe(false);
});

test("every command label is sentence case — the content fundamentals, enforced", () => {
  // design-language.md §2: menu items and prose are sentence case; Title Case
  // appears nowhere; `Swath` is the one capitalised word. The palette is the
  // product's largest single catalog of user-facing labels, so it is where a
  // regression would land first (#391).
  const labels = buildCommands(context()).map((c) => c.label);
  expect(labels.length).toBeGreaterThan(5);
  for (const label of labels) {
    // Everything after a `: ` is server or user data (a layer title, a granule
    // id) and is not ours to case.
    const ours = label.split(": ")[0] as string;
    const words = ours.split(" ").slice(1);
    const titleCased = words.filter(
      (w) => /^[A-Z][a-z]/.test(w) && w !== "Swath" && !/^X-/.test(w),
    );
    expect(titleCased, `"${label}" is Title Case, not sentence case`).toEqual([]);
  }
});

test("no command promises something the suite cannot assert", () => {
  // design-language.md §3: no adjective the test suite cannot assert. "Fast",
  // "instantly", "seamless" are claims; a number with a unit is a measurement.
  const banned =
    /\b(fast|instant|instantly|blazing|lightning|seamless|effortless|powerful|simple|easy|quick|quickly|smooth|smoothly)\b/i;
  for (const command of buildCommands(context())) {
    expect(banned.test(command.label), `"${command.label}" makes a claim`).toBe(false);
    expect(banned.test(command.hint ?? ""), `"${command.hint}" makes a claim`).toBe(false);
  }
});
