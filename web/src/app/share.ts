// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** The Share button (issue #211), lifted out of the entry script (#355). */
import type { SwathButton } from "../ui/button";
import { shareUrl, type ViewState } from "../view-state";

/** How long the Share button reads "copied" before reverting. */
const SHARE_FEEDBACK_MS = 1600;

/** The Share button (issue #211): copies the canonical deep link of the
 * current view — the same URL the address bar shows after an
 * interaction, written in full even on a bare landing. Clipboard
 * failure (no secure context, permission denied) falls back to a
 * prompt holding the link, so the URL is never unreachable. */
export function wireShare(button: SwathButton, snapshot: () => ViewState): void {
  const idle = (button.textContent ?? "").trim();
  let revert: number | undefined;
  const feedback = (state: "copied" | "failed"): void => {
    button.dataset["state"] = state;
    button.textContent = state === "copied" ? "copied" : "copy failed";
    window.clearTimeout(revert);
    revert = window.setTimeout(() => {
      delete button.dataset["state"];
      button.textContent = idle;
    }, SHARE_FEEDBACK_MS);
  };
  button.addEventListener("click", () => {
    const url = shareUrl(location.href, snapshot());
    button.dataset["url"] = url; // what was copied, inspectable (tests, tooling)
    navigator.clipboard
      .writeText(url)
      .then(() => feedback("copied"))
      .catch(() => {
        feedback("failed");
        window.prompt("Copy this link", url);
      });
  });
}
