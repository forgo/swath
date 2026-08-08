#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# PreToolUse hook (Claude Code): blocks `gh pr merge` unless every check on the
# PR is green. Exit 2 = block, stderr is shown to the model. Machinery over
# vigilance: this exists because a grep pipeline once masked a red ci-ok
# (PR #57). `gh pr checks` exits non-zero on failing OR pending checks.
set -uo pipefail

cmd=$(jq -r '.tool_input.command // empty')
case "$cmd" in
  *"gh pr merge"*) ;;
  *) exit 0 ;;
esac

# PR number if present in the command; otherwise gh infers from the branch.
pr=$(printf '%s' "$cmd" | grep -oE 'gh pr merge[[:space:]]+[0-9]+' | grep -oE '[0-9]+$' || true)

if gh pr checks ${pr:+"$pr"} >/dev/null 2>&1; then
  exit 0
fi
echo "BLOCKED: 'gh pr checks ${pr:-<current branch>}' is not fully green (failing or pending)." >&2
echo "Run 'gh pr checks ${pr:-} --watch' bare and merge only on a zero exit code." >&2
exit 2
