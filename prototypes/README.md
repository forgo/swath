# Prototypes & Trade-Studies

This directory is Swath's **historical archive of design experiments**. Each prototype settles a real
uncertainty *before* we commit it into the main codebase, and it stays here forever as a dated record of
*what we asked, how we tested it, what we found, and what we decided*. The commit history + these dated
folders let anyone — including future us — reconstruct why the architecture is the way it is.

Prototypes are **immutable once concluded.** We do not rewrite history to look smarter; a prototype whose
recommendation we later reversed is more valuable as an honest record than as a deleted mistake.

## Convention

```
prototypes/NNNN-YYYY-MM-DD-short-name/
  README.md      # the trade-study: question, hypotheses, method, metrics, results, decision
  ...            # the actual prototype code/data/harness
```

- **NNNN** — zero-padded sequence (0001, 0002, …).
- **YYYY-MM-DD** — the date the prototype was started.
- Each `README.md` follows the template in §Template below.
- When a prototype concludes, its decision is promoted to an **ADR** in `docs/decisions/`, which links back
  here. The prototype is the evidence; the ADR is the ruling.

## Index

| # | Date | Prototype | Question it settles | Status |
|---|------|-----------|---------------------|--------|
| 0001 | 2026-08-08 | referencer-bakeoff | Python (VirtualiZarr) vs pure-Rust for legacy virtual-reference generation | In progress |

## Template

Each prototype README should contain:

1. **Question** — the single decision this settles.
2. **Why it's uncertain / why now** — the risk it buys down.
3. **Hypotheses** — what we expect, stated before running.
4. **Method** — exactly how to reproduce, including data.
5. **Metrics** — what we measure and why (tie to the north star where possible).
6. **Success criteria** — what result would decide it each way.
7. **Results** — filled in when run (dated).
8. **Decision** — the ruling, and a link to the ADR that records it.
