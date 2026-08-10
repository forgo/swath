# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# ///
"""Support script for `just load` (issue #101) — the HTTP load harness.

Three subcommands, driven by ``tests/load/load.sh`` (which owns process
orchestration; this script owns everything that wants real math or JSON):

``params``
    Print the pinned scenario parameters as shell exports — the SINGLE
    source of truth for every knob (rationale in PARAMS below).

``cold``
    Scenario (b): the cold live-render burst. Requests a set of UNIQUE
    z/x/y tiles across the fixture footprint, each exactly once, at fixed
    concurrency — so every request exercises the Live (uncached) render
    path. Implemented here rather than with oha because oha samples
    ``--urls-from-file`` randomly WITH replacement: it cannot guarantee
    the never-repeated set this scenario is defined by. Per-request
    latency, status, and the `x-swath-trace` decision are recorded to
    ``cold.json``.

``report``
    Distill the raw scenario outputs under ``target/load/`` into the
    committed baseline: ``docs/perf/load-baseline.json`` (machine-readable
    p50/p95/p99/max, rps, error counts, machine metadata) and
    ``docs/perf/load-baseline.md`` (the rendered table, also printed by
    the recipe).
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

# The fixture granule's footprint (tests/e2e/drop-granule.sh bbox —
# lon/lat WGS84): every tile generated below lies strictly inside it.
BBOX = (-105.5370, 39.1954, -105.3581, 39.3345)

# Pinned scenario parameters. Rationale:
#   healthz idle   — c=4/5s: a cheap no-load reference point so the
#                    under-warps numbers have a denominator.
#   (a) hot storm  — c=32/20s on the proven truecolor tile (pre-warmed,
#                    asserted cache_hit first): pure cache-hit-path
#                    throughput/latency; 32 connections saturates a
#                    laptop without ulimit tuning.
#   (b) cold burst — 128 unique z15 tiles (~224 interior tiles exist in
#                    the footprint; z15 oversamples the ~30 m HLS data,
#                    so every tile is a full-res Live warp), concurrency
#                    8: a moderate burst, each tile requested exactly
#                    once so no request can be a cache hit.
#   (c) mixed      — c=16/40s over the 6 HEAVIEST tiles (truecolor+ndvi
#                    z12 rows 1561/1562 and z11 — full-footprint warps;
#                    ndvi adds band math) while a host-side cache-buster
#                    clears the tile cache every 250 ms so the storm
#                    stays on the Live path (§16.7 is about warps in
#                    flight, not cache reads). healthz is measured c=4
#                    for 20s starting 5s INTO the storm; an SSE /traces
#                    subscription is held across the whole window.
PARAMS: dict[str, str | int] = {
    "LOAD_HEALTHZ_CONNS": 4,
    "LOAD_HEALTHZ_IDLE_DURATION": "5s",
    "LOAD_HOT_CONNS": 32,
    "LOAD_HOT_DURATION": "20s",
    "LOAD_HOT_TILE": "/tilesets/truecolor/tiles/12/1561/848",
    "LOAD_COLD_ZOOM": 15,
    "LOAD_COLD_COUNT": 128,
    "LOAD_COLD_CONNS": 8,
    "LOAD_MIXED_CONNS": 16,
    "LOAD_MIXED_DURATION": "40s",
    # Paths are OGC order (z/row/col == z/y/x), matching swath-e2e.
    "LOAD_MIXED_TILES": " ".join(
        f"/tilesets/{layer}/tiles/{tile}"
        for layer in ("truecolor", "ndvi")
        for tile in ("12/1561/848", "12/1562/848", "11/780/424")
    ),
    "LOAD_HEALTHZ_DELAY": 5,
    "LOAD_HEALTHZ_LOAD_DURATION": "20s",
    "LOAD_SSE_WINDOW": 45,
    "LOAD_PROBE_TILE": "/tilesets/ndvi/tiles/12/1561/848",
    "LOAD_PROBE_COUNT": 15,
    "LOAD_PROBE_INTERVAL": 2,
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


def cold_tile_paths() -> list[str]:
    """The scenario-(b) tile set: unique interior z15 truecolor tiles.

    Interior = inset one tile from the footprint's tile bounding box, so
    every tile is fully inside declared bounds (edge tiles could clip).
    Row-major order, capped at LOAD_COLD_COUNT.
    """
    zoom = int(PARAMS["LOAD_COLD_ZOOM"])
    count = int(PARAMS["LOAD_COLD_COUNT"])
    west, south, east, north = BBOX
    x_min, y_min = tile_xy(west, north, zoom)  # y grows southward
    x_max, y_max = tile_xy(east, south, zoom)
    tiles = [
        f"/tilesets/truecolor/tiles/{zoom}/{y}/{x}"
        for y in range(y_min + 1, y_max)
        for x in range(x_min + 1, x_max)
    ]
    if len(tiles) < count:
        sys.exit(f"FAIL: only {len(tiles)} interior z{zoom} tiles, need {count}")
    return tiles[:count]


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


def cmd_cold(base: str, out: Path) -> None:
    tiles = cold_tile_paths()
    conns = int(PARAMS["LOAD_COLD_CONNS"])
    start = time.perf_counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=conns) as pool:
        results = list(pool.map(lambda path: fetch(base, path), tiles))
    wall = time.perf_counter() - start
    out.write_text(json.dumps({"wall_seconds": wall, "requests": results}, indent=1))
    errors = sum(1 for r in results if r["status"] != 200)
    print(f"cold burst: {len(results)} unique tiles in {wall:.1f}s ({errors} errors)")


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


def distill_oha(path: Path, params: str) -> dict[str, Any]:
    """One oha --output-format json run -> the baseline's scenario shape."""
    data = json.loads(path.read_text())
    statuses: dict[str, int] = data["statusCodeDistribution"]
    error_dist: dict[str, int] = data["errorDistribution"]
    ok = statuses.get("200", 0)
    total = sum(statuses.values()) + sum(error_dist.values())
    if total == 0:
        sys.exit(f"FAIL: {path.name}: oha recorded zero requests")
    percentiles = data["latencyPercentiles"]
    scenario: dict[str, Any] = {
        "tool": "oha",
        "params": params,
        "requests": total,
        "errors": total - ok,
        "rps": round(data["summary"]["requestsPerSec"], 1),
        "p50_ms": round(percentiles["p50"] * 1000, 2),
        "p95_ms": round(percentiles["p95"] * 1000, 2),
        "p99_ms": round(percentiles["p99"] * 1000, 2),
        "max_ms": round(data["summary"]["slowest"] * 1000, 2),
    }
    if total != ok:
        scenario["error_breakdown"] = {
            **{f"http_{code}": n for code, n in statuses.items() if code != "200"},
            **error_dist,
        }
    return scenario


