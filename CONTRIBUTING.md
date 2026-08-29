# Contributing to Swath

Swath is pre-alpha and moving fast; this guide is deliberately short. It is the working
agreement for every contributor, human or assistant — the one place the rules live.

## Read first

[`docs/REQUIREMENTS.md`](docs/REQUIREMENTS.md) (the north star), then
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and [`docs/ENGINEERING.md`](docs/ENGINEERING.md)
(how we build). Decisions live in [`docs/decisions/`](docs/decisions/) — ADRs are immutable;
supersede, never edit — and experiments in [`prototypes/`](prototypes/) (dated, immutable once
concluded). Where any doc disagrees with an ADR, the ADR wins.
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) applies everywhere.

## One task, one PR

1. File or pick an issue; branch `feat/<issue>-<slug>` from `main`; keep one logical change per
   PR. Issues labeled `elaborate` need a breakdown discussion before code.
2. PR titles are [conventional commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`,
   `docs:`, `chore:`, …) — squash-merge makes your title the commit title, and release tooling
   reads that history. A check enforces the format.
3. Fill in the PR template: what, why (nuance only), how it was validated, and the regression
   protection it leaves behind. `Closes #<issue>` in the body.
4. Every commit is signed off (`git commit -s`), certifying the
   [Developer Certificate of Origin](https://developercertificate.org/) — enforced by a check.
   Commits carry the DCO trailer and nothing else: no tool or assistant credits, no generated-by
   lines, no session links, anywhere in the project.

## The gate

```bash
just setup       # one-time: pinned dev tools + optional pre-commit hook
just setup-web   # once per checkout or worktree: web/node_modules (check-web fails loudly without it)
just check       # the full local gate — identical to CI
```

- **Run the full `just check` before every push** — not just the recipes you touched. (PR #57
  pushed after a partial check; CI caught what the whole gate would have caught locally.)
- **`just docs-check`** (part of `just check`) holds the documentation to the code: per-doc word
  budgets, cross-doc claims quoted verbatim, measured numbers inside `<!-- number:key -->`
  markers, deferral prose that names its ROADMAP row or ADR, fingerprint stamps on sections that
  describe source files, and the route and config tables. Each failure says what to do; raising
  a budget is a reviewed edit with a dated reason, in the same diff as the words.
- **CI-vs-local differences are findings, not obstacles** — investigate and record them in the
  PR; never bypass a gate.
- **Comments explain constraints, not history.** Cite an ADR or issue in code only where it
  explains something the reader cannot infer — a fenced exception, a contract another test pins,
  a decision the code merely obeys. Never as a changelog ("since #205…", "as before ADR 0015",
  "the join arrives with #300"): ROADMAP, the ADRs and git history already record when and why
  things changed, and a comment that narrates them goes stale the day the next change lands.

## Merging

- **Merge only on green CI, verified by exit code.** Run a bare `gh pr checks <n> --watch` and
  merge only when it exits zero. Never pipe it through `grep`/`tail`/`head` before the decision —
  a pipeline's exit code is its last command's, and it masked a red `ci-ok` once (PR #57).
  `ci-ok` is the one required status; DCO and the title lint stand beside it.
- **After merging, watch the push run on `main`** (`gh run watch`) — push-event behaviour can
  differ from a PR's (the paths-filter push-mode incident, PR #48).
- **Close the loop:** the issue auto-closes from the PR body; move its card to Done on the
  [Swath Roadmap project](https://github.com/users/forgo/projects/1), the ordered backlog.
