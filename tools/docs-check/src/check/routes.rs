// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The endpoints gate (issue #173): `docs/ENDPOINTS.md`'s route table is
//! verified against the actual axum routers. The doc carries
//! `<!-- docs-check:begin routes -->` markers around the table; the code
//! side is scraped from the `.route("<path>", <methods>)` registrations
//! in the three router files — the same files the doc names as the route
//! tables' home. Set equality is asserted per path, with the method set
//! AND the "Mounted" column (which router file the route lives in)
//! compared: zero undocumented routes, zero phantom routes, no
//! misdocumented methods or mounting. The router fallback (`.fallback(`)
//! is matched against the doc's dedicated `*fallback*` row.

use std::collections::{BTreeMap, BTreeSet};

use super::{marker_block, read_repo};

/// The endpoint reference under test.
pub(super) const DOC: &str = "docs/ENDPOINTS.md";

/// The router files and the "Mounted" value their routes carry in the
/// doc's table — exactly the files `docs/ENDPOINTS.md` names as where
/// "the route tables live in code".
const ROUTERS: [(&str, &str); 5] = [
    ("crates/swath-api/src/routes.rs", "always"),
    ("crates/swath-api/src/granules.rs", "catalog mode"),
    ("crates/swath-api/src/openeo/mod.rs", "catalog mode"),
    ("crates/swath-api/src/datasets.rs", "catalog mode"),
    ("crates/swath-api/src/sources.rs", "catalog mode"),
];

/// axum's routing method helpers — the identifiers that name HTTP
/// methods inside a `.route(...)` registration.
const METHOD_FNS: [&str; 7] = ["get", "post", "put", "delete", "patch", "head", "any"];

/// One route as compared: HTTP methods and the "Mounted" column value.
#[derive(Debug, PartialEq, Eq)]
struct Route {
    /// Uppercase HTTP method names, sorted.
    methods: BTreeSet<String>,
    /// `always` or `catalog mode` (which router mounts the path).
    mounted: String,
}

