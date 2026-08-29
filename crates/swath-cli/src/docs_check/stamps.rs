// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The source-fingerprint freshness gate (issues #173 and #224): every
//! `_Last verified against sources `<fingerprint>`._` stamp in
//! `docs/ARCHITECTURE.md` and `docs/EXTENDING.md` must equal the current
//! fingerprint of the section's **referenced source files** — the
//! explicit, per-section file sets in [`SECTIONS`], which name exactly
//! the files whose content each section quotes or mirrors. The
//! fingerprint is the first [`FINGERPRINT_LEN`] hex digits of SHA-256
//! over each referenced file's path and newline-normalized content, so
//! it depends on nothing but the bytes in the checkout: any edit to a
//! referenced file changes it (the gate fails, printing the new value,
//! until the section is re-verified and re-stamped), and nothing else
//! ever does.
//!
//! Content, not commit shas (issue #224): the gate originally stamped a
//! commit sha and asked git whether later commits touched the sources.
//! Squash-merging broke that design structurally — the in-PR sha a PR
//! had to stamp is discarded by the squash: it is neither an ancestor of
//! the new `main` nor even an *object* in a fresh `refs/heads`-only
//! clone (which is exactly what CI's `fetch-depth: 0` checkout is), so
//! `main` went red until a follow-up re-stamp (#219, #225, #228). A
//! content fingerprint is invariant under history rewrites: the squash
//! commit carries the PR's tree verbatim, so a stamp that was green on
//! the PR branch is green on `main` with no follow-up, and a stamp only
//! goes stale when the sources' content actually changes
//! ([`stamps_survive_squash_merge`] pins both halves).
//!
//! The map is closed in both directions: a stamp with no [`SECTIONS`]
//! entry fails (new stamped sections must declare their sources here),
//! and a [`SECTIONS`] entry whose stamp disappeared fails too.
//!
//! Git availability: the freshness check itself needs no git at all — it
//! runs on shallow and git-less checkouts alike. [`history_available`]
//! remains for the tests that DO need history (the mutation drift that
//! reconstructs pre-sweep source content from historical commits, the
//! squash simulation): on a shallow or git-less checkout they skip with
//! a notice — except when `SWATH_DOCS_CHECK_REQUIRE_GIT` is set (CI's
//! dedicated docs job checks out full history and sets it, so CI can
//! never skip silently).

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use sha2::Digest as _;

use super::{read_repo, repo_root};

/// One stamped section and the source files it verifies against.
struct Section {
    /// The section heading prefix (unique within the doc), e.g. `## 6.`.
    heading: &'static str,
    /// Repo-relative files whose content the stamp fingerprints.
    files: &'static [&'static str],
}

/// The two stamped documents and their per-section referenced sources.
/// The sets are deliberately narrow — the files a section quotes or
/// mirrors, not everything it mentions in passing:
///
/// - ARCHITECTURE §4 / §12 (component model, crate layout) reference the
///   workspace manifest: the crate set is the claim.
/// - ARCHITECTURE §6 (verbatim port traits + core entry points)
///   references the trait/entry-point source files it quotes.
/// - ARCHITECTURE §7 (inbound APIs) references the four router files —
///   the mounted surface is the claim (the adapter table's crate set is
///   covered by the §12 manifest watch).
/// - EXTENDING §2/§3/§4 reference the files their signature blocks are
///   copied verbatim from.
const SECTIONS: [(&str, &[Section]); 2] = [
    (
        "docs/ARCHITECTURE.md",
        &[
            Section {
                heading: "## 4.",
                files: &["Cargo.toml"],
            },
            Section {
                heading: "## 6.",
                files: &[
                    "crates/swath-core/src/source.rs",
                    "crates/swath-core/src/reproject.rs",
                    "crates/swath-core/src/catalog.rs",
                    "crates/swath-core/src/cache.rs",
                    "crates/swath-core/src/events.rs",
                    "crates/swath-core/src/ingest.rs",
                    "crates/swath-core/src/udf.rs",
                    "crates/swath-planner/src/lib.rs",
                    "crates/swath-render/src/process.rs",
                    "crates/swath-render/src/tiler.rs",
                ],
            },
            Section {
                heading: "## 7.",
                files: &[
                    "crates/swath-api/src/routes.rs",
                    "crates/swath-api/src/granules.rs",
                    "crates/swath-api/src/openeo/mod.rs",
                    "crates/swath-api/src/openeo/handlers.rs",
                    "crates/swath-api/src/openeo/types.rs",
                    "crates/swath-api/src/openeo/errors.rs",
                    "crates/swath-api/src/datasets.rs",
                ],
            },
            Section {
                heading: "## 12.",
                files: &["Cargo.toml"],
            },
        ],
    ),
    (
        "docs/EXTENDING.md",
        &[
            Section {
                heading: "## 2.",
                files: &[
                    "crates/swath-core/src/source.rs",
                    "crates/swath-testsupport/src/truth.rs",
                    "crates/swath-cli/src/source.rs",
                ],
            },
            Section {
                heading: "## 3.",
                files: &["crates/swath-render/src/process.rs"],
            },
            Section {
                heading: "## 4.",
                files: &[
                    "crates/swath-render/src/ir.rs",
                    "crates/swath-render/src/colormaps.rs",
                ],
            },
        ],
    ),
];

