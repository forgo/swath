// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The public register (#420): endpoints this deployment offers to import
 * from, each with whether it can actually be reached.
 *
 * The register is data the server holds, not a list compiled into this
 * bundle — so adding an endpoint is an operator's config edit, never a
 * release of the web app.
 */

export interface RegisterRow {
  id: string;
  title: string;
  url: string;
  host?: string;
  /** Reading it bills the reader. */
  requesterPays: boolean;
  /** Whether this deployment's egress allowlist permits its host. */
  allowed: boolean;
}

export interface Register {
  rows: RegisterRow[];
  /** Nothing is reachable: the allowlist is empty. Said once, not per
   * row. */
  federationOff: boolean;
}

export const EMPTY_REGISTER: Register = { rows: [], federationOff: true };

function textOf(raw: unknown): string | undefined {
  return typeof raw === "string" && raw !== "" ? raw : undefined;
}

/** `GET /sources/register` → the rows. An entry the response cannot
 * identify is dropped rather than shown as a blank offer. */
export function parseRegister(body: unknown): Register {
  const doc = (body as { register?: unknown[]; federationOff?: unknown } | null) ?? {};
  const rows: RegisterRow[] = [];
  for (const raw of doc.register ?? []) {
    if (typeof raw !== "object" || raw === null) {
      continue;
    }
    const item = raw as Record<string, unknown>;
    const id = textOf(item["id"]);
    const url = textOf(item["url"]);
    if (id === undefined || url === undefined) {
      continue;
    }
    const row: RegisterRow = {
      id,
      url,
      title: textOf(item["title"]) ?? id,
      requesterPays: item["requesterPays"] === true,
      allowed: item["allowed"] === true,
    };
    const host = textOf(item["host"]);
    if (host !== undefined) {
      row.host = host;
    }
    rows.push(row);
  }
  // Absent means "we do not know", and the safe reading of not knowing
  // is that nothing is reachable — only an explicit `false` says
  // federation is on. A malformed response must not imply reach.
  return { rows, federationOff: doc.federationOff !== false };
}

/** Why an entry cannot be used, or nothing when it can. Names the host,
 * because the operator's fix is to add that host to the allowlist. */
export function blockedNote(row: RegisterRow, register: Register): string | undefined {
  if (row.allowed) {
    return undefined;
  }
  if (register.federationOff) {
    return "This deployment fetches nothing yet — an operator turns federation on.";
  }
  return row.host === undefined
    ? "That link has no host we can check."
    : `${row.host} is not on this deployment's allowlist.`;
}

/** What to warn before a read that bills, or nothing. */
export function billingWarning(row: RegisterRow): string | undefined {
  return row.requesterPays
    ? "Reading this bills you — an operator agrees to that once, before the first read."
    : undefined;
}
