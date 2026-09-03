// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The docs-drift gate (issues #119 and #173): documentation that makes
//! mechanical claims about the code is verified MECHANICALLY against the
//! code. This module (the original #119 gate) checks `docs/CONFIG.md`
//! against the two sources of configuration truth — the clap command tree
//! (flags, env vars, positionals) and the serde TOML schema
//! (`config::ConfigFile` and everything under it). The submodules extend
//! the same pattern (issue #173):
//!
//! - [`routes`] — `docs/ENDPOINTS.md`'s route table vs the axum routers.
//! - [`stamps`] — `_Last verified against sources_` fingerprint stamps
//!   vs the current content of each stamped section's referenced source
//!   files (content-addressed, so squash-merges cannot stale them —
//!   issue #224).
//! - [`deferrals`] — prose deferral language must point at
//!   `docs/ROADMAP.md`'s deferral inventory (or the governing ADR).
//! - [`claims`] — cross-document claims (quoted sentences, §-citations,
//!   committed perf numbers, evidence-file headers) vs their canonical
//!   sources.
//! - [`numbers`] — inline `number:<key>` headline-figure markers (issue
//!   #174) vs the committed perf artifacts: marker content current,
//!   required markers present, and no naked headline literal anywhere
//!   outside a marker (`just perf-doc` regenerates; this verifies).
//! - [`mutation`] — the acceptance bar: each of the six documentation
//!   drifts fixed by the #172 sweep (PR #214) is re-introduced in memory
//!   and the gate must fail on every one.
//! - [`glossary`] — `web/src/glossary.ts`'s cited sources must exist and
//!   still use the term they define (issue #396).
//! - [`web_quotes`] — server wording the web reacts to must still be
//!   emitted by the Rust that owns it (issue #394).
//! - [`budgets`] — per-doc word budgets (issue #177): the word-reduction
//!   sweep's committed ceilings over `README.md` + `docs/*.md`, measured
//!   exactly as `just docs-words` measures (`wc -w`).
//!
//! Escape hatch policy: a legitimate exception goes on an explicit,
//! reason-carrying allowlist next to the check it exempts (and a stale
//! allowlist entry is itself a failure) — checks are never loosened.
//!
//! The doc carries `<!-- config-check:begin <scope> -->` /
//! `<!-- config-check:end <scope> -->` marker pairs around each reference
//! table; these tests extract the backticked key in each table row and
//! assert **set equality** with what the code actually accepts — zero
//! undocumented keys, zero phantom keys, in every scope. A new flag, TOML
//! key, or enum variant fails `just test` (and CI) until the reference
//! documents it; a documented key the code dropped fails the same way.
//!
//! Field names come from the schema itself, not a hand-kept list: serde's
//! `deny_unknown_fields` / enum errors name every accepted field
//! ("unknown field `zzz`, expected one of ..."), so probing each level
//! with a bogus key yields the authoritative vocabulary. The clap side
//! walks `Cli::command()` recursively, so a new subcommand with any
//! argument needs its own documented block too.

mod budgets;
mod claims;
mod deferrals;
mod glossary;
mod mutation;
mod numbers;
mod routes;
mod stamps;
mod web_quotes;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::CommandFactory as _;

use swath_cli::Cli;
use swath_cli::config::ConfigFile;

/// The repository root (the docs live at `<root>/docs`), resolved from
/// this crate's manifest so the gate works from any working directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// Reads a repo-relative file (the doc or source under test), with line
/// endings normalized to `\n` — a Windows checkout with `autocrlf` must
/// see the same text every `\n`-anchored check and fixture sees on
/// Linux/macOS.
fn read_repo(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {path}: {err}", path = path.display()))
        .replace("\r\n", "\n")
}

