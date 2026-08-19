// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Compiler-faithfulness property: any bounded random arithmetic reducer,
//! written out as a standard openEO `reduce_dimension` sub-graph, compiles
//! to a plan that evaluates byte-identically to the hand-built plan running
//! the same expression AST directly — on synthetic buffers including
//! zeros (division-by-zero pixels must invalidate identically).

use proptest::prelude::*;
use serde_json::{Value as Json, json};
use swath_render::ir::{
    BandInput, BinaryOp, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat,
};
use swath_render::{CompileContext, NoUdf, WarpedBuffer, eval};

/// The two-band context every generated graph loads against.
fn ctx() -> CompileContext {
    CompileContext::new("synthetic")
        .with_band("band-a", ["a"])
        .with_band("band-b", ["b"])
}

/// Expressions over the *dataset* bands `band-a`/`band-b` and small
/// constants, bounded depth — the language the reducer sub-graph must
/// round-trip.
fn expr() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        Just(Expr::band("band-a")),
        Just(Expr::band("band-b")),
        (-4i32..=4).prop_map(|c| Expr::Const(f64::from(c))),
    ];
    leaf.prop_recursive(4, 32, 2, |inner| {
        (
            prop_oneof![
                Just(BinaryOp::Add),
                Just(BinaryOp::Sub),
                Just(BinaryOp::Mul),
                Just(BinaryOp::Div),
            ],
            inner.clone(),
            inner,
        )
            .prop_map(|(op, lhs, rhs)| Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            })
    })
}

/// Serializes an expression as reducer sub-graph nodes (`array_element`
/// leaves by label, arithmetic nodes chained by `from_node`), returning
/// the JSON argument encoding of `expr`'s root.
fn to_nodes(expr: &Expr, nodes: &mut serde_json::Map<String, Json>) -> Json {
    match expr {
        Expr::Band(name) => {
            // Reference by openEO alias, exercising context resolution.
            let label = if name == "band-a" { "a" } else { "b" };
            let id = format!("n{}", nodes.len());
            nodes.insert(
                id.clone(),
                json!({
                    "process_id": "array_element",
                    "arguments": {"data": {"from_parameter": "data"}, "label": label}
                }),
            );
            json!({"from_node": id})
        }
        Expr::Const(c) => json!(c),
        Expr::Binary { op, lhs, rhs } => {
            let x = to_nodes(lhs, nodes);
            let y = to_nodes(rhs, nodes);
            let process_id = match op {
                BinaryOp::Add => "add",
                BinaryOp::Sub => "subtract",
                BinaryOp::Mul => "multiply",
                BinaryOp::Div => "divide",
                _ => unreachable!("no other BinaryOp variants exist"),
            };
            let id = format!("n{}", nodes.len());
            nodes.insert(
                id.clone(),
                json!({"process_id": process_id, "arguments": {"x": x, "y": y}}),
            );
            json!({"from_node": id})
        }
        _ => unreachable!("no other Expr variants exist"),
    }
}

/// The full openEO graph for a reducer expression: load both bands,
/// reduce over bands with the generated sub-graph, save png.
fn openeo_graph(expr: &Expr) -> Json {
    let mut nodes = serde_json::Map::new();
    let root = to_nodes(expr, &mut nodes);
    match root {
        // Mark the root node as the sub-graph result.
        Json::Object(ref obj) => {
            let id = obj["from_node"].as_str().expect("from_node ref").to_owned();
            nodes.get_mut(&id).expect("root node exists")["result"] = json!(true);
        }
        // A constant-only expression has no root node; wrap it in a
        // no-op multiply so the sub-graph has a result node.
        ref c => {
            nodes.insert(
                "root".into(),
                json!({
                    "process_id": "multiply",
                    "arguments": {"x": c, "y": 1.0},
                    "result": true
                }),
            );
        }
    }
    json!({
        "process_graph": {
            "load": {
                "process_id": "load_collection",
                "arguments": {
                    "id": "synthetic",
                    "spatial_extent": null,
                    "temporal_extent": null,
                    "bands": ["a", "b"]
                }
            },
            "reduce": {
                "process_id": "reduce_dimension",
                "arguments": {
                    "data": {"from_node": "load"},
                    "dimension": "bands",
                    "reducer": {"process_graph": Json::Object(nodes)}
                }
            },
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "reduce"}, "format": "png"},
                "result": true
            }
        }
    })
}

fn buffer(values: Vec<f64>) -> WarpedBuffer {
    #[allow(clippy::cast_possible_truncation, reason = "test sizes are tiny")]
    let width = values.len() as u32;
    let valid = vec![true; values.len()];
    WarpedBuffer {
        width,
        height: 1,
        values,
        valid,
    }
}

proptest! {
    /// Compile the graph form of a random expression; the compiled plan and
    /// the hand-built plan over the same AST must produce byte-identical
    /// tiles (colors and alpha — division-by-zero invalidation included).
    #[test]
    fn compiled_graphs_evaluate_like_the_source_ast(
        e in expr(),
        a in proptest::collection::vec(prop_oneof![Just(0.0f64), -100.0..100.0], 1..16),
    ) {
        let b: Vec<f64> = a.iter().rev().copied().collect();
        let graph = openeo_graph(&e);
        let product = swath_render::compile(&graph, &ctx()).expect("generated graph compiles");

        // The hand-built equivalent: same AST (or the wrapped constant),
        // same trailing colormap, inputs = the bands the compiler derived.
        let mut refs = Vec::new();
        collect_refs(&e, &mut refs);
        let expected_expr = if refs.is_empty() {
            Expr::Binary {
                op: BinaryOp::Mul,
                lhs: Box::new(e.clone()),
                rhs: Box::new(Expr::Const(1.0)),
            }
        } else {
            e.clone()
        };
        prop_assert_eq!(&product.bands, &refs);
        let inputs = product.bands.iter().map(BandInput::new).collect();
        let hand = RenderPlan::new(
            inputs,
            vec![
                PixelOp::BandMath(expected_expr),
                PixelOp::Colormap(Colormap::Grayscale),
            ],
            OutputSpec::new(TileFormat::Png),
        );

        let buffers: Vec<WarpedBuffer> = product
            .bands
            .iter()
            .map(|band| buffer(if band == "band-a" { a.clone() } else { b.clone() }))
            .collect();
        let ours = eval(&product.plan, &buffers, &NoUdf).expect("compiled plan evaluates");
        let reference = eval(&hand, &buffers, &NoUdf).expect("hand plan evaluates");
        prop_assert_eq!(ours.pixels, reference.pixels);
    }
}

/// First-reference-order deduplicated band names, mirroring the compiler's
/// documented input derivation.
fn collect_refs(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Band(name) => {
            if !out.iter().any(|n| n == name) {
                out.push(name.clone());
            }
        }
        Expr::Const(_) => {}
        Expr::Binary { lhs, rhs, .. } => {
            collect_refs(lhs, out);
            collect_refs(rhs, out);
        }
        _ => unreachable!("no other Expr variants exist"),
    }
}
