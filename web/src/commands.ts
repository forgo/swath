// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import type { ViewMode } from "./app-state.js";
/**
 * The shell's command registry (issue #292): what the palette offers, built
 * from live state each time it opens — layers, modes, the map toggles,
 * share, and in Data mode a jump to any listed granule. Pure: takes a
 * context of callbacks and returns commands; the demo supplies the
 * callbacks, tests supply fakes.
 */
import type { CatalogGranule } from "./catalog-model.js";
import type { SwathLayer } from "./swath-map.js";
import type { Command } from "./ui/command-model.js";

export interface CommandContext {
  layers: readonly SwathLayer[];
  activeLayer: string | undefined;
  mode: ViewMode;
  xray: boolean;
  compareAvailable: boolean;
  /** Granules of the dataset open in the catalog (Data mode only). */
  granules?: readonly { dataset: string; granule: CatalogGranule }[] | undefined;
  setLayer(id: string): void;
  setMode(mode: ViewMode): void;
  toggleXray(): void;
  toggleCompare(): void;
  zoomToData(): void;
  share(): void;
  zoomToGranule(dataset: string, granule: CatalogGranule): void;
}

const MODE_LABELS: Record<ViewMode, string> = {
  layers: "Layers",
  data: "Data",
  author: "Author",
  xray: "X-ray",
};

export function buildCommands(ctx: CommandContext): Command[] {
  const commands: Command[] = [];
  for (const layer of ctx.layers) {
    commands.push({
      id: `layer:${layer.id}`,
      label: `Show layer: ${layer.title}`,
      hint: layer.id === ctx.activeLayer ? `${layer.id} · viewing` : layer.id,
      group: "Layers",
      keywords: [layer.id],
      run: () => ctx.setLayer(layer.id),
    });
  }
  for (const mode of ["layers", "data", "author", "xray"] as const) {
    if (mode === ctx.mode) {
      continue;
    }
    commands.push({
      id: `mode:${mode}`,
      label: `${MODE_LABELS[mode]} mode`,
      hint: `view=${mode}`,
      group: "Modes",
      keywords: ["mode", "view", mode],
      run: () => ctx.setMode(mode),
    });
  }
  commands.push({
    id: "xray",
    label: ctx.xray ? "Turn the x-ray overlay off" : "Turn the x-ray overlay on",
    hint: "xray",
    group: "Map",
    keywords: ["overlay", "traces", "decisions"],
    run: () => ctx.toggleXray(),
  });
  if (ctx.compareAvailable) {
    commands.push({
      id: "compare",
      label: "Toggle compare swipe",
      hint: "compare",
      group: "Map",
      keywords: ["swipe", "before", "after"],
      run: () => ctx.toggleCompare(),
    });
  }
  commands.push({
    id: "zoom-to-data",
    label: "Zoom to data",
    hint: "fit the viewed layer",
    group: "Map",
    keywords: ["fit", "frame", "extent"],
    run: () => ctx.zoomToData(),
  });
  commands.push({
    id: "share",
    label: "Copy a link to this view",
    hint: "Share",
    group: "Share",
    keywords: ["copy", "link", "url"],
    run: () => ctx.share(),
  });
  if (ctx.mode === "data") {
    for (const { dataset, granule } of ctx.granules ?? []) {
      commands.push({
        id: `granule:${dataset}/${granule.id}`,
        label: `Jump to granule: ${granule.id}`,
        hint: granule.datetime.slice(0, 10) || dataset,
        group: "Data",
        keywords: [dataset, granule.datetime],
        run: () => ctx.zoomToGranule(dataset, granule),
      });
    }
  }
  return commands;
}
