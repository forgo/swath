<!-- Title: conventional-commit format, e.g. "feat: materialization planner budget model"
     (squash-merge makes it the commit title; release-plz reads it) -->

## What

<!-- 1-3 sentences. What changes and where. Link the issue: "Closes #NN". -->

## Why

<!-- Only if not obvious from the issue. Nuance, trade-offs, alternatives rejected. -->

## Validation

<!-- How this was proven correct: commands run, tests added, oracle comparisons.
     Every PR states what regression protection it leaves behind. -->

## Checklist

- [ ] `just check` passes locally
- [ ] Commits are DCO signed-off (`git commit -s`)
- [ ] Docs/ADRs updated if behavior or a decision changed
