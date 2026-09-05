// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The first-run action (#418): register the committed fixture stack, so
 * a new operator's first minute ends with pixels on the map rather than
 * with a form.
 *
 * This is what `docs/deploy/seed.sh` does by hand, said once here. The
 * assets are the COGs the deployment already carries — the action
 * registers **rows**, never bytes, and never invents a granule the store
 * does not have.
 *
 * # Idempotent, and it says what it did
 *
 * `POST /datasets` answers 409 when the id exists; that is "already
 * registered", not a failure. Granules upsert. So running it twice
 * leaves the same catalog and the second report says `already there`
 * rather than claiming a fresh registration.
 */
import type { SwathApi } from "./api.js";
import { ApiProblem } from "./api.js";

/** One fixture granule: the id its assets are named for, its footprint
 * and its acquisition instant. */
interface FixtureGranule {
  id: string;
  bbox: [number, number, number, number];
  datetime: string;
  bands: readonly string[];
}

interface FixtureDataset {
  id: string;
  title: string;
  bands: readonly string[];
  granules: FixtureGranule[];
}

const HLS_BBOX: [number, number, number, number] = [-105.537, 39.1954, -105.3581, 39.3345];
const FIRE_BBOX: [number, number, number, number] = [-121.7388, 39.9856, -121.6475, 40.0559];
const HLS_BANDS = ["b02", "b03", "b04", "b8a", "fmask"] as const;
const FIRE_BANDS = ["b04", "b12", "b8a", "fmask"] as const;

/** The six Park Fire acquisitions the fixtures carry, day-of-year to
 * sensing instant (`tests/fixtures/README.md`). */
const FIRE_DAYS: readonly (readonly [string, string])[] = [
  ["2024159", "2024-06-07T19:03:00Z"],
  ["2024204", "2024-07-22T19:03:00Z"],
  ["2024229", "2024-08-16T19:03:00Z"],
  ["2024249", "2024-09-05T19:03:00Z"],
  ["2024274", "2024-09-30T19:03:00Z"],
  ["2024289", "2024-10-15T19:03:00Z"],
];

/** The committed stack: one single-date HLS scene, and the fire series. */
export const FIXTURE_STACK: readonly FixtureDataset[] = [
  {
    id: "hls-s30",
    title: "HLS Sentinel-2 (S30)",
    bands: HLS_BANDS,
    granules: [
      {
        id: "hlss30-t13sdd-2024158",
        bbox: HLS_BBOX,
        datetime: "2024-06-06T17:54:00Z",
        bands: HLS_BANDS,
      },
    ],
  },
  {
    id: "hls-s30-fire",
    title: "HLS S30 Park Fire series",
    bands: FIRE_BANDS,
    granules: FIRE_DAYS.map(([day, datetime]) => ({
      id: `hlss30-t10tfk-${day}`,
      bbox: FIRE_BBOX,
      datetime,
      bands: FIRE_BANDS,
    })),
  },
];

/** What one registration did. */
export interface FixtureOutcome {
  dataset: string;
  /** Granules registered by this run. */
  granules: number;
  /** True when the dataset was already registered — an idempotent
   * re-run, not a failure. */
  alreadyThere: boolean;
}

export interface FixtureReport {
  outcomes: FixtureOutcome[];
  /** The first refusal, in the server's own words. Present means the run
   * stopped there; what came before it still happened. */
  error?: string;
}

/** The asset map for `granule`: bare fixture file names, which the
 * deployment's store root resolves. */
function assetsOf(granule: FixtureGranule): Record<string, { href: string }> {
  const assets: Record<string, { href: string }> = {};
  for (const band of granule.bands) {
    assets[band] = { href: `${granule.id}-${band}.tif` };
  }
  return assets;
}

/**
 * Registers the stack. Returns what it did, in the server's words where
 * something refused — the caller renders the report and never invents a
 * count.
 */
export async function loadFixtureStack(api: SwathApi): Promise<FixtureReport> {
  const outcomes: FixtureOutcome[] = [];
  for (const dataset of FIXTURE_STACK) {
    let alreadyThere = false;
    try {
      await api.json("/datasets", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          id: dataset.id,
          title: dataset.title,
          bands: [...dataset.bands],
        }),
      });
    } catch (error) {
      // 409 is "already registered", which is the whole point of running
      // this twice being safe. Anything else is a real refusal.
      if (error instanceof ApiProblem && error.status === 409) {
        alreadyThere = true;
      } else {
        return {
          outcomes,
          error: error instanceof Error ? error.message : String(error),
        };
      }
    }

    let granules = 0;
    for (const granule of dataset.granules) {
      try {
        await api.json(`/datasets/${encodeURIComponent(dataset.id)}/granules`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            id: granule.id,
            datetime: granule.datetime,
            bbox: granule.bbox,
            assets: assetsOf(granule),
          }),
        });
        granules += 1;
      } catch (error) {
        return {
          outcomes: [...outcomes, { dataset: dataset.id, granules, alreadyThere }],
          error: error instanceof Error ? error.message : String(error),
        };
      }
    }
    outcomes.push({ dataset: dataset.id, granules, alreadyThere });
  }
  return { outcomes };
}

/** The report in plain words: what was registered, and what was already
 * there. Never a total the run did not perform. */
export function describeReport(report: FixtureReport): string {
  if (report.outcomes.length === 0) {
    return report.error ?? "Nothing was registered.";
  }
  const parts = report.outcomes.map((outcome) => {
    const noun = outcome.granules === 1 ? "granule" : "granules";
    const already = outcome.alreadyThere ? " (already there)" : "";
    return `${outcome.dataset}: ${outcome.granules} ${noun}${already}`;
  });
  const done = `Registered ${parts.join(", ")}.`;
  return report.error === undefined ? done : `${done} Then: ${report.error}`;
}
