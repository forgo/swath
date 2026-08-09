// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Process-compiler round-trip evidence (issue #32, REQUIREMENTS.md R3/R5).
//!
//! * **NDVI two ways**: the standard `reduce_dimension` idiom and the `ndvi`
//!   convenience process both compile to the exact hand-built NDVI plan the
//!   golden suites run — structural equality *and* eval equivalence (byte-
//!   identical tiles on synthetic buffers and on real warped HLS fixtures).
//! * **True color**: a load → `linear_scale_range` → save graph compiles to
//!   the hand-built composite plan.
//! * **Golden**: the compiled plans render fixture tiles that pass the
//!   default perceptual-diff policy against the committed rio-tiler oracle
//!   goldens — graph-defined products serve identically to built-ins (R3).
//! * **Spec pins**: the committed openeo-processes 1.2.0 definitions
//!   (tests/data/openeo/, pinned truth) still say what the compiler
//!   assumes (parameter names, defaults, clipping semantics).
//! * **Diagnostics**: every `CompileError` variant is exercised by a
//!   minimal broken graph; the Display strings are UX and are pinned by
//!   insta snapshots.

#[allow(
    dead_code,
    reason = "shared with golden.rs; not every helper is used here"
)]
mod common;

use serde_json::{Value as Json, json};
use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{CompileContext, CompileError, NodataPolicy, Resampling, WarpedBuffer, eval};
use swath_testkit::{DiffPolicy, RgbaImage, diff, load_png};

const B02: &str = "hlss30-t13sdd-2024158-b02.tif";
const B03: &str = "hlss30-t13sdd-2024158-b03.tif";
const B04: &str = "hlss30-t13sdd-2024158-b04.tif";
const B8A: &str = "hlss30-t13sdd-2024158-b8a.tif";

/// The HLS S30 compile context: dataset bands (`swath:bands`) with their
/// openEO/common-name aliases.
fn hls_ctx() -> CompileContext {
    CompileContext::new("hls-s30")
        .with_band("b02", ["blue", "B02"])
        .with_band("b03", ["green", "B03"])
        .with_band("b04", ["red", "B04"])
        .with_band("b8a", ["nir", "B8A"])
}

/// Loads a committed graph from `tests/data/openeo/graphs/`.
fn graph(name: &str) -> Json {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/openeo/graphs")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("graph file exists");
    serde_json::from_str(&text).expect("graph file is valid JSON")
}

/// Loads a committed openeo-processes 1.2.0 definition (pinned truth).
fn process_def(name: &str) -> Json {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/openeo")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).expect("process definition exists");
    serde_json::from_str(&text).expect("process definition is valid JSON")
}

/// The hand-built NDVI plan the golden suite runs (`golden_ir.rs`).
fn hand_ndvi_plan() -> RenderPlan {
    RenderPlan::new(
        vec![BandInput::new("b8a"), BandInput::new("b04")],
        vec![
            PixelOp::BandMath(
                (Expr::band("b8a") - Expr::band("b04")) / (Expr::band("b8a") + Expr::band("b04")),
            ),
            PixelOp::Rescale {
                min: -1.0,
                max: 1.0,
            },
            PixelOp::Colormap(Colormap::Grayscale),
        ],
        OutputSpec::new(TileFormat::Png),
    )
}

/// The hand-built true-color plan the golden suite runs.
fn hand_truecolor_plan() -> RenderPlan {
    RenderPlan::new(
        vec![
            BandInput::new("b04"),
            BandInput::new("b03"),
            BandInput::new("b02"),
        ],
        vec![
            PixelOp::Composite {
                r: "b04".into(),
                g: "b03".into(),
                b: "b02".into(),
            },
            PixelOp::Rescale {
                min: 0.0,
                max: 3000.0,
            },
        ],
        OutputSpec::new(TileFormat::Png),
    )
}

fn buffer(width: u32, height: u32, values: Vec<f64>) -> WarpedBuffer {
    let valid = vec![true; values.len()];
    WarpedBuffer {
        width,
        height,
        values,
        valid,
    }
}

// --- NDVI two ways ------------------------------------------------------

