# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

"""Second-capture stability gate for `just screenshots` (issue #112).

Usage: python3 tests/screenshots/verify_stable.py FIRST_DIR SECOND_DIR

FIRST_DIR is the committed capture (docs/media/screenshots), SECOND_DIR a
re-capture of the same suite against the same stack from the same cold
cache state. The gate proves the shots are reproducible, not hand-made:

- both runs produced exactly the manifest's shot list (>= 8 shots),
- every pair passes swath-testkit's pdiff under the per-shot policy the
  capture suite declared in shots.json (a dimension mismatch is always a
  hard pdiff failure, so geometry is covered too).

Timings and wall-clock text are the only budgeted differences; the policy
per shot lives with the shot's definition (web/screenshots/capture.ts).
"""

import json
import pathlib
import subprocess
import sys

PDIFF = ["cargo", "run", "--quiet", "-p", "swath-testkit", "--bin", "pdiff", "--"]


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: verify_stable.py FIRST_DIR SECOND_DIR", file=sys.stderr)
        return 2
    first, second = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])

    manifest = json.loads((first / "shots.json").read_text())
    shots = manifest["shots"]
    expected = sorted(shot["file"] for shot in shots)
    if len(expected) < 8:
        print(f"FAIL: only {len(expected)} shots captured; the suite promises >= 8")
        return 1
    for name, directory in (("first", first), ("second", second)):
        actual = sorted(p.name for p in directory.glob("*.png"))
        if actual != expected:
            print(f"FAIL: {name} run's shot list does not match the manifest")
            print(f"  expected: {expected}")
            print(f"  actual:   {actual}")
            return 1

    failures = []
    for shot in shots:
        file = shot["file"]
        policy = [
            "--tolerance",
            str(shot["tolerance"]),
            "--max-bad-frac",
            str(shot["maxBadFrac"]),
        ]
        print(f"pdiff {file} (tolerance {shot['tolerance']}, max-bad-frac {shot['maxBadFrac']})")
        result = subprocess.run([*PDIFF, *policy, str(first / file), str(second / file)])
        if result.returncode != 0:
            failures.append(file)

    if failures:
        print(f"FAIL: {len(failures)} shot(s) did not reproduce: {', '.join(failures)}")
        return 1
    print(f"screenshot stability PASS: {len(shots)} shots reproduced within policy")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
