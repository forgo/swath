// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** `localStorage` with one guard (#355): touching it can throw (storage
 * disabled, a private window), and a stored value can be anything — so
 * reads come back as `undefined` rather than an exception, and the
 * feature that wanted them degrades to no-op. */

/** `window.localStorage`, or undefined where touching it throws. */
export function safeLocalStorage(): Storage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

/** The JSON stored under `key`, parsed, or `undefined` when storage is
 * unavailable, the key is absent, or the text is not JSON. The caller
 * still validates the shape. */
export function readJson(key: string, storage = safeLocalStorage()): unknown {
  let raw: string | null;
  try {
    raw = storage?.getItem(key) ?? null;
  } catch {
    return undefined;
  }
  if (raw === null) {
    return undefined;
  }
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return undefined;
  }
}
