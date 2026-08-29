# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

import datetime
import json
import pathlib
import platform
import subprocess

def sh(*args: str) -> str:
    return subprocess.run(args, capture_output=True, text=True, check=True).stdout.strip()

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

benches = []
for est_path in sorted(pathlib.Path("target/criterion").glob("**/new/estimates.json")):
    meta = json.loads((est_path.parent / "benchmark.json").read_text())
    est = json.loads(est_path.read_text())
    benches.append(
        {
            "id": meta["full_id"],
            "median_ns": round(est["median"]["point_estimate"], 1),
            "mad_ns": round(est["median_abs_dev"]["point_estimate"], 1),
        }
    )
benches.sort(key=lambda b: b["id"])
assert benches, "no criterion estimates found under target/criterion"

out = {
    "schema": "swath-bench-baseline/1",
    "captured": datetime.date.today().isoformat(),
    "git_sha": sh("git", "rev-parse", "HEAD"),
    "rustc": sh("rustc", "--version"),
    "machine": {"model": model, "arch": platform.machine(), "os": system},
    "benches": benches,
}
path = pathlib.Path("docs/perf/bench-baseline.json")
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(out, indent=2) + "\n")
print(f"wrote {path} ({len(benches)} benches)")