/// The text between `<!-- docs-check:begin <scope> -->` /
/// `<!-- docs-check:end <scope> -->` markers, or an error naming the
/// missing marker (the generic twin of the CONFIG-specific [`block`]).
fn marker_block(doc_label: &str, doc: &str, scope: &str) -> Result<String, String> {
    let begin = format!("<!-- docs-check:begin {scope} -->");
    let end = format!("<!-- docs-check:end {scope} -->");
    let start = doc
        .find(&begin)
        .ok_or_else(|| format!("{doc_label} has no `{begin}` marker"))?;
    let rest = &doc[start + begin.len()..];
    let stop = rest
        .find(&end)
        .ok_or_else(|| format!("{doc_label} has no `{end}` marker"))?;
    Ok(rest[..stop].to_owned())
}

/// Whitespace-normalized text: every run of whitespace (line breaks
/// included) collapsed to one space — so quoted sentences match across
/// the hard wrapping both documents apply.
fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` with the inline `<!-- number:<key> -->` / `<!-- /number:<key> -->`
/// tags removed and their content kept — checks that quote doc sentences
/// verbatim must see the prose as it renders, not the marker plumbing
/// (the [`numbers`] module checks the markers themselves).
fn strip_number_tags(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    loop {
        let begin = rest.find("<!-- number:");
        let end = rest.find("<!-- /number:");
        let Some(start) = begin.map_or(end, |b| Some(end.map_or(b, |e| b.min(e)))) else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(stop) = rest.find("-->") else {
            out.push_str(rest);
            return out;
        };
        rest = &rest[stop + "-->".len()..];
    }
}

/// `text` with fenced code blocks (``` … ```) removed — prose checks must
/// not trip over example commands or captured output.
fn strip_code_fences(text: &str) -> String {
    text.split("```")
        .enumerate()
        .filter_map(|(i, part)| (i % 2 == 0).then_some(part))
        .collect::<Vec<_>>()
        .join("")
}

/// The config reference, relative to this crate's manifest.
const DOC_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CONFIG.md");

/// Reads `docs/CONFIG.md` (the file under test).
fn doc() -> String {
    std::fs::read_to_string(DOC_PATH).unwrap_or_else(|err| panic!("cannot read {DOC_PATH}: {err}"))
}

/// The text between the `config-check` markers for `scope`.
fn block(doc: &str, scope: &str) -> String {
    let begin = format!("<!-- config-check:begin {scope} -->");
    let end = format!("<!-- config-check:end {scope} -->");
    let start = doc
        .find(&begin)
        .unwrap_or_else(|| panic!("docs/CONFIG.md has no `{begin}` marker"));
    let rest = &doc[start + begin.len()..];
    let stop = rest
        .find(&end)
        .unwrap_or_else(|| panic!("docs/CONFIG.md has no `{end}` marker"));
    rest[..stop].to_owned()
}

/// The documented keys of a block: the first backticked token of every
/// table row, with flag dashes and positional angle brackets stripped
/// (`--bind` -> `bind`, `<granule>` -> `granule`).
fn documented_keys(block: &str) -> BTreeSet<String> {
    block
        .lines()
        .filter(|line| line.trim_start().starts_with("| `"))
        .map(|line| {
            line.split('`')
                .nth(1)
                .expect("table row has a backticked key")
                .trim_start_matches("--")
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_owned()
        })
        .collect()
}

/// Every backticked `SWATH_*` token in a block (the documented env vars).
fn documented_envs(block: &str) -> BTreeSet<String> {
    block
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|token| token.starts_with("SWATH_"))
        .map(str::to_owned)
        .collect()
}

/// Asserts documented == actual, naming the drift in both directions.
fn assert_same(scope: &str, documented: &BTreeSet<String>, actual: &BTreeSet<String>) {
    let undocumented: Vec<&String> = actual.difference(documented).collect();
    let phantom: Vec<&String> = documented.difference(actual).collect();
    assert!(
        undocumented.is_empty() && phantom.is_empty(),
        "docs/CONFIG.md block `{scope}` has drifted from the code:\n  \
         undocumented (in code, not in docs): {undocumented:?}\n  \
         phantom (in docs, not in code): {phantom:?}"
    );
}