#[test]
fn ndvi_reduce_graph_compiles_to_the_hand_built_plan() {
    let product = swath_render::compile(&graph("ndvi-reduce.json"), &hls_ctx()).expect("compiles");
    assert_eq!(product.plan, hand_ndvi_plan());
    assert_eq!(product.collection, "hls-s30");
    // Serving metadata: exactly the referenced bands, plan-input order.
    assert_eq!(product.bands, ["b8a", "b04"]);
}

#[test]
fn ndvi_convenience_graph_compiles_to_the_same_plan() {
    let product =
        swath_render::compile(&graph("ndvi-convenience.json"), &hls_ctx()).expect("compiles");
    assert_eq!(product.plan, hand_ndvi_plan());
    assert_eq!(product.bands, ["b8a", "b04"]);
}

#[test]
fn truecolor_graph_compiles_to_the_hand_built_plan() {
    let product = swath_render::compile(&graph("truecolor.json"), &hls_ctx()).expect("compiles");
    assert_eq!(product.plan, hand_truecolor_plan());
    assert_eq!(product.bands, ["b04", "b03", "b02"]);
}

#[test]
fn compiled_and_hand_built_ndvi_evaluate_byte_identically() {
    let product = swath_render::compile(&graph("ndvi-reduce.json"), &hls_ctx()).expect("compiles");
    let nir = buffer(2, 2, vec![3000.0, 1000.0, 0.0, 500.0]);
    let red = buffer(2, 2, vec![1000.0, 3000.0, 0.0, 250.0]);
    let ours = eval(&product.plan, &[nir.clone(), red.clone()]).expect("compiled plan evaluates");
    let hand = eval(&hand_ndvi_plan(), &[nir, red]).expect("hand plan evaluates");
    assert_eq!(ours.pixels, hand.pixels);
}

// --- Golden renders through compiled plans (R3) -------------------------

#[allow(clippy::print_stdout, reason = "diff metrics are the test's report")]
async fn assert_compiled_matches_oracle(graph_file: &str, fixtures: &[&str], golden: &str) {
    let product = swath_render::compile(&graph(graph_file), &hls_ctx()).expect("compiles");
    let tile = swath_core::tile::TileCoord::new(12, 848, 1561).expect("valid tile");
    let mut warped = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        let (buffer, _, _) = common::render_warped(
            fixture,
            tile,
            Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
        )
        .await;
        warped.push(buffer);
    }
    let tile = eval(&product.plan, &warped).expect("compiled plan evaluates");
    let ours = RgbaImage::from_raw(tile.width, tile.height, tile.pixels).expect("tile buffer");
    let reference = load_png(&common::goldens_dir().join(golden)).expect("golden loads");
    let report = diff(&ours, &reference).expect("dimensions match");
    let policy = DiffPolicy::default();
    let bad_pct = report.pct_pixels_exceeding_tolerance(policy.per_channel_tolerance) * 100.0;
    println!(
        "{graph_file} -> {golden}: max |diff| {max}, mean |diff| {mean:.4}, bad pixels {bad_pct:.4}%",
        max = report.max_abs_channel_diff,
        mean = report.mean_abs_diff,
    );
    assert!(
        report.passes(&policy),
        "{graph_file}: compiled plan fails default policy vs {golden} — max |diff| {}, \
         {bad_pct:.4}% pixels over tolerance {}",
        report.max_abs_channel_diff,
        policy.per_channel_tolerance,
    );
}

#[tokio::test]
async fn compiled_ndvi_graph_matches_the_oracle_golden() {
    assert_compiled_matches_oracle("ndvi-reduce.json", &[B8A, B04], "ndvi-12-848-1561.png").await;
}

#[tokio::test]
async fn compiled_truecolor_graph_matches_the_oracle_golden() {
    assert_compiled_matches_oracle(
        "truecolor.json",
        &[B04, B03, B02],
        "truecolor-12-848-1561.png",
    )
    .await;
}

// --- Spec pins (pinned truth: openeo-processes 1.2.0) -------------------

/// Parameter names, in declaration order.
fn param_names(def: &Json) -> Vec<&str> {
    def["parameters"]
        .as_array()
        .expect("parameters array")
        .iter()
        .map(|p| p["name"].as_str().expect("param name"))
        .collect()
}

fn param<'a>(def: &'a Json, name: &str) -> &'a Json {
    def["parameters"]
        .as_array()
        .expect("parameters array")
        .iter()
        .find(|p| p["name"] == name)
        .expect("parameter exists")
}

