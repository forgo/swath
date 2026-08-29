// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** The openEO parameter-schema interpreter the authoring panel drives its
 * widgets and value parsing with (#355): which types an alternative admits,
 * the band-name/band-array subtypes, enums, and the literal parser. Pure
 * functions over the served JSON Schema; nothing here touches the DOM. */
import type { ProcessParameter } from "./authoring-model";

/** A schema (or one-of list of schemas) as its list of alternatives. */
export function alternatives(schema: unknown): Record<string, unknown>[] {
  const list = Array.isArray(schema) ? schema : [schema];
  return list.filter((alt): alt is Record<string, unknown> => {
    return typeof alt === "object" && alt !== null && !Array.isArray(alt);
  });
}

/** The `type` values one alternative admits (openEO allows an array). */
export function typesOf(alt: Record<string, unknown>): string[] {
  const type = alt["type"];
  if (typeof type === "string") {
    return [type];
  }
  return Array.isArray(type) ? type.filter((t): t is string => typeof t === "string") : [];
}

export function hasSubtype(schema: unknown, subtype: string): boolean {
  return alternatives(schema).some((alt) => alt["subtype"] === subtype);
}

export function allowsNull(schema: unknown): boolean {
  return alternatives(schema).some((alt) => typesOf(alt).includes("null"));
}

export function isNumeric(schema: unknown): boolean {
  return alternatives(schema).some((alt) => {
    const types = typesOf(alt);
    return types.includes("number") || types.includes("integer");
  });
}

export function isStringArray(schema: unknown): boolean {
  return alternatives(schema).some((alt) => {
    if (!typesOf(alt).includes("array")) {
      return false;
    }
    const items = alt["items"];
    return (
      typeof items === "object" && items !== null && typesOf(items as never).includes("string")
    );
  });
}

export function isString(schema: unknown): boolean {
  return alternatives(schema).some((alt) => typesOf(alt).includes("string"));
}

/** Enumerated string values, when any alternative pins an `enum`. */
export function enumValues(schema: unknown): string[] {
  for (const alt of alternatives(schema)) {
    const values = alt["enum"];
    if (Array.isArray(values)) {
      const strings = values.filter((value): value is string => typeof value === "string");
      if (strings.length > 0) {
        return strings;
      }
    }
  }
  return [];
}

/** Does the parameter name a single band? Drives the band-select
 * widget (the loaded-band vocabulary promoted into the field, B7). */
export function isBandName(schema: unknown): boolean {
  if (hasSubtype(schema, "band-name")) {
    return true;
  }
  return isBandArray(schema);
}

/** Does the parameter name an ARRAY of bands (`load_collection.bands`)?
 * Drives the band-checkbox widget. */
export function isBandArray(schema: unknown): boolean {
  return alternatives(schema).some((alt) => {
    const items = alt["items"];
    return typeof items === "object" && items !== null && !Array.isArray(items)
      ? (items as Record<string, unknown>)["subtype"] === "band-name"
      : false;
  });
}

/** A raw field value, parsed by what the parameter's schema admits. */
export function parseLiteral(raw: string, schema: unknown): unknown {
  if (hasSubtype(schema, "output-format-options")) {
    return { colormap: raw };
  }
  if (isStringArray(schema)) {
    return raw
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry !== "");
  }
  if (isNumeric(schema)) {
    const value = Number(raw);
    return Number.isNaN(value) ? raw : value;
  }
  if (isString(schema)) {
    return raw;
  }
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return raw;
  }
}

/** The prefill for a literal input, from the definition's `default`. */
export function defaultText(param: ProcessParameter): string {
  const value = param.default;
  if (value === undefined || value === null) {
    return "";
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  if (Array.isArray(value)) {
    return value.join(",");
  }
  return "";
}