/// The stamp marker as it appears in the docs.
const STAMP_PREFIX: &str = "_Last verified against sources `";

/// Hex digits kept from the SHA-256 digest — 48 bits, plenty for a
/// drift detector (collisions would have to be engineered, and an
/// engineered collision defeats only the author's own gate).
const FINGERPRINT_LEN: usize = 12;

/// The fingerprint of a file set under `read`: the first
/// [`FINGERPRINT_LEN`] hex digits of SHA-256 over each file's path and
/// content (NUL-separated, in declaration order), line endings
/// normalized to `\n` so autocrlf checkouts fingerprint identically.
fn fingerprint<F>(files: &[&str], read: F) -> Result<String, String>
where
    F: Fn(&str) -> Result<String, String>,
{
    let mut hasher = sha2::Sha256::new();
    for file in files {
        hasher.update(file.as_bytes());
        hasher.update([0]);
        hasher.update(read(file)?.replace("\r\n", "\n").as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(FINGERPRINT_LEN);
    for byte in digest.iter().take(FINGERPRINT_LEN / 2) {
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(hex)
}

/// The fingerprint of `files` as checked out under `root`.
fn checkout_fingerprint(root: &Path, files: &[&str]) -> Result<String, String> {
    fingerprint(files, |file| {
        let path = root.join(file);
        std::fs::read_to_string(&path)
            .map_err(|err| format!("cannot read referenced source {file}: {err}"))
    })
}

/// Runs git in the repo root, returning raw stdout or the failure.
fn git_stdout(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(args)
        .output()
        .map_err(|err| format!("cannot run git: {err}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// [`git_stdout`] with the output trimmed (for ref-shaped answers).
fn git(args: &[&str]) -> Result<String, String> {
    git_stdout(args).map(|out| out.trim().to_owned())
}

/// Whether full git history is available for the history-dependent tests
/// (the mutation drift reconstructing pre-sweep content, the squash
/// simulation — the freshness check itself no longer needs git). `Err`
/// when it is not but `SWATH_DOCS_CHECK_REQUIRE_GIT` demands it.
#[expect(
    clippy::print_stderr,
    reason = "test-only skip notice; tracing is not initialized in the docs gate"
)]
pub(super) fn history_available() -> Result<bool, String> {
    let reason = match git(&["rev-parse", "--is-shallow-repository"]) {
        Ok(shallow) if shallow == "false" => return Ok(true),
        Ok(_) => "the checkout is shallow (no history)".to_owned(),
        Err(err) => format!("git is unavailable here: {err}"),
    };
    if std::env::var_os("SWATH_DOCS_CHECK_REQUIRE_GIT").is_some() {
        Err(format!(
            "SWATH_DOCS_CHECK_REQUIRE_GIT is set but {reason} — \
             the stamp gate's history-dependent tests cannot run"
        ))
    } else {
        eprintln!("docs_check::stamps history-dependent tests skipped: {reason}");
        Ok(false)
    }
}

/// The section map of `doc_label`, or an error for an unmapped doc.
fn doc_sections(doc_label: &str) -> Result<&'static [Section], String> {
    SECTIONS
        .iter()
        .find(|(label, _)| *label == doc_label)
        .map(|(_, sections)| *sections)
        .ok_or_else(|| format!("{doc_label} has no section map"))
}

/// The stamp fingerprint of `section` in `doc`: the first stamp line
/// after the section's heading and before the next `## ` heading.
fn section_stamp(doc_label: &str, doc: &str, heading: &str) -> Result<String, String> {
    let start = doc
        .find(&format!("\n{heading}"))
        .ok_or_else(|| format!("{doc_label} has no `{heading}` section"))?;
    let body = &doc[start + 1..];
    let end = body[heading.len()..]
        .find("\n## ")
        .map_or(body.len(), |pos| pos + heading.len());
    let section = &body[..end];
    let stamp = section
        .find(STAMP_PREFIX)
        .ok_or_else(|| format!("{doc_label} `{heading}` has no `{STAMP_PREFIX}…` stamp"))?;
    let token = &section[stamp + STAMP_PREFIX.len()..];
    let token = &token[..token
        .find('`')
        .ok_or_else(|| format!("{doc_label} `{heading}` stamp is not backtick-terminated"))?];
    Ok(token.to_owned())
}

/// The freshness check for one document's text against its section map.
pub(super) fn check_doc(doc_label: &str, doc: &str) -> Result<(), String> {
    let sections = doc_sections(doc_label)?;

    // Both directions closed: every stamp mapped, every mapping stamped.
    let stamp_count = doc.matches(STAMP_PREFIX).count();
    if stamp_count != sections.len() {
        return Err(format!(
            "{doc_label} carries {stamp_count} `{STAMP_PREFIX}…` stamps but the \
             docs_check::stamps section map declares {} — every stamped section \
             must declare its referenced source files there",
            sections.len()
        ));
    }

    let mut drift = Vec::new();
    for section in sections {
        let heading = section.heading;
        let stamp = section_stamp(doc_label, doc, heading)?;
        let expected = checkout_fingerprint(&repo_root(), section.files)?;
        if stamp != expected {
            drift.push(format!(
                "`{heading}` stamp `{stamp}` is stale — the section's referenced \
                 sources {:?} currently fingerprint to `{expected}`",
                section.files
            ));
        }
    }
    if drift.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{doc_label} source-fingerprint stamps have gone stale (re-verify each \
             section against the named sources, then re-stamp with the printed \
             fingerprint):\n  {}",
            drift.join("\n  ")
        ))
    }
}

