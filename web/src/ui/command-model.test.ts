// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import { type Command, matchCommands, subsequenceScore } from "./command-model.js";

const noop = (): void => undefined;
const COMMANDS: Command[] = [
  { id: "layer:ndvi", label: "Show layer: HLS NDVI", hint: "ndvi", group: "Layers", run: noop },
  {
    id: "layer:truecolor",
    label: "Show layer: HLS true color",
    hint: "truecolor",
    group: "Layers",
    run: noop,
  },
  { id: "zoom", label: "Zoom to data", group: "Map", keywords: ["fit", "frame"], run: noop },
  { id: "mode:data", label: "Data mode", hint: "view=data", group: "Modes", run: noop },
  { id: "share", label: "Copy a link to this view", hint: "Share", group: "Share", run: noop },
];

test("subsequenceScore: in-order characters match; word starts and adjacency score higher; no match → undefined", () => {
  expect(subsequenceScore("ndvi", "Show layer: HLS NDVI")?.positions).toEqual([16, 17, 18, 19]);
  expect(subsequenceScore("sl", "Show layer")?.score).toBeGreaterThan(
    subsequenceScore("hw", "Show layer")?.score ?? 0,
  );
  expect(subsequenceScore("xyz", "Show layer")).toBeUndefined();
  expect(subsequenceScore("", "anything")).toEqual({ score: 0, positions: [] });
});

test("matchCommands: 'ndvi' puts the NDVI layer first; keywords match; empty query lists all in order", () => {
  expect(matchCommands(COMMANDS, "ndvi")[0]?.command.id).toBe("layer:ndvi");
  expect(matchCommands(COMMANDS, "fit").map((m) => m.command.id)).toEqual(["zoom"]);
  expect(matchCommands(COMMANDS, "").map((m) => m.command.id)).toEqual(COMMANDS.map((c) => c.id));
  expect(matchCommands(COMMANDS, "share")[0]?.command.id).toBe("share");
  expect(matchCommands(COMMANDS, "qqq")).toEqual([]);
  // Label matches outrank hint/keyword matches for the same query.
  const tc = matchCommands(COMMANDS, "true");
  expect(tc[0]?.command.id).toBe("layer:truecolor");
  expect(tc[0]?.positions.length).toBe(4);
});
