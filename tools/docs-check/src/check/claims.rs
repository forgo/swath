// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Cross-document claim checks (issue #173): documentation that quotes,
//! cites, or restates another committed artifact is verified against
//! that artifact. Four checks, one per claim shape the #172 drift sweep
//! fixed:
//!
//! - `docs/DEMO.md`'s measured-numbers table must quote the committed
//!   ingest-to-pixel baseline (`docs/perf/i2p-baseline.json`) — value
//!   and stamp sha.
//! - `docs/COMPARISON.md`'s quoted README sentences must appear verbatim
//!   (whitespace-normalized) in `README.md`.
//! - `README.md`'s oracle-history citation must point at the
//!   `docs/CHARTER.md` section that actually discusses the oracles.
//! - `docs/perf/load-2cpu-16.7-evidence.md`'s header must describe the
//!   2-CPU pinned run its filename and data claim (ADR 0012's rerun).
//!
//! A fifth check (#338): the mission sentence has one home,
//! `docs/REQUIREMENTS.md` §1, and `README.md` opens with it verbatim.

use super::{normalize_ws, read_repo, strip_code_fences, strip_number_tags};

/// Quoted spans shorter than this are not treated as verbatim quotes
/// (short fragments like process names are legitimately paraphrased).
const MIN_QUOTE_LEN: usize = 20;

/// `docs/DEMO.md` vs `docs/perf/i2p-baseline.json`: the committed
/// baseline row must exist and carry the artifact's value and git sha.
pub(super) fn demo_quotes_the_i2p_baseline(demo: &str) -> Result<(), String> {
    // The row's figures sit inside generated `number:` markers (issue
    // #174); this check reads the prose as rendered.
    let demo = strip_number_tags(demo);
    let baseline: serde_json::Value =
        serde_json::from_str(&read_repo("docs/perf/i2p-baseline.json"))
            .map_err(|err| format!("docs/perf/i2p-baseline.json is not JSON: {err}"))?;
    let value = baseline["value"]
        .as_u64()
        .ok_or_else(|| "i2p-baseline.json has no numeric `value`".to_owned())?;
    let sha = baseline["git_sha"]
        .as_str()
        .ok_or_else(|| "i2p-baseline.json has no `git_sha`".to_owned())?;

    let row = demo
        .lines()
        .find(|line| line.contains("**Committed baseline**"))
        .ok_or_else(|| {
            "docs/DEMO.md has no `**Committed baseline**` row — its measured \
             numbers must quote docs/perf/i2p-baseline.json"
                .to_owned()
        })?;
    if !row.contains(&format!("**{value} ms**")) {
        return Err(format!(
            "docs/DEMO.md committed-baseline row does not carry the artifact's \
             value `{value} ms`: {row}"
        ));
    }
    let stamped = row
        .split("stamped at `")
        .nth(1)
        .and_then(|rest| rest.split('`').next())
        .ok_or_else(|| {
            format!("docs/DEMO.md committed-baseline row has no `stamped at `<sha>``: {row}")
        })?;
    if !sha.starts_with(stamped) || stamped.len() < 7 {
        return Err(format!(
            "docs/DEMO.md committed-baseline stamp `{stamped}` does not match \
             i2p-baseline.json's git_sha `{sha}`"
        ));
    }
    Ok(())
}

/// How far (in chars) before a quote's opening `"` the attribution
/// (`README`) must appear for the span to count as a README quote.
const ATTRIBUTION_WINDOW: usize = 80;

/// The `"…"` spans of a whitespace-normalized paragraph, each with the
/// text window immediately preceding its opening quote.
fn quoted_spans(paragraph: &str) -> Vec<(String, String)> {
    let marks: Vec<usize> = paragraph
        .char_indices()
        .filter_map(|(i, ch)| (ch == '"').then_some(i))
        .collect();
    marks
        .chunks_exact(2)
        .map(|pair| {
            let prefix = &paragraph[..pair[0]];
            let window_start = prefix
                .char_indices()
                .rev()
                .nth(ATTRIBUTION_WINDOW - 1)
                .map_or(0, |(i, _)| i);
            (
                paragraph[pair[0] + 1..pair[1]].to_owned(),
                prefix[window_start..].to_owned(),
            )
        })
        .collect()
}

/// `docs/COMPARISON.md` vs `README.md`: every long quoted span whose
/// immediately preceding text attributes it to the README must appear
/// there (whitespace-normalized).
pub(super) fn comparison_quotes_the_readme(comparison: &str, readme: &str) -> Result<(), String> {
    let readme_normalized = normalize_ws(readme);
    let mut drift = Vec::new();
    let mut readme_quotes = 0_usize;
    for paragraph in strip_code_fences(comparison).split("\n\n") {
        for (span, attribution) in quoted_spans(&normalize_ws(paragraph)) {
            if span.len() < MIN_QUOTE_LEN || !attribution.contains("README") {
                continue;
            }
            readme_quotes += 1;
            if !readme_normalized.contains(&span) {
                drift.push(span);
            }
        }
    }
    if readme_quotes == 0 {
        return Err(
            "docs/COMPARISON.md no longer quotes the README positioning sentence \
             at all — its conjunction claim must quote the current README"
                .to_owned(),
        );
    }
    if drift.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "docs/COMPARISON.md attributes text to the README that README.md \
             does not contain:\n  {}",
            drift.join("\n  ")
        ))
    }
}

