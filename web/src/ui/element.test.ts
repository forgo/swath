// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Real browser (Vitest Browser Mode): a real registry, real shadow roots,
// real `adoptedStyleSheets` — none of which jsdom represents.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { el } from "./dom.js";
import { SwathElement } from "./element.js";
import { base, css } from "./styles.js";

declare module "./events.js" {
  interface SwathEventMap {
    "swath-probe-change": { count: number };
  }
}

const probeSheet = css`
  :host { display: block; padding: var(--swath-space-2); }
`;

class SwathProbe extends SwathElement {
  static override tagName = "swath-probe";
  static override styles = [probeSheet];
  static override properties = {
    label: { type: "string" },
    count: { type: "number", reflect: true },
    active: { type: "boolean", reflect: true },
    dataKey: { type: "string", attribute: "data-key" },
    secret: { type: "string", attribute: false },
  } as const;

  declare label: string | undefined;
  declare count: number | undefined;
  declare active: boolean;
  declare dataKey: string | undefined;
  declare secret: string | undefined;

  renders = 0;

  protected render(): void {
    this.renders += 1;
    this.renderRoot.replaceChildren(
      el("span", { part: "label" }, this.label ?? "—"),
      el("span", { part: "count" }, String(this.count ?? 0)),
    );
  }

  fire(): boolean {
    return this.emit("swath-probe-change", { count: this.count ?? 0 });
  }

  listen(target: EventTarget, type: string, handler: () => void): void {
    target.addEventListener(type, handler, { signal: this.disconnected });
  }
}

beforeAll(() => {
  SwathProbe.define();
});

afterEach(() => {
  document.body.replaceChildren();
});

function mount(): SwathProbe {
  const probe = document.createElement("swath-probe") as SwathProbe;
  document.body.append(probe);
  return probe;
}

test("define() is idempotent and derives observedAttributes from the table", () => {
  SwathProbe.define(); // second call: no registry error
  expect(customElements.get("swath-probe")).toBe(SwathProbe);
  expect(SwathProbe.observedAttributes).toEqual(["label", "count", "active", "data-key"]);
});

test("shadow root adopts base then the element's sheets; tokens reach inside", async () => {
  const probe = mount();
  await probe.updateComplete;
  const root = probe.shadowRoot;
  expect(root).not.toBeNull();
  expect(root?.adoptedStyleSheets).toEqual([base, probeSheet]);
  // base.css writes only var(--swath-…); the host resolves them through the
  // document-level token sheet — a real value, not the empty fallback.
  expect(getComputedStyle(probe).paddingTop).toBe("8px");
});

test("attribute → property, coerced per type", () => {
  const probe = mount();
  probe.setAttribute("label", "ingest");
  probe.setAttribute("count", "42");
  probe.setAttribute("active", "");
  probe.setAttribute("data-key", "k1");
  expect(probe.label).toBe("ingest");
  expect(probe.count).toBe(42);
  expect(probe.active).toBe(true);
  expect(probe.dataKey).toBe("k1");
  probe.setAttribute("active", "false"); // aria-style: the one falsy spelling
  expect(probe.active).toBe(false);
  probe.setAttribute("active", "true");
  expect(probe.active).toBe(true);
  probe.removeAttribute("active");
  probe.removeAttribute("count");
  expect(probe.active).toBe(false);
  expect(probe.count).toBeUndefined();
  probe.setAttribute("count", "not-a-number");
  expect(probe.count).toBeUndefined();
});

test("property → attribute only when reflect is set", () => {
  const probe = mount();
  probe.count = 7;
  probe.active = true;
  probe.label = "x";
  probe.secret = "s";
  expect(probe.getAttribute("count")).toBe("7");
  expect(probe.getAttribute("active")).toBe("");
  expect(probe.hasAttribute("label")).toBe(false);
  expect(probe.hasAttribute("secret")).toBe(false);
  probe.active = false;
  probe.count = undefined;
  expect(probe.hasAttribute("active")).toBe(false);
  expect(probe.hasAttribute("count")).toBe(false);
});

