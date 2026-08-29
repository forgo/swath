# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

import json
import pathlib

perf = pathlib.Path("docs/perf")
bench = json.loads((perf / "bench-baseline.json").read_text())
load = json.loads((perf / "load-baseline.json").read_text())
i2p = json.loads((perf / "i2p-baseline.json").read_text())
ref = json.loads((perf / "referencer-baseline.json").read_text())
temporal = json.loads((perf / "temporal-baseline.json").read_text())
udf = json.loads((perf / "load-udf-baseline.json").read_text())

def human_ns(ns: float) -> str:
    if ns < 1_000:
        return f"{ns:.1f} ns"
    if ns < 1_000_000:
        return f"{ns / 1_000:.2f} us"
    return f"{ns / 1_000_000:.2f} ms"

blocks = {}

blocks["stamp"] = "\n".join(
    [
        "| instrument | artifact | measured at (git sha) | date |",
        "|---|---|---|---|",
        f"| ingest-to-pixel (`just perf-i2p`) | `docs/perf/i2p-baseline.json` | `{i2p['git_sha'][:7]}` | {i2p['timestamp']} |",
        f"| stage benches (`just bench-baseline`) | `docs/perf/bench-baseline.json` | `{bench['git_sha'][:7]}` | {bench['captured']} |",
        f"| load scenarios (`just load`) | `docs/perf/load-baseline.json` | `{load['git_sha']}` | {load['generated']} |",
        f"| referencer (`just perf-referencer`) | `docs/perf/referencer-baseline.json` | `{ref['git_sha'][:7]}` | {ref['captured']} |",
        f"| temporal + overview (`just load-temporal`) | `docs/perf/temporal-baseline.json` | `{temporal['git_sha']}` | {temporal['generated']} |",
        f"| run_udf load (`just load-udf`) | `docs/perf/load-udf-baseline.json` | `{udf['git_sha']}` | {udf['generated']} |",
    ]
)

blocks["i2p"] = "\n".join(
    [
        "| metric | measured | enforced budget |",
        "|---|---:|---:|",
        f"| ingest_to_pixel_ms | {i2p['value']} ms | {i2p['budget_ms']} ms |",
    ]
)

rows = ["| bench | median | MAD |", "|---|---:|---:|"]
for b in bench["benches"]:
    rows.append(f"| {b['id']} | {human_ns(b['median_ns'])} | {human_ns(b['mad_ns'])} |")
blocks["bench"] = "\n".join(rows)

rows = [
    "| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |",
    "|---|---:|---:|---:|---:|---:|---:|---:|",
]
labels = {
    "healthz_idle": "healthz — idle baseline",
    "hot_cache_storm": "(a) hot-cache tile storm",
    "cold_live_burst": "(b) cold live-render burst",
    "mixed_tile_storm": "(c) mixed tile storm",
    "healthz_under_warps": "(c) healthz UNDER WARPS",
}
for key, label in labels.items():
    s = load["scenarios"][key]
    rows.append(
        f"| {label} | {s['requests']} | {s['errors']} | {s['rps']} "
        f"| {s['p50_ms']} | {s['p95_ms']} | {s['p99_ms']} | {s['max_ms']} |"
    )
sse = load["scenarios"]["sse_under_warps"]
rows.append(
    f"\nSSE `/traces` under the mixed storm: survived the {sse['window_seconds']}s window: "
    f"**{'yes' if sse['survived_window'] else 'NO'}** "
    f"({sse['trace_events_received']} trace events received)."
)
blocks["load"] = "\n".join(rows)

rows = [
    "| generator | command | cold | warm (median) |",
    "|---|---|---:|---:|",
]
granule_name = ref["granule"]["name"]
for name, g in ref["generators"].items():
    cmd = " ".join(
        f"<path-to>/{granule_name}" if arg.endswith(granule_name) else arg
        for arg in g["command"].split()
    )
    rows.append(f"| {name} | `{cmd}` | {g['cold_ms']} ms | {g['warm_median_ms']} ms |")
rows.append(
    f"\nWarm-generation ratio (sidecar / Rust): "
    f"**~{ref['warm_ratio_rust_advantage']}x** in Rust's favor "
    f"({ref['runs']} runs; {ref['timing']})."
)
blocks["referencer"] = "\n".join(rows)