/// The argument list of a call, from its opening `(` (inclusive) to the
/// matching `)` — paren counting that skips string literals.
fn balanced_call(src: &str) -> &str {
    debug_assert!(src.starts_with('('));
    let mut depth = 0_u32;
    let mut in_str = false;
    for (i, ch) in src.char_indices() {
        if in_str {
            in_str = ch != '"';
            continue;
        }
        match ch {
            '"' => in_str = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &src[..=i];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced parentheses in route registration: {src}");
}

/// The first `"…"` string literal in a call argument list (the path).
fn first_string_literal(call: &str) -> String {
    let start = call.find('"').expect("route call has a path literal") + 1;
    let end = start + call[start..].find('"').expect("path literal is terminated");
    call[start..end].to_owned()
}

/// The HTTP methods a `.route(...)` argument list registers: every
/// `get(`/`post(`/… method-helper invocation in it, uppercased.
fn methods_in_call(call: &str) -> BTreeSet<String> {
    let mut methods = BTreeSet::new();
    for name in METHOD_FNS {
        let needle = format!("{name}(");
        let mut idx = 0;
        while let Some(pos) = call[idx..].find(&needle) {
            let at = idx + pos;
            let preceded_by_ident = call[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            if !preceded_by_ident {
                methods.insert(name.to_uppercase());
            }
            idx = at + needle.len();
        }
    }
    assert!(!methods.is_empty(), "no method helper found in: {call}");
    methods
}

/// Every route a router source file registers, plus whether it installs
/// a fallback handler.
fn scrape_router(src: &str) -> (Vec<(String, BTreeSet<String>)>, bool) {
    let mut routes = Vec::new();
    let mut idx = 0;
    while let Some(pos) = src[idx..].find(".route(") {
        let open = idx + pos + ".route".len();
        let call = balanced_call(&src[open..]);
        routes.push((first_string_literal(call), methods_in_call(call)));
        idx = open + call.len();
    }
    (routes, src.contains(".fallback("))
}

/// The code truth: path → route for every mounted route, plus whether a
/// fallback exists. A path registered by two router files with the same
/// mount context merges its methods (the granule surface: browsing GETs
/// in `granules.rs`, registration POSTs in the separately-unmountable
/// `datasets.rs`, #196); the same path+method twice, or one path across
/// different mount contexts, is still a hard clash.
fn actual_routes() -> (BTreeMap<String, Route>, bool) {
    let mut map: BTreeMap<String, Route> = BTreeMap::new();
    let mut fallback = false;
    for (file, mounted) in ROUTERS {
        let (routes, has_fallback) = scrape_router(&read_repo(file));
        fallback |= has_fallback;
        for (path, methods) in routes {
            match map.get_mut(&path) {
                None => {
                    map.insert(
                        path,
                        Route {
                            methods,
                            mounted: mounted.to_owned(),
                        },
                    );
                }
                Some(existing) => {
                    assert_eq!(
                        existing.mounted, mounted,
                        "route `{path}` registered under two mount contexts ({file})"
                    );
                    for method in methods {
                        assert!(
                            existing.methods.insert(method.clone()),
                            "route `{path}` registers {method} twice ({file})"
                        );
                    }
                }
            }
        }
    }
    (map, fallback)
}

/// The documented truth: path → route from the marker-delimited table,
/// plus whether the `*fallback*` row is present.
fn documented_routes(block: &str) -> Result<(BTreeMap<String, Route>, bool), String> {
    let mut map = BTreeMap::new();
    let mut fallback = false;
    for line in block.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('|') else {
            continue;
        };
        let cells: Vec<&str> = rest
            .trim_end_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() < 4 || cells[0] == "Method" || cells[0].starts_with('-') {
            continue;
        }
        if cells[1] == "*fallback*" {
            fallback = true;
            continue;
        }
        let path = cells[1]
            .strip_prefix('`')
            .and_then(|p| p.strip_suffix('`'))
            .ok_or_else(|| format!("route-table path cell is not backticked: {line}"))?
            .to_owned();
        let methods: BTreeSet<String> = cells[0].split(',').map(|m| m.trim().to_owned()).collect();
        let route = Route {
            methods,
            mounted: cells[2].to_owned(),
        };
        if map.insert(path.clone(), route).is_some() {
            return Err(format!("route `{path}` documented twice in the table"));
        }
    }
    Ok((map, fallback))
}

/// The check: the doc's marker-delimited route table equals the scraped
/// router registrations — paths, methods, and mounting, both directions.
pub(super) fn check(endpoints_doc: &str) -> Result<(), String> {
    let table = marker_block(DOC, endpoints_doc, "routes")?;
    let (documented, doc_fallback) = documented_routes(&table)?;
    let (actual, code_fallback) = actual_routes();

    let mut drift = Vec::new();
    for (path, route) in &actual {
        match documented.get(path) {
            None => drift.push(format!("undocumented route (in code, not in docs): {path}")),
            Some(doc_route) if doc_route != route => drift.push(format!(
                "route `{path}` documented as {doc_route:?} but the code mounts {route:?}"
            )),
            Some(_) => {}
        }
    }
    for path in documented.keys() {
        if !actual.contains_key(path) {
            drift.push(format!("phantom route (in docs, not in code): {path}"));
        }
    }
    if doc_fallback != code_fallback {
        drift.push(format!(
            "fallback drift: doc row present = {doc_fallback}, `.fallback(` in routes.rs = {code_fallback}"
        ));
    }
    if drift.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{DOC} route table has drifted from the axum routers:\n  {}",
            drift.join("\n  ")
        ))
    }
}

#[test]
fn endpoints_route_table_matches_the_axum_routers() {
    check(&read_repo(DOC)).unwrap();
}

#[test]
fn a_phantom_route_is_caught() {
    let doc = read_repo(DOC).replace(
        "<!-- docs-check:end routes -->",
        "| GET | `/zzz-phantom` | always | Never mounted |\n<!-- docs-check:end routes -->",
    );
    let err = check(&doc).expect_err("a documented-but-unmounted route must fail");
    assert!(
        err.contains("phantom") && err.contains("/zzz-phantom"),
        "{err}"
    );
}
