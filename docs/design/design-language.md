<!-- SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
     SPDX-License-Identifier: Apache-2.0 -->

# The Swath design language — voice, elevation, and the closed sets

`ui-system.md` says what the interface *is*: tokens, primitives, five regions, four modes. It does
not say what it should look and sound like, so a reviewer arguing about a label has nothing to cite.
This is that companion. Where the two disagree, `ui-system.md` and the ADRs win — this document
describes taste, not structure.

## 1. Two registers, never mixed in one sentence

**Instrument.** Mono, lowercase, terse, unit-bearing. Readouts, badges, mode labels, status cells:
`545 ms · 78.8 KB`, `cache`, `ingest→pixel`. No articles, no verbs, no apology. This is the register
of something being measured.

**Explaining.** Sentence case, complete sentences, no jargon the reader has not already been given.
Field help, refusals, empty states, glossary definitions. This is the register of something being
taught.

A sentence belongs to one register. "Cache hit — 545 ms · 78.8 KB, which means the tile was already
built" is two voices in one breath; the card says the numbers and the explanation offers the
sentence.

## 2. Casing and naming

- Labels are **lowercase in source**, uppercased by CSS. The transform is a style decision, so it
  stays in the stylesheet where a theme can undo it.
- Menu items, buttons and all prose are **sentence case**.
- Title Case appears nowhere. `Swath` is the one capitalised word, and only as the product's name.
- Nothing is named after a milestone, an issue, or an internal module. The reader is an EO
  practitioner, not a contributor.

## 3. Numbers

Every figure carries its unit and, where it is a measurement, its measurement. A figure we do not
have renders **an em dash** — never `0`, never a spinner, never a placeholder that will be right in
a moment. `0` is a measured zero and means something; `—` means we have not measured it.

Numbers that change while you watch them are set in tabular figures, so a readout never re-lays-out
under the pointer. That is set once on the shared host sheet, not per component.

No adjective the test suite cannot assert. "Fast" is not a claim we make; a number with a unit is.

## 4. Refusals

A refusal is a first-class result, not an error state. It explains itself **in the server's own
words** — we do not paraphrase a reason we did not compute — is routed to the control that caused
it, and clears on the next edit to that control. A refusal that appears far from its cause, or that
survives the fix, is a bug regardless of the words in it.

The product refuses rather than degrades (ADR 0014). The interface's job is to make the refusal
legible, not to soften it.

## 5. Elevation is translucency

A floating surface earns depth by letting the map show through it, never by casting a shadow.
`--swath-shadow-hud` is `none` deliberately: a shadow over a live render reads as dirt on the glass.
Translucency also keeps the promise that the pixels are always present — you can see what the card is
covering.

Under `prefers-reduced-transparency`, the blur goes to zero and the surface goes opaque. Depth is a
convenience; legibility is not.

## 6. Motion

Two durations and one curve, both tokens. Motion exists to show where something came from — a panel
sliding from the edge it belongs to, a value counting to its new figure — and for nothing else.
Everything stops under `prefers-reduced-motion`, enforced by the shared sheet rather than by each
component remembering.

## 7. One hover gesture

Hover reveals detail in place; it never moves layout. If hovering a row changes what is on the map,
the same information is reachable by keyboard focus, because hover is not an interaction a keyboard
or a touchscreen has.

## 8. Closed sets

Three of the system's vocabularies are closed, and each is closed by a test rather than by
convention:

| Vocabulary | Where | What the test pins |
|---|---|---|
| Tokens | `tokens.css` | the exact set of names, plus pinned shell geometry |
| Icons | `icons.svg` | the exact set of names, and that no two are the same drawing |
| Decision colours | `tokens.css` | contrast against every surface, and distinctness from `info` |

Closed means: adding one is a deliberate edit in the same diff as its reason, visible in review.
It does not mean frozen.

**No emoji anywhere** — not in the interface, not in labels, not in docs headings. An emoji is an
uncontrolled glyph from a font we do not ship, at a size we do not set, in a colour no theme can
reach.

## 9. Colour carries meaning, or it is not used

Semantic colour (decision, info, warning, danger) is separate from the accent, and no two semantic
roles share a value — a badge whose colour is also a link's colour cannot be read as a badge. The
heat ramp is the one sequential scale; it carries an ink colour for each end, because every bucket in
it has a number written on it.

Colour is never the only encoding. A broken source reads as broken in its shape and its words too.

## 10. Themes

The dark instrument palette is the product's identity, not a preference. The one alternative is a
**high-contrast** theme under `prefers-contrast: more`, which exists for an accessibility need and
not for a second look: hairlines go opaque, translucency goes off, the focus ring thickens.

Every theme declares the complete token set. A theme that defines a subset leaves components reading
values from the palette they are no longer part of.
