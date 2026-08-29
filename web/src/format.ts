// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** Number formatting with one home (#355): the fixed-precision trim the
 * view state, status bar and share link agree on, and the byte and
 * millisecond readouts the overlay and the authoring panel show. */

/** Fixed-precision decimal with trailing zeros (and a bare `.`) trimmed —
 * `-106.00000` → `-106`, `39.30000` → `39.3`. */
export function fixed(value: number, decimals: number): string {
  return value
    .toFixed(decimals)
    .replace(/(\.\d*?)0+$/, "$1")
    .replace(/\.$/, "");
}

/** Kilobytes with one decimal below 100 KB, whole above (`12.3`, `456`). */
export function formatKb(bytes: number): string {
  const kb = bytes / 1024;
  return kb >= 100 ? String(Math.round(kb)) : kb.toFixed(1);
}

/** Human-formatted byte count with a unit (B / KB / MB). */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const kb = bytes / 1024;
  if (kb < 1024) {
    return `${formatKb(bytes)} KB`;
  }
  const mb = kb / 1024;
  return `${mb >= 100 ? String(Math.round(mb)) : mb.toFixed(1)} MB`;
}

/** `bytes` in the user's units: "1.5 MiB" / "820 KiB". */
export function formatMib(bytes: number): string {
  const mib = bytes / (1024 * 1024);
  if (mib >= 1) {
    return `${Number.isInteger(mib) ? mib : mib.toFixed(1)} MiB`;
  }
  return `${Math.max(1, Math.round(bytes / 1024))} KiB`;
}

/** `12` / `17.5` — interpolated percentiles carry at most the one decimal a
 * half-step between integer millisecond samples produces worth showing. */
export function formatMs(value: number): string {
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}