/// `README.md` vs `docs/CHARTER.md`: the oracle-relationship paragraph
/// cites `CHARTER.md §N`; section N must actually discuss the oracles.
pub(super) fn readme_cites_the_charter_oracle_section(
    readme: &str,
    charter: &str,
) -> Result<(), String> {
    let mut checked = 0_usize;
    let mut drift = Vec::new();
    for paragraph in strip_code_fences(readme).split("\n\n") {
        if !paragraph.to_lowercase().contains("oracle") {
            continue;
        }
        let normalized = normalize_ws(paragraph);
        for citation in normalized.split("(docs/CHARTER.md) §").skip(1) {
            let digits: String = citation.chars().take_while(char::is_ascii_digit).collect();
            let number: u32 = digits
                .parse()
                .map_err(|_| format!("unparseable CHARTER § citation in README: {citation:.20}"))?;
            checked += 1;
            let heading = format!("\n## {number}. ");
            let start = charter.find(&heading).ok_or_else(|| {
                format!("README cites CHARTER.md §{number}, which does not exist")
            })?;
            let body = &charter[start + 1..];
            let body = &body[..body.find("\n## ").unwrap_or(body.len())];
            if !body.to_lowercase().contains("oracle") {
                drift.push(format!(
                    "README's oracle paragraph cites CHARTER.md §{number}, but that \
                     section never mentions the oracles — the citation has drifted"
                ));
            }
        }
    }
    if checked == 0 {
        return Err(
            "README.md's oracle paragraph no longer cites docs/CHARTER.md §N — \
             the oracle-relationship history citation is gone"
                .to_owned(),
        );
    }
    if drift.is_empty() {
        Ok(())
    } else {
        Err(drift.join("\n"))
    }
}

/// `docs/perf/load-2cpu-16.7-evidence.md`: the header must identify the
/// 2-CPU pinned run (ADR 0012's rerun) its filename and numbers are.
pub(super) fn load_2cpu_header_names_the_pinned_run(evidence: &str) -> Result<(), String> {
    let generated = evidence
        .lines()
        .find(|line| line.starts_with("Generated "))
        .ok_or_else(|| {
            "docs/perf/load-2cpu-16.7-evidence.md has no `Generated …` header line".to_owned()
        })?;
    if generated.contains("pinned to 2 CPUs") && generated.contains("ADR 0012") {
        Ok(())
    } else {
        Err(format!(
            "docs/perf/load-2cpu-16.7-evidence.md's header must identify the run \
             as `pinned to 2 CPUs` and cite ADR 0012 (the file's numbers are that \
             ADR's 2-CPU column, not the 12-core baseline): {generated}"
        ))
    }
}

/// The first `**…**` span of `text`, if any.
fn first_bold_span(text: &str) -> Option<&str> {
    let start = text.find("**")? + 2;
    let len = text[start..].find("**")?;
    Some(&text[start..start + len])
}

/// `README.md` vs `docs/REQUIREMENTS.md`: the README's opening bold
/// paragraph is REQUIREMENTS §1's mission sentence, verbatim
/// (whitespace-normalized) — the mission is stated once.
pub(super) fn readme_opens_with_the_mission(
    readme: &str,
    requirements: &str,
) -> Result<(), String> {
    let heading = "\n## 1. Mission";
    let start = requirements
        .find(heading)
        .ok_or_else(|| "docs/REQUIREMENTS.md has no `## 1. Mission` section".to_owned())?;
    let section = &requirements[start + 1..];
    let section = &section[..section.find("\n## ").unwrap_or(section.len())];
    let mission = first_bold_span(section)
        .map(normalize_ws)
        .ok_or_else(|| "docs/REQUIREMENTS.md §1 carries no bold mission sentence".to_owned())?;
    let lede = first_bold_span(&strip_code_fences(readme))
        .map(normalize_ws)
        .ok_or_else(|| "README.md has no bold opening sentence".to_owned())?;
    if lede == mission {
        Ok(())
    } else {
        Err(format!(
            "README.md's opening sentence is not docs/REQUIREMENTS.md §1's mission sentence \
             (the mission has one home; quote it):\n  README:       {lede}\n  REQUIREMENTS: {mission}"
        ))
    }
}

#[test]
fn readme_opening_sentence_is_the_mission() {
    readme_opens_with_the_mission(&read_repo("README.md"), &read_repo("docs/REQUIREMENTS.md"))
        .unwrap();
}

#[test]
fn demo_measured_numbers_match_the_committed_baseline() {
    demo_quotes_the_i2p_baseline(&read_repo("docs/DEMO.md")).unwrap();
}

#[test]
fn comparison_readme_quotes_are_current() {
    comparison_quotes_the_readme(&read_repo("docs/COMPARISON.md"), &read_repo("README.md"))
        .unwrap();
}

#[test]
fn readme_charter_citation_names_the_oracle_section() {
    readme_cites_the_charter_oracle_section(&read_repo("README.md"), &read_repo("docs/CHARTER.md"))
        .unwrap();
}

#[test]
fn load_2cpu_evidence_header_is_honest() {
    load_2cpu_header_names_the_pinned_run(&read_repo("docs/perf/load-2cpu-16.7-evidence.md"))
        .unwrap();
}
