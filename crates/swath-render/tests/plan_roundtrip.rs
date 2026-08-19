// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The PlanKind/RenderPlan round-trip property (issue #95): an arbitrary
//! valid plan spec — every kind, every `Colormap` variant, random rescale
//! ranges and band bindings — authored as a standard openEO process graph
//! (`to_openeo_graph`) and compiled back through the #32 compiler
//! (`from_openeo_graph` = [`swath_render::compile`]) reproduces, with
//! structural equality, exactly the plan the single constructor
//! ([`swath_render::plan_for`]) builds, and recovers the spec itself.
//! The same property pins the **dual representations**: the persisted
//! metadata (`PlanKind`/`Rescale`/domain `Colormap`) the constructor
//! emits beside the plan must agree with the plan op for op — the
//! agreement `to_catalog_layer` used to assert nowhere.

use proptest::prelude::*;
use serde_json::{Value as Json, json};
use swath_core::catalog::{Colormap as DomainColormap, PlanKind, Rescale};
use swath_render::ir::{BinaryOp, Colormap, Expr, PixelOp};
use swath_render::{CompileContext, PlanSpec, UdfStage, plan_for};

/// The dataset band vocabulary every generated spec draws from.
const BANDS: [&str; 4] = ["b02", "b03", "b04", "b8a"];

/// The context graphs compile against: each band bound by its own name
/// (exactly how the openEO services surface builds its contexts).
fn ctx() -> CompileContext {
    BANDS
        .iter()
        .fold(CompileContext::new("synthetic"), |ctx, band| {
            ctx.with_band(*band, std::iter::empty::<String>())
        })
}

fn band() -> impl Strategy<Value = String> {
    prop::sample::select(&BANDS[..]).prop_map(str::to_owned)
}

/// Band-math expressions over the vocabulary and small integer constants
/// (integers survive the JSON round trip bit-exactly), bounded depth,
/// referencing at least one band (a constant-only plan loads no bands —
/// not a servable layer shape).
fn expr() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        3 => band().prop_map(Expr::Band),
        1 => (-4i32..=4).prop_map(|c| Expr::Const(f64::from(c))),
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
    .prop_filter("expression must reference at least one band", |e| {
        !referenced(e).is_empty()
    })
}

/// Integer-valued rescale ranges with `min < max` (the compiler and the
/// IR both reject degenerate ranges); integers survive JSON exactly.
fn rescale() -> impl Strategy<Value = Option<(f64, f64)>> {
    prop_oneof![
        1 => Just(None),
        3 => (-1000i32..=1000, 1i32..=2000)
            .prop_map(|(min, span)| Some((f64::from(min), f64::from(min + span)))),
    ]
}

/// Every `Colormap` variant, by name (the `save_result` option spelling).
fn colormap() -> impl Strategy<Value = Colormap> {
    prop_oneof![
        Just(Colormap::Grayscale),
        Just(Colormap::Viridis),
        Just(Colormap::Magma),
        Just(Colormap::RdYlGn),
    ]
}

/// Arbitrary valid plan specs: both kinds, all colormaps, optional
/// rescale, random band bindings (composites may repeat a band).
fn spec() -> impl Strategy<Value = PlanSpec> {
    prop_oneof![
        (band(), band(), band(), rescale())
            .prop_map(|(r, g, b, rescale)| { PlanSpec::Composite { r, g, b, rescale } }),
        (expr(), rescale(), colormap()).prop_map(|(expr, rescale, colormap)| {
            PlanSpec::BandMath {
                expr,
                rescale,
                colormap,
            }
        }),
    ]
}

/// First-reference-order deduplicated band names of an expression.
fn referenced(expr: &Expr) -> Vec<String> {
    fn walk(expr: &Expr, out: &mut Vec<String>) {
        match expr {
            Expr::Band(name) => {
                if !out.iter().any(|n| n == name) {
                    out.push(name.clone());
                }
            }
            Expr::Const(_) => {}
            Expr::Binary { lhs, rhs, .. } => {
                walk(lhs, out);
                walk(rhs, out);
            }
            _ => unreachable!("no other Expr variants exist"),
        }
    }
    let mut out = Vec::new();
    walk(expr, &mut out);
    out
}

