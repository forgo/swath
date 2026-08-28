// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-command-palette>` (docs/design/ui-system.md §5, issue #292):
 * every action reachable by name. `commands` is a property; `open`
 * reflects. A centred dialog on wide viewports, a bottom sheet below the
 * `narrow` breakpoint. ↑↓ move, Enter runs (`swath-command` fires with
 * the id, then the command's `run`), Esc closes and focus returns to
 * where it was. Parts: `base input list item hint`.
 */
import { BREAKPOINTS } from "./breakpoints.js";
import { type Command, type Match, matchCommands } from "./command-model.js";
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { css } from "./styles.js";

export class SwathCommandPalette extends SwathElement {
  static override tagName = "swath-command-palette";
  static override styles = [
    css`
      :host {
        position: fixed;
        inset: 0;
        z-index: var(--swath-z-palette);
        display: grid;
        place-items: start center;
        padding-block-start: 12vh;
        background: color-mix(in srgb, var(--swath-color-bg) 60%, transparent);
      }
      :host(:not([open])) { display: none; }
      :host([presentation="sheet"]) { place-items: end stretch; padding: 0; }
      [part="base"] {
        display: flex;
        flex-direction: column;
        inline-size: min(560px, calc(100% - var(--swath-space-6)));
        max-block-size: 60vh;
        border: var(--swath-border-hairline);
        border-radius: var(--swath-radius-md);
        background: var(--swath-color-bg-raised);
        color: var(--swath-color-fg);
        box-shadow: var(--swath-shadow-hud);
        overflow: hidden;
      }
      :host([presentation="sheet"]) [part="base"] {
        inline-size: 100%;
        max-block-size: 70vh;
        border-radius: var(--swath-radius-md) var(--swath-radius-md) 0 0;
        padding-block-end: env(safe-area-inset-bottom, 0);
      }
      [part="input"] {
        inline-size: 100%;
        min-block-size: var(--swath-size-target);
        padding: var(--swath-space-2) var(--swath-space-3);
        border: 0;
        border-block-end: var(--swath-border-hairline);
        background: none;
        color: var(--swath-color-fg);
        font: inherit;
        font-size: var(--swath-text-md);
        outline: none;
      }
      [part="list"] {
        flex: 1;
        margin: 0;
        padding: var(--swath-space-1);
        list-style: none;
        overflow: auto;
      }
      [part="item"] {
        display: flex;
        align-items: center;
        gap: var(--swath-space-2);
        inline-size: 100%;
        min-block-size: var(--swath-space-7);
        padding: var(--swath-space-1) var(--swath-space-2);
        border: 0;
        border-radius: var(--swath-radius-sm);
        background: none;
        color: var(--swath-color-fg);
        font: inherit;
        text-align: start;
        cursor: pointer;
      }
      [part="item"][aria-selected="true"], [part="item"]:hover { background: var(--swath-color-accent-bg); }
      [part="item"] mark { background: none; color: var(--swath-color-accent); font-weight: 700; }
      [part="group"] {
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        letter-spacing: var(--swath-tracking-wide);
        text-transform: uppercase;
        color: var(--swath-color-fg-muted);
        inline-size: var(--swath-space-8);
        flex: none;
      }
      [part="hint"] {
        margin-inline-start: auto;
        font-family: var(--swath-font-mono);
        font-size: var(--swath-text-xs);
        color: var(--swath-color-fg-muted);
      }
      [part="empty"] { padding: var(--swath-space-3); color: var(--swath-color-fg-muted); font-size: var(--swath-text-sm); }
      @media (pointer: coarse) { [part="item"] { min-block-size: var(--swath-size-target); } }
    `,
  ];
  static override properties = {
    open: { type: "boolean", reflect: true },
    presentation: { type: "string", reflect: true },
    label: { type: "string" },
  } as const;

  declare open: boolean;
  declare presentation: string | undefined;
  declare label: string | undefined;

  #commands: readonly Command[] = [];
  #query = "";
  #index = 0;
  #input: HTMLInputElement | undefined;
  #list: HTMLUListElement | undefined;
  #restore: Element | null = null;

  get commands(): readonly Command[] {
    return this.#commands;
  }

  set commands(commands: readonly Command[]) {
    this.#commands = commands;
    this.requestUpdate();
  }

  get results(): Match[] {
    return matchCommands(this.#commands, this.#query);
  }

  /** Open with an empty query; remembers what had focus for Esc. */
  show(): void {
    this.#restore = this.#activeElement();
    this.#query = "";
    this.#index = 0;
    this.presentation = window.matchMedia(`(min-width: ${BREAKPOINTS.narrow}px)`).matches
      ? "dialog"
      : "sheet";
    this.open = true;
    this.requestUpdate();
    void this.updateComplete.then(() => {
      if (this.#input) {
        this.#input.value = "";
        this.#input.focus();
      }
    });
  }

  close(): void {
    if (!this.open) {
      return;
    }
    this.open = false;
    const restore = this.#restore;
    this.#restore = null;
    if (restore instanceof HTMLElement) {
      restore.focus();
    }
  }

  toggle(): void {
    if (this.open) {
      this.close();
    } else {
      this.show();
    }
  }

  #activeElement(): Element | null {
    let active: Element | null = document.activeElement;
    while (active?.shadowRoot?.activeElement) {
      active = active.shadowRoot.activeElement;
    }
    return active;
  }

  #ensure(): void {
    if (this.#input) {
      return;
    }
    const input = el("input", {
      part: "input",
      type: "search",
      placeholder: "Type a command…",
      autocomplete: "off",
      spellcheck: "false",
      role: "combobox",
      "aria-expanded": "true",
      "aria-autocomplete": "list",
      "aria-controls": "list",
    });
    input.addEventListener("input", () => {
      this.#query = input.value;
      this.#index = 0;
      this.requestUpdate();
    });
    input.addEventListener("keydown", (event) => this.#onKey(event));
    const list = el("ul", { part: "list", id: "list", role: "listbox" });
    this.#input = input;
    this.#list = list;
    this.addEventListener("click", (event) => {
      if (event.target === this) {
        this.close(); // the scrim
      }
    });
    this.renderRoot.replaceChildren(
      el("div", { part: "base", role: "dialog", "aria-modal": "true" }, input, list),
    );
  }

  #run(match: Match): void {
    this.emit("swath-command", { id: match.command.id });
    this.close();
    match.command.run();
  }

  #onKey(event: KeyboardEvent): void {
    const matches = this.results;
    switch (event.key) {
      case "ArrowDown":
        this.#index = matches.length === 0 ? 0 : (this.#index + 1) % matches.length;
        break;
      case "ArrowUp":
        this.#index =
          matches.length === 0 ? 0 : (this.#index - 1 + matches.length) % matches.length;
        break;
      case "Home":
        this.#index = 0;
        break;
      case "End":
        this.#index = Math.max(0, matches.length - 1);
        break;
      case "Enter": {
        const match = matches[this.#index];
        if (match) {
          this.#run(match);
        }
        break;
      }
      case "Escape":
        this.close();
        break;
      default:
        return;
    }
    event.preventDefault();
    this.requestUpdate();
  }

  protected render(): void {
    this.#ensure();
    const input = this.#input as HTMLInputElement;
    const list = this.#list as HTMLUListElement;
    const base = this.renderRoot.querySelector('[part="base"]');
    if (this.label !== undefined && this.label !== "") {
      base?.setAttribute("aria-label", this.label);
    }
    const matches = this.results;
    if (this.#index >= matches.length) {
      this.#index = 0;
    }
    if (matches.length === 0) {
      list.replaceChildren(
        el(
          "li",
          { part: "empty", role: "option", "aria-selected": "false" },
          "No command matches.",
        ),
      );
      input.removeAttribute("aria-activedescendant");
      return;
    }
    list.replaceChildren(
      ...matches.map((match, index) => {
        const selected = index === this.#index;
        const label = el("span");
        const positions = new Set(match.positions);
        let run = "";
        const flush = (marked: boolean): void => {
          if (run !== "") {
            label.append(marked ? el("mark", {}, run) : run);
            run = "";
          }
        };
        let marked = false;
        for (const [i, ch] of [...match.command.label].entries()) {
          const isMarked = positions.has(i);
          if (isMarked !== marked) {
            flush(marked);
            marked = isMarked;
          }
          run += ch;
        }
        flush(marked);
        const button = el(
          "button",
          {
            part: "item",
            type: "button",
            role: "option",
            id: `cmd-${index}`,
            "aria-selected": String(selected),
            "data-command": match.command.id,
          },
          el("span", { part: "group" }, match.command.group),
          label,
          match.command.hint ? el("span", { part: "hint" }, match.command.hint) : null,
        );
        button.addEventListener("click", () => this.#run(match));
        button.addEventListener("pointermove", () => {
          if (this.#index !== index) {
            this.#index = index;
            this.requestUpdate();
          }
        });
        if (selected) {
          input.setAttribute("aria-activedescendant", `cmd-${index}`);
        }
        return el("li", { role: "none" }, button);
      }),
    );
    list.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "swath-command-palette": SwathCommandPalette;
  }
}
