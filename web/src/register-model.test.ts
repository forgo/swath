// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import { billingWarning, blockedNote, EMPTY_REGISTER, parseRegister } from "./register-model.js";

const served = (rows: unknown[], federationOff = false) => ({
  register: rows,
  federationOff,
});

test("the register comes from the server, so adding one needs no release", () => {
  const register = parseRegister(
    served([
      {
        id: "earth-search",
        title: "Earth Search",
        url: "https://stac.example.org/v1",
        host: "stac.example.org",
        allowed: true,
      },
    ]),
  );
  expect(register.rows).toEqual([
    {
      id: "earth-search",
      title: "Earth Search",
      url: "https://stac.example.org/v1",
      host: "stac.example.org",
      requesterPays: false,
      allowed: true,
    },
  ]);
  expect(register.federationOff).toBe(false);
});

test("an entry the response cannot identify is dropped, not shown blank", () => {
  expect(parseRegister(served([{ title: "nameless" }, "nope", { id: "x" }])).rows).toEqual([]);
  expect(parseRegister(null)).toEqual(EMPTY_REGISTER);
});

test("a blocked entry says why, naming the host the operator must allow", () => {
  const register = parseRegister(
    served([
      { id: "a", url: "https://ok.example.org/c", host: "ok.example.org", allowed: true },
      { id: "b", url: "https://no.example.net/c", host: "no.example.net", allowed: false },
    ]),
  );
  const [ok, blocked] = register.rows;
  expect(ok && blockedNote(ok, register)).toBeUndefined();
  expect(blocked && blockedNote(blocked, register)).toBe(
    "no.example.net is not on this deployment's allowlist.",
  );
});

test("with federation off the reason is said once, not per host", () => {
  const register = parseRegister(
    served([{ id: "a", url: "https://x.example.org/c", host: "x.example.org" }], true),
  );
  const [row] = register.rows;
  expect(row && blockedNote(row, register)).toBe(
    "This deployment fetches nothing yet — an operator turns federation on.",
  );
});

test("a requester-pays entry warns before the read, and names no price", () => {
  const register = parseRegister(
    served([{ id: "a", url: "https://x.example.org/c", allowed: true, requesterPays: true }]),
  );
  const [row] = register.rows;
  const warning = (row && billingWarning(row)) ?? "";
  expect(warning).toContain("bills you");
  for (const money of ["$", "USD", "price", "dollar"]) {
    expect(warning).not.toContain(money);
  }
  const free = parseRegister(served([{ id: "b", url: "https://y.example.org/c" }])).rows[0];
  expect(free && billingWarning(free)).toBeUndefined();
});
