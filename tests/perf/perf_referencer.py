# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

import datetime
import json
import os
import pathlib
import platform
import statistics
import subprocess
import time

def sh(*args: str) -> str:
    return subprocess.run(args, capture_output=True, text=True, check=True).stdout.strip()

granule = os.environ["SWATH_PERF_GRANULE"]
runs = int(os.environ["SWATH_PERF_RUNS"])
assert runs >= 2, "need at least a cold run and one warm run"
out_manifest = "target/referencer/perf-rs.vmanifest.json"
generators = {
    "referencer-rs": [
        "target/release/swath", "ingest", "reference", granule,
        "--output", out_manifest,
    ],
    "virtualizarr-sidecar": ["python/.venv/bin/swath-referencer", granule],
}

def measure(cmd: list[str]) -> list[float]:
    times = []
    for _ in range(runs):
        t0 = time.perf_counter()
        subprocess.run(
            cmd, check=True,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        times.append(round((time.perf_counter() - t0) * 1000, 1))
    return times

results = {}
for name, cmd in generators.items():
    samples = measure(cmd)
    results[name] = {
        "command": " ".join(cmd),
        "cold_ms": samples[0],
        "warm_median_ms": round(statistics.median(samples[1:]), 1),
        "runs_ms": samples,
    }
    print(f"{name}: cold {samples[0]} ms, warm median {results[name]['warm_median_ms']} ms")

system = platform.system()
if system == "Darwin":
    model = sh("sysctl", "-n", "machdep.cpu.brand_string")
else:
    model = next(
        (
            line.split(":", 1)[1].strip()
            for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines()
            if line.startswith("model name")
        ),
        platform.machine(),
    )

ratio = (
    results["virtualizarr-sidecar"]["warm_median_ms"]
    / results["referencer-rs"]["warm_median_ms"]
)
out = {
    "schema": "swath-referencer-baseline/1",
    "captured": datetime.date.today().isoformat(),
    "git_sha": sh("git", "rev-parse", "HEAD"),
    "rustc": sh("rustc", "--version"),
    "python": platform.python_version(),
    "machine": {"model": model, "arch": platform.machine(), "os": system},
    "granule": {
        "name": pathlib.Path(granule).name,
        "bytes": pathlib.Path(granule).stat().st_size,
    },
    "timing": "full-process wall clock; run 1 = cold, warm = median of the rest",
    "runs": runs,
    "generators": results,
    "warm_ratio_rust_advantage": round(ratio, 1),
}
path = pathlib.Path("docs/perf/referencer-baseline.json")
path.write_text(json.dumps(out, indent=2) + "\n")
print(f"wrote {path} (warm ratio {out['warm_ratio_rust_advantage']}x)")
