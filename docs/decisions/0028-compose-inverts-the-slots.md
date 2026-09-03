<!-- SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
     SPDX-License-Identifier: Apache-2.0 -->

# ADR 0028 — Composing inverts the slots; the map is never smaller than a preview

**Status:** Accepted · **Date:** 2026-09-03 · **Amends:** ADR 0021 §1 ·
**Refs:** ADR 0014 (the bounded `POST /result` preview), ADR 0025 (the always-valid canvas),
`docs/design/ui-system.md` §6, issue #400

## Context

ADR 0021 §1 fixed the shell at five regions and required that **the map is always present**. The
reason was sound and still is: Swath's claim is that you can always see the pixels, and a
form-filling app that hides them is the thing the product exists not to be.

What shipped honours the letter and defeats the spirit. The pipeline lives in a bottom drawer that
**overlays** the map — at the pinned 960×860 viewport it covers roughly 327px of it. So while
composing, the map is simultaneously present and useless: it is there, and the thing you are
building is on top of it. The canvas, meanwhile, gets a 38%-height strip to draw a graph in.

The design work of M15 wants the canvas to take the screen. Read strictly, ADR 0021 §1 forbids that.
Read for its intent, ADR 0021 §1 is what the overlay already violates.

## Decision

1. **The map is always present *and never smaller than a live preview*.** That is the amended rule.
   "Present" is not satisfied by a region something else is drawn over.

2. **Composing inverts the slot relationship.** The canvas takes the main region; the map becomes a
   preview column beside it, `--swath-size-preview` wide (320px, the same width as the inspector,
   because it is the same kind of column — a readable secondary panel). It is never covered.

3. **No new region.** The five regions of §1 are unchanged; this is which slot fills which, and it is
   expressed as one attribute on the shell (`compose`) plus CSS. `swath-shell` gains no new part.

4. **Below the medium tier the map keeps the region** and the canvas returns to a sheet over it.
   Two columns need width that a phone does not have, and a 320px preview beside a 100px canvas
   serves nobody.

## Consequences

- The canvas gets the space a graph editor needs, which M15's remaining work (typed insertion, joins,
  the order warning) assumes.
- The preview column is where #401's "the preview follows the selected step" renders — the map
  becomes the thing that answers the question the canvas is asking, rather than scenery behind it.
- The rule is now testable in a way "always present" was not: a region that is `inset: 0` under an
  overlay passes "present" and fails "never smaller than a preview".
- One more layout state to keep honest across tiers, and the screenshots change.

## Alternatives considered

**Leave the drawer and make it taller.** It is the same defeat at a larger size: the map stays
covered, and the deeper the drawer the less true "always present" becomes.

**Drop the map while composing.** Coherent, and rejected: the preview is the product's argument.
ADR 0014's bounded `POST /result` exists so that composing can show pixels, and a compose mode with
nowhere to put them wastes it.

**A floating preview card over the canvas.** Rejected for the reason §1 exists — a floating thing is
dismissible, coverable, and easy to lose. A column is a promise.
