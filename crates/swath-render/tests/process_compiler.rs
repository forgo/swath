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

mod common;

use serde_json::{Value as Json, json};
use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{
    CompileContext, CompileError, NoUdf, NodataPolicy, Resampling, WarpedBuffer, eval,
};
use swath_testsupport::RgbaImage;

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
    let ours =
        eval(&product.plan, &[nir.clone(), red.clone()], &NoUdf).expect("compiled plan evaluates");
    let hand = eval(&hand_ndvi_plan(), &[nir, red], &NoUdf).expect("hand plan evaluates");
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
    let tile = eval(&product.plan, &warped, &NoUdf).expect("compiled plan evaluates");
    let ours = RgbaImage::from_raw(tile.width, tile.height, tile.pixels).expect("tile buffer");
    swath_testsupport::pdiff::assert_matches_golden(
        &format!("{graph_file} -> {golden}"),
        &ours,
        &common::goldens_dir().join(golden),
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
    // load_collection.temporal_extent / filter_temporal.extent: the
    // interval is LEFT-CLOSED (start included, end excluded) — why the
    // compiler steps the end back one millisecond when lowering to the
    // catalog's inclusive TimeRange — and either bound may be null,
    // never both.
    for (def, name) in [
        (&load, "temporal_extent"),
        (&process_def("filter_temporal"), "extent"),
    ] {
        let description = param(def, name)["description"]
            .as_str()
            .expect("description");
        assert!(description.to_lowercase().contains("left-closed"), "{name}");
        assert!(description.contains("**excluded**"), "{name}");
        assert!(description.contains("never both"), "{name}");
    }
    // filter_temporal: data + extent + dimension, dimension defaulting
    // to null (= the only temporal dimension), with the
    // DimensionNotAvailable exception the compiler's diagnostic mirrors.
    let filter = process_def("filter_temporal");
    assert_eq!(param_names(&filter), ["data", "extent", "dimension"]);
    assert_eq!(param(&filter, "dimension")["default"], Json::Null);
    assert_eq!(param(&filter, "dimension")["optional"], json!(true));
    assert!(
        filter["exceptions"]
            .as_object()
            .expect("exceptions")
            .contains_key("DimensionNotAvailable")
    );
    // save_result: data + format.
    assert!(param_names(&process_def("save_result")).starts_with(&["data", "format"]));
    // merge_cubes (ADR 0022): cube1 + cube2 + an OPTIONAL overlap_resolver
    // (why the compiler must require it explicitly — the spec's default,
    // failing on overlap, would reject every pixel of a join) + context;
    // the resolver's parameters are named x and y.
    let merge = process_def("merge_cubes");
    assert_eq!(
        param_names(&merge),
        ["cube1", "cube2", "overlap_resolver", "context"]
    );
    assert_eq!(param(&merge, "overlap_resolver")["optional"], json!(true));
    assert_eq!(param(&merge, "context")["optional"], json!(true));
    let resolver_params: Vec<&str> = param(&merge, "overlap_resolver")["schema"]["parameters"]
        .as_array()
        .expect("resolver parameters")
        .iter()
        .map(|p| p["name"].as_str().expect("name"))
        .collect();
    assert!(resolver_params.starts_with(&["x", "y"]));
    assert_eq!(merge["returns"]["schema"]["subtype"], json!("raster-cube"));
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
        @"node `blur`: unsupported process `apply_kernel` — the supported subset is: load_collection, filter_temporal, reduce_dimension, array_element, add, subtract, multiply, divide, linear_scale_range, ndvi, merge_cubes, run_udf, save_result"
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

// --- save_result colormap option (issue #94) ----------------------------

#[test]
fn save_result_colormap_option_selects_the_palette() {
    for (name, expected) in [
        ("grayscale", Colormap::Grayscale),
        ("viridis", Colormap::Viridis),
        ("magma", Colormap::Magma),
        ("rdylgn", Colormap::RdYlGn),
    ] {
        let mut g = graph("ndvi-convenience.json");
        g["process_graph"]["save"]["arguments"]["options"] = json!({ "colormap": name });
        let product = swath_render::compile(&g, &hls_ctx()).expect("compiles");
        assert_eq!(
            product.plan.ops.last(),
            Some(&PixelOp::Colormap(expected)),
            "colormap option `{name}`"
        );
    }
    // Empty options object: same as absent — gray defaults to grayscale.
    let mut g = graph("ndvi-convenience.json");
    g["process_graph"]["save"]["arguments"]["options"] = json!({});
    let product = swath_render::compile(&g, &hls_ctx()).expect("compiles");
    assert_eq!(product.plan, hand_ndvi_plan());
}

#[test]
fn save_result_colormap_errors_are_pinned() {
    // Unknown colormap name.
    let mut g = graph("ndvi-convenience.json");
    g["process_graph"]["save"]["arguments"]["options"] = json!({ "colormap": "jet" });
    insta::assert_snapshot!(
        err(&g),
        @r#"node `save` (save_result): invalid argument `options`: unknown colormap "jet": expected one of "grayscale", "viridis", "magma", "rdylgn""#
    );
    // Unknown option key.
    let mut g = graph("ndvi-convenience.json");
    g["process_graph"]["save"]["arguments"]["options"] = json!({ "quality": 9 });
    insta::assert_snapshot!(
        err(&g),
        @r#"node `save` (save_result): invalid argument `options`: the only supported format option is "colormap", got key "quality""#
    );
    // A colormap on a multi-band (composite) result.
    let mut g = graph("truecolor.json");
    g["process_graph"]["save"]["arguments"]["options"] = json!({ "colormap": "viridis" });
    insta::assert_snapshot!(
        err(&g),
        @"node `save` (save_result): invalid argument `options`: a colormap maps one gray value per pixel; it cannot apply to a multi-band (composite) result — reduce to gray first"
    );
}

// --- Temporal windows (ADR 0015 frame selection, issue #181) ------------

use swath_core::catalog::{Datetime, TimeRange};

fn dt(s: &str) -> Datetime {
    Datetime::new(s).expect("valid test datetime")
}

/// The ndvi-convenience graph with `temporal_extent` set on its load node.
fn windowed_graph(extent: Json) -> Json {
    let mut g = graph("ndvi-convenience.json");
    g["process_graph"]["load"]["arguments"]["temporal_extent"] = extent;
    g
}

#[test]
fn absent_or_null_temporal_extent_leaves_the_window_open() {
    // The committed graphs carry `temporal_extent: null` (no filter).
    for name in [
        "ndvi-convenience.json",
        "ndvi-reduce.json",
        "truecolor.json",
    ] {
        let product = swath_render::compile(&graph(name), &hls_ctx()).expect("compiles");
        assert_eq!(product.window, TimeRange::default(), "{name}");
    }
}

#[test]
fn temporal_extent_compiles_into_the_resolution_window() {
    // Left-closed per the pinned definition: the start is included, the
    // end excluded — lowered to the catalog's inclusive TimeRange by
    // stepping the end back one millisecond (the domain's comparison
    // resolution, provider.rs `latest`).
    let product = swath_render::compile(
        &windowed_graph(json!(["2024-06-01T00:00:00Z", "2024-07-01T00:00:00Z"])),
        &hls_ctx(),
    )
    .expect("compiles");
    assert_eq!(
        product.window,
        TimeRange {
            start: Some(dt("2024-06-01T00:00:00Z")),
            end: Some(dt("2024-06-30T23:59:59.999Z")),
        }
    );
    // The window changes granule resolution only — the executable plan
    // is byte-for-byte the windowless NDVI plan.
    assert_eq!(product.plan, hand_ndvi_plan());
}

#[test]
fn date_and_year_bounds_denote_their_first_instant() {
    let product = swath_render::compile(&windowed_graph(json!(["2024", "2024-08-16"])), &hls_ctx())
        .expect("compiles");
    assert_eq!(
        product.window,
        TimeRange {
            start: Some(dt("2024-01-01T00:00:00Z")),
            end: Some(dt("2024-08-15T23:59:59.999Z")),
        }
    );
}

#[test]
fn open_ended_bounds_stay_open() {
    let product = swath_render::compile(
        &windowed_graph(json!([null, "2024-08-16T00:00:00Z"])),
        &hls_ctx(),
    )
    .expect("compiles");
    assert_eq!(
        product.window,
        TimeRange {
            start: None,
            end: Some(dt("2024-08-15T23:59:59.999Z")),
        }
    );
    let product = swath_render::compile(
        &windowed_graph(json!(["2024-08-16T00:00:00Z", null])),
        &hls_ctx(),
    )
    .expect("compiles");
    assert_eq!(
        product.window,
        TimeRange {
            start: Some(dt("2024-08-16T00:00:00Z")),
            end: None,
        }
    );
}

/// Splices a `filter_temporal` between `load` and the ndvi node of the
/// convenience graph.
fn filtered_graph(load_extent: Json, filter_args: Json) -> Json {
    let mut g = windowed_graph(load_extent);
    let mut args = filter_args;
    args["data"] = json!({"from_node": "load"});
    g["process_graph"]["filter"] = json!({
        "process_id": "filter_temporal",
        "arguments": args,
    });
    g["process_graph"]["ndvi"]["arguments"]["data"] = json!({"from_node": "filter"});
    g
}

#[test]
fn filter_temporal_intersects_with_the_loaded_window() {
    let product = swath_render::compile(
        &filtered_graph(
            json!(["2024-06-01T00:00:00Z", "2024-12-01T00:00:00Z"]),
            json!({"extent": ["2024-08-01T00:00:00Z", "2025-01-01T00:00:00Z"], "dimension": "t"}),
        ),
        &hls_ctx(),
    )
    .expect("compiles");
    assert_eq!(
        product.window,
        TimeRange {
            start: Some(dt("2024-08-01T00:00:00Z")),
            end: Some(dt("2024-11-30T23:59:59.999Z")),
        }
    );
}

#[test]
fn filter_temporal_composes_after_reduction_too() {
    // filter_temporal never touches pixels, so it applies to a gray
    // (reduced) cube exactly as to a loaded one.
    let mut g = graph("ndvi-convenience.json");
    g["process_graph"]["filter"] = json!({
        "process_id": "filter_temporal",
        "arguments": {
            "data": {"from_node": "scale"},
            "extent": ["2024-08-16", null],
        },
    });
    g["process_graph"]["save"]["arguments"]["data"] = json!({"from_node": "filter"});
    let product = swath_render::compile(&g, &hls_ctx()).expect("compiles");
    assert_eq!(
        product.window,
        TimeRange {
            start: Some(dt("2024-08-16T00:00:00Z")),
            end: None,
        }
    );
    assert_eq!(product.plan, hand_ndvi_plan());
}

#[test]
fn temporal_window_errors_are_pinned() {
    // Empty interval: the left-closed [t, t) contains no instant.
    insta::assert_snapshot!(
        err(&windowed_graph(json!(["2024-06-01T00:00:00Z", "2024-06-01T00:00:00Z"]))),
        @"node `load` (load_collection): empty temporal window: the left-closed interval [2024-06-01T00:00:00Z, 2024-06-01T00:00:00Z) contains no instant — the end must be after the start"
    );
    // Reversed interval.
    insta::assert_snapshot!(
        err(&windowed_graph(json!(["2024-07-01T00:00:00Z", "2024-06-01T00:00:00Z"]))),
        @"node `load` (load_collection): empty temporal window: the left-closed interval [2024-07-01T00:00:00Z, 2024-06-01T00:00:00Z) contains no instant — the end must be after the start"
    );
    // Disjoint filter: the combined window provably selects nothing.
    insta::assert_snapshot!(
        err(&filtered_graph(
            json!(["2024-06-01T00:00:00Z", "2024-07-01T00:00:00Z"]),
            json!({"extent": ["2024-08-01T00:00:00Z", "2024-09-01T00:00:00Z"]}),
        )),
        @"node `filter` (filter_temporal): empty temporal window: this interval does not overlap the window already applied — the combined window (2024-08-01T00:00:00Z .. 2024-06-30T23:59:59.999Z) selects nothing"
    );
    // Both bounds null: the spec says never both.
    insta::assert_snapshot!(
        err(&windowed_graph(json!([null, null]))),
        @"node `load` (load_collection): invalid argument `temporal_extent`: an interval open on both sides selects everything — use null for the whole argument instead of [null, null]"
    );
    // A non-UTC datetime: the Swath profile narrows to Z.
    insta::assert_snapshot!(
        err(&windowed_graph(json!(["2024-06-01T00:00:00+02:00", null]))),
        @"node `load` (load_collection): invalid argument `temporal_extent`: `2024-06-01T00:00:00+02:00` is not an RFC 3339 UTC (Z) date-time, date, or year"
    );
    // Not an interval at all.
    insta::assert_snapshot!(
        err(&windowed_graph(json!("2024-06-01T00:00:00Z"))),
        @r#"node `load` (load_collection): invalid argument `temporal_extent`: expected a temporal interval [start, end], got "2024-06-01T00:00:00Z""#
    );
    // filter_temporal on a non-temporal dimension: the spec's
    // DimensionNotAvailable exception.
    insta::assert_snapshot!(
        err(&filtered_graph(
            json!(null),
            json!({"extent": ["2024-06-01T00:00:00Z", null], "dimension": "bands"}),
        )),
        @"node `filter` (filter_temporal): dimension `bands` does not exist — the temporal dimension is `t` (DimensionNotAvailable)"
    );
    // filter_temporal without its required extent.
    insta::assert_snapshot!(
        err(&filtered_graph(json!(null), json!({}))),
        @"node `filter` (filter_temporal): missing required argument `extent`"
    );
}

// --- run_udf (ADR 0018, #204): the compiler + registrar seam ------------

mod udf {
    use std::sync::{Arc, Mutex};

    use base64::Engine as _;
    use serde_json::{Value as Json, json};
    use swath_core::udf::code_hash;
    use swath_render::ir::PixelOp;
    use swath_render::{
        CompileContext, CompileError, PlanSpec, UdfError, UdfRegistrar, UdfRegistration, UdfStage,
    };

    use super::{err, hls_ctx, param, param_names, process_def};

    /// A registrar double: hashes like the real one, answers a fixed
    /// output arity (or a fixed refusal), and records every call.
    struct FakeRegistrar {
        answer: Result<u32, UdfError>,
        calls: Mutex<Vec<(String, u32)>>,
    }

    impl FakeRegistrar {
        fn planes(output_planes: u32) -> Arc<Self> {
            Arc::new(Self {
                answer: Ok(output_planes),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn refusing(err: UdfError) -> Arc<Self> {
            Arc::new(Self {
                answer: Err(err),
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    impl UdfRegistrar for FakeRegistrar {
        fn register(&self, bytes: &[u8], input_planes: u32) -> Result<UdfRegistration, UdfError> {
            let hash = code_hash(bytes);
            self.calls
                .lock()
                .unwrap()
                .push((hash.clone(), input_planes));
            self.answer
                .clone()
                .map(|planes| UdfRegistration::new(hash, planes))
        }
    }

    /// Stand-in module bytes (the compiler never parses them; the
    /// registrar double accepts anything).
    const MODULE: &[u8] = b"\0asm\x01\0\0\0 not really";

    fn data_url(bytes: &[u8]) -> String {
        format!(
            "data:application/wasm;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    fn ctx(registrar: &Arc<FakeRegistrar>) -> CompileContext {
        hls_ctx().with_udf_registrar(Arc::clone(registrar) as Arc<dyn UdfRegistrar>)
    }

    /// `load(b8a, b04)` → `run_udf` (with `extra` merged in) → scale → save.
    fn udf_graph(udf: &str, extra: &Json) -> Json {
        let mut arguments = json!({
            "data": { "from_node": "load" },
            "udf": udf,
            "runtime": "wasm",
            "version": "1",
        });
        for (key, value) in extra.as_object().expect("object") {
            arguments[key] = value.clone();
        }
        json!({ "process_graph": {
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "bands": ["b8a", "b04"],
            }},
            "udf": { "process_id": "run_udf", "arguments": arguments },
            "scale": { "process_id": "linear_scale_range", "arguments": {
                "x": { "from_node": "udf" },
                "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255,
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "scale" }, "format": "png",
            }, "result": true },
        }})
    }

    #[test]
    fn inline_module_compiles_to_the_udf_plan_and_carries_the_bytes_out() {
        let registrar = FakeRegistrar::planes(1);
        let graph = udf_graph(&data_url(MODULE), &json!({ "context": { "scale": 2 } }));
        let product = swath_render::compile(&graph, &ctx(&registrar)).expect("compiles");
        let stage = UdfStage::new(code_hash(MODULE), 1, json!({ "scale": 2 }));
        assert_eq!(
            product.spec,
            PlanSpec::Udf {
                bands: vec!["b8a".into(), "b04".into()],
                stage: stage.clone(),
                rescale: Some((-1.0, 1.0)),
            }
        );
        assert_eq!(
            product.plan.ops,
            [
                PixelOp::Udf(stage),
                PixelOp::Rescale {
                    min: -1.0,
                    max: 1.0
                }
            ]
        );
        assert_eq!(product.bands, ["b8a", "b04"]);
        assert_eq!(product.udf_module.as_deref(), Some(MODULE));
        // Registered exactly once, against the plan's input arity.
        assert_eq!(*registrar.calls.lock().unwrap(), [(code_hash(MODULE), 2)]);
        // The IR shape a published UDF service persists: hash, arity,
        // params — never bytes.
        insta::assert_json_snapshot!("run_udf_plan_json_shape", product.plan);
    }

    #[test]
    fn context_defaults_to_null_and_version_may_be_omitted_or_null() {
        let registrar = FakeRegistrar::planes(3);
        for version in [json!({}), json!({ "version": null })] {
            let product =
                swath_render::compile(&udf_graph(&data_url(MODULE), &version), &ctx(&registrar))
                    .expect("compiles");
            let PlanSpec::Udf { stage, .. } = &product.spec else {
                panic!("udf spec");
            };
            assert_eq!(stage.params, Json::Null);
            assert_eq!(stage.output_planes, 3);
        }
    }

    #[test]
    fn duplicate_loaded_bands_register_the_deduplicated_arity() {
        let registrar = FakeRegistrar::planes(1);
        let mut graph = udf_graph(&data_url(MODULE), &json!({}));
        graph["process_graph"]["load"]["arguments"]["bands"] = json!(["b8a", "b04", "b8a"]);
        let product = swath_render::compile(&graph, &ctx(&registrar)).expect("compiles");
        assert_eq!(product.bands, ["b8a", "b04"]);
        assert_eq!(*registrar.calls.lock().unwrap(), [(code_hash(MODULE), 2)]);
    }

    /// Remote modules are never fetched by the compiler: the caller hands
    /// the bytes in under the exact URL, or the node fails.
    #[test]
    fn remote_modules_come_from_the_context_never_from_the_network() {
        let registrar = FakeRegistrar::planes(1);
        let url = "https://udf.example/ndvi.wasm";
        let graph = udf_graph(url, &json!({}));
        let product = swath_render::compile(
            &graph,
            &ctx(&registrar).with_udf_module(url, MODULE.to_vec()),
        )
        .expect("compiles");
        assert_eq!(product.udf_module.as_deref(), Some(MODULE));
        let PlanSpec::Udf { stage, .. } = &product.spec else {
            panic!("udf spec");
        };
        assert_eq!(stage.code_hash, code_hash(MODULE));
        insta::assert_snapshot!(
            swath_render::compile(&graph, &ctx(&registrar)).expect_err("not fetched"),
            @"node `udf` (run_udf): invalid argument `udf`: remote module `https://udf.example/ndvi.wasm` was not fetched for this compile motion"
        );
    }

    #[test]
    fn without_a_registrar_run_udf_is_unavailable() {
        insta::assert_snapshot!(
            err(&udf_graph(&data_url(MODULE), &json!({}))),
            @"node `udf`: run_udf is not available — this deployment wires no UDF executor or module store (ADR 0018)"
        );
    }

    #[test]
    fn argument_error_displays_are_pinned() {
        let registrar = FakeRegistrar::planes(1);
        let fail = |graph: &Json| -> CompileError {
            swath_render::compile(graph, &ctx(&registrar)).expect_err("must not compile")
        };
        insta::assert_snapshot!(
            fail(&udf_graph(&data_url(MODULE), &json!({ "runtime": "Python" }))),
            @r#"node `udf` (run_udf): invalid argument `runtime`: only the "wasm" runtime is supported (InvalidRuntime), got "Python""#
        );
        insta::assert_snapshot!(
            fail(&udf_graph(&data_url(MODULE), &json!({ "version": "2" }))),
            @r#"node `udf` (run_udf): invalid argument `version`: the wasm runtime speaks version "1" only (InvalidVersion), got "2""#
        );
        insta::assert_snapshot!(
            fail(&udf_graph("udf.py", &json!({}))),
            @"node `udf` (run_udf): invalid argument `udf`: expected `data:application/wasm;base64,…` or an absolute http(s) URL, got `udf.py`"
        );
        insta::assert_snapshot!(
            fail(&udf_graph("data:application/wasm;base64,!!!", &json!({}))),
            @"node `udf` (run_udf): invalid argument `udf`: inline module is not valid base64: Invalid symbol 33, offset 0."
        );
        let mut missing = udf_graph(&data_url(MODULE), &json!({}));
        missing["process_graph"]["udf"]["arguments"]
            .as_object_mut()
            .unwrap()
            .remove("udf");
        insta::assert_snapshot!(
            fail(&missing),
            @"node `udf` (run_udf): missing required argument `udf`"
        );
        // The registrar refusing the bytes: the port's typed reason,
        // naming the node and the parameter.
        let refusing = FakeRegistrar::refusing(UdfError::ForbiddenImport {
            module: "wasi_snapshot_preview1".into(),
            name: "fd_write".into(),
        });
        insta::assert_snapshot!(
            swath_render::compile(&udf_graph(&data_url(MODULE), &json!({})), &ctx(&refusing))
                .expect_err("rejected"),
            @"node `udf` (run_udf): invalid argument `udf`: module rejected at registration: module imports `wasi_snapshot_preview1`.`fd_write`: zero-import rule (ADR 0018)"
        );
        // An arity the IR cannot render.
        let two = FakeRegistrar::planes(2);
        insta::assert_snapshot!(
            swath_render::compile(&udf_graph(&data_url(MODULE), &json!({})), &ctx(&two))
                .expect_err("rejected"),
            @"node `udf` (run_udf): invalid argument `udf`: module declares 2 output planes for 2 input bands; v1 renders 1 (gray) or 3 (RGB)"
        );
    }

    /// One `run_udf` per graph, over a loaded cube: its result feeds
    /// nothing but `linear_scale_range` and `save_result`.
    #[test]
    fn udf_results_are_terminal_in_v1() {
        let registrar = FakeRegistrar::planes(1);
        let fail = |graph: &Json| -> CompileError {
            swath_render::compile(graph, &ctx(&registrar)).expect_err("must not compile")
        };
        // A second run_udf over the first's result.
        let mut chained = udf_graph(&data_url(MODULE), &json!({}));
        chained["process_graph"]["udf2"] = json!({ "process_id": "run_udf", "arguments": {
            "data": { "from_node": "udf" }, "udf": data_url(MODULE), "runtime": "wasm",
        }});
        chained["process_graph"]["scale"]["arguments"]["x"] = json!({ "from_node": "udf2" });
        insta::assert_snapshot!(
            fail(&chained),
            @"node `udf2` (run_udf): type mismatch — expected a loaded (multi-band) data cube — v1 runs one run_udf per graph, after any reduction, got a UDF result data cube"
        );
        // ndvi over a UDF result.
        let mut reduced = udf_graph(&data_url(MODULE), &json!({}));
        reduced["process_graph"]["ndvi"] = json!({ "process_id": "ndvi", "arguments": {
            "data": { "from_node": "udf" },
        }});
        reduced["process_graph"]["scale"]["arguments"]["x"] = json!({ "from_node": "ndvi" });
        insta::assert_snapshot!(
            fail(&reduced),
            @"node `ndvi` (ndvi): type mismatch — expected a loaded (multi-band) data cube — v1 runs one run_udf per graph, after any reduction, got a UDF result data cube"
        );
        // run_udf over an already-reduced (gray) cube.
        let mut over_gray = udf_graph(&data_url(MODULE), &json!({}));
        over_gray["process_graph"]["ndvi"] = json!({ "process_id": "ndvi", "arguments": {
            "data": { "from_node": "load" },
        }});
        over_gray["process_graph"]["udf"]["arguments"]["data"] = json!({ "from_node": "ndvi" });
        insta::assert_snapshot!(
            fail(&over_gray),
            @"node `udf` (run_udf): type mismatch — expected a data cube with a bands dimension, got a gray (reduced) data cube"
        );
        // A colormap on a UDF result.
        let mut mapped = udf_graph(&data_url(MODULE), &json!({}));
        mapped["process_graph"]["save"]["arguments"]["options"] = json!({ "colormap": "viridis" });
        insta::assert_snapshot!(
            fail(&mapped),
            @"node `save` (save_result): invalid argument `options`: a colormap cannot apply to a run_udf result in v1 — UDF output renders directly (1 plane gray, 3 planes RGB)"
        );
    }

    /// The pinned openeo-processes 1.2.0 definition still says what the
    /// compiler assumes: the parameter names, `version`'s null default
    /// (= the runtime's default, which is "1" here), `context` optional,
    /// and the two exceptions the diagnostics mirror.
    #[test]
    fn pinned_run_udf_definition_still_says_what_the_compiler_assumes() {
        let def = process_def("run_udf");
        assert_eq!(
            param_names(&def),
            ["data", "udf", "runtime", "version", "context"]
        );
        assert_eq!(param(&def, "version")["default"], Json::Null);
        assert_eq!(param(&def, "version")["optional"], json!(true));
        assert_eq!(param(&def, "context")["optional"], json!(true));
        let exceptions = def["exceptions"].as_object().expect("exceptions");
        assert!(exceptions.contains_key("InvalidRuntime"));
        assert!(exceptions.contains_key("InvalidVersion"));
        // The spec's `udf` schema offers an absolute URL form: the
        // profile keeps it (plus `data:`), drops workspace paths and
        // inline source code.
        let udf_forms = param(&def, "udf")["schema"].as_array().expect("schemas");
        assert!(udf_forms.iter().any(|s| s["pattern"] == "^https?://"));
    }
}

// --- merge_cubes (ADR 0022): the two-cube join ------------------------------

mod merge_cubes {
    use super::*;
    use swath_render::SourceWindow;

    /// The committed change-detection graph: NDVI(after) − NDVI(before),
    /// two `load_collection` nodes of the same collection with disjoint
    /// month windows, joined by a `subtract` resolver.
    fn change_detection() -> Json {
        graph("change-detection.json")
    }

    #[test]
    fn change_detection_compiles_to_one_band_math_plan_over_two_sources() {
        let product = swath_render::compile(&change_detection(), &hls_ctx()).expect("compiles");
        // Inputs are qualified per source (first-reference order: cube1's
        // branch first) and name the dataset band each reads.
        let inputs: Vec<(&str, Option<&str>)> = product
            .plan
            .inputs
            .iter()
            .map(|i| (i.name.as_str(), i.source.as_deref()))
            .collect();
        assert_eq!(
            inputs,
            [
                ("b8a@after", Some("after")),
                ("b04@after", Some("after")),
                ("b8a@before", Some("before")),
                ("b04@before", Some("before")),
            ]
        );
        let bands: Vec<&str> = product.plan.inputs.iter().map(BandInput::band).collect();
        assert_eq!(bands, ["b8a", "b04", "b8a", "b04"]);
        assert_eq!(
            product.bands,
            ["b8a@after", "b04@after", "b8a@before", "b04@before"]
        );
        // Each source keeps its own resolution window; the product's window
        // is their hull.
        let may = TimeRange {
            start: Some(dt("2024-05-01T00:00:00Z")),
            end: Some(dt("2024-05-31T23:59:59.999Z")),
        };
        let june = TimeRange {
            start: Some(dt("2024-06-01T00:00:00Z")),
            end: Some(dt("2024-06-30T23:59:59.999Z")),
        };
        assert_eq!(
            product.sources,
            [
                SourceWindow {
                    node: "after".into(),
                    window: june,
                },
                SourceWindow {
                    node: "before".into(),
                    window: may,
                },
            ]
        );
        assert_eq!(
            product.window,
            TimeRange {
                start: Some(dt("2024-05-01T00:00:00Z")),
                end: Some(dt("2024-06-30T23:59:59.999Z")),
            }
        );
        // One band-math plan: BandMath → Rescale → Colormap, exactly the
        // single-source shape with qualified inputs.
        insta::assert_json_snapshot!("change_detection_plan_json_shape", product.plan);
    }

    #[test]
    fn filter_temporal_after_the_join_narrows_every_branch() {
        let mut g = change_detection();
        let nodes = g["process_graph"].as_object_mut().expect("nodes");
        nodes.insert(
            "late".into(),
            json!({
                "process_id": "filter_temporal",
                "arguments": {
                    "data": {"from_node": "change"},
                    "extent": ["2024-05-15T00:00:00Z", null]
                }
            }),
        );
        nodes["scale"]["arguments"]["x"] = json!({"from_node": "late"});
        let product = swath_render::compile(&g, &hls_ctx()).expect("compiles");
        assert_eq!(
            product.sources[0].window.start,
            Some(dt("2024-06-01T00:00:00Z"))
        );
        assert_eq!(
            product.sources[1].window.start,
            Some(dt("2024-05-15T00:00:00Z"))
        );
    }

    #[test]
    fn single_source_graphs_keep_unqualified_inputs_and_one_source() {
        let product =
            swath_render::compile(&graph("ndvi-convenience.json"), &hls_ctx()).expect("compiles");
        assert!(product.plan.inputs.iter().all(|i| i.source.is_none()));
        assert_eq!(product.sources.len(), 1);
        assert_eq!(product.sources[0].node, "load");
        assert_eq!(product.sources[0].window, product.window);
        // And the persisted plan JSON has no `source` key at all.
        let json = serde_json::to_string(&product.plan).expect("serializes");
        assert!(!json.contains("source"), "{json}");
    }

    fn err(g: &Json) -> CompileError {
        swath_render::compile(g, &hls_ctx()).expect_err("must not compile")
    }

    #[test]
    fn rejections_are_pinned() {
        // multi × multi: the load nodes joined directly.
        let mut g = change_detection();
        g["process_graph"]["change"]["arguments"]["cube1"] = json!({"from_node": "after"});
        g["process_graph"]["change"]["arguments"]["cube2"] = json!({"from_node": "before"});
        insta::assert_snapshot!(
            err(&g),
            @"node `change` (merge_cubes): invalid argument `cube1`: expected a gray (reduced) data cube, got a data cube with 2 bands — reduce to one value per pixel first (ndvi or reduce_dimension)"
        );
        // A scaled input: linear_scale_range before the join.
        let mut g = change_detection();
        let nodes = g["process_graph"].as_object_mut().expect("nodes");
        nodes.insert(
            "early".into(),
            json!({
                "process_id": "linear_scale_range",
                "arguments": {
                    "x": {"from_node": "ndvi_before"},
                    "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255
                }
            }),
        );
        nodes["change"]["arguments"]["cube2"] = json!({"from_node": "early"});
        insta::assert_snapshot!(
            err(&g),
            @"node `change` (merge_cubes): invalid argument `cube2`: expected an unscaled data cube, got an already-scaled one — apply linear_scale_range to the merged result instead"
        );
        // No resolver.
        let mut g = change_detection();
        g["process_graph"]["change"]["arguments"]
            .as_object_mut()
            .expect("arguments")
            .remove("overlap_resolver");
        insta::assert_snapshot!(
            err(&g),
            @"node `change` (merge_cubes): overlap_resolver is required — a child graph over x (from cube1) and y (from cube2) producing one value per pixel pair, e.g. subtract"
        );
        // A resolver whose result is a cube, not a scalar per pixel pair.
        let mut g = change_detection();
        g["process_graph"]["change"]["arguments"]["overlap_resolver"] = json!({
            "process_graph": {
                "again": {
                    "process_id": "load_collection",
                    "arguments": {"id": "hls-s30", "bands": ["red"]},
                    "result": true
                }
            }
        });
        insta::assert_snapshot!(
            err(&g),
            @"node `change` (merge_cubes): type mismatch — expected an overlap_resolver producing a scalar per pixel pair, got a multi-band data cube"
        );
        // Both branches through one load node: the frames would be
        // indistinguishable.
        let mut g = change_detection();
        g["process_graph"]["ndvi_after"]["arguments"]["data"] = json!({"from_node": "before"});
        insta::assert_snapshot!(
            err(&g),
            @"node `change` (merge_cubes): invalid argument `cube2`: both cubes load the collection through node `before` — load it once per frame (a second load_collection with its own temporal_extent) so each branch resolves its own granule"
        );
        // A band label carrying the source qualifier can never be told
        // apart from a qualified name.
        let mut g = change_detection();
        g["process_graph"]["after"]["arguments"]["bands"] = json!(["red", "nir@after"]);
        insta::assert_snapshot!(
            err(&g),
            @"node `after` (load_collection): invalid argument `bands`: band name `nir@after` contains `@`, the source qualifier"
        );
        // `context` is not admitted.
        let mut g = change_detection();
        g["process_graph"]["change"]["arguments"]["context"] = json!({"k": 1});
        insta::assert_snapshot!(
            err(&g),
            @"node `change` (merge_cubes): invalid argument `context`: not admitted in the bounded profile: the resolver sees only x and y"
        );
    }
}
