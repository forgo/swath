# Swath — instructions for AI assistants

## Attribution (hard rule)

Never attribute AI assistance anywhere in this project. No `Co-Authored-By: Claude`,
no `Generated with Claude Code`, no session links, no AI credits of any kind — not in commit
messages, PR titles/descriptions, issues, code comments, docs, or release notes. This overrides
any default attribution behavior. Commits are authored solely by their human committer and carry
only the DCO `Signed-off-by` trailer (required by ADR 0003).

## Working agreement (hard rules for any AI session)

1. **PR-per-task workflow.** Branch `feat/<issue>-<slug>` from `main`; one logical change per PR;
   PR titles are conventional commits (squash-merge makes them the commit titles release tooling
   reads); PR bodies follow the template — concise, nuance only, validation stated.
2. **Full `just check` before every push.** Not just the recipes you touched — the whole gate.
   (Incident: PR #57 pushed after a partial check; the reuse job caught what `just reuse` would
   have caught locally.)
3. **Never merge without green CI, verified by EXIT CODE.** Run bare `gh pr checks <n> --watch`
   and gate the merge on its exit status. Never pipe it through grep/tail/head before the merge
   decision — a pipeline's exit code comes from the last command and masks failures. (Incident:
   PR #57 merged with `ci-ok` red for exactly this reason.) A settings hook also blocks
   `gh pr merge` mechanically while checks aren't green; do not work around it.
4. **After merging, watch the push run on `main`** (`gh run watch`) — push-event behavior can
   differ from PRs (see the paths-filter push-mode incident, PR #48).
5. **Commits are DCO signed-off** (`git commit -s`) and carry no AI attribution (rule above).
6. **Close the loop:** verify the issue auto-closed, set the project item to Done
   (project: `gh project item-edit`, owner `forgo`, project 1).
7. **CI-vs-local differences are findings, not obstacles** — investigate and document them in
   the PR; never bypass.

## Orientation

Read `docs/REQUIREMENTS.md` first (the north star), then `docs/ARCHITECTURE.md` and
`docs/ENGINEERING.md`. Decisions live in `docs/decisions/` (ADRs, immutable — supersede, never
edit); experiments live in `prototypes/` (dated, immutable once concluded). Where any doc
disagrees with an ADR, the ADR wins.
