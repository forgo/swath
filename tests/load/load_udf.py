# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# ///
"""Support script for `just load-udf` (issue #207) — the `run_udf` live-latency
evidence under the ADR 0012 guard.

Same discipline as `tests/load/load.py` (decision-probed scenarios, pinned
parameters, committed artifact); driven by ``tests/load/load_udf.sh`` (process
orchestration — oha, the SSE capture, the cache-buster). This script owns
parameters, the publish motion, the HTTP probes, and the distilled baseline
(``docs/perf/load-udf-baseline.{json,md}``).

Two scenarios, both against a *published* `run_udf` service:

``(u) UDF storm``
    The reference NDVI UDF (``examples/udf/ndvi``) published as an xyz
    service, hammered on its heaviest live tiles (c=16, cache cleared every
    250 ms so every request is a Live render THROUGH the sandboxed module)
    while ``/healthz`` is probed and an SSE ``/traces`` subscription is held
    across the window. The ADR 0012 reopen signals — ``/healthz`` p99 and
    SSE survival — are recorded and verdicted against the 50 ms trigger.

``(f) fuel bomb``
    A runaway-loop UDF (``tests/load/fuelbomb.wasm``) published just as
    cleanly, then refused on the tile path with the RFC 7807 fuel problem
    (500) and on the preview with ``ProcessGraphComplexity`` (400) — while
    the SAME ``/healthz`` + SSE probes prove ZERO collateral: refusing user
    code costs the control plane nothing.

``report`` FAILS on scenario-integrity violations (the storm is not actually
Live-through-UDF, or the bomb is not actually refused) — invalid evidence,
not a footnote. The ADR 0012 signals are RECORDED and verdicted, never a
build break: a trip is a maintainer lane-decision on evidence (ADR 0018
rollback / ADR 0012 reopen), not something this harness silently fixes.
"""

from __future__ import annotations

import argparse
import base64
import json
import math
import os
import platform
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

# The ADR 0012 reopen trigger this harness stands guard on: /healthz p99
# above this under scenario (c)-style load reopens §16.7 (docs/decisions/
# 0012-render-stays-inline-async.md). The UDF storm is exactly a (c)-style
# storm whose Live renders run user code.
ADR_0012_HEALTHZ_P99_TRIGGER_MS = 50.0

# The committed UDF fixtures, base64'd into `data:` module URLs at publish
# time (no network — the module bytes travel inline, ADR 0018's inline
# `data:application/wasm` grammar).
NDVI_WASM = Path("crates/adapters/swath-udf-wasmtime/tests/fixtures/ndvi.wasm")
FUELBOMB_WASM = Path("tests/load/fuelbomb.wasm")

# Pinned scenario parameters. Rationale mirrors load.py's (c) mixed storm:
#   tiles     — the heaviest full-footprint z12/z11 warps of the fixture
#               granule, now each carrying the NDVI UDF pixel stage. Path
#               SUFFIXES only (z/row/col, OGC order); the shell prefixes
#               the PUBLISHED service id (content-derived, known only after
#               POST /services).
#   storm     — c=16/40s, matching load.py (c): saturates the render lane
#               with UDF work while the cache-buster keeps it on the Live
#               path (a UDF stage only runs on a Live render).
#   healthz   — c=4 for 20s starting 5s into each storm (the §16.7 probe).
#   sse       — one /traces subscription held the whole 45s window.
#   probes    — 15 tile samples at 2s spacing, recording the decision AND
#               the deterministic udf_fuel_used (proves the storm is Live
#               through the module, not a cache read).
PARAMS: dict[str, str | int] = {
    "UDF_TILES": " ".join(("12/1561/848", "12/1562/848", "11/780/424")),
    "UDF_PROBE_TILE": "12/1561/848",
    "UDF_STORM_CONNS": 16,
    "UDF_STORM_DURATION": "40s",
    "UDF_HEALTHZ_CONNS": 4,
    "UDF_HEALTHZ_IDLE_DURATION": "5s",
    "UDF_HEALTHZ_LOAD_DURATION": "20s",
    "UDF_HEALTHZ_DELAY": 5,
    "UDF_SSE_WINDOW": 45,
    "UDF_PROBE_COUNT": 15,
    "UDF_PROBE_INTERVAL": 2,
}


