// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import {
  countsLine,
  credentialNote,
  freshness,
  isFirstRun,
  parseSources,
  type SourceRow,
  stateLabel,
  stateTone,
  UNKNOWN,
} from "./sources-model.js";

const row = (over: Partial<SourceRow> = {}): SourceRow => ({
  id: "fire",
  title: "Fire drops",
  kind: "filedrop",
  scheme: "file",
  origin: "config",
  datasets: [],
  state: "watching",
  ingested: 0,
  failures: 0,
  ...over,
});

const served = (status: Record<string, unknown>) => ({
  sources: [
    {
      id: "fire",
      title: "Fire drops",
      kind: "filedrop",
      scheme: "file",
      origin: "config",
      datasets: ["hls-s30-fire"],
      status,
    },
  ],
});

test("every field comes from the response; nothing is inferred", () => {
  const [only] = parseSources(
    served({
      state: "watching",
      reachable: true,
      lastEvent: "2026-09-04T10:00:00Z",
      ingested: 7,
      failures: 1,
    }),
  );
  expect(only).toEqual({
    id: "fire",
    title: "Fire drops",
    kind: "filedrop",
    scheme: "file",
    origin: "config",
    datasets: ["hls-s30-fire"],
    state: "watching",
    reachable: true,
    lastEvent: "2026-09-04T10:00:00Z",
    ingested: 7,
    failures: 1,
  });
});

test("absent is not false: a source nothing has looked at carries no verdict", () => {
  const [only] = parseSources(served({ state: "unknown", reachable: null }));
  expect(only?.reachable).toBeUndefined();
  expect(only && stateLabel(only)).toBe("no reports yet");
  expect(only && stateTone(only)).toBe("neutral");
  expect(only && freshness(only, Date.now())).toBe(UNKNOWN);
});

test("an unrecognised state is unknown rather than a guess", () => {
  const [only] = parseSources(served({ state: "probably fine" }));
  expect(only?.state).toBe("unknown");
});

test("a row the response cannot identify is dropped, not rendered blank", () => {
  expect(parseSources({ sources: [{ title: "nameless" }, "nope", null] })).toEqual([]);
  expect(parseSources(null)).toEqual([]);
});

test("a broken source reads as broken — in form, not only in number", () => {
  const broken = row({ state: "failing", lastError: "permission denied", failures: 3 });
  expect(stateLabel(broken)).toBe("not answering");
  expect(stateTone(broken)).toBe("danger");
  expect(stateTone(row({ state: "watching" }))).toBe("ok");
  expect(stateTone(row({ state: "stopped" }))).toBe("neutral");
});

test("freshness is measured from the server's instant", () => {
  const at = Date.parse("2026-09-04T10:00:00Z");
  const fresh = row({ lastEvent: "2026-09-04T10:00:00Z" });
  expect(freshness(fresh, at + 30_000)).toBe("just now");
  expect(freshness(fresh, at + 4 * 60_000)).toBe("4 min ago");
  expect(freshness(fresh, at + 3 * 3_600_000)).toBe("3 h ago");
  expect(freshness(fresh, at + 5 * 86_400_000)).toBe("5 d ago");
  // Clock skew reads as "just now" rather than as a negative age: the
  // skew is the client's, and a number invented from it would be worse.
  expect(freshness(fresh, at - 60_000)).toBe("just now");
  // A server instant that is not one claims nothing.
  expect(freshness(row({ lastEvent: "soon" }), at)).toBe(UNKNOWN);
});

test("the counts line says what the source did, and stays quiet about zero failures", () => {
  expect(countsLine(row({ ingested: 7 }))).toBe("7 ingested");
  expect(countsLine(row({ ingested: 7, failures: 2 }))).toBe("7 ingested · 2 failed");
});

test("the first-run offer stands while nothing has been ingested", () => {
  expect(isFirstRun([])).toBe(true);
  expect(isFirstRun([row({ ingested: 0 })])).toBe(true);
  expect(isFirstRun([row({ ingested: 0 }), row({ id: "b", ingested: 3 })])).toBe(false);
});

test("the credential note names the profile, and tells unchecked from broken", () => {
  const [named] = parseSources(served({ state: "watching" }));
  expect(named && credentialNote(named)).toBeUndefined();

  const withProfile = { ...served({ state: "watching" }) } as {
    sources: Record<string, unknown>[];
  };
  withProfile.sources[0] = {
    ...withProfile.sources[0],
    credentialProfile: "imagery-reader",
    credentialResolved: null,
  };
  const [unchecked] = parseSources(withProfile);
  expect(unchecked && credentialNote(unchecked)).toBe(
    "credential imagery-reader · not checked yet",
  );

  withProfile.sources[0] = { ...withProfile.sources[0], credentialResolved: false };
  const [missing] = parseSources(withProfile);
  expect(missing && credentialNote(missing)).toBe("credential imagery-reader · did not resolve");

  withProfile.sources[0] = { ...withProfile.sources[0], credentialResolved: true };
  const [ok] = parseSources(withProfile);
  expect(ok && credentialNote(ok)).toBe("credential imagery-reader · resolved");
});
