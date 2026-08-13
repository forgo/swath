// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The deferral-pointer gate (issue #173): `docs/ROADMAP.md` §2 is the
//! canonical deferral inventory, and its own convention says every prose
//! "future work" note points there (or, for ADR-governed deferrals, at
//! the governing ADR whose reopen condition wins). This gate makes the
//! convention self-enforcing: in the scanned prose docs, any paragraph
//! carrying deferral language ([`TRIGGERS`]) must also carry a pointer —
//! `ROADMAP`, an `ADR` reference, or a `decisions/` link — in the same
//! paragraph (a contiguous run of non-blank lines; whole lists and
//! tables count as one paragraph, fenced code blocks are ignored).
//!
//! Scope: `README.md`, `docs/*.md`, and `docs/design/*.md` — minus
//! `docs/ROADMAP.md` (it IS the inventory). `docs/decisions/` and
//! `prototypes/` are immutable by project rule and out of scope;
//! `docs/perf/` and `docs/media/` hold generated evidence and diagram
//! notes, not deferral-owning prose.
//!
//! False positives go on [`ALLOWLIST`] — explicit, reason-carrying,
//! per-paragraph — and a stale entry (matching nothing) fails the gate,
//! so the escape hatch can only ever shrink the check, never rot it.

use super::{repo_root, strip_code_fences};

/// Deferral language (matched case-insensitively).
const TRIGGERS: [&str; 3] = ["deferred", "deferral", "future work"];

/// Accepted pointers (case-sensitive; matched within the paragraph):
/// the roadmap by name, an ADR reference (`ADR 0013` / `per-ADR-0005`),
/// or a link into `docs/decisions/`.
const POINTERS: [&str; 3] = ["ROADMAP", "ADR", "decisions/"];

/// The explicit exceptions: (file, unique paragraph snippet, reason).
/// Every entry must match a currently-flagged paragraph — a stale entry
/// fails the gate.
const ALLOWLIST: [(&str, &str, &str); 2] = [
    (
        "docs/ARCHITECTURE.md",
        "Nothing here is aspirational",
        "§4's preamble points deferred surfaces at the §7 phase tables and the \
         standards map, which carry the inventory/ADR pointers themselves",
    ),
    (
        "docs/design/materialization-planner.md",
        "is named future work below",
        "an in-document forward reference to §6, whose 'Recorded future work' \
         list carries the ROADMAP deferral-inventory pointer",
    ),
];

/// The scanned files, repo-relative: `README.md`, `docs/*.md` (minus
/// `ROADMAP.md`), `docs/design/*.md`.
fn scope() -> Vec<String> {
    let root = repo_root();
    let mut files = vec!["README.md".to_owned()];
    for dir in ["docs", "docs/design"] {
        let mut entries: Vec<String> = std::fs::read_dir(root.join(dir))
            .expect("docs directory exists")
            .map(|entry| entry.expect("readable dir entry").file_name())
            .filter_map(|name| {
                let name = name.to_str().expect("utf-8 filename").to_owned();
                let is_md = std::path::Path::new(&name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
                (is_md && name != "ROADMAP.md").then(|| format!("{dir}/{name}"))
            })
            .collect();
        entries.sort();
        files.append(&mut entries);
    }
    files
}

/// The check over one file's text: every deferral paragraph carries a
/// pointer or a (consumed) allowlist entry; returns the violations and
/// the allowlist entries used.
fn scan(file: &str, text: &str) -> (Vec<String>, Vec<usize>) {
    let mut violations = Vec::new();
    let mut allowlist_hits = Vec::new();
    let prose = strip_code_fences(text);
    for paragraph in prose.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() || paragraph.starts_with('#') {
            continue;
        }
        let lower = paragraph.to_lowercase();
        if !TRIGGERS.iter().any(|t| lower.contains(t)) {
            continue;
        }
        if POINTERS.iter().any(|p| paragraph.contains(p)) {
            continue;
        }
        if let Some(idx) = ALLOWLIST
            .iter()
            .position(|(f, snippet, _)| *f == file && paragraph.contains(snippet))
        {
            allowlist_hits.push(idx);
            continue;
        }
        let head = paragraph.lines().next().unwrap_or_default();
        violations.push(format!(
            "{file}: deferral language with no ROADMAP/ADR pointer in the \
             paragraph starting: {head:?}"
        ));
    }
    (violations, allowlist_hits)
}

/// The gate: scans every in-scope file; zero unpointed deferrals, zero
/// stale allowlist entries.
pub(super) fn check() -> Result<(), String> {
    let mut violations = Vec::new();
    let mut used = [false; ALLOWLIST.len()];
    for file in scope() {
        let text = super::read_repo(&file);
        let (mut file_violations, hits) = scan(&file, &text);
        violations.append(&mut file_violations);
        for idx in hits {
            used[idx] = true;
        }
    }
    for (entry, used) in ALLOWLIST.iter().zip(used) {
        if !used {
            violations.push(format!(
                "stale allowlist entry (no longer matches any flagged paragraph — \
                 remove it): {entry:?}"
            ));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "deferral-pointer gate: every prose deferral must point at \
             docs/ROADMAP.md's inventory (or its governing ADR) from the same \
             paragraph:\n  {}",
            violations.join("\n  ")
        ))
    }
}

#[test]
fn every_deferral_points_at_the_roadmap_or_an_adr() {
    check().unwrap();
}

#[test]
fn an_unpointed_deferral_is_caught() {
    let (violations, _) = scan(
        "docs/EXAMPLE.md",
        "This capability is deferred until someone needs it.",
    );
    assert_eq!(violations.len(), 1, "{violations:?}");
}

#[test]
fn a_pointed_deferral_passes() {
    let (violations, _) = scan(
        "docs/EXAMPLE.md",
        "This capability is deferred until someone needs it — tracked in \
         [`ROADMAP.md`](ROADMAP.md)'s deferral inventory.",
    );
    assert!(violations.is_empty(), "{violations:?}");
}
