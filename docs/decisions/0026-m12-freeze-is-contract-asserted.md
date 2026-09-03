<!-- SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
     SPDX-License-Identifier: Apache-2.0 -->

# ADR 0026 — The M12 freeze is contract-asserted, not path-asserted

**Status:** Accepted · **Date:** 2026-09-03 · **Amends:** ADR 0021 (the UI system) ·
**Refs:** `docs/design/ui-system.md` §8, `web/src/ui/contract.test.ts`,
`web/src/ui/tokens.test.ts`, `web/src/ui/icon.test.ts`,
`web/scripts/check-ui-dry.mjs`, issue #378

## Context

ADR 0021 fenced M12 to values, glyphs and themes, and both it and `ui-system.md` §8 state that "a CI
path check on M12 PRs makes it mechanical: a diff outside those files fails".

**No such check exists.** A sweep of `.github/workflows/`, the justfile modules and the test tree
finds only the two sentences claiming it. What actually holds the line is three tests and a script:
`tokens.test.ts` (the token name set, plus pinned shell geometry), `icon.test.ts` (the icon name
set), `check-ui-dry.mjs` (one home per kind of literal), and the per-primitive suites.

The gap surfaced when four M12 tasks needed to edit a primitive for the express purpose of *removing*
structure-independent values from it — moving `12px` out of `icon.ts` into a token, moving a
hard-coded page colour out of `field.ts` into a role. The path fence forbids exactly those edits,
while permitting a rewrite of `tokens.css` that changes what every organism can address. It is
pointed at the wrong thing.

## Decision

1. **The rule is about the contract, not the file.** During M12 a primitive may be edited only to
   replace a literal with a token reference, or to consume a token. It may not add, remove or rename
   a part, a slot, an observed attribute, or an event.

2. **The rule is asserted, not enumerated.** `web/src/ui/contract.test.ts` pins, for every primitive:
   its part vocabulary, its observed attributes, and — via the catalog every element dispatches
   through — the event-name set. A structural change fails there no matter which file it is written
   in, which an allowlist of paths cannot do.

3. **The claim of a path check is withdrawn** from ADR 0021 and `ui-system.md` §8. A gate that does
   not exist is worse than no gate: it is a sentence a reviewer trusts.

4. **Changing a contract row is a contract change.** It lands in the same PR as its reason, and the
   PR says what consumes the new part or attribute. This is the same discipline the token and icon
   name lists already carry.

## Consequences

- The four M12 value-extraction tasks are unblocked without weakening anything.
- Structural drift is caught in any file, including new ones — the path fence would have missed a
  primitive added under a different directory.
- The contract table is a maintenance surface: a genuine new part means a table edit. That cost is
  the point; it is what makes the addition visible in review.
- The freeze's other halves are unchanged: token names, icon names, the region layout and the
  accessibility rules stay as ADR 0021 wrote them.

## Alternatives considered

**Build the path check the ADR described.** It would forbid the edits M12 needs, and permit
structural change inside the allowed files. Enumerating paths tests a proxy for the property; the
property is testable directly.

**Leave the sentence and rely on review.** Rejected on the same grounds the rest of this repo's
gates exist: a rule nothing checks is a rule that has already drifted.
