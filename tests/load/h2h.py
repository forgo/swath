# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# ///
"""Support script for `just load-h2h` (issue #121) — the TiTiler head-to-head.

Scope: the ONE honest overlapping capability, serving a static COG as web
tiles. Everything else either server does is explicitly out of scope here
(that is COMPARISON.md's job, issue #120). Both servers run on the same
machine, one at a time, CPU-pinned identically, over the same committed
HLS fixture COGs (tests/fixtures/), driven through the same scenarios and
parameters as `just load` where applicable (imported from load.py — one
source of truth, no drift).

Maintainer pre-commitment (recorded in the issue): results are published
under docs/perf/ REGARDLESS of which server wins, framed honestly.

Subcommands (driven by tests/load/h2h.sh, which owns process lifecycle):

``params``    shared scenario knobs + the pinned TiTiler image, as shell
              exports (digest pinned HERE — the single source of truth).
``item``      the STAC item TiTiler renders from — same fixture COGs,
              asset hrefs as mounted into the TiTiler container.
``urls``      scenario URL sets for one side (swath|titiler); heavy-storm
              URLs feed oha --urls-from-file.
``verify``    pre-flight: both product tiles return 200 and a 256x256 PNG
              on the given side — proof the two servers are being asked
              for equivalent output before anything is timed.
``cold``      the unique-tile cold burst (same tile set as `just load`
              scenario (b), each tile exactly once) against either side.
``report``    distill target/h2h/{swath,titiler}/ into the committed
              docs/perf/load-h2h-titiler.{json,md}.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import struct
import sys
import time
import urllib.request
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import load  # tests/load/load.py — the `just load` single source of truth

# The pinned TiTiler under test: release 2.2.1 (2026-07-29), by multi-arch
# index digest so amd64 and arm64 hosts both run NATIVE binaries (emulating
# amd64 on an arm64 laptop would be a strawman).
TITILER_TAG = "2.2.1"
TITILER_IMAGE = (
    "ghcr.io/developmentseed/titiler"
    "@sha256:bf753ccf0fe0f231bc51a0ddbaebf7c0c82253a26db8ab25d1c30ea417e704ff"
)

# TiTiler is configured per its OWN documented guidance — no strawman:
#   - GDAL/VSI environment: the recommended values from
#     https://developmentseed.org/titiler/advanced/performance_tuning/
#     (set in h2h.sh's docker run; recorded in the report by cite_config()).
#   - Process model: the docs' docker command runs
#     `uvicorn titiler.application.main:app` (https://developmentseed.org/titiler/)
#     with `--workers 1`; we run `--workers <pinned CPUs>` — one worker per
#     pinned CPU, i.e. MORE generous than the documented example.
TITILER_DOC_TUNING = "https://developmentseed.org/titiler/advanced/performance_tuning/"
TITILER_DOC_DOCKER = "https://developmentseed.org/titiler/"

# TiTiler renders the same two products Swath's catalog defines
# (tests/e2e/swath-catalog.toml): truecolor = B04/B03/B02 rescaled 0..3000;
# ndvi = (B8A-B04)/(B8A+B04) rescaled -1..1, RdYlGn colormap. Multi-asset
# composition over separate band COGs is TiTiler's /stac (STACReader) path —
# its canonical way to do this, not a handicap. `%2B` is an URL-encoded `+`.
TITILER_QUERY = {
    "truecolor": "url=/data/item.json&assets=b04&assets=b03&assets=b02&rescale=0,3000",
    "ndvi": (
        "url=/data/item.json&assets=b8a&assets=b04"
        "&expression=(b1-b2)/(b1%2Bb2)&rescale=-1,1&colormap_name=rdylgn"
    ),
}

GRANULE = "hlss30-t13sdd-2024158"


def swath_to_titiler(path: str) -> str:
    """Map a Swath tile path to the equivalent TiTiler request.

    Swath serves OGC `/tilesets/{layer}/tiles/{z}/{row}/{col}`; TiTiler
    serves `/stac/tiles/WebMercatorQuad/{z}/{x}/{y}.png` — same
    WebMercatorQuad tile, x=col, y=row.
    """
    parts = path.strip("/").split("/")
    layer, z, row, col = parts[1], parts[3], parts[4], parts[5]
    return f"/stac/tiles/WebMercatorQuad/{z}/{col}/{row}.png?{TITILER_QUERY[layer]}"


def tile_url(side: str, path: str) -> str:
    return path if side == "swath" else swath_to_titiler(path)


def heavy_tile_paths() -> list[str]:
    return str(load.PARAMS["LOAD_MIXED_TILES"]).split()


def cmd_params() -> None:
    shared = {
        "H2H_CPUS": os.environ.get("H2H_CPUS", "4"),
        "H2H_TITILER_IMAGE": TITILER_IMAGE,
        "H2H_TITILER_TAG": TITILER_TAG,
        "H2H_HEALTHZ_CONNS": load.PARAMS["LOAD_HEALTHZ_CONNS"],
        "H2H_HEALTHZ_DURATION": load.PARAMS["LOAD_HEALTHZ_IDLE_DURATION"],
        "H2H_HOT_CONNS": load.PARAMS["LOAD_HOT_CONNS"],
        "H2H_HOT_DURATION": load.PARAMS["LOAD_HOT_DURATION"],
        "H2H_HOT_TILE": load.PARAMS["LOAD_HOT_TILE"],
        "H2H_HEAVY_CONNS": load.PARAMS["LOAD_MIXED_CONNS"],
        "H2H_HEAVY_DURATION": load.PARAMS["LOAD_MIXED_DURATION"],
        "H2H_PROBE_TILE": load.PARAMS["LOAD_PROBE_TILE"],
        "H2H_PROBE_COUNT": load.PARAMS["LOAD_PROBE_COUNT"],
        "H2H_PROBE_INTERVAL": load.PARAMS["LOAD_PROBE_INTERVAL"],
    }
    for key, value in shared.items():
        print(f'export {key}="{value}"')


def cmd_item(out: Path) -> None:
    """The STAC item TiTiler renders from: the SAME committed fixture COGs,
    mounted read-only at /data/fixtures inside the TiTiler container."""
    west, south, east, north = load.BBOX
    item = {
        "type": "Feature",
        "stac_version": "1.0.0",
        "id": GRANULE,
        "geometry": {
            "type": "Polygon",
            "coordinates": [[
                [west, south], [east, south], [east, north], [west, north], [west, south],
            ]],
        },
        "bbox": list(load.BBOX),
        "properties": {"datetime": "2024-06-06T17:54:00Z"},
        "links": [],
        "assets": {
            band: {
                "href": f"/data/fixtures/{GRANULE}-{band}.tif",
                "type": "image/tiff; application=geotiff; profile=cloud-optimized",
            }
            for band in ("b02", "b03", "b04", "b8a")
        },
    }
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(item, indent=1) + "\n")
    print(f"STAC item written: {out}")


def cmd_urls(side: str, scenario: str) -> None:
    if scenario == "hot":
        paths = [str(load.PARAMS["LOAD_HOT_TILE"])]
    else:
        paths = heavy_tile_paths()
    for path in paths:
        print(tile_url(side, path))


def cmd_verify(side: str, base: str) -> None:
    """Both products must come back as a 256x256 PNG before anything is
    timed — the proof the two servers are asked for equivalent output."""
    for layer in ("truecolor", "ndvi"):
        z, row, col = str(load.PARAMS["LOAD_HOT_TILE"]).strip("/").split("/")[3:6]
        path = f"/tilesets/{layer}/tiles/{z}/{row}/{col}"
        url = base + tile_url(side, path)
        with urllib.request.urlopen(url, timeout=120) as response:
            body = response.read()
            if response.status != 200:
                sys.exit(f"FAIL: {side} {layer} tile -> HTTP {response.status}")
        if body[:8] != b"\x89PNG\r\n\x1a\n":
            sys.exit(f"FAIL: {side} {layer} tile is not a PNG")
        width, height = struct.unpack(">II", body[16:24])
        if (width, height) != (256, 256):
            sys.exit(f"FAIL: {side} {layer} tile is {width}x{height}, want 256x256")
        print(f"verify: {side} {layer} z{z} -> 200, 256x256 PNG, {len(body)} bytes")


def cmd_cold(side: str, base: str, out: Path) -> None:
    """Scenario (cold): `just load` (b) verbatim — the same unique z15 tile
    set, each exactly once, same concurrency — against either server."""
    urls = [tile_url(side, path) for path in load.cold_tile_paths()]
    conns = int(load.PARAMS["LOAD_COLD_CONNS"])
    start = time.perf_counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=conns) as pool:
        results = list(pool.map(lambda url: load.fetch(base, url), urls))
    wall = time.perf_counter() - start
    out.write_text(json.dumps({"wall_seconds": wall, "requests": results}, indent=1))
    errors = sum(1 for r in results if r["status"] != 200)
    print(f"cold burst [{side}]: {len(results)} unique tiles in {wall:.1f}s ({errors} errors)")


# --- report ---------------------------------------------------------------

SCENARIOS = [
    ("healthz_idle", "healthz — idle baseline"),
    ("repeated_tile", "repeated-tile storm (see note)"),
    ("cold_burst", "cold burst — 128 unique tiles, each once"),
    ("heavy_storm", "heavy-tile storm — 6 heaviest products"),
]


def distill_side(directory: Path, side: str) -> dict[str, Any]:
    p = load.PARAMS
    scenarios: dict[str, Any] = {
        "healthz_idle": load.distill_oha(
            directory / "healthz-idle.json",
            f"GET /healthz, c={p['LOAD_HEALTHZ_CONNS']}, {p['LOAD_HEALTHZ_IDLE_DURATION']}, no other load",
        ),
        "repeated_tile": load.distill_oha(
            directory / "hot.json",
            f"one truecolor z12 tile repeatedly, c={p['LOAD_HOT_CONNS']}, {p['LOAD_HOT_DURATION']} "
            + ("(swath: asserted cache_hit)" if side == "swath" else "(titiler: re-rendered per request, by design)"),
        ),
        "cold_burst": load.distill_cold(directory / "cold.json"),
        "heavy_storm": load.distill_oha(
            directory / "heavy.json",
            f"6 heavy tiles (truecolor+ndvi z12 x2, z11 — full-footprint warps), "
            f"c={p['LOAD_MIXED_CONNS']}, {p['LOAD_MIXED_DURATION']}"
            + (", cache cleared every 250 ms to stay on the Live path" if side == "swath" else ""),
        ),
    }
    if side == "swath":
        scenarios["heavy_storm"]["decision_probes"] = load.distill_probes(directory / "probes.txt")
    else:
        # TiTiler has no trace header; drop the empty decision breakdown the
        # shared cold distiller records for Swath.
        scenarios["cold_burst"].pop("decisions", None)
    return scenarios


def docker_metadata() -> dict[str, str]:
    """Where the containers actually ran (on macOS this is a VM — disclose)."""
    info = json.loads(load.run_out("docker", "info", "--format", "{{json .}}"))
    return {
        key: str(info.get(key, "unknown"))
        for key in ("NCPU", "MemTotal", "OperatingSystem", "ServerVersion", "Architecture")
    }


def cite_config(cpus: int) -> dict[str, Any]:
    return {
        "resource_matching": (
            f"each server's container pinned to --cpus {cpus} (Docker CPU quota); "
            f"memory unlimited for both; servers run ONE AT A TIME on an otherwise idle machine"
        ),
        "swath": {
            "image": "built from this commit's Dockerfile (cargo build --release)",
            "config": "tests/e2e/swath-catalog.toml — the same compose stack `just e2e`/`just load` use "
                      "(pgstac + minio sidecars up but idle-cost only; tiles read band COGs from a local mount)",
        },
        "titiler": {
            "image": f"{TITILER_IMAGE} (tag {TITILER_TAG}, multi-arch index digest — native binaries on amd64 and arm64)",
            "command": "uvicorn titiler.application.main:app --workers <pinned CPUs> — the docs' documented command "
                       f"({TITILER_DOC_DOCKER}) with MORE workers than its `--workers 1` example (one per pinned CPU)",
            "gdal_env": {
                "GDAL_CACHEMAX": "200",
                "VSI_CACHE": "TRUE",
                "VSI_CACHE_SIZE": "5000000",
                "GDAL_BAND_BLOCK_CACHE": "HASHSET",
                "GDAL_DISABLE_READDIR_ON_OPEN": "EMPTY_DIR",
                "GDAL_HTTP_MERGE_CONSECUTIVE_RANGES": "YES",
            },
            "gdal_env_source": TITILER_DOC_TUNING,
            "data_access": "same committed fixture COGs, read-only local mount; multi-asset products via its "
                           "/stac router over a local STAC item (its canonical multi-COG composition path)",
        },
    }


def fmt_ratio(swath_value: float, titiler_value: float, lower_is_better: bool) -> str:
    """Honest per-cell ratio: who leads, by how much, no rounding games."""
    if swath_value <= 0 or titiler_value <= 0:
        return "n/a"
    ratio = (titiler_value / swath_value) if lower_is_better else (swath_value / titiler_value)
    if ratio >= 1:
        return f"swath {ratio:.1f}x"
    return f"titiler {1 / ratio:.1f}x"


def render_markdown(report: dict[str, Any]) -> str:
    swath = report["sides"]["swath"]["scenarios"]
    titiler = report["sides"]["titiler"]["scenarios"]
    machine = report["machine"]
    docker = report["docker"]
    config = report["configuration"]
    lines = [
        "# Head-to-head: Swath vs TiTiler on static COG serving (`just load-h2h`, issue #121)",
        "",
        "**This is a laptop benchmark.** One developer machine "
        f"({machine['cpu']}, {machine['cores']} cores, {machine['os']}; containers in "
        f"Docker {docker['ServerVersion']}, {docker['OperatingSystem']}, {docker['NCPU']} VM CPUs, "
        f"{docker['Architecture']}), generated {report['generated']} at `{report['git_sha']}`, "
        f"oha {report['oha_version']}. It is NOT a capacity-planning study; treat every number "
        "as one machine's evidence, reproducible with one command: `just load-h2h`.",
        "",
        "**Pre-commitment.** Before this was first run, the maintainer committed (issue #121) to "
        "publishing the results REGARDLESS of which server wins, with honest framing. This document "
        "is that publication; the numbers below are whatever the run produced.",
        "",
        "## What is (and is not) compared",
        "",
        "Exactly one capability overlaps enough for a fair head-to-head: **serving a static,",
        "already-ingested COG as WebMercatorQuad PNG tiles**. Both servers render the same two",
        "products from the same committed HLS fixture COGs (`tests/fixtures/`, ~1.4 MB, real",
        "Sentinel-2 data): truecolor (B04/B03/B02, rescale 0..3000) and NDVI ((B8A-B04)/(B8A+B04),",
        "RdYlGn). A pre-flight check asserts both sides return 200 with a 256x256 PNG for both",
        "products before anything is timed.",
        "",
        "Explicitly **out of scope** here (COMPARISON.md and issue #120 own capability claims):",
        "",
        "- **What TiTiler does that this does not test:** dynamic tiling of arbitrary COGs/STAC",
        "  items/mosaics anywhere on the internet with zero pre-registration, xarray/zarr backends,",
        "  many tile matrix sets and output formats, statistics/point endpoints, the plugin",
        "  ecosystem. TiTiler is a general dynamic tiler; this test pins it to one narrow job.",
        "- **What Swath does that TiTiler does not do (and is NOT scored here):** watch-dir",
        "  ingest-to-pixel, openEO process products, per-tile provenance traces (`x-swath-trace`,",
        "  SSE x-ray), the write-through tile cache as a *capability*, catalog/granule browsing.",
        "- **Caching as a capability** is out of scope, but one scenario (repeated-tile) runs each",
        "  architecture as designed — see the note under the table. Dynamic products and",
        "  provenance are not exercised at all.",
        "",
        "## Configuration (both sides disclosed, no strawman)",
        "",
        f"- **Resource matching:** {config['resource_matching']}.",
        f"- **Swath:** {config['swath']['image']}; {config['swath']['config']}.",
        f"- **TiTiler:** `{config['titiler']['image'].split(' ')[0]}` (release {report['titiler_tag']}), "
        f"{config['titiler']['command']}. GDAL environment set to the documented recommended values "
        f"from its performance-tuning guide (<{config['titiler']['gdal_env_source']}>): "
        + ", ".join(f"`{k}={v}`" for k, v in config["titiler"]["gdal_env"].items()) + ". "
        f"Data access: {config['titiler']['data_access']}.",
        "- **Scenario parameters** are `just load`'s own, imported from `tests/load/load.py` "
        "(one source of truth); URL mapping and TiTiler product queries: `tests/load/h2h.py`.",
        "",
        "## Results",
        "",
        "| scenario | server | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for key, label in SCENARIOS:
        for side, scenarios in (("swath", swath), ("titiler", titiler)):
            s = scenarios[key]
            lines.append(
                f"| {label if side == 'swath' else ''} | {side} | {s['requests']} | {s['errors']} "
                f"| {s['rps']} | {s['p50_ms']} | {s['p95_ms']} | {s['p99_ms']} | {s['max_ms']} |"
            )
    lines += [
        "",
        "Same rows as throughput/latency ratios (who leads, by how much):",
        "",
        "| scenario | rps | p50 | p99 |",
        "|---|---|---|---|",
    ]
    for key, label in SCENARIOS:
        s, t = swath[key], titiler[key]
        lines.append(
            f"| {label} | {fmt_ratio(s['rps'], t['rps'], False)} "
            f"| {fmt_ratio(s['p50_ms'], t['p50_ms'], True)} "
            f"| {fmt_ratio(s['p99_ms'], t['p99_ms'], True)} |"
        )
    # The pre-committed bottom line, computed from the data so it can never
    # drift from the table: render-vs-render rows are cold_burst and
    # heavy_storm (both sides render every request there).
    render_leads = [
        titiler[key]["rps"] / swath[key]["rps"]
        for key in ("cold_burst", "heavy_storm")
        if swath[key]["rps"] > 0
    ]
    worst = max(render_leads) if render_leads else 0.0
    if worst > 1:
        bottom = (
            f"**Bottom line (the pre-committed framing).** On the render-vs-render rows — "
            f"stateless tile rendering, TiTiler's specialty — **TiTiler is faster on this "
            f"machine**: Swath is within {worst:.1f}x of it on throughput at worst "
            f"(see the ratio table). Swath's leads are the hot-tile path (its write-through "
            f"cache) and control-plane latency. Neither fact cancels the other; both are "
            f"published, as committed, and what each system does beyond this narrow overlap "
            f"is deliberately not scored here."
        )
    else:
        bottom = (
            f"**Bottom line (the pre-committed framing).** On the render-vs-render rows — "
            f"stateless tile rendering, TiTiler's specialty — Swath led on this machine "
            f"(TiTiler within {1 / min(render_leads):.1f}x at worst; see the ratio table). "
            f"Published as committed; what each system does beyond this narrow overlap is "
            f"deliberately not scored here."
        )
    heavy_probes = swath["heavy_storm"].get("decision_probes", {})
    cold_decisions = report["sides"]["swath"]["scenarios"]["cold_burst"].get("decisions", {})
    lines += [
        "",
        bottom,
        "",
        "### Scenario notes (read before quoting any row)",
        "",
        "- **healthz** is each server's own liveness route; TiTiler's returns a versions document,",
        "  Swath's a bare liveness body — a reference point, not a comparison of equals.",
        "- **repeated-tile** is an *architecture contrast, not a render-vs-render comparison*:",
        "  Swath serves its write-through cache (asserted `cache_hit` before the storm); TiTiler",
        "  recomputes every request by design and delegates HTTP caching to the deployment layer.",
        "  Read it as \"what a client sees on a hot tile\", nothing more.",
        "- **cold burst** and **heavy storm** are the honest render-vs-render rows: every request",
        f"  renders. Swath's cache is cleared every 250 ms during the heavy storm (decision probes: "
        f"{json.dumps(heavy_probes)}) and the cold burst is unique-by-construction "
        f"(decisions: {json.dumps(cold_decisions)}); TiTiler never caches tiles.",
        "- Each server pays its own per-request metadata cost as deployed: Swath resolves the",
        "  granule through its catalog; TiTiler re-reads the local STAC item JSON. Both are how",
        "  the servers actually serve.",
        "",
        "## Regression policy",
        "",
        "Internal baselines (`docs/perf/load-baseline.*`, PERFORMANCE.md) remain the regression",
        "reference. This document is a point-in-time comparison, regenerated only deliberately",
        "(`just load-h2h`), never a CI gate.",
        "",
    ]
    return "\n".join(lines)


def cmd_report(directory: Path, started: int, cpus: int, json_out: Path, md_out: Path) -> None:
    report = {
        "schema": "swath-h2h-titiler/1",
        "generated": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "git_sha": load.run_out("git", "rev-parse", "--short", "HEAD"),
        "oha_version": load.run_out("oha", "--version").split()[-1],
        "titiler_image": TITILER_IMAGE,
        "titiler_tag": TITILER_TAG,
        "cpus_pinned": cpus,
        "machine": load.machine_metadata(),
        "docker": docker_metadata(),
        "configuration": cite_config(cpus),
        "duration_seconds": int(time.time()) - started,
        "sides": {
            side: {"scenarios": distill_side(directory / side, side)}
            for side in ("swath", "titiler")
        },
    }
    json_out.parent.mkdir(parents=True, exist_ok=True)
    json_out.write_text(json.dumps(report, indent=1) + "\n")
    md_out.write_text(render_markdown(report))
    print(f"h2h report written: {json_out} + {md_out}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("params")
    item = commands.add_parser("item")
    item.add_argument("--out", type=Path, default=Path("target/h2h/item.json"))
    urls = commands.add_parser("urls")
    urls.add_argument("--side", choices=("swath", "titiler"), required=True)
    urls.add_argument("--scenario", choices=("hot", "heavy"), required=True)
    verify = commands.add_parser("verify")
    verify.add_argument("--side", choices=("swath", "titiler"), required=True)
    verify.add_argument("--base", required=True)
    cold = commands.add_parser("cold")
    cold.add_argument("--side", choices=("swath", "titiler"), required=True)
    cold.add_argument("--base", required=True)
    cold.add_argument("--out", type=Path, required=True)
    report = commands.add_parser("report")
    report.add_argument("--dir", type=Path, default=Path("target/h2h"))
    report.add_argument("--started", type=int, required=True)
    report.add_argument("--cpus", type=int, required=True)
    report.add_argument("--json", type=Path, default=Path("docs/perf/load-h2h-titiler.json"))
    report.add_argument("--md", type=Path, default=Path("docs/perf/load-h2h-titiler.md"))
    args = parser.parse_args()
    if args.command == "params":
        cmd_params()
    elif args.command == "item":
        cmd_item(args.out)
    elif args.command == "urls":
        cmd_urls(args.side, args.scenario)
    elif args.command == "verify":
        cmd_verify(args.side, args.base)
    elif args.command == "cold":
        cmd_cold(args.side, args.base, args.out)
    else:
        cmd_report(args.dir, args.started, args.cpus, args.json, args.md)


if __name__ == "__main__":
    main()