def cmd_params() -> None:
    for key, value in PARAMS.items():
        print(f'export {key}="{value}"')


def data_url(path: Path) -> str:
    encoded = base64.standard_b64encode(path.read_bytes()).decode("ascii")
    return f"data:application/wasm;base64,{encoded}"


def udf_service_graph(module_url: str) -> dict[str, Any]:
    """load(b8a,b04) → run_udf → scale(-1..1 → 0..255) → save: the NDVI
    UDF product, the same graph the API tests publish."""
    return {
        "type": "xyz",
        "title": "NDVI (UDF, load-udf)",
        "process": {
            "process_graph": {
                "load": {
                    "process_id": "load_collection",
                    "arguments": {
                        "id": "hls-s30",
                        "spatial_extent": None,
                        "temporal_extent": None,
                        "bands": ["b8a", "b04"],
                    },
                },
                "udf": {
                    "process_id": "run_udf",
                    "arguments": {
                        "data": {"from_node": "load"},
                        "udf": module_url,
                        "runtime": "wasm",
                        "version": "1",
                    },
                },
                "scale": {
                    "process_id": "linear_scale_range",
                    "arguments": {
                        "x": {"from_node": "udf"},
                        "inputMin": -1,
                        "inputMax": 1,
                        "outputMin": 0,
                        "outputMax": 255,
                    },
                },
                "save": {
                    "process_id": "save_result",
                    "arguments": {"data": {"from_node": "scale"}, "format": "png"},
                    "result": True,
                },
            }
        },
    }