/// Whether `file` existed at commit `sha`.
fn exists_at(sha: &str, file: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["cat-file", "-e", &format!("{sha}:{file}")])
        .status()
        .is_ok_and(|status| status.success())
}

/// `doc` with every section's stamp replaced by the fingerprint its
/// referenced sources had at commit `sha` — the mutation tests' way of
/// reconstructing a genuinely pre-sweep stamp set (needs git history).
/// A referenced file that did not exist at `sha` (source extracted or
/// renamed since — e.g. the planner's ADR 0016 move into
/// `swath-planner`) fingerprints as empty content rather than failing
/// the reconstruction.
pub(super) fn restamped_at(doc_label: &str, doc: &str, sha: &str) -> Result<String, String> {
    let mut out = doc.to_owned();
    for section in doc_sections(doc_label)? {
        let current = section_stamp(doc_label, doc, section.heading)?;
        let historical = fingerprint(section.files, |file| {
            if exists_at(sha, file) {
                git_stdout(&["show", &format!("{sha}:{file}")])
            } else {
                Ok(String::new())
            }
        })?;
        out = out.replace(&format!("`{current}`"), &format!("`{historical}`"));
    }
    Ok(out)
}

#[test]
fn architecture_stamps_are_fresh() {
    check_doc("docs/ARCHITECTURE.md", &read_repo("docs/ARCHITECTURE.md")).unwrap();
}

#[test]
fn extending_stamps_are_fresh() {
    check_doc("docs/EXTENDING.md", &read_repo("docs/EXTENDING.md")).unwrap();
}

/// The issue #224 acceptance scenario, run against a scratch repository:
/// a stamp minted on a PR branch survives the squash-merge that discards
/// the PR's commits (sha still an object, no longer an ancestor — the
/// exact shape that reddened `main` under the sha-stamp design in #219,
/// #225 and #228), and still goes stale when a source genuinely changes
/// afterwards.
#[test]
fn stamps_survive_squash_merge() {
    if !history_available().unwrap() {
        return; // needs a usable git binary; CI's docs job never skips
    }
    let tmp = swath_testsupport::TempDir::new("squash-sim");
    let root = tmp.path();
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "gate@invalid"]);
    run(&["config", "user.name", "gate"]);
    std::fs::write(root.join("port.rs"), "pub fn port() {}\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-qm", "seed"]);

    // The PR branch changes the referenced source and mints a stamp.
    run(&["checkout", "-qb", "pr"]);
    std::fs::write(root.join("port.rs"), "pub fn port(v2: u8) {}\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-qm", "feat: widen the port"]);
    let pr_sha = run(&["rev-parse", "HEAD"]);
    let stamp = checkout_fingerprint(root, &["port.rs"]).unwrap();

    // Squash-merge: main gains ONE new commit carrying the PR's tree
    // verbatim; the PR branch (and its shas) are discarded.
    run(&["checkout", "-q", "main"]);
    run(&["merge", "--squash", "-q", "pr"]);
    run(&["commit", "-qm", "feat: widen the port (#1)"]);
    run(&["branch", "-qD", "pr"]);
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["cat-file", "-e", &format!("{pr_sha}^{{commit}}")])
            .status()
            .unwrap()
            .success(),
        "the discarded PR sha must still exist as a local object for the simulation"
    );
    assert!(
        !Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["merge-base", "--is-ancestor", &pr_sha, "HEAD"])
            .status()
            .unwrap()
            .success(),
        "the squash must have discarded the PR sha from main's ancestry"
    );

    // The content fingerprint survives the squash unchanged...
    assert_eq!(
        checkout_fingerprint(root, &["port.rs"]).unwrap(),
        stamp,
        "a stamp minted in-PR must stay fresh after the squash-merge"
    );

    // ...and still reddens when the source genuinely changes afterwards.
    std::fs::write(root.join("port.rs"), "pub fn port(v3: u16) {}\n").unwrap();
    assert_ne!(
        checkout_fingerprint(root, &["port.rs"]).unwrap(),
        stamp,
        "a genuine source change must still invalidate the stamp"
    );
}