/// Serializes an expression as reducer sub-graph nodes (`array_element`
/// leaves by label, arithmetic chained by `from_node`), returning the
/// JSON argument encoding of the root.
fn to_nodes(expr: &Expr, nodes: &mut serde_json::Map<String, Json>) -> Json {
    match expr {
        Expr::Band(name) => {
            let id = format!("n{}", nodes.len());
            nodes.insert(
                id.clone(),
                json!({
                    "process_id": "array_element",
                    "arguments": {"data": {"from_parameter": "data"}, "label": name}
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

/// The `save_result` option spelling of a colormap.
fn colormap_name(map: Colormap) -> &'static str {
    match map {
        Colormap::Grayscale => "grayscale",
        Colormap::Viridis => "viridis",
        Colormap::Magma => "magma",
        Colormap::RdYlGn => "rdylgn",
        _ => unreachable!("no other Colormap variants exist"),
    }
}

/// Authors the spec as a standard openEO process graph — the inverse of
/// the compiler: `load_collection` over exactly the bands the plan reads,
/// a `reduce_dimension` reducer for band math, `linear_scale_range` when
/// rescaled, the colormap named as a `save_result` option.
fn to_openeo_graph(spec: &PlanSpec) -> Json {
    let mut nodes = serde_json::Map::new();
    let (loaded, save_data, options) = match spec {
        PlanSpec::Composite { r, g, b, rescale } => {
            // A composite is the loaded three-band cube itself, in band
            // order; a repeated band is simply listed again.
            let loaded = vec![r.clone(), g.clone(), b.clone()];
            let data = rescale_node(&mut nodes, "load", *rescale);
            (loaded, data, json!({}))
        }
        PlanSpec::BandMath {
            expr,
            rescale,
            colormap,
        } => {
            let mut reducer = serde_json::Map::new();
            let root = to_nodes(expr, &mut reducer);
            let id = root["from_node"]
                .as_str()
                .expect("band-referencing exprs always have a root node")
                .to_owned();
            reducer.get_mut(&id).expect("root node exists")["result"] = json!(true);
            nodes.insert(
                "reduce".into(),
                json!({
                    "process_id": "reduce_dimension",
                    "arguments": {
                        "data": {"from_node": "load"},
                        "dimension": "bands",
                        "reducer": {"process_graph": Json::Object(reducer)}
                    }
                }),
            );
            let data = rescale_node(&mut nodes, "reduce", *rescale);
            (
                referenced(expr),
                data,
                json!({"colormap": colormap_name(*colormap)}),
            )
        }
        _ => unreachable!("no other PlanSpec variants exist"),
    };
    nodes.insert(
        "load".into(),
        json!({
            "process_id": "load_collection",
            "arguments": {
                "id": "synthetic",
                "spatial_extent": null,
                "temporal_extent": null,
                "bands": loaded,
            }
        }),
    );
    nodes.insert(
        "save".into(),
        json!({
            "process_id": "save_result",
            "arguments": {
                "data": save_data,
                "format": "png",
                "options": options,
            },
            "result": true
        }),
    );
    json!({"process_graph": Json::Object(nodes)})
}

/// Appends a `linear_scale_range` node over `input` when the spec is
/// rescaled, returning the reference `save_result` should consume.
fn rescale_node(
    nodes: &mut serde_json::Map<String, Json>,
    input: &str,
    rescale: Option<(f64, f64)>,
) -> Json {
    match rescale {
        None => json!({"from_node": input}),
        Some((min, max)) => {
            nodes.insert(
                "scale".into(),
                json!({
                    "process_id": "linear_scale_range",
                    "arguments": {
                        "x": {"from_node": input},
                        "inputMin": min, "inputMax": max,
                        "outputMin": 0, "outputMax": 255,
                    }
                }),
            );
            json!({"from_node": "scale"})
        }
    }
}

/// Arbitrary valid UDF specs (ADR 0018, #201): random band bindings
/// (possibly repeated), sha256-hex module identities, the two v1 output
/// arities, opaque params, optional rescale.
fn udf_spec() -> impl Strategy<Value = PlanSpec> {
    (
        prop::collection::vec(band(), 1..=4),
        "[0-9a-f]{64}",
        prop_oneof![Just(1u32), Just(3u32)],
        prop_oneof![
            Just(Json::Null),
            (-4i32..=4).prop_map(|k| json!({ "k": k })),
        ],
        rescale(),
    )
        .prop_map(
            |(bands, code_hash, output_planes, params, rescale)| PlanSpec::Udf {
                bands,
                stage: UdfStage::new(code_hash, output_planes, params),
                rescale,
            },
        )
}

proptest! {
    /// The UDF spec's dual representations (#201), through the same
    /// single construction site the other kinds use: exactly one
    /// producing UDF op (the v1 rule the `PlanSpec::Udf` type enforces),
    /// the persisted `PlanKind::Udf` mirroring the stage's `code_hash`,
    /// inputs derived first-reference-deduped, and the rescale record
    /// agreeing with the op. The openEO authoring/compile leg of the
    /// round trip arrives with the `run_udf` compiler work (#204) and
    /// extends this file then.
    #[test]
    fn udf_specs_agree_with_their_metadata(s in udf_spec()) {
        let (plan, meta) = plan_for(&s);
        let PlanSpec::Udf { bands, stage, rescale } = &s else {
            return Err(TestCaseError::fail("udf_spec generates only Udf specs"));
        };

        // One UDF op per plan (v1), leading — the producing op.
        let udf_ops: Vec<_> = plan.ops.iter().filter(|op| matches!(op, PixelOp::Udf(_))).collect();
        prop_assert_eq!(udf_ops.len(), 1);
        prop_assert_eq!(plan.ops.first(), Some(&PixelOp::Udf(stage.clone())));

        // Inputs: first-reference order, deduplicated.
        let mut expected = Vec::new();
        for band in bands {
            if !expected.iter().any(|n| n == band) {
                expected.push(band.clone());
            }
        }
        prop_assert_eq!(
            plan.inputs.iter().map(|i| i.name.clone()).collect::<Vec<_>>(),
            expected
        );

        // The persisted mirror: the module hash is the whole identity.
        prop_assert_eq!(
            &meta.kind,
            &PlanKind::Udf { code_hash: stage.code_hash.clone() }
        );
        let op_rescale = plan.ops.iter().find_map(|op| match op {
            PixelOp::Rescale { min, max } => Some((*min, *max)),
            _ => None,
        });
        prop_assert_eq!(op_rescale, *rescale);
        prop_assert_eq!(
            meta.rescale,
            rescale.map_or(Rescale { min: 0.0, max: 255.0 }, |(min, max)| Rescale { min, max })
        );
        prop_assert_eq!(meta.colormap, None);
    }
}

proptest! {
    /// construct → author as an openEO graph → compile back: the compiled
    /// plan equals the constructed plan structurally, the compiler
    /// recovers the spec itself, and the persisted metadata agrees with
    /// the executable plan op for op.
    #[test]
    fn specs_round_trip_through_the_openeo_graph_and_metadata_agrees(s in spec()) {
        let (plan, meta) = plan_for(&s);
        let graph = to_openeo_graph(&s);
        let product = swath_render::compile(&graph, &ctx()).expect("authored graph compiles");

        // The round trip: to_openeo_graph then from_openeo_graph == plan.
        prop_assert_eq!(&product.plan, &plan);
        prop_assert_eq!(&product.spec, &s);
        prop_assert_eq!(
            &product.bands,
            &plan.inputs.iter().map(|i| i.name.clone()).collect::<Vec<_>>()
        );

        // The dual representations agree: the persisted metadata mirrors
        // the executable ops exactly.
        match &meta.kind {
            PlanKind::BandMath { expression } => {
                let Some(PixelOp::BandMath(expr)) = plan.ops.first() else {
                    return Err(TestCaseError::fail("BandMath kind must lead with a BandMath op"));
                };
                prop_assert_eq!(expression, &expr.to_string());
            }
            PlanKind::Composite { r, g, b } => {
                let Some(PixelOp::Composite { r: pr, g: pg, b: pb }) = plan.ops.first() else {
                    return Err(TestCaseError::fail("Composite kind must lead with a Composite op"));
                };
                prop_assert_eq!((r, g, b), (pr, pg, pb));
            }
            _ => return Err(TestCaseError::fail("unexpected PlanKind variant")),
        }
        let op_rescale = plan.ops.iter().find_map(|op| match op {
            PixelOp::Rescale { min, max } => Some(Rescale { min: *min, max: *max }),
            _ => None,
        });
        // An absent Rescale op renders the identity 0..255 mapping — the
        // persisted record spells that out.
        prop_assert_eq!(
            meta.rescale,
            op_rescale.unwrap_or(Rescale { min: 0.0, max: 255.0 })
        );
        let op_colormap = plan.ops.iter().find_map(|op| match op {
            PixelOp::Colormap(map) => Some(match map {
                Colormap::Grayscale => DomainColormap::Grayscale,
                Colormap::Viridis => DomainColormap::Viridis,
                Colormap::Magma => DomainColormap::Magma,
                Colormap::RdYlGn => DomainColormap::RdYlGn,
                _ => return None,
            }),
            _ => None,
        });
        prop_assert_eq!(meta.colormap, op_colormap);
    }
}
