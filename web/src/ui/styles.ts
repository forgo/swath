// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * Stylesheet plumbing for the UI system (docs/design/ui-system.md §4.1).
 *
 * `tokens.css` is adopted ONCE per document (custom properties inherit
 * through shadow boundaries); `base.css` and each element's own `css`
 * sheets are adopted into shadow roots by `SwathElement`. Constructed
 * `CSSStyleSheet`s are cached by text, so a sheet declared once at module
 * scope is shared by every instance — no per-instance `<style>` nodes.
 */
import baseCss from "./base.css?inline";
import tokensCss from "./tokens.css?inline";

const sheets = new Map<string, CSSStyleSheet>();

function sheet(text: string): CSSStyleSheet {
  let cached = sheets.get(text);
  if (!cached) {
    cached = new CSSStyleSheet();
    cached.replaceSync(text);
    sheets.set(text, cached);
  }
  return cached;
}

/** Tagged template → a cached, constructed `CSSStyleSheet`. Interpolations
 * are for token references and numbers, never raw values (the DRY gate
 * polices literals regardless of where they hide). */
export function css(
  strings: TemplateStringsArray,
  ...values: readonly (string | number)[]
): CSSStyleSheet {
  return sheet(String.raw({ raw: strings }, ...values));
}

/** The token sheet — one `:root` block; adopted at document level. */
export const tokens: CSSStyleSheet = sheet(tokensCss);

/** The shared shadow-root reset; adopted into every `SwathElement`. */
export const base: CSSStyleSheet = sheet(baseCss);

const adopted = new WeakSet<Document>();

/** Adopt the token sheet into `doc` exactly once (idempotent per document).
 * A constructed sheet belongs to one realm, so a foreign document (an
 * iframe, a popup) gets its own copy built there; a document with no window
 * (`createHTMLDocument`) gets a `<style>` instead. */
export function adoptTokens(doc: Document = document): void {
  if (adopted.has(doc)) {
    return;
  }
  adopted.add(doc);
  if (doc === document) {
    doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, tokens];
    return;
  }
  const Realm = doc.defaultView?.CSSStyleSheet;
  if (Realm) {
    const copy = new Realm();
    copy.replaceSync(tokensCss);
    doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, copy];
    return;
  }
  const style = doc.createElement("style");
  style.textContent = tokensCss;
  doc.head.append(style);
}

/** Read a token's computed value where custom properties can't be consumed
 * directly — MapLibre paint properties, canvas drawing. `name` is the full
 * property name (`--swath-color-accent`). Empty string when undeclared. */
export function readToken(name: string, doc: Document = document): string {
  adoptTokens(doc);
  return getComputedStyle(doc.documentElement).getPropertyValue(name).trim();
}
