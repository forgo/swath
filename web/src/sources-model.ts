// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The Sources screen's pure model (#418, ADR 0030): what `GET /sources`
 * says, turned into rows the screen renders without deciding anything of
 * its own.
 *
 * Every state here traces to a served field. Nothing is inferred: a
 * source the server called `unknown` reads as unknown, freshness is
 * computed from the server's own `lastEvent` instant, and where the
 * server said nothing the row says `—`.
 */

/** The em dash an unknown is rendered as, everywhere in this module. */
export const UNKNOWN = "—";

export type SourceState = "unknown" | "watching" | "failing" | "stopped";

/** How a row reads at a glance — form as well as number, so a broken
 * source is visibly broken and not merely a smaller figure. */
export type SourceTone = "neutral" | "ok" | "danger";

export interface SourceRow {
  id: string;
  title: string;
  kind: string;
  /** `file`, `s3`, `https` — never a path. */
  scheme: string;
  /** `config` (this deployment's file owns it) or `api`. */
  origin: string;
  datasets: string[];
  /** The credential profile's name, when the source names one. Never a
   * value: the server has no field for one. */
  credentialProfile?: string;
  /** Whether that profile resolved the last time anything checked.
   * `undefined` both when the source names no profile and when nothing
   * has checked yet — `credentialNote` tells those apart. */
  credentialResolved?: boolean;
  /** Whether reading this source bills the reader (#424). */
  requesterPays?: boolean;
  /** Who agreed to be billed, when anyone has. */
  consentedBy?: string;
  state: SourceState;
  /** `undefined` when nothing has looked yet — not `false`. */
  reachable?: boolean;
  /** The server's own instant for the most recent event. */
  lastEvent?: string;
  lastError?: string;
  ingested: number;
  failures: number;
}

const STATES: readonly SourceState[] = ["unknown", "watching", "failing", "stopped"];

function stateOf(raw: unknown): SourceState {
  return STATES.find((state) => state === raw) ?? "unknown";
}

function countOf(raw: unknown): number {
  return typeof raw === "number" && Number.isFinite(raw) && raw >= 0 ? raw : 0;
}

function textOf(raw: unknown): string | undefined {
  return typeof raw === "string" && raw !== "" ? raw : undefined;
}

/** `GET /sources` → the rows. A row the response cannot identify is
 * dropped rather than rendered as a blank. */
export function parseSources(body: unknown): SourceRow[] {
  const items = (body as { sources?: unknown[] } | null)?.sources ?? [];
  const rows: SourceRow[] = [];
  for (const raw of items) {
    if (typeof raw !== "object" || raw === null) {
      continue;
    }
    const item = raw as Record<string, unknown>;
    const id = textOf(item["id"]);
    if (id === undefined) {
      continue;
    }
    const status = (item["status"] ?? {}) as Record<string, unknown>;
    const row: SourceRow = {
      id,
      title: textOf(item["title"]) ?? id,
      kind: textOf(item["kind"]) ?? UNKNOWN,
      scheme: textOf(item["scheme"]) ?? UNKNOWN,
      origin: textOf(item["origin"]) ?? UNKNOWN,
      datasets: Array.isArray(item["datasets"])
        ? item["datasets"].filter((each): each is string => typeof each === "string")
        : [],
      state: stateOf(status["state"]),
      ingested: countOf(status["ingested"]),
      failures: countOf(status["failures"]),
    };
    const profile = textOf(item["credentialProfile"]);
    if (profile !== undefined) {
      row.credentialProfile = profile;
    }
    if (item["requesterPays"] === true) {
      row.requesterPays = true;
    }
    const consented = textOf(item["consentedBy"]);
    if (consented !== undefined) {
      row.consentedBy = consented;
    }
    // Tri-state on purpose: `null` means "named but never checked", and
    // an absent field means "names no profile at all".
    if (typeof item["credentialResolved"] === "boolean") {
      row.credentialResolved = item["credentialResolved"];
    }
    // Tri-state on purpose: absent is not false.
    if (typeof status["reachable"] === "boolean") {
      row.reachable = status["reachable"];
    }
    const lastEvent = textOf(status["lastEvent"]);
    if (lastEvent !== undefined) {
      row.lastEvent = lastEvent;
    }
    const lastError = textOf(status["lastError"]);
    if (lastError !== undefined) {
      row.lastError = lastError;
    }
    rows.push(row);
  }
  return rows;
}

/** What the row's state reads as, in the words the operator uses. */
export function stateLabel(row: SourceRow): string {
  switch (row.state) {
    case "watching":
      return "watching";
    case "failing":
      return "not answering";
    case "stopped":
      return "stopped";
    default:
      // Nothing has looked yet. Saying so is the honest answer; "healthy"
      // would be the invented one.
      return "no reports yet";
  }
}

/** The row's tone, so a broken source is broken in form as well as in
 * number (the design language's rule). */
export function stateTone(row: SourceRow): SourceTone {
  if (row.state === "failing") {
    return "danger";
  }
  return row.state === "watching" ? "ok" : "neutral";
}

/**
 * How long ago the last event was, in plain words, computed from the
 * **server's** instant and the client's `now` — never from the client's
 * clock alone, and never at all when the server named no instant.
 *
 * A future instant (clock skew between the two machines) reads as "just
 * now" rather than as a negative age: the skew is the client's problem
 * and inventing a number from it would be worse than rounding it away.
 */
export function freshness(row: SourceRow, now: number): string {
  if (row.lastEvent === undefined) {
    return UNKNOWN;
  }
  const at = Date.parse(row.lastEvent);
  if (Number.isNaN(at)) {
    return UNKNOWN;
  }
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 60) {
    return "just now";
  }
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) {
    return `${minutes} min ago`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 48) {
    return `${hours} h ago`;
  }
  return `${Math.round(hours / 24)} d ago`;
}

/** The counts line: what this source has actually done. */
export function countsLine(row: SourceRow): string {
  const ingested = `${row.ingested} ingested`;
  return row.failures === 0 ? ingested : `${ingested} · ${row.failures} failed`;
}

/** What to say about the source's credential, in plain words — or
 * nothing at all when it names no profile. A named-but-unchecked profile
 * says so; it is neither working nor broken, and claiming either would be
 * an invention. The profile's **name** appears here and its value never
 * can: the server has no field for one. */
export function credentialNote(row: SourceRow): string | undefined {
  if (row.credentialProfile === undefined) {
    return undefined;
  }
  const state =
    row.credentialResolved === undefined
      ? "not checked yet"
      : row.credentialResolved
        ? "resolved"
        : "did not resolve";
  return `credential ${row.credentialProfile} · ${state}`;
}

/** What to say about billing, or nothing at all for a source that does
 * not bill the reader. Names who agreed, or says plainly that nobody has
 * and the source will not be read until someone does. **Never a price**:
 * Swath does not know the operator's rate card, and a wrong figure is
 * worse than none. */
export function billingNote(row: SourceRow): string | undefined {
  if (row.requesterPays !== true) {
    return undefined;
  }
  return row.consentedBy === undefined
    ? "requester-pays · not read until someone agrees to be billed"
    : `requester-pays · ${row.consentedBy} agreed to be billed`;
}

/** Whether the screen should offer the first-run action: nothing is
 * watching anything, so there is nothing to look at yet. */
export function isFirstRun(rows: readonly SourceRow[]): boolean {
  return rows.length === 0 || rows.every((row) => row.ingested === 0);
}