def cmd_publish(base: str, which: str) -> None:
    """POST /services for one module; print the openeo-identifier (the
    service id whose tiles the shell then storms)."""
    module = NDVI_WASM if which == "ndvi" else FUELBOMB_WASM
    body = json.dumps(udf_service_graph(data_url(module))).encode()
    request = urllib.request.Request(
        base + "/services",
        data=body,
        method="POST",
        headers={"content-type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            identifier = response.headers.get("openeo-identifier", "")
            status = response.status
    except urllib.error.HTTPError as error:
        sys.exit(f"FAIL: publish {which}: HTTP {error.code}: {error.read().decode(errors='replace')}")
    if status not in (200, 201) or not identifier:
        sys.exit(f"FAIL: publish {which}: status {status}, id {identifier!r}")
    print(identifier)


def fetch(base: str, path: str) -> dict[str, Any]:
    """One GET: wall latency, status, the x-swath-trace decision, and
    udf_fuel_used when the render ran a module."""
    request = urllib.request.Request(base + path, method="GET")
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            response.read()
            status = response.status
            header = response.headers.get("x-swath-trace", "")
    except urllib.error.HTTPError as error:
        return {"path": path, "seconds": time.perf_counter() - start, "status": error.code}
    except OSError as error:
        return {"path": path, "seconds": time.perf_counter() - start, "status": 0, "error": str(error)}
    entry: dict[str, Any] = {"path": path, "seconds": time.perf_counter() - start, "status": status}
    if header:
        try:
            trace = json.loads(header)
            entry["decision"] = trace.get("decision", "")
            if "udf_fuel_used" in trace:
                entry["udf_fuel_used"] = trace["udf_fuel_used"]
        except json.JSONDecodeError:
            entry["decision"] = "unparseable"
    return entry


def cmd_probe(base: str, tile: str, out: Path) -> None:
    """Sequential decision+fuel probes of one live UDF tile, cache cleared
    by the shell's buster so each is a fresh Live render through the
    module."""
    count = int(PARAMS["UDF_PROBE_COUNT"])
    interval = int(PARAMS["UDF_PROBE_INTERVAL"])
    results = []
    for _ in range(count):
        results.append(fetch(base, tile))
        time.sleep(interval)
    out.write_text(json.dumps({"requests": results}, indent=1))
    print(f"probes: {len(results)} samples of {tile}")


def cmd_preview(base: str, which: str, out: Path) -> None:
    """POST /result with a UDF graph; record status + body. The fuel bomb
    must be the spec's ProcessGraphComplexity (400)."""
    module = NDVI_WASM if which == "ndvi" else FUELBOMB_WASM
    process = udf_service_graph(data_url(module))["process"]
    body = json.dumps({"process": process}).encode()
    request = urllib.request.Request(
        base + "/result", data=body, method="POST", headers={"content-type": "application/json"}
    )
    record: dict[str, Any] = {}
    try:
        with urllib.request.urlopen(request, timeout=180) as response:
            record = {"status": response.status, "content_type": response.headers.get("content-type", "")}
            response.read()
    except urllib.error.HTTPError as error:
        payload = error.read().decode(errors="replace")
        try:
            record = {"status": error.code, "body": json.loads(payload)}
        except json.JSONDecodeError:
            record = {"status": error.code, "body": payload}
    out.write_text(json.dumps(record, indent=1))
    print(f"preview {which}: status {record.get('status')}")


def cmd_tileprobe(base: str, tile: str, out: Path) -> None:
    """A single fuel-bomb tile fetch, recording the RFC 7807 problem body
    the tile path answers (500 + the fuel detail)."""
    request = urllib.request.Request(base + tile, method="GET")
    record: dict[str, Any] = {}
    try:
        with urllib.request.urlopen(request, timeout=180) as response:
            record = {"status": response.status}
            response.read()
    except urllib.error.HTTPError as error:
        payload = error.read().decode(errors="replace")
        try:
            record = {"status": error.code, "body": json.loads(payload)}
        except json.JSONDecodeError:
            record = {"status": error.code, "body": payload}
    out.write_text(json.dumps(record, indent=1))
    print(f"fuel-bomb tile probe: status {record.get('status')}")


# --- distillation (shared shapes with load.py) --------------------------


def percentile(sorted_values: list[float], q: float) -> float:
    rank = max(1, math.ceil(q * len(sorted_values)))
    return sorted_values[rank - 1]


def distill_oha(path: Path, params: str) -> dict[str, Any]:
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
        scenario["status_breakdown"] = {
            **{f"http_{code}": n for code, n in statuses.items()},
            **error_dist,
        }
    return scenario


def distill_sse(directory: Path, name: str) -> dict[str, Any]:
    meta = json.loads((directory / f"sse-{name}-meta.json").read_text())
    events = keepalives = 0
    for raw_line in (directory / f"sse-{name}.log").read_bytes().splitlines():
        if raw_line.startswith(b"event:"):
            events += 1
        elif raw_line.startswith(b":"):
            keepalives += 1
    return {
        "window_seconds": PARAMS["UDF_SSE_WINDOW"],
        "survived_window": meta["curl_exit"] == 28,
        "curl_exit": meta["curl_exit"],
        "trace_events_received": events,
        "keepalives_received": keepalives,
    }


def distill_storm_probes(path: Path) -> dict[str, Any]:
    """The storm's own live-through-UDF proof, RECORDING the decision mix
    exactly as `load.py`'s (c) mixed storm does: the 250 ms cache-buster
    races the sampler, so a probe may catch a just-rendered tile as a
    cache_hit — that is the storm staying on the Live path, not a failure.
    The evidence the storm exercised the module is (a) no request errored,
    (b) at least one probe was a Live render through the module carrying a
    positive udf_fuel_used, and (c) that fuel is deterministic across every
    Live sample (ADR 0018 — same bytes, same fuel)."""
    data = json.loads(path.read_text())
    requests = data["requests"]
    decisions: dict[str, int] = {}
    fuels: list[int] = []
    for entry in requests:
        key = entry.get("decision") or (
            f"http_{entry['status']}" if entry["status"] != 200 else "no-trace"
        )
        decisions[key] = decisions.get(key, 0) + 1
        if entry.get("decision") == "live" and "udf_fuel_used" in entry:
            fuels.append(int(entry["udf_fuel_used"]))
    stray = set(decisions) - {"live", "cache_hit"}
    if stray:
        sys.exit(
            f"FAIL: udf_storm probes: unexpected outcomes {stray} (full mix {decisions}) "
            f"— the storm must stay on the Live/cache path, never error"
        )
    if not fuels or any(f <= 0 for f in fuels):
        sys.exit(
            f"FAIL: udf_storm probes: no Live UDF render charged fuel — the storm never "
            f"exercised the module (mix {decisions})"
        )
    if len(set(fuels)) != 1:
        sys.exit(f"FAIL: udf_storm probes: Live fuel must be deterministic, got {sorted(set(fuels))}")
    return {"decisions": decisions, "udf_fuel_used": fuels[0], "samples": len(requests)}


def require_all_refused(scenario: dict[str, Any]) -> None:
    """The fuel-bomb storm is valid evidence only if EVERY request was
    refused (the runaway module never serves a tile)."""
    if scenario["errors"] != scenario["requests"]:
        sys.exit(
            f"FAIL: fuelbomb_storm: {scenario['errors']}/{scenario['requests']} refused — "
            f"the runaway module must never serve a tile"
        )
    breakdown = scenario.get("status_breakdown", {})
    non500 = {k: v for k, v in breakdown.items() if k != "http_500"}
    if non500:
        sys.exit(f"FAIL: fuelbomb_storm: non-500 outcomes {non500} — the refusal must be the tile-path 500")


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
    return {"cpu": cpu, "cores": os.cpu_count(), "os": f"{platform.system()} {platform.release()} {platform.machine()}"}


def run_out(*argv: str) -> str:
    return subprocess.run(list(argv), capture_output=True, text=True, check=True).stdout.strip()


def verdict(healthz: dict[str, Any], sse: dict[str, Any]) -> dict[str, Any]:
    """The ADR 0012 signals as a recorded verdict — never a build break:
    a trip is a maintainer lane-decision on evidence, not a silent fix."""
    tripped = (
        healthz["p99_ms"] >= ADR_0012_HEALTHZ_P99_TRIGGER_MS
        or not sse["survived_window"]
    )
    return {
        "healthz_p99_ms": healthz["p99_ms"],
        "healthz_p99_trigger_ms": ADR_0012_HEALTHZ_P99_TRIGGER_MS,
        "sse_survived": sse["survived_window"],
        "adr_0012_trigger_tripped": tripped,
    }


ROWS = [
    ("udf_storm", "(u) UDF mixed storm (Live NDVI + cache, buster on)"),
    ("healthz_under_udf_storm", "(u) healthz UNDER the UDF storm"),
    ("fuelbomb_storm", "(f) fuel-bomb storm — every tile refused"),
    ("healthz_under_fuelbomb", "(f) healthz UNDER the fuel-bomb refusals"),
]


def render_markdown(baseline: dict[str, Any]) -> str:
    scenarios = baseline["scenarios"]
    machine = baseline["machine"]
    storm_v = baseline["adr_0012"]["udf_storm"]
    bomb_v = baseline["adr_0012"]["fuel_bomb"]
    preview = baseline["fuelbomb_preview"]
    lines = [
        "# `run_udf` live-latency baseline (`just load-udf`, issue #207)",
        "",
        f"Generated {baseline['generated']} at `{baseline['git_sha']}` — "
        f"{machine['cpu']} ({machine['cores']} cores), {machine['os']}, "
        f"oha {baseline['oha_version']}. "
        f"Recipe wall time to this point: {baseline['duration_seconds']}s.",
        "",
        "Regenerate with `just load-udf` (parameters and rationale: "
        "`tests/load/load_udf.py`; scenarios: `tests/load/load_udf.sh`). "
        "This file and `load-udf-baseline.json` are the committed evidence "
        "for `run_udf` under the ADR 0012 guard (ADR 0018 tile path).",
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
        "## ADR 0012 signals (recorded — a trip is a maintainer lane-decision)",
        "",
        f"- **UDF storm** `/healthz` p99: **{storm_v['healthz_p99_ms']} ms** "
        f"(trigger: {storm_v['healthz_p99_trigger_ms']} ms) — "
        f"{'TRIPPED' if storm_v['adr_0012_trigger_tripped'] else 'holds'}. "
        f"SSE `/traces`: {'SURVIVED' if storm_v['sse_survived'] else 'DROPPED'} "
        f"({scenarios['sse_under_udf_storm']['trace_events_received']} trace events). "
        f"Storm exercised the module (probe mix, buster racing the sampler): "
        f"{json.dumps(scenarios['udf_storm']['probe_decisions'])}, every Live sample charging the "
        f"same deterministic udf_fuel_used {scenarios['udf_storm']['reference_udf_fuel_used']}.",
        f"- **Fuel bomb** — refused with ZERO collateral. `/healthz` p99 while the "
        f"runaway module is being refused: **{bomb_v['healthz_p99_ms']} ms** "
        f"({'TRIPPED' if bomb_v['adr_0012_trigger_tripped'] else 'holds'}); "
        f"SSE {'SURVIVED' if bomb_v['sse_survived'] else 'DROPPED'} "
        f"({scenarios['sse_under_fuelbomb']['trace_events_received']} trace events). "
        f"Tile path: 500 RFC 7807 fuel problem; preview `POST /result`: "
        f"{preview['status']} `{preview['code']}`.",
        "",
    ]
    return "\n".join(lines)


def cmd_report(directory: Path, started: int, json_out: Path, md_out: Path) -> None:
    p = PARAMS
    storm_probes = distill_storm_probes(directory / "storm-probes.json")
    udf_storm = distill_oha(
        directory / "udf-storm.json",
        f"3 heavy NDVI-UDF tiles (z12 x2, z11 — full-footprint warps through the "
        f"sandboxed module), c={p['UDF_STORM_CONNS']}, {p['UDF_STORM_DURATION']}, "
        f"cache cleared every 250 ms to stay on the Live+UDF path",
    )
    udf_storm["probe_decisions"] = storm_probes["decisions"]
    udf_storm["reference_udf_fuel_used"] = storm_probes["udf_fuel_used"]
    fuelbomb_storm = distill_oha(
        directory / "fuelbomb-storm.json",
        f"3 runaway-UDF tiles (the fuelbomb module past its fuel budget), "
        f"c={p['UDF_STORM_CONNS']}, {p['UDF_STORM_DURATION']}",
    )
    require_all_refused(fuelbomb_storm)
    healthz_storm = distill_oha(
        directory / "healthz-under-udf-storm.json",
        f"GET /healthz, c={p['UDF_HEALTHZ_CONNS']}, {p['UDF_HEALTHZ_LOAD_DURATION']}, "
        f"started {p['UDF_HEALTHZ_DELAY']}s into the UDF storm",
    )
    healthz_bomb = distill_oha(
        directory / "healthz-under-fuelbomb.json",
        f"GET /healthz, c={p['UDF_HEALTHZ_CONNS']}, {p['UDF_HEALTHZ_LOAD_DURATION']}, "
        f"started {p['UDF_HEALTHZ_DELAY']}s into the fuel-bomb refusals",
    )
    sse_storm = distill_sse(directory, "udf-storm")
    sse_bomb = distill_sse(directory, "fuelbomb")

    preview = json.loads((directory / "fuelbomb-preview.json").read_text())
    code = (preview.get("body") or {}).get("code") if isinstance(preview.get("body"), dict) else None
    if preview.get("status") != 400 or code != "ProcessGraphComplexity":
        sys.exit(
            f"FAIL: fuelbomb preview must be 400 ProcessGraphComplexity, "
            f"got {preview.get('status')} {code!r}"
        )
    tileprobe = json.loads((directory / "fuelbomb-tileprobe.json").read_text())
    tp_body = tileprobe.get("body") if isinstance(tileprobe.get("body"), dict) else {}
    detail = tp_body.get("detail", "") if isinstance(tp_body, dict) else ""
    if tileprobe.get("status") != 500 or "fuel" not in detail.lower():
        sys.exit(
            f"FAIL: fuelbomb tile must be a 500 RFC 7807 fuel problem, "
            f"got {tileprobe.get('status')}: {detail!r}"
        )
    # A healthy NDVI preview must still succeed — the refusal is the bomb's,
    # not the surface's.
    ndvi_preview = json.loads((directory / "ndvi-preview.json").read_text())
    if ndvi_preview.get("status") != 200:
        sys.exit(f"FAIL: the reference NDVI preview must render (200), got {ndvi_preview.get('status')}")

    baseline = {
        "schema": "swath-load-udf-baseline/1",
        "generated": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "git_sha": run_out("git", "rev-parse", "--short", "HEAD"),
        "oha_version": run_out("oha", "--version").split()[-1],
        "machine": machine_metadata(),
        "duration_seconds": int(time.time()) - started,
        "scenarios": {
            "udf_storm": udf_storm,
            "healthz_under_udf_storm": healthz_storm,
            "sse_under_udf_storm": sse_storm,
            "fuelbomb_storm": fuelbomb_storm,
            "healthz_under_fuelbomb": healthz_bomb,
            "sse_under_fuelbomb": sse_bomb,
        },
        "adr_0012": {
            "udf_storm": verdict(healthz_storm, sse_storm),
            "fuel_bomb": verdict(healthz_bomb, sse_bomb),
        },
        "fuelbomb_preview": {"status": preview["status"], "code": code},
        "fuelbomb_tile": {"status": tileprobe["status"], "detail": detail},
    }
    json_out.parent.mkdir(parents=True, exist_ok=True)
    json_out.write_text(json.dumps(baseline, indent=1) + "\n")
    md_out.write_text(render_markdown(baseline))
    # Loud verdict — the recipe completes so the evidence commits either way.
    for name, v in baseline["adr_0012"].items():
        state = "TRIPPED (maintainer lane-decision)" if v["adr_0012_trigger_tripped"] else "holds"
        print(
            f"ADR 0012 [{name}]: /healthz p99 {v['healthz_p99_ms']} ms "
            f"(< {v['healthz_p99_trigger_ms']}), SSE "
            f"{'survived' if v['sse_survived'] else 'DROPPED'} -> {state}"
        )
    print(f"baseline written: {json_out} + {md_out}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("params")
    publish = commands.add_parser("publish")
    publish.add_argument("--base", default="http://localhost:8080")
    publish.add_argument("which", choices=["ndvi", "fuelbomb"])
    probe = commands.add_parser("probe")
    probe.add_argument("--base", default="http://localhost:8080")
    probe.add_argument("--tile", required=True)
    probe.add_argument("--out", type=Path, required=True)
    preview = commands.add_parser("preview")
    preview.add_argument("--base", default="http://localhost:8080")
    preview.add_argument("which", choices=["ndvi", "fuelbomb"])
    preview.add_argument("--out", type=Path, required=True)
    tileprobe = commands.add_parser("tileprobe")
    tileprobe.add_argument("--base", default="http://localhost:8080")
    tileprobe.add_argument("--tile", required=True)
    tileprobe.add_argument("--out", type=Path, required=True)
    report = commands.add_parser("report")
    report.add_argument("--dir", type=Path, default=Path("target/load-udf"))
    report.add_argument("--started", type=int, required=True)
    report.add_argument("--json", type=Path, default=Path("docs/perf/load-udf-baseline.json"))
    report.add_argument("--md", type=Path, default=Path("docs/perf/load-udf-baseline.md"))
    args = parser.parse_args()
    if args.command == "params":
        cmd_params()
    elif args.command == "publish":
        cmd_publish(args.base, args.which)
    elif args.command == "probe":
        cmd_probe(args.base, args.tile, args.out)
    elif args.command == "preview":
        cmd_preview(args.base, args.which, args.out)
    elif args.command == "tileprobe":
        cmd_tileprobe(args.base, args.tile, args.out)
    else:
        cmd_report(args.dir, args.started, args.json, args.md)


if __name__ == "__main__":
    main()
