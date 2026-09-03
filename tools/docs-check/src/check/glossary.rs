// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The glossary gate (issue #396): `web/src/glossary.ts` defines the terms
//! the interface offers to explain, and each entry names the document it was
//! drawn from. This checks that the citation is real — the document exists
//! and still uses the term — so a definition cannot outlive the prose behind
//! it.
//!
//! Closed in both directions, like every allowlist in this gate: an entry
//! whose source has been deleted or renamed fails, and an entry whose source
//! no longer mentions the term fails. What it deliberately does NOT check is
//! whether the wording agrees — prose paraphrases, and a byte comparison
//! there would only teach people to stop citing.

use super::{read_repo, repo_root};

/// The glossary module, as text. Parsed rather than generated: the gate must
/// read what ships, not a copy of it.
fn glossary_source() -> String {
    read_repo("web/src/glossary.ts")
}

/// `(term, source)` for every entry, in file order.
fn entries(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut term: Option<String> = None;
    for line in source.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("term: \"") {
            term = rest.strip_suffix("\",").map(str::to_owned);
        } else if let Some(rest) = line.strip_prefix("source: \"")
            && let (Some(name), Some(path)) = (term.take(), rest.strip_suffix("\","))
        {
            found.push((name, path.to_owned()));
        }
    }
    found
}

/// Every entry cites a document that exists and still uses the term.
#[test]
fn every_glossary_entry_cites_a_real_source() {
    let source = glossary_source();
    let entries = entries(&source);
    assert!(
        entries.len() >= 8,
        "expected the glossary's entries to parse, got {}",
        entries.len()
    );
    for (term, doc) in &entries {
        let path = repo_root().join(doc);
        assert!(
            path.is_file(),
            "glossary term `{term}` cites {doc}, which is not a file"
        );
        let text = read_repo(doc).to_lowercase();
        assert!(
            text.contains(&term.to_lowercase()),
            "glossary term `{term}` cites {doc}, which no longer mentions it"
        );
    }
}

/// No term is defined twice, and every term is lowercase — the interface
/// uppercases labels in CSS, so a capital here would ship as a capital.
#[test]
fn terms_are_unique_and_lowercase() {
    let source = glossary_source();
    let mut terms: Vec<String> = entries(&source).into_iter().map(|(t, _)| t).collect();
    let count = terms.len();
    terms.sort();
    terms.dedup();
    assert_eq!(count, terms.len(), "a term is defined twice: {terms:?}");
    for term in &terms {
        assert_eq!(
            term,
            &term.to_lowercase(),
            "glossary terms are lowercase in source"
        );
    }
}

/// A definition that cites a document which does not mention the term fails.
/// The gate's own acceptance bar — the parser and the assertion are exercised
/// against a fixture, not just against the passing tree.
#[test]
fn a_stale_citation_fails() {
    let fixture = "\
  {
    term: \"nonesuch\",
    definition: \"A word no document uses.\",
    source: \"docs/CHARTER.md\",
  },
";
    let parsed = entries(fixture);
    assert_eq!(parsed.len(), 1, "the fixture must parse as one entry");
    let (term, doc) = &parsed[0];
    assert_eq!(term, "nonesuch");
    assert!(
        !read_repo(doc).to_lowercase().contains(term),
        "the fixture's whole point is that {doc} does not contain `{term}`"
    );
}
