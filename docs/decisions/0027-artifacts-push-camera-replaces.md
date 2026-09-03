<!-- SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
     SPDX-License-Identifier: Apache-2.0 -->

# ADR 0027 — Artifacts push history, the camera replaces it

**Status:** Accepted · **Date:** 2026-09-03 · **Amends:** ADR 0021 decision 3 ·
**Refs:** ADR 0011 (embedded UI, no SPA rewrite), `docs/design/ui-system.md` §4.5,
`web/src/view-state.ts`, `web/demo/main.ts`, issue #392

## Context

ADR 0021 decision 3 wrote modes into the URL with `replaceState` **only**, and recorded the trade
explicitly: it "accepts no back-button between modes". That was the right call when the URL carried a
camera and a mode, and when `pushState` would have meant forty history entries from one pan.

The cost has since become the product's. The URL now names a layer, a frame, a compare pairing, an
authoring selection, a panel and the x-ray — things a person navigates *to*. With `replaceState`
only, the browser's back button leaves the application entirely, and there is no way back to the view
you were looking at a moment ago. The design work of M14 makes those artifacts visible as breadcrumb
chips, and a chip you cannot step back through is a label rather than a control.

ADR 0021 named its own reopen condition: "a real need for back-button navigation". This is it.

## Decision

1. **An artifact change pushes.** The layer, the frame, the compare pairing and its handle, the
   x-ray, the mode and the authoring selection are navigation; each change is a `pushState`.

2. **A camera move replaces.** `center` and `zoom` are not navigation. Panning and zooming continue
   to `replaceState`, coalesced as before, so a pan never buries the view you came from.

3. **State the app drives replaces, whatever it changes.** The cinematic loop's frame advances change
   an artifact but are not a person navigating; they update the URL in place. A loop that pushed a
   frame per second would fill history with a slideshow.

4. **The comparison is against the last state written, not against the URL.** On the first
   interaction the URL gains fields that were implicit all along — the layer the server picked, the
   frame it opened on — and a pan that merely makes them explicit is not navigation.

5. **`popstate` drives the shell from the URL**, which is the same path a cold load takes, so a
   restored state is indistinguishable from a pasted one.

6. **Query params, not paths.** ADR 0011 is untouched: unknown paths still return a plain 404, the
   server is unchanged, and every URL the app writes preserves `location.pathname`.

## Consequences

- Back and forward walk the artifacts, without a reload.
- The byte-stability property is unchanged: a pasted deep link is still never rewritten, because the
  no-op guard runs before the push/replace choice is made.
- Deciding push-vs-replace is now a real distinction the code has to keep making. It is enforced by
  `viewArtifactsEqual`, which is the one place that says which fields are artifacts, and by e2e tests
  asserting both halves: that back walks modes, and that three pans add no history entries.
- The trade ADR 0021 accepted is withdrawn deliberately, not quietly — this ADR exists so a reader of
  0021 finds the amendment rather than a contradiction.

## Alternatives considered

**Push everything.** What ADR 0021 rejected, and rightly: a pan-heavy session would make the back
button useless, which is worse than not having it.

**Real path routing** (`/layers/ndvi/2024-06-06`). It needs an SPA fallback on the server, which ADR
0011 forbids for good reason — a static embedded UI that 404s honestly is a property worth more than
prettier URLs. Chips over query params give the same shareability and the same back-button behaviour
with no server change.