def distill_cold(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text())
    requests = data["requests"]
    decisions: dict[str, int] = {}
    for entry in requests:
        key = entry.get("decision") or (f"http_{entry['status']}" if entry["status"] != 200 else "no-trace")
        decisions[key] = decisions.get(key, 0) + 1
    return {
        "tool": "python-urllib (unique-URL driver; oha samples URL files randomly with replacement)",
        "params": (
            f"{len(requests)} unique z{PARAMS['LOAD_COLD_ZOOM']} truecolor tiles, "
            f"each exactly once, c={PARAMS['LOAD_COLD_CONNS']}"
        ),
        "requests": len(requests),
        "errors": sum(1 for entry in requests if entry["status"] != 200),
        "rps": round(len(requests) / data["wall_seconds"], 1),
        **stats_ms([entry["seconds"] for entry in requests]),
        "decisions": decisions,
    }


def distill_sse(directory: Path) -> dict[str, Any]:
    meta = json.loads((directory / "sse-meta.json").read_text())
    events = keepalives = 0
    for raw_line in (directory / "sse.log").read_bytes().splitlines():
        if raw_line.startswith(b"event:"):
            events += 1
        elif raw_line.startswith(b":"):
            keepalives += 1
    # curl exit 28 == --max-time expired == the connection outlived the
    # whole storm window; anything else means the stream died early.
    return {
        "window_seconds": PARAMS["LOAD_SSE_WINDOW"],
        "survived_window": meta["curl_exit"] == 28,
        "curl_exit": meta["curl_exit"],
        "trace_events_received": events,
        "keepalives_received": keepalives,
    }


def distill_probes(path: Path) -> dict[str, int]:
    decisions: dict[str, int] = {}
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        try:
            key = json.loads(line).get("decision", "unparseable")
        except json.JSONDecodeError:
            key = "unparseable"
        decisions[key] = decisions.get(key, 0) + 1
    return decisions


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
    ("healthz_idle", "healthz — idle baseline"),
    ("hot_cache_storm", "(a) hot-cache tile storm"),
    ("cold_live_burst", "(b) cold live-render burst"),
    ("mixed_tile_storm", "(c) mixed tile storm"),
    ("healthz_under_warps", "(c) healthz UNDER WARPS"),
]