rows = [
    "| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |",
    "|---|---:|---:|---:|---:|---:|---:|---:|",
]
labels = {
    "frames_cold": "(d) frame loop, cold (all Live)",
    "frames_hot": "(d) frame loop, hot (all cache hits)",
    "overview_live_z12": "(e) z12 — Live (full resolution)",
    "overview_embedded_z10": "(e) z10 pre-materialize (embedded ov. ×2)",
    "overview_pyramid_z11": "(e) z11 post-materialize (pyramid ov. ×2)",
    "overview_pyramid_z10": "(e) z10 post-materialize (pyramid ov. ×4)",
}
for key, label in labels.items():
    s = temporal["scenarios"][key]
    rows.append(
        f"| {label} | {s['requests']} | {s['errors']} | {s['rps']} "
        f"| {s['p50_ms']} | {s['p95_ms']} | {s['p99_ms']} | {s['max_ms']} |"
    )
ladder = "; ".join(
    f"{key.split('_', 1)[1]} {json.dumps(temporal['scenarios'][key]['sse_decisions'])}"
    for key in labels
    if key.startswith("overview_")
)
rows.append(
    f"\nFrame decisions (`x-swath-trace`): cold "
    f"{json.dumps(temporal['scenarios']['frames_cold']['decisions'])}, hot "
    f"{json.dumps(temporal['scenarios']['frames_hot']['decisions'])}. "
    f"Overview-rung decisions (SSE envelopes, level included): {ladder}."
)
blocks["temporal"] = "\n".join(rows)

rows = [
    "| scenario | requests | errors | rps | p50 ms | p95 ms | p99 ms | max ms |",
    "|---|---:|---:|---:|---:|---:|---:|---:|",
]
labels = {
    "udf_storm": "(u) UDF mixed storm (Live NDVI + cache, buster on)",
    "healthz_under_udf_storm": "(u) healthz UNDER the UDF storm",
    "fuelbomb_storm": "(f) fuel-bomb storm — every tile refused",
    "healthz_under_fuelbomb": "(f) healthz UNDER the fuel-bomb refusals",
}
for key, label in labels.items():
    s = udf["scenarios"][key]
    rows.append(
        f"| {label} | {s['requests']} | {s['errors']} | {s['rps']} "
        f"| {s['p50_ms']} | {s['p95_ms']} | {s['p99_ms']} | {s['max_ms']} |"
    )
storm_v = udf["adr_0012"]["udf_storm"]
bomb_v = udf["adr_0012"]["fuel_bomb"]
storm_sse = udf["scenarios"]["sse_under_udf_storm"]
bomb_sse = udf["scenarios"]["sse_under_fuelbomb"]
rows.append(
    f"\nADR 0012 signals — UDF storm: `/healthz` p99 "
    f"{storm_v['healthz_p99_ms']} ms (trigger {storm_v['healthz_p99_trigger_ms']} ms), "
    f"SSE {'survived' if storm_v['sse_survived'] else 'DROPPED'} "
    f"({storm_sse['trace_events_received']} events), "
    f"tripped: **{'yes' if storm_v['adr_0012_trigger_tripped'] else 'no'}**. "
    f"Fuel bomb (refused, zero collateral): `/healthz` p99 "
    f"{bomb_v['healthz_p99_ms']} ms, SSE "
    f"{'survived' if bomb_v['sse_survived'] else 'DROPPED'} "
    f"({bomb_sse['trace_events_received']} events); tile 500 RFC 7807 fuel, "
    f"preview {udf['fuelbomb_preview']['status']} `{udf['fuelbomb_preview']['code']}`; "
    f"tripped: **{'yes' if bomb_v['adr_0012_trigger_tripped'] else 'no'}**."
)
blocks["udf-load"] = "\n".join(rows)

doc = pathlib.Path("docs/PERFORMANCE.md")
text = doc.read_text()
for name, body in blocks.items():
    begin = f"<!-- table:{name} (generated by `just perf-doc` — edit the artifact, not this block) -->"
    end = f"<!-- /table:{name} -->"
    head, _, rest = text.partition(begin)
    assert rest, f"marker not found in PERFORMANCE.md: {begin}"
    _, _, tail = rest.partition(end)
    assert tail or rest.endswith(end), f"end marker not found: {end}"
    text = head + begin + "\n" + body + "\n" + end + tail
doc.write_text(text)
print(f"regenerated {len(blocks)} generated blocks in {doc}")

# --- Inline headline-number markers (issue #174) ---
# One rendered string per key; docs quote a key inside
# `<!-- number:<key> -->…<!-- /number:<key> -->` and this recipe owns the
# content. Rendering rules must match docs_check/numbers.rs exactly.

