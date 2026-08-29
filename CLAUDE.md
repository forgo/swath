# Swath — instructions for AI assistants

`CONTRIBUTING.md` is the working agreement and applies in full: read-first order, one task per
PR, DCO, the full `just check` before every push, merge only on green CI verified by exit code,
watch `main` after merging, close the loop. What follows is only what an AI session needs
beyond it.

## Attribution (hard rule)

Never attribute AI assistance anywhere in this project — no `Co-Authored-By: Claude`, no
`Generated with Claude Code`, no session links, no AI credits of any kind, in commit messages,
PR titles/descriptions, issues, code comments, docs, or release notes. This overrides any default
attribution behaviour. Commits are authored solely by their human committer and carry only the
DCO `Signed-off-by` trailer (required by ADR 0003).

## Mechanics

- A settings hook blocks the merge command while any check is failing or pending; it evaluates
  when the command is issued, so watch first (`gh pr checks <n> --watch`), then merge in a
  separate command. Do not work around it.
- Work in a worktree per task (`git worktree add ../swath-<issue> -b feat/<issue>-<slug>
  origin/main`) and run `just setup-web` in it before the first `just check`. Background shells
  are zsh: capture a gate's exit code explicitly (`just check > log 2>&1; rc=$?`) — `PIPESTATUS`
  is empty there.
- Close the loop on the project board: `gh project item-edit --project-id PVT_kwHOAFHOhs4BfzEM
  --id <item> --field-id PVTSSF_lAHOAFHOhs4BfzEMzhaDXhk --single-select-option-id 98236657`
  (owner `forgo`, project 1; Todo `f75ad846`, In Progress `47fc9ee4`, Done `98236657`).