#[test]
fn pinned_definitions_still_say_what_the_compiler_assumes() {
    // The arithmetic processes take x and y.
    for op in ["add", "subtract", "multiply", "divide"] {
        assert_eq!(param_names(&process_def(op)), ["x", "y"], "{op}");
    }
    // array_element: index or label, both optional (exactly-one enforced
    // at runtime), over data.
    let array_element = process_def("array_element");
    assert!(param_names(&array_element).starts_with(&["data", "index", "label"]));
    assert_eq!(param(&array_element, "index")["optional"], json!(true));
    assert_eq!(param(&array_element, "label")["optional"], json!(true));
    // reduce_dimension: data + reducer callback + dimension.
    let reduce = process_def("reduce_dimension");
    assert!(param_names(&reduce).starts_with(&["data", "reducer", "dimension"]));
    // ndvi: nir/red default to the common names.
    let ndvi = process_def("ndvi");
    assert_eq!(param(&ndvi, "nir")["default"], json!("nir"));
    assert_eq!(param(&ndvi, "red")["default"], json!("red"));
    assert_eq!(param(&ndvi, "target_band")["default"], Json::Null);
    // linear_scale_range: output range defaults 0..1 (why the compiler
    // must reject a defaulted output range rather than assume 0..255),
    // and the input range clips — exactly Rescale's clamp.
    let scale = process_def("linear_scale_range");
    assert_eq!(param(&scale, "outputMin")["default"], json!(0));
    assert_eq!(param(&scale, "outputMax")["default"], json!(1));
    assert!(
        scale["description"]
            .as_str()
            .expect("description")
            .contains("clipped")
    );
    // load_collection: bands is optional in the spec (v0 requires it).
    let load = process_def("load_collection");
    assert_eq!(param(&load, "bands")["optional"], json!(true));
    // save_result: data + format.
    assert!(param_names(&process_def("save_result")).starts_with(&["data", "format"]));
}

// --- Error paths: every variant, snapshot-pinned diagnostics ------------

fn err(graph: &Json) -> CompileError {
    swath_render::compile(graph, &hls_ctx()).expect_err("graph must not compile")
}

fn load_node() -> Json {
    json!({
        "process_id": "load_collection",
        "arguments": {
            "id": "hls-s30",
            "spatial_extent": null,
            "temporal_extent": null,
            "bands": ["red", "green", "blue"]
        }
    })
}

/// Wraps a node map in the `{"process_graph": ...}` envelope.
fn save_graph(nodes: &Json) -> Json {
    json!({ "process_graph": nodes })
}

#[test]
fn structural_error_displays_are_pinned() {
    // Malformed: not an object.
    insta::assert_snapshot!(
        err(&json!([1, 2, 3])),
        @"malformed process graph: a process graph must be a JSON object"
    );
    // Malformed: node without process_id.
    insta::assert_snapshot!(
        err(&save_graph(&json!({"load": {"arguments": {}}}))),
        @r#"malformed process graph: node `load` has no string "process_id""#
    );
    // NoResult.
    insta::assert_snapshot!(
        err(&save_graph(&json!({"load": load_node()}))),
        @r#"no result node: exactly one node must set "result": true"#
    );
    // MultipleResults.
    let mut load_a = load_node();
    load_a["result"] = json!(true);
    let mut load_b = load_node();
    load_b["result"] = json!(true);
    insta::assert_snapshot!(
        err(&save_graph(&json!({"a": load_a, "b": load_b}))),
        @r#"multiple result nodes (["a", "b"]): exactly one node may set "result": true"#
    );
    // UnsavedResult: the result node is not save_result.
    let mut load_result = load_node();
    load_result["result"] = json!(true);
    insta::assert_snapshot!(
        err(&save_graph(&json!({"load": load_result}))),
        @r#"result node `load` is `load_collection`: the graph must end in save_result (format "png")"#
    );
}

#[test]
fn reference_error_displays_are_pinned() {
    // DanglingReference.
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "nope"}, "format": "png"},
                "result": true
            }
        }))),
        @"node `save` references `nope` via from_node, but no such node exists"
    );
    // Cycle: a -> b -> a.
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "a": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "b"}, "format": "png"}
            },
            "b": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "a"}, "format": "png"},
                "result": true
            }
        }))),
        @"cycle detected through node `b`: process graphs must be acyclic"
    );
}

