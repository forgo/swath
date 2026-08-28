// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The command palette's pure half (issue #292): a command is a label, a
 * group and a run function; a query is matched by subsequence with a
 * small scoring so "ndvi" finds "Show layer: HLS NDVI" above "Zoom to
 * data". No DOM here.
 */

export interface Command {
  id: string;
  label: string;
  /** Shown as a hint after the label (a layer id, a shortcut). */
  hint?: string | undefined;
  group: string;
  /** Extra words the query may match (ids, aliases). */
  keywords?: readonly string[] | undefined;
  run: () => void;
}

export interface Match {
  command: Command;
  score: number;
  /** Indices into `label` that matched, for highlighting. */
  positions: number[];
}

/** Subsequence match of `query` in `text`, case-insensitive. Score rewards
 * a match at the start of a word, adjacent characters and a short text;
 * `undefined` when not every query character appears in order. */
export function subsequenceScore(
  query: string,
  text: string,
): { score: number; positions: number[] } | undefined {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (q === "") {
    return { score: 0, positions: [] };
  }
  const positions: number[] = [];
  let score = 0;
  let from = 0;
  let previous = -2;
  for (const ch of q) {
    const at = t.indexOf(ch, from);
    if (at === -1) {
      return undefined;
    }
    positions.push(at);
    const wordStart = at === 0 || /[\s:/_-]/.test(t[at - 1] ?? "");
    score += wordStart ? 3 : 1;
    if (at === previous + 1) {
      score += 2; // adjacency
    }
    previous = at;
    from = at + 1;
  }
  // Prefer compact matches and short labels.
  const span = (positions[positions.length - 1] ?? 0) - (positions[0] ?? 0) + 1;
  score += Math.max(0, 10 - (span - q.length));
  score -= Math.min(5, Math.floor(t.length / 20));
  return { score, positions };
}

/** Commands matching `query`, best first; an empty query lists all in
 * their given order. Keywords count, but a label match ranks higher. */
export function matchCommands(commands: readonly Command[], query: string): Match[] {
  const trimmed = query.trim();
  if (trimmed === "") {
    return commands.map((command) => ({ command, score: 0, positions: [] }));
  }
  const out: Match[] = [];
  for (const command of commands) {
    const label = subsequenceScore(trimmed, command.label);
    const hint = command.hint ? subsequenceScore(trimmed, command.hint) : undefined;
    const keyword = (command.keywords ?? [])
      .map((k) => subsequenceScore(trimmed, k))
      .filter((m): m is { score: number; positions: number[] } => m !== undefined)
      .sort((a, b) => b.score - a.score)[0];
    const best = [
      label,
      hint && { score: hint.score - 1, positions: [] },
      keyword && { score: keyword.score - 2, positions: [] },
    ]
      .filter((m): m is { score: number; positions: number[] } => m !== undefined)
      .sort((a, b) => b.score - a.score)[0];
    if (best) {
      out.push({ command, score: best.score, positions: label ? label.positions : [] });
    }
  }
  return out.sort((a, b) => b.score - a.score || a.command.label.localeCompare(b.command.label));
}