/// The field/variant vocabulary serde accepts at the schema position the
/// probe TOML addresses: parse a document carrying a bogus key or variant
/// there and read the names out of the `deny_unknown_fields` /
/// unknown-variant error ("expected one of `a`, `b`, ..." — the list is
/// generated from the struct/enum itself, so it cannot go stale).
fn schema_vocabulary(probe_toml: &str) -> BTreeSet<String> {
    let err = toml::from_str::<ConfigFile>(probe_toml)
        .expect_err("the probe document must be rejected")
        .to_string();
    let tail = &err[err
        .rfind("expected")
        .unwrap_or_else(|| panic!("not an unknown-field/variant error: {err}"))..];
    let names: BTreeSet<String> = tail
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    assert!(!names.is_empty(), "no field names parsed from: {err}");
    names
}

// --- The TOML schema blocks ---

#[test]
fn file_keys_match_the_serde_schema() {
    let doc = doc();
    assert_same(
        "file",
        &documented_keys(&block(&doc, "file")),
        &schema_vocabulary("zzz-bogus = 1"),
    );
}

#[test]
fn budget_keys_match_the_serde_schema() {
    let doc = doc();
    assert_same(
        "budget",
        &documented_keys(&block(&doc, "budget")),
        &schema_vocabulary("[budget]\nzzz-bogus = 1"),
    );
}

#[test]
fn layer_keys_match_the_serde_schema() {
    let doc = doc();
    let actual = schema_vocabulary("[[layers]]\nzzz-bogus = 1");
    assert_same("layer", &documented_keys(&block(&doc, "layer")), &actual);
    // `[[datasets.layers]]` deserializes through the same struct; the doc
    // says so in prose, and this pins that the schemas really are one.
    assert_eq!(
        actual,
        schema_vocabulary("[[datasets]]\nid = \"d\"\n[[datasets.layers]]\nzzz-bogus = 1"),
        "[[layers]] and [[datasets.layers]] no longer share a schema — \
         docs/CONFIG.md documents them as one table"
    );
}

#[test]
fn dataset_keys_match_the_serde_schema() {
    let doc = doc();
    assert_same(
        "dataset",
        &documented_keys(&block(&doc, "dataset")),
        &schema_vocabulary("[[datasets]]\nzzz-bogus = 1"),
    );
}

#[test]
fn enum_values_match_the_serde_schema() {
    let doc = doc();
    for (scope, probe) in [
        ("enum kind", "[[layers]]\nkind = \"zzz-bogus\""),
        ("enum colormap", "[[layers]]\ncolormap = \"zzz-bogus\""),
        ("enum resampling", "[[layers]]\nresampling = \"zzz-bogus\""),
    ] {
        assert_same(
            scope,
            &documented_keys(&block(&doc, scope)),
            &schema_vocabulary(probe),
        );
    }
}

// --- The clap tree blocks ---

/// The non-builtin arguments of one command: long flags and positionals
/// (as one key set) plus env var names.
fn command_args(cmd: &clap::Command) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut keys = BTreeSet::new();
    let mut envs = BTreeSet::new();
    for arg in cmd.get_arguments() {
        let id = arg.get_id().as_str();
        if id == "help" || id == "version" {
            continue;
        }
        if let Some(long) = arg.get_long() {
            keys.insert(long.to_owned());
        } else if arg.is_positional() {
            keys.insert(id.to_owned());
        }
        if let Some(env) = arg.get_env() {
            envs.insert(env.to_string_lossy().into_owned());
        }
    }
    (keys, envs)
}

/// Recursively asserts every command with arguments has a matching,
/// exact `flags <path>` block (env vars included).
fn assert_command_documented(doc: &str, cmd: &clap::Command, path: &str) {
    let (keys, envs) = command_args(cmd);
    if !keys.is_empty() {
        let scope = format!("flags {path}");
        let body = block(doc, &scope);
        assert_same(&scope, &documented_keys(&body), &keys);
        assert_same(&format!("{scope} (env)"), &documented_envs(&body), &envs);
    }
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" {
            continue;
        }
        assert_command_documented(doc, sub, &format!("{path} {name}", name = sub.get_name()));
    }
}

#[test]
fn cli_flags_and_env_vars_match_the_clap_tree() {
    let doc = doc();
    let mut cli = Cli::command();
    cli.build();
    assert_command_documented(&doc, &cli, "swath");
}