#[test]
fn unsupported_and_unknown_error_displays_are_pinned() {
    // UnsupportedProcess: a real openEO process outside the subset.
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "load": load_node(),
            "blur": {
                "process_id": "apply_kernel",
                "arguments": {"data": {"from_node": "load"}, "kernel": [[1.0]]}
            },
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "blur"}, "format": "png"},
                "result": true
            }
        }))),
        @"node `blur`: unsupported process `apply_kernel` — the supported subset is: load_collection, reduce_dimension, array_element, add, subtract, multiply, divide, linear_scale_range, ndvi, save_result"
    );
    // UnknownCollection.
    let mut wrong_collection = load_node();
    wrong_collection["arguments"]["id"] = json!("sentinel-2-l2a");
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "load": wrong_collection,
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "load"}, "format": "png"},
                "result": true
            }
        }))),
        @"node `load`: unknown collection `sentinel-2-l2a` (this product compiles against `hls-s30`)"
    );
    // UnknownBand.
    let mut wrong_band = load_node();
    wrong_band["arguments"]["bands"] = json!(["swir"]);
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "load": wrong_band,
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "load"}, "format": "png"},
                "result": true
            }
        }))),
        @r#"node `load`: unknown band `swir` — known bands and aliases: ["b02", "blue", "B02", "b03", "green", "B03", "b04", "red", "B04", "b8a", "nir", "B8A"]"#
    );
}

#[test]
fn argument_error_displays_are_pinned() {
    // MissingArgument: v0 requires an explicit bands list.
    let mut no_bands = load_node();
    no_bands["arguments"]
        .as_object_mut()
        .expect("arguments object")
        .remove("bands");
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "load": no_bands,
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "load"}, "format": "png"},
                "result": true
            }
        }))),
        @"node `load` (load_collection): missing required argument `bands`"
    );
    // InvalidArgument: save_result in a format the IR cannot emit.
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "load": load_node(),
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "load"}, "format": "GTiff"},
                "result": true
            }
        }))),
        @r#"node `save` (save_result): invalid argument `format`: only "png" is supported in v0, got "GTiff""#
    );
    // InvalidArgument: a defaulted (0..1) output range cannot quantize.
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "load": load_node(),
            "scale": {
                "process_id": "linear_scale_range",
                "arguments": {"x": {"from_node": "load"}, "inputMin": 0, "inputMax": 3000}
            },
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "scale"}, "format": "png"},
                "result": true
            }
        }))),
        @"node `scale` (linear_scale_range): invalid argument `outputMin`: the Render IR quantizes to 8-bit; the output range must be exactly 0..255, got 0..1"
    );
}

#[test]
fn type_error_displays_are_pinned() {
    // TypeMismatch: linear_scale_range on a multi-band cube that never
    // reduces and cannot composite (2 bands at save).
    let mut two_bands = load_node();
    two_bands["arguments"]["bands"] = json!(["nir", "red"]);
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "load": two_bands,
            "scale": {
                "process_id": "linear_scale_range",
                "arguments": {
                    "x": {"from_node": "load"},
                    "inputMin": 0, "inputMax": 3000,
                    "outputMin": 0, "outputMax": 255
                }
            },
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "scale"}, "format": "png"},
                "result": true
            }
        }))),
        @"node `save` (save_result): type mismatch — expected exactly 3 bands for an RGB composite (or reduce to gray first), got a data cube with 2 bands"
    );
    // TypeMismatch: a cube flowing into scalar arithmetic.
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "load": load_node(),
            "double": {
                "process_id": "add",
                "arguments": {"x": {"from_node": "load"}, "y": {"from_node": "load"}}
            },
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "double"}, "format": "png"},
                "result": true
            }
        }))),
        @"node `double` (add): type mismatch — expected a scalar (number, band element, or arithmetic result), got a multi-band data cube"
    );
    // UnknownParameter: from_parameter outside any reducer scope.
    insta::assert_snapshot!(
        err(&save_graph(&json!({
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_parameter": "data"}, "format": "png"},
                "result": true
            }
        }))),
        @"node `save`: from_parameter `data` is not defined in this scope"
    );
}

// --- Additional compiler semantics --------------------------------------