test("a property set before upgrade survives the upgrade", async () => {
  const host = document.createElement("div");
  host.innerHTML = "<swath-late></swath-late>";
  const late = host.firstElementChild as SwathProbe;
  (late as unknown as Record<string, unknown>).count = 3;
  class SwathLate extends SwathProbe {
    static override tagName = "swath-late";
  }
  SwathLate.define();
  document.body.append(host);
  await late.updateComplete;
  expect(late.count).toBe(3);
  expect(late.getAttribute("count")).toBe("3");
});

test("N property sets → 1 render per microtask; updateComplete awaits it", async () => {
  const probe = mount();
  await probe.updateComplete;
  const before = probe.renders;
  probe.label = "a";
  probe.count = 1;
  probe.active = true;
  probe.setAttribute("label", "b");
  expect(probe.renders).toBe(before);
  await probe.updateComplete;
  expect(probe.renders).toBe(before + 1);
  expect(probe.shadowRoot?.querySelector('[part="label"]')?.textContent).toBe("b");
  await probe.updateComplete; // nothing pending: resolves without rendering
  expect(probe.renders).toBe(before + 1);
});

test("no render while disconnected; one on (re)connect", async () => {
  const probe = document.createElement("swath-probe") as SwathProbe;
  probe.label = "offline";
  await probe.updateComplete;
  expect(probe.renders).toBe(0);
  document.body.append(probe);
  await probe.updateComplete;
  expect(probe.renders).toBe(1);
  expect(probe.shadowRoot?.querySelector('[part="label"]')?.textContent).toBe("offline");
});

test("listeners bound to `disconnected` die on disconnect and rebind after reconnect", () => {
  const probe = mount();
  const handler = vi.fn();
  probe.listen(window, "probe-ping", handler);
  window.dispatchEvent(new Event("probe-ping"));
  expect(handler).toHaveBeenCalledTimes(1);
  probe.remove();
  window.dispatchEvent(new Event("probe-ping"));
  expect(handler).toHaveBeenCalledTimes(1);
  document.body.append(probe);
  expect(probe.listen(window, "probe-ping", handler)).toBeUndefined();
  window.dispatchEvent(new Event("probe-ping"));
  expect(handler).toHaveBeenCalledTimes(2);
});

test("emit() is bubbling and composed, typed through the catalog", () => {
  const outer = el("div");
  const probe = document.createElement("swath-probe") as SwathProbe;
  outer.append(probe);
  document.body.append(outer);
  probe.count = 5;
  const seen: number[] = [];
  outer.addEventListener("swath-probe-change", (event) => {
    seen.push(event.detail.count);
    expect(event.composed).toBe(true);
    expect(event.bubbles).toBe(true);
  });
  expect(probe.fire()).toBe(true);
  expect(seen).toEqual([5]);
});

test("a re-parented element keeps its own shadow listeners; only disconnected-bound ones die", () => {
  const probe = mount();
  const inner = document.createElement("button");
  const clicks = vi.fn();
  inner.addEventListener("click", clicks); // an element's own node: no signal
  probe.shadowRoot?.append(inner);
  const external = vi.fn();
  probe.listen(window, "probe-ping", external);
  const other = document.createElement("div");
  document.body.append(other);
  other.append(probe); // moved: disconnect + connect
  inner.click();
  window.dispatchEvent(new Event("probe-ping"));
  expect(clicks).toHaveBeenCalledTimes(1);
  expect(external).toHaveBeenCalledTimes(0);
});

test("shadowOptions: null renders into light DOM with the sheet adopted by the document", async () => {
  class SwathLightProbe extends SwathElement {
    static override tagName = "swath-light-probe";
    static override shadowOptions = null;
    static override styles = [
      css`swath-light-probe { display: block; padding: var(--swath-space-2); }`,
    ];
    static override properties = { label: { type: "string" } } as const;
    declare label: string | undefined;
    protected render(): void {
      this.renderRoot.textContent = this.label ?? "light";
    }
  }
  SwathLightProbe.define();
  const probe = document.createElement("swath-light-probe") as SwathLightProbe;
  probe.setAttribute("label", "hello");
  document.body.append(probe);
  await probe.updateComplete;
  expect(probe.shadowRoot).toBeNull();
  expect(probe.textContent).toBe("hello");
  expect(document.querySelector("swath-light-probe")?.textContent).toBe("hello");
  expect(getComputedStyle(probe).paddingTop).toBe("8px");
});