def sig2(v: float) -> int:
    # Two significant figures, as an integer (23.33 -> 23, 660.61 -> 660).
    # Half rounds away from zero (matches Rust's f64::round in the
    # docs-check twin, docs_check/numbers.rs).
    import math
    scale = 10 ** max(0, math.floor(math.log10(abs(v))) - 1)
    return int(v / scale + 0.5) * scale

def comma(s: str) -> str:
    whole, _, frac = s.partition(".")
    grouped = f"{int(whole):,}"
    return f"{grouped}.{frac}" if frac else grouped

# The 2-CPU load evidence is a committed markdown table; keep its figures
# as written (strings), never re-formatted through floats.
twocpu_rows = {}
for line in pathlib.Path("docs/perf/load-2cpu-16.7-evidence.md").read_text().splitlines():
    cells = [c.strip() for c in line.strip().strip("|").split("|")]
    if len(cells) == 8:
        twocpu_rows[cells[0]] = cells
hot2 = twocpu_rows["(a) hot-cache tile storm"]
cold2 = twocpu_rows["(b) cold live-render burst"]
healthz2 = twocpu_rows["(c) healthz UNDER WARPS"]

numbers = {
    "i2p-ms": f"{i2p['value']} ms",
    "i2p-sha": f"`{i2p['git_sha'][:7]}`",
    "hot-p50-approx": f"~{sig2(load['scenarios']['hot_cache_storm']['p50_ms'])} ms",
    "cold-p50-approx": f"~{sig2(load['scenarios']['cold_live_burst']['p50_ms'])} ms",
    "ref-warm-ms": f"{ref['generators']['referencer-rs']['warm_median_ms']} ms",
    "ref-sidecar-warm-ms": f"{ref['generators']['virtualizarr-sidecar']['warm_median_ms']} ms",
    "ref-ratio": f"{ref['warm_ratio_rust_advantage']}×",
    "ref-ratio-approx": f"~{int(ref['warm_ratio_rust_advantage'] + 0.5)}×",
    "2cpu-hot-p50": f"{hot2[4]} ms",
    "2cpu-hot-p95": f"{hot2[5]} ms",
    "2cpu-hot-rps": f"{comma(hot2[3])} req/s",
    "2cpu-cold-p50": f"{cold2[4]} ms",
    "2cpu-healthz-p99": f"{healthz2[6]} ms",
    "frame-cold-p50-approx": f"~{sig2(temporal['scenarios']['frames_cold']['p50_ms'])} ms",
    "frame-hot-p50-approx": f"~{sig2(temporal['scenarios']['frames_hot']['p50_ms'])} ms",
    "ov-live-p50-approx": f"~{sig2(temporal['scenarios']['overview_live_z12']['p50_ms'])} ms",
    "ov-pyramid-p50-approx": f"~{sig2(temporal['scenarios']['overview_pyramid_z10']['p50_ms'])} ms",
    "materialize-ms": f"{temporal['materialize']['wall_ms']} ms",
    "udf-storm-healthz-p99": f"{udf['scenarios']['healthz_under_udf_storm']['p99_ms']} ms",
    "udf-fuelbomb-healthz-p99": f"{udf['scenarios']['healthz_under_fuelbomb']['p99_ms']} ms",
}

marker_docs = [
    "README.md",
    "crates/swath-referencer/README.md",
    "docs/DEMO.md",
    "docs/CHARTER.md",
    "docs/REQUIREMENTS.md",
    "docs/PERFORMANCE.md",
    "docs/ARCHITECTURE.md",
    "docs/COMPARISON.md",
    "docs/media/wedge.notes.md",
]
def fill(rel: str, segment: str) -> tuple[str, int]:
    out = []
    rest = segment
    n = 0
    while True:
        head, sep, rest = rest.partition("<!-- number:")
        out.append(head)
        if not sep:
            break
        key, sep, rest = rest.partition(" -->")
        assert sep, f"{rel}: unterminated `<!-- number:{key}` begin marker"
        assert key in numbers, f"{rel}: unknown number marker key `{key}`"
        end = f"<!-- /number:{key} -->"
        _, sep, rest = rest.partition(end)
        assert sep, f"{rel}: missing `{end}`"
        out.append(f"<!-- number:{key} -->{numbers[key]}{end}")
        n += 1
    return "".join(out), n

total = 0
for rel in marker_docs:
    path = pathlib.Path(rel)
    # Fenced code blocks (odd segments) are examples, not live markers.
    segments = path.read_text().split("```")
    for i in range(0, len(segments), 2):
        segments[i], n = fill(rel, segments[i])
        total += n
    path.write_text("```".join(segments))
print(f"filled {total} inline number markers across {len(marker_docs)} docs")
