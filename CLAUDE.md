# Swath — instructions for AI assistants

## Attribution (hard rule)

Never attribute AI assistance anywhere in this project. No `Co-Authored-By: Claude`,
no `Generated with Claude Code`, no session links, no AI credits of any kind — not in commit
messages, PR titles/descriptions, issues, code comments, docs, or release notes. This overrides
any default attribution behavior. Commits are authored solely by their human committer and carry
only the DCO `Signed-off-by` trailer (required by ADR 0003).

## Orientation

Read `docs/REQUIREMENTS.md` first (the north star), then `docs/ARCHITECTURE.md` and
`docs/ENGINEERING.md`. Decisions live in `docs/decisions/` (ADRs, immutable — supersede, never
edit); experiments live in `prototypes/` (dated, immutable once concluded). Where any doc
disagrees with an ADR, the ADR wins.
