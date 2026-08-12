// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The sha-stamp freshness gate (issue #173): every
//! `_Last verified against `<sha>`._` stamp in `docs/ARCHITECTURE.md` and
//! `docs/EXTENDING.md` is checked against git history. The stamped sha
//! must (a) exist, (b) be an ancestor of `HEAD`, and (c) postdate the
//! last commit touching the section's **referenced source files** — the
//! explicit, per-section file sets in [`SECTIONS`], which name exactly
//! the files whose content each section quotes or mirrors. A commit that
//! touches a referenced file after the stamp fails the gate until the
//! section is re-verified and re-stamped.
//!
//! The map is closed in both directions: a stamp with no [`SECTIONS`]
//! entry fails (new stamped sections must declare their sources here),
//! and a [`SECTIONS`] entry whose stamp disappeared fails too.
//!
//! Git availability: the gate needs real history. On a shallow or
//! git-less checkout it skips with a notice — except when
//! `SWATH_DOCS_CHECK_REQUIRE_GIT` is set (CI's dedicated docs job checks
//! out full history and sets it, so CI can never skip silently).

use std::process::Command;

use super::{read_repo, repo_root};

/// One stamped section and the source files it verifies against.
struct Section {
    /// The section heading prefix (unique within the doc), e.g. `## 6.`.
    heading: &'static str,
    /// Repo-relative files whose history invalidates the stamp.
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
/// - ARCHITECTURE §7 (inbound APIs) references the three router files —
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
                    "crates/swath-core/src/planner.rs",
                    "crates/swath-render/src/process.rs",
                    "crates/swath-render/src/tiler.rs",
                ],
            },
            Section {
                heading: "## 7.",
                files: &[
                    "crates/swath-api/src/routes.rs",
                    "crates/swath-api/src/granules.rs",
                    "crates/swath-api/src/openeo.rs",
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
const STAMP_PREFIX: &str = "_Last verified against `";

/// Runs git in the repo root, returning trimmed stdout or the failure.
fn git(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(args)
        .output()
        .map_err(|err| format!("cannot run git: {err}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Whether full git history is available for freshness checks. `Err`
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
             the sha-stamp gate cannot run"
        ))
    } else {
        eprintln!("docs_check::stamps skipped: {reason}");
        Ok(false)
    }
}

/// The stamp sha of `section` in `doc`: the first stamp line after the
/// section's heading and before the next `## ` heading.
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
    let sha = &section[stamp + STAMP_PREFIX.len()..];
    let sha = &sha[..sha
        .find('`')
        .ok_or_else(|| format!("{doc_label} `{heading}` stamp is not backtick-terminated"))?];
    Ok(sha.to_owned())
}

/// The freshness check for one document's text against its section map.
pub(super) fn check_doc(doc_label: &str, doc: &str) -> Result<(), String> {
    if !history_available()? {
        return Ok(());
    }
    let sections = SECTIONS
        .iter()
        .find(|(label, _)| *label == doc_label)
        .map(|(_, sections)| *sections)
        .ok_or_else(|| format!("{doc_label} has no section map"))?;

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
        let sha = section_stamp(doc_label, doc, heading)?;
        if git(&["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_err() {
            drift.push(format!("`{heading}` stamp `{sha}` is not a commit"));
            continue;
        }
        if git(&["merge-base", "--is-ancestor", &sha, "HEAD"]).is_err() {
            drift.push(format!(
                "`{heading}` stamp `{sha}` is not an ancestor of HEAD"
            ));
            continue;
        }
        let range = format!("{sha}..HEAD");
        let mut args: Vec<&str> = vec!["log", "--format=%h %s", &range, "--"];
        args.extend(section.files.iter().copied());
        let touching = git(&args)?;
        if !touching.is_empty() {
            drift.push(format!(
                "`{heading}` stamp `{sha}` is stale — commits since it touch the \
                 section's referenced sources {:?}:\n    {}",
                section.files,
                touching.replace('\n', "\n    ")
            ));
        }
    }
    if drift.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{doc_label} sha stamps have gone stale (re-verify each section \
             against the named sources, then re-stamp):\n  {}",
            drift.join("\n  ")
        ))
    }
}

#[test]
fn architecture_stamps_are_fresh() {
    check_doc("docs/ARCHITECTURE.md", &read_repo("docs/ARCHITECTURE.md")).unwrap();
}

#[test]
fn extending_stamps_are_fresh() {
    check_doc("docs/EXTENDING.md", &read_repo("docs/EXTENDING.md")).unwrap();
}
