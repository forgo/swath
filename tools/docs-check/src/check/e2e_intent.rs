// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The e2e intent gate (issue #426): `docs/design/e2e-intent.md` says what
//! every browser spec is protecting, and this keeps the list closed in both
//! directions — a spec with no row fails, a row naming no spec fails.
//!
//! # Why this exists
//!
//! #426 is the debt record of the refactor-era CI lane, and its stated
//! failure mode is "a rewrite that drops one of these without saying so".
//! A prose list alone cannot prevent that: it rots the first time someone
//! renames a spec. This makes the list load-bearing, so dropping a
//! protection is a failing check rather than a quiet deletion.
//!
//! # What it deliberately does NOT check
//!
//! Whether a row is still *true*. A spec can be rewritten to assert
//! something weaker while keeping its title, and no gate can see that —
//! review does. What the gate guarantees is narrower and still worth
//! having: the set of protections is enumerated, and it cannot shrink in
//! silence.

use std::collections::BTreeSet;

use super::{read_repo, repo_root};

/// The intent list.
const DOC: &str = "docs/design/e2e-intent.md";

/// Every `test("…")` title in `web/e2e/*.e2e.ts`, as `file · title`.
///
/// Parsed from the source that ships rather than from a Playwright run:
/// the gate must be runnable without a browser or a stack.
fn specs() -> BTreeSet<String> {
    let dir = repo_root().join("web/e2e");
    let mut found = BTreeSet::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("cannot read {dir}: {err}", dir = dir.display()));
    for entry in entries {
        let path = entry.expect("a readable dir entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".e2e.ts") else {
            continue;
        };
        let source = read_repo(&format!("web/e2e/{name}"));
        for line in source.lines() {
            // A spec declares its title first: `test("…"` or
            // `test.skip("…"` (a skipped spec is still a protection
            // someone has to account for). A file-level guard —
            // `test.skip(cond, "reason")` — puts an expression first, and
            // is not a spec; requiring the quote to follow the paren
            // immediately is what tells them apart.
            // Nested specs (inside a `test.describe` group) are indented;
            // they are specs like any other, and the group itself is not
            // one — a group's title names a heading, not a protection.
            let Some(rest) = line.trim_start().strip_prefix("test") else {
                continue;
            };
            let rest = ["", ".skip", ".only", ".fixme"]
                .into_iter()
                .find_map(|prefix| rest.strip_prefix(prefix)?.strip_prefix("(\""));
            let Some(after) = rest else {
                continue;
            };
            let Some(end) = after.find('"') else {
                continue;
            };
            found.insert(format!("{stem} · {title}", title = &after[..end]));
        }
    }
    found
}

/// Every spec named by a row of the intent list, as `file · title`.
///
/// The doc groups specs under a heading naming the spec file, and lists
/// them as the first cell of a two-column table — so a row is keyed by its
/// heading plus its cell, which is exactly how a reader reads it.
fn documented() -> BTreeSet<String> {
    let doc = read_repo(DOC);
    let mut found = BTreeSet::new();
    let mut file: Option<String> = None;
    for line in doc.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("## `") {
            file = rest.split('`').next().map(str::to_owned);
            continue;
        }
        let Some(file) = file.as_deref() else {
            continue;
        };
        if !trimmed.starts_with('|') {
            continue;
        }
        let cell = trimmed
            .trim_start_matches('|')
            .split('|')
            .next()
            .unwrap_or_default()
            .trim();
        // Skip the header and its separator.
        if cell.is_empty() || cell == "Spec" || cell.starts_with("---") {
            continue;
        }
        found.insert(format!("{file} · {cell}"));
    }
    found
}

/// The gate: the two sets are equal.
///
/// # Errors
///
/// A message naming what is undocumented and what is phantom.
pub(super) fn intent_covers_every_spec() -> Result<(), String> {
    let specs = specs();
    let documented = documented();
    assert!(
        specs.len() > 50,
        "the spec scan found only {} specs — the parser is broken, not the suite",
        specs.len()
    );

    let undocumented: Vec<&String> = specs.difference(&documented).collect();
    let phantom: Vec<&String> = documented.difference(&specs).collect();
    if undocumented.is_empty() && phantom.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{DOC} has drifted from web/e2e (issue #426 — the list is the acceptance \
         criterion for the suite, so it is closed in both directions):\n  \
         specs with no row (add one saying what it protects): {undocumented:?}\n  \
         rows naming no spec (the protection was dropped or renamed): {phantom:?}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{documented, intent_covers_every_spec, specs};

    #[test]
    fn every_spec_has_a_recorded_intent() {
        intent_covers_every_spec().unwrap();
    }

    /// The parser reads real titles, not an empty set that would make the
    /// gate vacuously green.
    #[test]
    fn the_scan_finds_the_suite() {
        let specs = specs();
        assert!(specs.len() > 50, "found {}", specs.len());
        assert!(
            specs
                .iter()
                .any(|spec| spec.starts_with("swath-map · map loads")),
            "the smoke spec is missing from the scan: {specs:?}"
        );
        // And the doc side is read the same way.
        assert_eq!(specs.len(), documented().len());
    }
}
