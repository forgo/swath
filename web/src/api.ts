// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `SwathApi` — the one HTTP seam of the web tree (docs/design/ui-system.md
 * §4.4), and the one home of `fetch(` (scripts/check-ui-dry.mjs; the
 * basemap style cache in swath-map.ts is a foreign URL and allow-listed).
 *
 * Every organism talks to the server through an instance: same base-URL
 * handling, same error shape (`ApiProblem`), same capabilities cache, one
 * injection point for tests (`fetch` / `eventSource` options) instead of a
 * `fetchImpl` per element. The `fetch` option defaults to `globalThis.fetch`
 * resolved AT CALL TIME, so a test that stubs the global still works.
 */

/** The subset of `EventSource` the x-ray consumes (swath-xray.ts). */
export interface EventSourceLike {
  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void;
  close(): void;
}

export interface SwathApiOptions {
  /** Base URL, no trailing slash needed; "" = same origin. */
  readonly base?: string | undefined;
  /** Test seam; default resolves `globalThis.fetch` per call. */
  readonly fetch?: typeof fetch | undefined;
  /** Test seam; default `new EventSource(url)`. */
  readonly eventSource?: ((url: string) => EventSourceLike) | undefined;
}

export type Query = Readonly<Record<string, string | number | boolean | undefined>>;

/** A non-2xx answer, carrying whatever diagnostics the body offered:
 * RFC 7807 (`title` / `detail`, every Swath surface — crates/swath-api
 * error.rs), openEO's `{ code, message }`, or nothing parseable (then the
 * status line, honestly). */
export class ApiProblem extends Error {
  override readonly name = "ApiProblem";
  /** RFC 7807 `title`, or the openEO `code`; "" when the body had none. */
  readonly title: string;
  /** RFC 7807 `detail`, or the openEO `message`; "" when the body had none. */
  readonly detail: string;
  readonly body: unknown;
  readonly status: number;
  readonly url: string;

  constructor(status: number, body: unknown, url = "") {
    const record =
      typeof body === "object" && body !== null ? (body as Record<string, unknown>) : {};
    const str = (key: string): string =>
      typeof record[key] === "string" ? (record[key] as string) : "";
    const title = str("title") || str("code");
    const detail = str("detail") || str("message");
    super(detail !== "" ? (title !== "" ? `${title}: ${detail}` : detail) : `HTTP ${status}`);
    this.status = status;
    this.url = url;
    this.title = title;
    this.detail = detail;
    this.body = body;
  }

  /** The body is consumed; a non-JSON body degrades to the status line. */
  static async from(response: Response): Promise<ApiProblem> {
    const body: unknown = await response.json().catch(() => undefined);
    return new ApiProblem(response.status, body, response.url);
  }
}

/** A server problem routed onto a form field ("" = the flow-level line). */
export interface FieldNote<F extends string> {
  field: F | "";
  note: string;
}

/** One rule of a `fieldFor` table: a substring of `detail` decides the
 * field, optionally rewording the note. First match wins. */
export interface FieldRule<F extends string> {
  readonly includes: string;
  readonly field: F | "";
  readonly note?: ((detail: string) => string) | undefined;
}

/** Map a server refusal onto the field it is about, so every organism
 * routes diagnostics the same way (generalises add-data-model's
 * `mapProblem`). No `detail` → the status alone, unrouted. */
export function fieldFor<F extends string>(
  problem: ApiProblem,
  rules: readonly FieldRule<F>[],
): FieldNote<F> {
  const { detail } = problem;
  if (detail === "") {
    return { field: "", note: `the server refused with HTTP ${problem.status}` };
  }
  for (const rule of rules) {
    if (detail.includes(rule.includes)) {
      return { field: rule.field, note: rule.note ? rule.note(detail) : detail };
    }
  }
  return { field: "", note: detail };
}

const ABSOLUTE = /^[a-z][a-z0-9+.-]*:/i;

export class SwathApi {
  readonly base: string;
  readonly #fetch: typeof fetch | undefined;
  readonly #eventSource: (url: string) => EventSourceLike;
  #capabilities: Promise<unknown> | undefined;

  constructor(options: SwathApiOptions = {}) {
    this.base = (options.base ?? "").replace(/\/+$/, "");
    this.#fetch = options.fetch;
    this.#eventSource = options.eventSource ?? ((url) => new EventSource(url));
  }

  /** `path` under the base (absolute URLs pass through), `query` appended
   * with `undefined` values dropped. */
  url(path: string, query?: Query): string {
    const target = ABSOLUTE.test(path) ? path : `${this.base}${path}`;
    if (!query) {
      return target;
    }
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(query)) {
      if (value !== undefined) {
        params.set(key, String(value));
      }
    }
    const qs = params.toString();
    return qs === "" ? target : `${target}${target.includes("?") ? "&" : "?"}${qs}`;
  }

  /** The raw request — for callers that branch on status (a 409 that is
   * fine, a liveness probe). Everything else uses `json()` / `blob()`. */
  fetch(path: string, init?: RequestInit): Promise<Response> {
    const call = this.#fetch ?? globalThis.fetch;
    return call(this.url(path), init);
  }

  /** GET (by default) JSON; a non-2xx rejects with `ApiProblem`. */
  async json<T = unknown>(path: string, init: RequestInit = {}): Promise<T> {
    const response = await this.fetch(path, {
      ...init,
      headers: { accept: "application/json", ...init.headers },
    });
    if (!response.ok) {
      throw await ApiProblem.from(response);
    }
    return (await response.json()) as T;
  }

  /** A binary answer (a preview PNG); a non-2xx rejects with `ApiProblem`. */
  async blob(path: string, init?: RequestInit): Promise<Blob> {
    const response = await this.fetch(path, init);
    if (!response.ok) {
      throw await ApiProblem.from(response);
    }
    return response.blob();
  }

  /** The landing/capabilities document (`GET /`, JSON): one request per
   * success, shared by every caller; a failure clears the cache so the
   * next call is a real retry (the add-data panel's "close and re-open to
   * retry" rule). */
  capabilities(): Promise<unknown> {
    this.#capabilities ??= this.json("/").catch((error: unknown) => {
      this.#capabilities = undefined;
      throw error;
    });
    return this.#capabilities;
  }

  /** An SSE stream under the base (the x-ray's `/traces`). */
  events(path: string, query?: Query): EventSourceLike {
    return this.#eventSource(this.url(path, query));
  }
}