def render_markdown(baseline: dict[str, Any]) -> str:
    scenarios = baseline["scenarios"]
    machine = baseline["machine"]
    lines = [
        "# Load baseline (`just load`, issue #101)",
        "",
        f"Generated {baseline['generated']} at `{baseline['git_sha']}` — "
        f"{machine['cpu']} ({machine['cores']} cores), {machine['os']}, "
        f"oha {baseline['oha_version']}. "
        f"Recipe wall time to this point: {baseline['duration_seconds']}s.",
        "",
        "Regenerate with `just load` (parameters and rationale: "
        "`tests/load/load.py`; scenarios: `tests/load/load.sh`). "
        "This file and `load-baseline.json` are the committed evidence for "
        "ARCHITECTURE §16.7 (async-vs-blocking render boundary, issue #102).",
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
    warps = scenarios["healthz_under_warps"]
    idle = scenarios["healthz_idle"]
    sse = scenarios["sse_under_warps"]
    lines += [
        "",
        "## §16.7: control plane under concurrent large warps",
        "",
        f"- `/healthz` p99 under warps: **{warps['p99_ms']} ms** "
        f"(idle: {idle['p99_ms']} ms); max {warps['max_ms']} ms "
        f"(idle: {idle['max_ms']} ms). Scenario params: {warps['params']}.",
        f"- SSE `/traces` subscription: "
        f"{'SURVIVED' if sse['survived_window'] else 'DIED'} the "
        f"{sse['window_seconds']}s window "
        f"({sse['trace_events_received']} trace events, "
        f"{sse['keepalives_received']} keepalives received).",
        f"- Storm decision probes (is the storm actually Live?): "
        f"{json.dumps(scenarios['mixed_tile_storm']['decision_probes'])}; "
        f"cold-burst decisions: "
        f"{json.dumps(scenarios['cold_live_burst']['decisions'])}.",
        "",
    ]
    return "\n".join(lines)


def cmd_report(directory: Path, started: int, json_out: Path, md_out: Path) -> None:
    p = PARAMS
    scenarios = {
        "healthz_idle": distill_oha(
            directory / "healthz-idle.json",
            f"GET /healthz, c={p['LOAD_HEALTHZ_CONNS']}, {p['LOAD_HEALTHZ_IDLE_DURATION']}, no other load",
        ),
        "hot_cache_storm": distill_oha(
            directory / "hot.json",
            f"GET {p['LOAD_HOT_TILE']} (pre-warmed, asserted cache_hit), "
            f"c={p['LOAD_HOT_CONNS']}, {p['LOAD_HOT_DURATION']}",
        ),
        "cold_live_burst": distill_cold(directory / "cold.json"),
        "mixed_tile_storm": {
            **distill_oha(
                directory / "mixed.json",
                f"6 heavy tiles (truecolor+ndvi z12 x2, z11 — full-footprint warps), "
                f"c={p['LOAD_MIXED_CONNS']}, {p['LOAD_MIXED_DURATION']}, "
                f"cache cleared every 250 ms to stay on the Live path",
            ),
            "decision_probes": distill_probes(directory / "probes.txt"),
        },
        "healthz_under_warps": distill_oha(
            directory / "healthz-under-warps.json",
            f"GET /healthz, c={p['LOAD_HEALTHZ_CONNS']}, {p['LOAD_HEALTHZ_LOAD_DURATION']}, "
            f"started {p['LOAD_HEALTHZ_DELAY']}s into the mixed storm",
        ),
        "sse_under_warps": distill_sse(directory),
    }
    baseline = {
        "schema": "swath-load-baseline/1",
        "generated": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "git_sha": run_out("git", "rev-parse", "--short", "HEAD"),
        "oha_version": run_out("oha", "--version").split()[-1],
        "machine": machine_metadata(),
        "duration_seconds": int(time.time()) - started,
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
    cold = commands.add_parser("cold")
    cold.add_argument("--base", default="http://localhost:8080")
    cold.add_argument("--out", type=Path, default=Path("target/load/cold.json"))
    report = commands.add_parser("report")
    report.add_argument("--dir", type=Path, default=Path("target/load"))
    report.add_argument("--started", type=int, required=True)
    report.add_argument("--json", type=Path, default=Path("docs/perf/load-baseline.json"))
    report.add_argument("--md", type=Path, default=Path("docs/perf/load-baseline.md"))
    args = parser.parse_args()
    if args.command == "params":
        cmd_params()
    elif args.command == "cold":
        cmd_cold(args.base, args.out)
    else:
        cmd_report(args.dir, args.started, args.json, args.md)


if __name__ == "__main__":
    main()
