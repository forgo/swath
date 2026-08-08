# Contributing to Swath

Thanks for your interest! Swath is pre-alpha and moving fast; this guide is deliberately short.

## Ground rules

- **Read first:** [`docs/REQUIREMENTS.md`](docs/REQUIREMENTS.md) (the north star), then
  [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and [`docs/ENGINEERING.md`](docs/ENGINEERING.md)
  (how we build). Decisions live in [`docs/decisions/`](docs/decisions/) — ADRs are immutable;
  supersede, never edit.
- **DCO:** every commit must be signed off (`git commit -s`), certifying the
  [Developer Certificate of Origin](https://developercertificate.org/). Enforced by a check.
- **Code of Conduct:** [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

## Workflow

```bash
just setup   # one-time: pinned dev tools + optional pre-commit hook
just check   # the full local gate — identical to CI (fmt, clippy, tests, deny, zizmor)
```

1. Branch from `main`; keep one logical change per PR.
2. PR titles are [conventional commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`,
   `docs:`, `chore:`, …) — squash-merge makes your title the commit title, and release tooling
   reads that history. A check enforces the format.
3. Fill in the PR template: what, why (nuance only), and how it was validated. Every PR states
   what regression protection it leaves behind.
4. CI (`ci-ok`) must be green. If CI disagrees with your local run, that's a finding — say so in
   the PR rather than working around it.

## What to work on

The [Swath Roadmap project](https://github.com/users/forgo/projects/1) is the ordered backlog;
issues are numbered in suggested execution order and carry explicit validation criteria. Issues
labeled `elaborate` need a breakdown discussion before code.