#[test]
fn array_element_by_index_uses_loaded_band_order() {
    let g = json!({
        "process_graph": {
            "load": {
                "process_id": "load_collection",
                "arguments": {
                    "id": "hls-s30",
                    "spatial_extent": null,
                    "temporal_extent": null,
                    "bands": ["nir", "red"]
                }
            },
            "reduce": {
                "process_id": "reduce_dimension",
                "arguments": {
                    "data": {"from_node": "load"},
                    "dimension": "bands",
                    "reducer": {"process_graph": {
                        "nir": {
                            "process_id": "array_element",
                            "arguments": {"data": {"from_parameter": "data"}, "index": 0}
                        },
                        "red": {
                            "process_id": "array_element",
                            "arguments": {"data": {"from_parameter": "data"}, "index": 1}
                        },
                        "diff": {
                            "process_id": "subtract",
                            "arguments": {"x": {"from_node": "nir"}, "y": {"from_node": "red"}},
                            "result": true
                        }
                    }}
                }
            },
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "reduce"}, "format": "png"},
                "result": true
            }
        }
    });
    let product = swath_render::compile(&g, &hls_ctx()).expect("compiles");
    assert_eq!(
        product.plan.ops[0],
        PixelOp::BandMath(Expr::band("b8a") - Expr::band("b04"))
    );
    // No linear_scale_range in the graph: gray still gets its colormap,
    // but no rescale materializes.
    assert_eq!(product.plan.ops.len(), 2);
    assert_eq!(product.plan.ops[1], PixelOp::Colormap(Colormap::Grayscale));
}

#[test]
fn numbers_and_shared_nodes_lower_into_the_expression() {
    // savi-like: ((nir - red) / (nir + red + 0.5)) * 1.5 — constants and a
    // node referenced twice (the DAG, not a tree).
    let g = json!({
        "process_graph": {
            "load": {
                "process_id": "load_collection",
                "arguments": {
                    "id": "hls-s30",
                    "spatial_extent": null,
                    "temporal_extent": null,
                    "bands": ["nir", "red"]
                }
            },
            "reduce": {
                "process_id": "reduce_dimension",
                "arguments": {
                    "data": {"from_node": "load"},
                    "dimension": "bands",
                    "reducer": {"process_graph": {
                        "nir": {
                            "process_id": "array_element",
                            "arguments": {"data": {"from_parameter": "data"}, "label": "nir"}
                        },
                        "red": {
                            "process_id": "array_element",
                            "arguments": {"data": {"from_parameter": "data"}, "label": "red"}
                        },
                        "num": {
                            "process_id": "subtract",
                            "arguments": {"x": {"from_node": "nir"}, "y": {"from_node": "red"}}
                        },
                        "sum": {
                            "process_id": "add",
                            "arguments": {"x": {"from_node": "nir"}, "y": {"from_node": "red"}}
                        },
                        "den": {
                            "process_id": "add",
                            "arguments": {"x": {"from_node": "sum"}, "y": 0.5}
                        },
                        "ratio": {
                            "process_id": "divide",
                            "arguments": {"x": {"from_node": "num"}, "y": {"from_node": "den"}}
                        },
                        "savi": {
                            "process_id": "multiply",
                            "arguments": {"x": {"from_node": "ratio"}, "y": 1.5},
                            "result": true
                        }
                    }}
                }
            },
            "save": {
                "process_id": "save_result",
                "arguments": {"data": {"from_node": "reduce"}, "format": "png"},
                "result": true
            }
        }
    });
    let product = swath_render::compile(&g, &hls_ctx()).expect("compiles");
    let (nir, red) = (Expr::band("b8a"), Expr::band("b04"));
    let expected = (nir.clone() - red.clone()) / (nir + red + Expr::Const(0.5)) * Expr::Const(1.5);
    assert_eq!(product.plan.ops[0], PixelOp::BandMath(expected));
    assert_eq!(product.bands, ["b8a", "b04"]);
}

#[test]
fn save_result_format_is_case_insensitive() {
    let mut g = graph("truecolor.json");
    g["process_graph"]["save"]["arguments"]["format"] = json!("PNG");
    let product = swath_render::compile(&g, &hls_ctx()).expect("compiles");
    assert_eq!(product.plan.output, OutputSpec::new(TileFormat::Png));
}
