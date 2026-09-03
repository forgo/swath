// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Strings the web quotes from Rust (issue #394).
//!
//! The UI is allowed to react to specific server wording — the explain card
//! turns the planner's "this dataset has no overviews" into an actionable
//! sentence naming `swath materialize`. That is a mechanical claim about the
//! code, so it is checked mechanically, like every other claim in this gate.
//!
//! The failure this prevents is silent: a reworded reason on the Rust side
//! leaves the TypeScript compiling, the tests passing, and the fix line
//! simply never appearing again.

use super::read_repo;

/// `(the web file, its exported constant, the Rust file that must emit it)`.
const QUOTED: [(&str, &str, &str); 1] = [(
    "web/src/explain-model.ts",
    "NO_OVERVIEWS_REASON",
    "crates/swath-planner/src/lib.rs",
)];

/// The literal assigned to `export const <name> = "..."` in `source`.
fn quoted_literal(source: &str, name: &str) -> Option<String> {
    let anchor = format!("export const {name} = \"");
    let start = source.find(&anchor)? + anchor.len();
    let rest = &source[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// Every string the web quotes is still emitted by the Rust that owns it.
#[test]
fn quoted_server_strings_still_exist() {
    for (web_file, name, rust_file) in QUOTED {
        let web = read_repo(web_file);
        let literal = quoted_literal(&web, name).unwrap_or_else(|| {
            panic!("{web_file} no longer exports a string constant named {name}")
        });
        assert!(
            !literal.is_empty(),
            "{web_file}'s {name} must not be empty — an empty quote matches nothing"
        );
        let rust = read_repo(rust_file);
        assert!(
            rust.contains(&literal),
            "{web_file} quotes `{literal}` as {name}, but {rust_file} no longer emits it — \
             the UI that reacts to it has gone silent"
        );
    }
}

/// The parser is exercised against a fixture, so a passing tree is not the
/// only evidence it works.
#[test]
fn the_quote_parser_reads_the_literal() {
    let fixture = "export const NAME = \"source has no overviews\";\n";
    assert_eq!(
        quoted_literal(fixture, "NAME").as_deref(),
        Some("source has no overviews")
    );
    assert_eq!(quoted_literal(fixture, "MISSING"), None);
}
