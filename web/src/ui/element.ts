// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `SwathElement` — ADR 0005's reactive layer (docs/design/ui-system.md §4.2),
 * kept deliberately small. It gives every element: an open shadow root with
 * `base` + its own sheets adopted, a static property table that becomes
 * attribute-reflecting accessors (no decorators: `erasableSyntaxOnly`), a
 * microtask-batched `requestUpdate()`, an `AbortSignal` that fires on
 * disconnect for listener cleanup, and a typed `emit()`.
 *
 * It deliberately has no templating, diffing, store, DI, directives or
 * theming API beyond tokens + `::part`. Rendering is imperative (`dom.ts`).
 */
import { createSwathEvent, type SwathEventMap } from "./events.js";
import { adoptSheet, adoptTokens, base } from "./styles.js";

export type PropType = "string" | "number" | "boolean";

export interface PropSpec {
  readonly type: PropType;
  /** Attribute name; defaults to the property name. `false`: property only. */
  readonly attribute?: string | false;
  /** Write property changes back to the attribute. */
  readonly reflect?: boolean;
}

type PropValue = string | number | boolean | undefined;

function coerce(type: PropType, raw: unknown): PropValue {
  if (raw === undefined || raw === null) {
    return type === "boolean" ? false : undefined;
  }
  switch (type) {
    case "boolean":
      return raw !== false;
    case "number": {
      const n = Number(raw);
      return Number.isNaN(n) ? undefined : n;
    }
    default:
      return String(raw);
  }
}

function attributeName(name: string, spec: PropSpec): string | undefined {
  return spec.attribute === false ? undefined : (spec.attribute ?? name.toLowerCase());
}

export abstract class SwathElement extends HTMLElement {
  static tagName: string;
  static styles: readonly CSSStyleSheet[] = [];
  static properties: Record<string, PropSpec> = {};
  /** `null` renders into LIGHT DOM (`renderRoot === this`, styles adopted
   * at document level): the escape for an organism whose ids and tests
   * live in the light tree (the authoring panel, #291). Everything else
   * keeps a shadow root. */
  static shadowOptions: ShadowRootInit | null = { mode: "open" };

  // `this` in these statics is the SUBCLASS (its table, its tag): the whole
  // point. Biome's noThisInStatic would rewrite it to the base class.
  // biome-ignore-start lint/complexity/noThisInStatic: polymorphic statics
  static get observedAttributes(): string[] {
    return Object.entries(this.properties).flatMap(([name, spec]) => {
      const attr = attributeName(name, spec);
      return attr === undefined ? [] : [attr];
    });
  }

  /** Register the element once; installs the property accessors first. */
  static define(): void {
    if (customElements.get(this.tagName)) {
      return;
    }
    for (const [name, spec] of Object.entries(this.properties)) {
      const attr = attributeName(name, spec);
      Object.defineProperty(this.prototype, name, {
        configurable: true,
        enumerable: true,
        get(this: SwathElement) {
          return this.#props.get(name) ?? coerce(spec.type, undefined);
        },
        set(this: SwathElement, raw: unknown) {
          const value = coerce(spec.type, raw);
          if (Object.is(value, this.#props.get(name))) {
            return;
          }
          this.#props.set(name, value);
          if (spec.reflect && attr !== undefined) {
            this.#reflect(attr, value);
          }
          this.requestUpdate();
        },
      });
    }
    customElements.define(this.tagName, this as unknown as CustomElementConstructor);
  }
  // biome-ignore-end lint/complexity/noThisInStatic: polymorphic statics

  protected readonly renderRoot: ShadowRoot | HTMLElement;
  readonly #props = new Map<string, PropValue>();
  #abort = new AbortController();
  #update: Promise<void> | undefined;

  constructor() {
    super();
    const ctor = this.constructor as typeof SwathElement;
    adoptTokens(this.ownerDocument);
    if (ctor.shadowOptions === null) {
      this.renderRoot = this;
      for (const sheet of ctor.styles) {
        adoptSheet(sheet, this.ownerDocument);
      }
    } else {
      const root = this.attachShadow(ctor.shadowOptions);
      root.adoptedStyleSheets = [base, ...ctor.styles];
      this.renderRoot = root;
    }
    // A property set before upgrade is an own data property shadowing the
    // prototype accessor; re-route it through the setter.
    for (const name of Object.keys(ctor.properties)) {
      if (Object.hasOwn(this, name)) {
        const value = (this as Record<string, unknown>)[name];
        delete (this as Record<string, unknown>)[name];
        (this as Record<string, unknown>)[name] = value;
      }
    }
  }

  /** The focused element inside this element's render root (shadow or light). */
  protected get focused(): Element | null {
    return this.renderRoot instanceof ShadowRoot
      ? this.renderRoot.activeElement
      : this.ownerDocument.activeElement;
  }

  /** Aborts on disconnect — for listeners on `window` / `document` / other
   * elements: `addEventListener(…, { signal: this.disconnected })`. Never
   * for the element's own shadow nodes: re-parenting fires disconnect, and
   * a control built once would lose its handlers for good. */
  protected get disconnected(): AbortSignal {
    return this.#abort.signal;
  }

  connectedCallback(): void {
    if (this.#abort.signal.aborted) {
      this.#abort = new AbortController();
    }
    this.requestUpdate();
  }

  disconnectedCallback(): void {
    this.#abort.abort();
  }

  attributeChangedCallback(attr: string, _old: string | null, value: string | null): void {
    const ctor = this.constructor as typeof SwathElement;
    for (const [name, spec] of Object.entries(ctor.properties)) {
      if (attributeName(name, spec) === attr) {
        // Boolean: present = true, except the literal "false" (aria-style),
        // so a toggle can start unpressed: `<swath-button pressed="false">`.
        const next =
          spec.type === "boolean" ? value !== null && value !== "false" : (value ?? undefined);
        (this as Record<string, unknown>)[name] = next;
      }
    }
  }

  #reflect(attr: string, value: PropValue): void {
    if (value === undefined || value === false) {
      this.removeAttribute(attr);
    } else {
      this.setAttribute(attr, value === true ? "" : String(value));
    }
  }

  /** Schedule one render per microtask, however many properties changed. */
  requestUpdate(): void {
    this.#update ??= Promise.resolve().then(() => {
      this.#update = undefined;
      if (this.isConnected) {
        this.render();
      }
    });
  }

  /** Resolves after the pending render (immediately when none is pending). */
  get updateComplete(): Promise<void> {
    return this.#update ?? Promise.resolve();
  }

  /** Imperative DOM into `renderRoot`; idempotent across calls. */
  protected abstract render(): void;

  /** Dispatch a catalogued event (always bubbling and composed). */
  protected emit<K extends keyof SwathEventMap>(type: K, detail: SwathEventMap[K]): boolean {
    return this.dispatchEvent(createSwathEvent(type, detail));
  }
}
