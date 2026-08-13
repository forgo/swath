# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# ///
"""Support script for `just load-temporal` (issue #184) — the M7 evidence.

Measures what the M7 time track built, with the same discipline as
`tests/load/load.py` (decision-probed scenarios, pinned parameters,
committed artifact): temporal frame-serving over the six-date Park Fire
series (ADR 0015 / #223) and overview-backed tile serving through the
materialized pyramid path (#183/#218). Driven by ``tests/load/temporal.sh``
(process orchestration); this script owns parameters, HTTP loops, and the
distilled baseline.

``params``
    The pinned scenario parameters as shell exports — the single source
    of truth (rationale in PARAMS below).

``frames``
    One animation pass: for each of the six Park Fire acquisition
    instants, in chronological order, fetch every viewport tile with
    ``datetime=<instant>`` — the exact request loop the time slider
    replays. The caller runs it twice: once against a fresh tile cache
    (every frame is a Live render) and once immediately again (every
    frame is a granule-scoped cache hit). Per-request latency, status,
    and the `x-swath-trace` decision are recorded.

``overview``
    One zoom-ladder rung: N sequential requests of a single tile with the
    tile cache cleared before each, so every request exercises the RENDER
    path its zoom plans — Live at z12, overview-backed at z10/z11. The
    header decision is recorded per request; the caller additionally holds
    an SSE `/traces` capture across the rung so the overview LEVEL (the
    envelope's `{"overview":{"level":…}}`) is in the evidence too.

``report``
    Distill the raw outputs under ``target/load-temporal/`` into the
    committed baseline: ``docs/perf/temporal-baseline.json`` (+ the `.md`
    rendering). FAILS unless the decisions are exactly what each scenario
    is defined by — a wrong decision mix is invalid evidence, not a
    footnote.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import math
import os
import platform
import subprocess
import sys
import time
import urllib.request
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

# The Park Fire fixture footprint (tests/e2e/drop-fire-granules.sh bbox —
# lon/lat WGS84); every frame tile generated below lies strictly inside it.
FIRE_BBOX = (-121.7388, 39.9856, -121.6475, 40.0559)

# The six acquisition instants, chronological — the exact datetimes
# drop-fire-granules.sh stamps (tests/fixtures/README.md date table).
# `datetime=<instant>` resolves each to its own granule under
# latest_at_or_before, so the six frames are six distinct cache
# identities.
FIRE_DATES = (
    "2024-06-07T19:03:00Z",
    "2024-07-22T19:03:00Z",
    "2024-08-16T19:03:00Z",
    "2024-09-05T19:03:00Z",
    "2024-09-30T19:03:00Z",
    "2024-10-15T19:03:00Z",
)

# Pinned scenario parameters. Rationale:
#   frames — the animation viewport is every interior z14 tile of the
#            fire footprint (interior = inset one tile, so no tile clips
#            declared bounds; 9 tiles exist). z14 oversamples the 30 m
#            fixture (~7.3 m/px ground res), so every frame is a
#            full-resolution Live warp on a cold cache. c=6 mirrors a
#            browser's per-host connection budget. 9 tiles × 6 dates =
#            54 requests per pass.
#   overview — one tile per zoom rung, 24 sequential requests each
#            (enough for a stable p50 on a laptop; sequential because
#            this measures per-tile render latency, not throughput).
#            z12 1561/848 is the proven north-star tile (Live: ~30 m
#            data at ~29.6 m/px). The z11/z10 tiles are the footprint's
#            covering tiles at those zooms; z11 plans overview factor 2,
#            z10 factor 4 once materialized (factor 2 embedded-only
#            before — the pre/post contrast is the #218 pyramid path).
PARAMS: dict[str, str | int] = {
    "TEMPORAL_FRAME_ZOOM": 14,
    "TEMPORAL_FRAME_CONNS": 6,
    "TEMPORAL_FRAME_LAYER": "park-fire-ndvi",
    "TEMPORAL_OV_REPEATS": 24,
    "TEMPORAL_OV_LIVE_TILE": "/tilesets/truecolor/tiles/12/1561/848",
    "TEMPORAL_OV_Z11_TILE": "/tilesets/truecolor/tiles/11/780/424",
    "TEMPORAL_OV_Z10_TILE": "/tilesets/truecolor/tiles/10/390/212",
    "TEMPORAL_MATERIALIZE_MIN_DIM": 64,
}


def cmd_params() -> None:
    for key, value in PARAMS.items():
        print(f'export {key}="{value}"')


def tile_xy(lon: float, lat: float, zoom: int) -> tuple[int, int]:
    """Web-Mercator XYZ tile containing (lon, lat)."""
    n = 1 << zoom
    x = int((lon + 180.0) / 360.0 * n)
    y = int((1.0 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2.0 * n)
    return x, y


def frame_tile_paths() -> list[str]:
    """The animation viewport: unique interior z14 fire tiles (z/y/x)."""
    zoom = int(PARAMS["TEMPORAL_FRAME_ZOOM"])
    layer = PARAMS["TEMPORAL_FRAME_LAYER"]
    west, south, east, north = FIRE_BBOX
    x_min, y_min = tile_xy(west, north, zoom)  # y grows southward
    x_max, y_max = tile_xy(east, south, zoom)
    tiles = [
        f"/tilesets/{layer}/tiles/{zoom}/{y}/{x}"
        for y in range(y_min + 1, y_max)
        for x in range(x_min + 1, x_max)
    ]
    if len(tiles) < 4:
        sys.exit(f"FAIL: only {len(tiles)} interior z{zoom} fire tiles — no viewport")
    return tiles


def fetch(base: str, path: str) -> dict[str, Any]:
    """One GET: wall latency, status, and the x-swath-trace decision."""
    request = urllib.request.Request(base + path, method="GET")
    start = time.perf_counter()
    decision = ""
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            response.read()
            status = response.status
            header = response.headers.get("x-swath-trace", "")
    except urllib.error.HTTPError as error:
        status, header = error.code, ""
    except OSError as error:
        return {"path": path, "seconds": time.perf_counter() - start, "status": 0, "error": str(error)}
    if header:
        try:
            decision = json.loads(header).get("decision", "")
        except json.JSONDecodeError:
            decision = "unparseable"
    return {"path": path, "seconds": time.perf_counter() - start, "status": status, "decision": decision}


def cmd_frames(base: str, out: Path) -> None:
    """One slider pass: all viewport tiles, per date, chronological."""
    tiles = frame_tile_paths()
    conns = int(PARAMS["TEMPORAL_FRAME_CONNS"])
    results = []
    start = time.perf_counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=conns) as pool:
        for date in FIRE_DATES:
            paths = [f"{tile}?datetime={date}" for tile in tiles]
            for entry in pool.map(lambda path: fetch(base, path), paths):
                entry["date"] = date
                results.append(entry)
    wall = time.perf_counter() - start
    out.write_text(json.dumps({"wall_seconds": wall, "requests": results}, indent=1))
    errors = sum(1 for r in results if r["status"] != 200)
    print(
        f"frames: {len(tiles)} tiles x {len(FIRE_DATES)} dates = "
        f"{len(results)} requests in {wall:.1f}s ({errors} errors)"
    )


def clear_cache(cache: Path) -> None:
    """Delete cached tile files (files only — the server's dirs stay)."""
    for path in cache.rglob("*"):
        if path.is_file():
            path.unlink(missing_ok=True)


def cmd_overview(base: str, tile: str, cache: Path, out: Path) -> None:
    """One ladder rung: N sequential renders, cache cleared before each."""
    repeats = int(PARAMS["TEMPORAL_OV_REPEATS"])
    results = []
    start = time.perf_counter()
    for _ in range(repeats):
        clear_cache(cache)
        results.append(fetch(base, tile))
    wall = time.perf_counter() - start
    out.write_text(json.dumps({"tile": tile, "wall_seconds": wall, "requests": results}, indent=1))
    errors = sum(1 for r in results if r["status"] != 200)
    print(f"overview rung {tile}: {len(results)} renders in {wall:.1f}s ({errors} errors)")


def percentile(sorted_values: list[float], q: float) -> float:
    """Nearest-rank percentile over an ascending-sorted list."""
    rank = max(1, math.ceil(q * len(sorted_values)))
    return sorted_values[rank - 1]


def stats_ms(seconds: list[float]) -> dict[str, float]:
    ordered = sorted(seconds)
    return {
        "p50_ms": round(percentile(ordered, 0.50) * 1000, 2),
        "p95_ms": round(percentile(ordered, 0.95) * 1000, 2),
        "p99_ms": round(percentile(ordered, 0.99) * 1000, 2),
        "max_ms": round(ordered[-1] * 1000, 2),
    }


def decision_counts(requests: list[dict[str, Any]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for entry in requests:
        key = entry.get("decision") or (
            f"http_{entry['status']}" if entry["status"] != 200 else "no-trace"
        )
        counts[key] = counts.get(key, 0) + 1
    return counts


def require_decisions(name: str, counts: dict[str, int], expected: str) -> None:
    """All-or-fail: every request served the decision the scenario is
    DEFINED by, or the evidence is invalid."""
    if set(counts) != {expected}:
        sys.exit(f"FAIL: {name}: expected every decision `{expected}`, got {counts}")


def sse_overview_levels(log: Path, tile_path: str) -> dict[str, int]:
    """Envelope decisions for `tile_path` in one SSE capture, with the
    overview LEVEL the header decision cannot carry (`overview:<factor>`)."""
    _, _, layer, _, z, y, x = tile_path.split("?")[0].split("/")
    tile_xyz = f"{z}/{x}/{y}"  # envelope order is z/x/y
    counts: dict[str, int] = {}
    for raw in log.read_bytes().splitlines():
        if not raw.startswith(b"data:"):
            continue
        try:
            envelope = json.loads(raw[len(b"data:"):])
        except json.JSONDecodeError:
            continue
        if envelope.get("tile") != tile_xyz or envelope.get("layer") != layer:
            continue
        decision = envelope["trace"]["decision"]
        if isinstance(decision, dict):
            if "overview" in decision:
                key = f"overview:{decision['overview']['level']}"
            else:
                key = next(iter(decision))
        else:
            key = decision
        counts[key] = counts.get(key, 0) + 1
    return counts


def distill_frames(path: Path, phase: str) -> dict[str, Any]:
    data = json.loads(path.read_text())
    requests = data["requests"]
    decisions = decision_counts(requests)
    expected = "live" if phase == "cold" else "cache_hit"
    require_decisions(f"frames_{phase}", decisions, expected)
    tiles = len(frame_tile_paths())
    return {
        "tool": "python-urllib (tests/load/temporal.py frames)",
        "params": (
            f"{tiles} interior z{PARAMS['TEMPORAL_FRAME_ZOOM']} "
            f"{PARAMS['TEMPORAL_FRAME_LAYER']} tiles x {len(FIRE_DATES)} dated frames "
            f"(datetime=<acquisition instant>), chronological, "
            f"c={PARAMS['TEMPORAL_FRAME_CONNS']}, "
            + ("fresh tile cache" if phase == "cold" else "same loop again, cache warm")
        ),
        "requests": len(requests),
        "errors": sum(1 for entry in requests if entry["status"] != 200),
        "rps": round(len(requests) / data["wall_seconds"], 1),
        **stats_ms([entry["seconds"] for entry in requests]),
        "decisions": decisions,
    }


def distill_overview(
    directory: Path, name: str, expected_header: str, expected_sse: str
) -> dict[str, Any]:
    data = json.loads((directory / f"{name}.json").read_text())
    requests = data["requests"]
    decisions = decision_counts(requests)
    require_decisions(name, decisions, expected_header)
    sse = sse_overview_levels(directory / f"sse-{name}.log", data["tile"])
    if expected_sse not in sse:
        sys.exit(f"FAIL: {name}: no `{expected_sse}` envelope in the SSE capture, got {sse}")
    unexpected = {k: n for k, n in sse.items() if k != expected_sse}
    if unexpected:
        sys.exit(f"FAIL: {name}: unexpected SSE decisions {unexpected}")
    return {
        "tool": "python-urllib (tests/load/temporal.py overview; SSE capture via curl)",
        "params": (
            f"GET {data['tile']}, {PARAMS['TEMPORAL_OV_REPEATS']} sequential renders, "
            f"tile cache cleared before each"
        ),
        "requests": len(requests),
        "errors": sum(1 for entry in requests if entry["status"] != 200),
        "rps": round(len(requests) / data["wall_seconds"], 1),
        **stats_ms([entry["seconds"] for entry in requests]),
        "decisions": decisions,
        "sse_decisions": sse,
    }


def machine_metadata() -> dict[str, Any]:
    if platform.system() == "Darwin":
        cpu = subprocess.run(
            ["sysctl", "-n", "machdep.cpu.brand_string"],
            capture_output=True, text=True, check=False,
        ).stdout.strip()
    else:
        cpu = platform.processor() or platform.machine()
        cpuinfo = Path("/proc/cpuinfo")
        if cpuinfo.exists():
            for line in cpuinfo.read_text().splitlines():
                if line.startswith("model name"):
                    cpu = line.split(":", 1)[1].strip()
                    break
    return {
        "cpu": cpu,
        "cores": os.cpu_count(),
        "os": f"{platform.system()} {platform.release()} {platform.machine()}",
    }


def run_out(*argv: str) -> str:
    return subprocess.run(list(argv), capture_output=True, text=True, check=True).stdout.strip()


ROWS = [
    ("frames_cold", "(d) frame loop, cold (all Live)"),
    ("frames_hot", "(d) frame loop, hot (all cache hits)"),
    ("overview_live_z12", "(e) z12 — Live (full resolution)"),
    ("overview_embedded_z10", "(e) z10 pre-materialize (embedded ov. x2)"),
    ("overview_pyramid_z11", "(e) z11 post-materialize (pyramid ov. x2)"),
    ("overview_pyramid_z10", "(e) z10 post-materialize (pyramid ov. x4)"),
]


def render_markdown(baseline: dict[str, Any]) -> str:
    scenarios = baseline["scenarios"]
    machine = baseline["machine"]
    materialize = baseline["materialize"]
    lines = [
        "# Temporal + overview load baseline (`just load-temporal`, issue #184)",
        "",
        f"Generated {baseline['generated']} at `{baseline['git_sha']}` — "
        f"{machine['cpu']} ({machine['cores']} cores), {machine['os']}. "
        f"Recipe wall time to this point: {baseline['duration_seconds']}s.",
        "",
        "Regenerate with `just load-temporal` (parameters and rationale: "
        "`tests/load/temporal.py`; scenarios: `tests/load/temporal.sh`). "
        "This file and `temporal-baseline.json` are the committed M7 evidence "
        "(ADR 0015 frame-serving + the #183/#218 pyramid path) quoted by "
        "`docs/PERFORMANCE.md`.",
        "",
        "| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for key, label in ROWS:
        s = scenarios[key]
        lines.append(
            f"| {label} | {s['requests']} | {s['errors']} | {s['rps']} "
            f"| {s['p50_ms']} | {s['p95_ms']} | {s['p99_ms']} | {s['max_ms']} |"
        )
    lines += [
        "",
        f"- `swath materialize --min-dim {PARAMS['TEMPORAL_MATERIALIZE_MIN_DIM']}`: "
        f"{materialize['wall_ms']} ms wall for the whole store "
        f"(every layer of both datasets, run once between the pre- and "
        f"post-materialize rungs).",
        f"- Frame decisions (from `x-swath-trace`): cold "
        f"{json.dumps(scenarios['frames_cold']['decisions'])}, hot "
        f"{json.dumps(scenarios['frames_hot']['decisions'])} — the cold pass "
        f"is all Live renders, the hot pass all granule-scoped cache hits.",
        f"- Overview-rung decisions (SSE envelopes, level included): "
        + "; ".join(
            f"{key.split('_', 1)[1]} {json.dumps(scenarios[key]['sse_decisions'])}"
            for key, _ in ROWS[2:]
        )
        + ".",
        "",
    ]
    return "\n".join(lines)


def cmd_report(directory: Path, started: int, json_out: Path, md_out: Path) -> None:
    scenarios = {
        "frames_cold": distill_frames(directory / "frames-cold.json", "cold"),
        "frames_hot": distill_frames(directory / "frames-hot.json", "hot"),
        "overview_live_z12": distill_overview(
            directory, "overview_live_z12", "live", "live"
        ),
        "overview_embedded_z10": distill_overview(
            directory, "overview_embedded_z10", "overview", "overview:2"
        ),
        "overview_pyramid_z11": distill_overview(
            directory, "overview_pyramid_z11", "overview", "overview:2"
        ),
        "overview_pyramid_z10": distill_overview(
            directory, "overview_pyramid_z10", "overview", "overview:4"
        ),
    }
    materialize = json.loads((directory / "materialize.json").read_text())
    baseline = {
        "schema": "swath-temporal-baseline/1",
        "generated": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "git_sha": run_out("git", "rev-parse", "--short", "HEAD"),
        "machine": machine_metadata(),
        "duration_seconds": int(time.time()) - started,
        "fire_dates": list(FIRE_DATES),
        "materialize": materialize,
        "scenarios": scenarios,
    }
    json_out.parent.mkdir(parents=True, exist_ok=True)
    json_out.write_text(json.dumps(baseline, indent=1) + "\n")
    md_out.write_text(render_markdown(baseline))
    print(f"baseline written: {json_out} + {md_out}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("params")
    frames = commands.add_parser("frames")
    frames.add_argument("--base", default="http://localhost:8080")
    frames.add_argument("--out", type=Path, required=True)
    overview = commands.add_parser("overview")
    overview.add_argument("--base", default="http://localhost:8080")
    overview.add_argument("--tile", required=True)
    overview.add_argument("--cache", type=Path, default=Path("target/e2e/cache"))
    overview.add_argument("--out", type=Path, required=True)
    report = commands.add_parser("report")
    report.add_argument("--dir", type=Path, default=Path("target/load-temporal"))
    report.add_argument("--started", type=int, required=True)
    report.add_argument("--json", type=Path, default=Path("docs/perf/temporal-baseline.json"))
    report.add_argument("--md", type=Path, default=Path("docs/perf/temporal-baseline.md"))
    args = parser.parse_args()
    if args.command == "params":
        cmd_params()
    elif args.command == "frames":
        cmd_frames(args.base, args.out)
    elif args.command == "overview":
        cmd_overview(args.base, args.tile, args.cache, args.out)
    else:
        cmd_report(args.dir, args.started, args.json, args.md)


if __name__ == "__main__":
    main()
