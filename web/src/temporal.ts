// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** The authoring form's temporal-interval value (#355): the stored JSON
 * `[start, end]` with `null` for an open side, its two date-picker bounds,
 * and the plain words the narrative says for it. */

/** The `[from, until]` date strings of a stored temporal-interval value
 * (`""` = open on that side; both `""` = no interval stored). */
export function temporalBounds(raw: string): [string, string] {
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (Array.isArray(parsed) && parsed.length === 2) {
      return [
        typeof parsed[0] === "string" ? parsed[0] : "",
        typeof parsed[1] === "string" ? parsed[1] : "",
      ];
    }
  } catch {
    // Not an interval — treated as unset.
  }
  return ["", ""];
}

/** The stored temporal-interval value for two date-picker bounds:
 * `""` (field omitted → null, no filter) when both are empty, the
 * `[start, end]` JSON with `null` for an open side otherwise. */
export function temporalValue(from: string, until: string): string {
  if (from === "" && until === "") {
    return "";
  }
  return JSON.stringify([from === "" ? null : from, until === "" ? null : until]);
}

/** The when line's plain words for a stored temporal-interval value. */
export function temporalPhrase(raw: string): string {
  const [from, until] = temporalBounds(raw);
  if (from === "" && until === "") {
    return "everything available";
  }
  if (from === "") {
    return `until ${until}`;
  }
  if (until === "") {
    return `from ${from}`;
  }
  return `${from} until ${until}`;
}
