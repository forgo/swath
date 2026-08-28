// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `run_udf` through the openEO surface, end to end (ADR 0018, #204):
//! a graph naming a WASM module — inline or by URL — publishes as a
//! service whose persisted plan is `PlanKind::Udf { code_hash }`, the
//! module bytes land in the content-addressed store, a remote URL is
//! fetched exactly once, and rehydration resolves the hash from the
//! store without ever fetching again — a mutated remote cannot change a
//! published service. Every failure path answers a registry code naming
//! the node and the parameter.
//!
//! Serving the tile itself (the executor behind `render_tile`) is #205.

#[allow(
    dead_code,
    reason = "shared between the API test targets; not every helper is used in each"
)]
mod common;

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use base64::Engine as _;
use serde_json::{Value, json};
use swath_core::catalog::PlanKind;
use swath_core::udf::{MODULE_MAX_BYTES, ModuleFetchError, ModuleFetcher, ModuleStore, code_hash};
use swath_render::ir::PixelOp;
use swath_render::udf::UdfStage;

/// The committed NDVI fixture module (`examples/udf/ndvi`): 2 planes in,
/// 1 out.
const NDVI: &[u8] = include_bytes!("../../adapters/swath-udf-wasmtime/tests/fixtures/ndvi.wasm");
/// A different valid module — what a mutated remote would serve.
const HILLSHADE: &[u8] =
    include_bytes!("../../adapters/swath-udf-wasmtime/tests/fixtures/hillshade.wasm");

const REMOTE: &str = "https://udf.example.org/ndvi.wasm";

/// A fetcher double: serves whatever bytes it currently holds under one
/// URL, counts every call, and can be mutated under a published service.
#[derive(Clone, Default)]
struct CountingFetcher {
    served: Arc<Mutex<Option<Vec<u8>>>>,
    calls: Arc<Mutex<u32>>,
}

impl CountingFetcher {
    fn serving(bytes: &[u8]) -> Self {
        Self {
            served: Arc::new(Mutex::new(Some(bytes.to_vec()))),
            calls: Arc::new(Mutex::new(0)),
        }
    }

    fn calls(&self) -> u32 {
        *self.calls.lock().unwrap()
    }

    fn mutate(&self, bytes: &[u8]) {
        *self.served.lock().unwrap() = Some(bytes.to_vec());
    }
}

impl ModuleFetcher for CountingFetcher {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, ModuleFetchError> {
        *self.calls.lock().unwrap() += 1;
        if url != REMOTE {
            return Err(ModuleFetchError::NotFound {
                url: url.to_owned(),
            });
        }
        self.served
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| ModuleFetchError::NotFound {
                url: url.to_owned(),
            })
    }
}

fn data_url(bytes: &[u8]) -> String {
    format!(
        "data:application/wasm;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// load(b8a, b04) → `run_udf` → scale(-1..1) → save: the NDVI UDF product.
fn udf_process(udf: &str, bands: &[&str]) -> Value {
    json!({ "process_graph": {
        "load": { "process_id": "load_collection", "arguments": {
            "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
            "bands": bands,
        }},
        "udf": { "process_id": "run_udf", "arguments": {
            "data": { "from_node": "load" },
            "udf": udf,
            "runtime": "wasm",
            "version": "1",
            "context": { "note": "passes through" },
        }},
        "scale": { "process_id": "linear_scale_range", "arguments": {
            "x": { "from_node": "udf" },
            "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255,
        }},
        "save": { "process_id": "save_result", "arguments": {
            "data": { "from_node": "scale" }, "format": "png",
        }, "result": true },
    }})
}

/// The operator budget rehydration compiles under (#272): the default,
/// as the test app publishes with.
fn budget() -> swath_core::planner::Budget {
    swath_core::planner::Budget::default()
}

fn service_request(process: &Value) -> Value {
    json!({ "type": "xyz", "title": "NDVI (UDF)", "process": process })
}

async fn publish(app: &axum::Router, process: Value) -> (StatusCode, String) {
    let response =
        common::request_on(app, "POST", "/services", Some(service_request(&process))).await;
    let status = response.status();
    let id = response
        .headers()
        .get("openeo-identifier")
        .map(|v| v.to_str().expect("ascii").to_owned())
        .unwrap_or_default();
    (status, id)
}

/// The plan the compiler must produce for the NDVI UDF product.
fn expected_ops() -> [PixelOp; 2] {
    [
        PixelOp::Udf(UdfStage::new(
            code_hash(NDVI),
            1,
            json!({ "note": "passes through" }),
        )),
        PixelOp::Rescale {
            min: -1.0,
            max: 1.0,
        },
    ]
}

#[tokio::test]
async fn inline_module_publishes_persists_by_hash_and_rehydrates() {
    let udf_app = common::openeo_app_with_udf(CountingFetcher::default());
    let (status, id) = publish(&udf_app.app, udf_process(&data_url(NDVI), &["b8a", "b04"])).await;
    assert_eq!(status, StatusCode::CREATED);

    // Persisted: the layer names the module by content hash only.
    let dataset = udf_app.catalog.stored_dataset("hls-s30").expect("dataset");
    let layer = dataset
        .layers
        .iter()
        .find(|layer| layer.id == id)
        .expect("service layer persisted");
    assert_eq!(
        layer.plan,
        PlanKind::Udf {
            code_hash: code_hash(NDVI)
        }
    );
    assert!(layer.process.is_some(), "the graph is carried verbatim");
    // The module bytes are in the store under that hash.
    assert_eq!(
        udf_app.store.get(&code_hash(NDVI)).await.expect("store"),
        Some(NDVI.to_vec())
    );
    // The service description shows the graph (with its data: URL) as
    // submitted.
    let response = common::request_on(&udf_app.app, "GET", &format!("/services/{id}"), None).await;
    let doc = common::body_json(response).await;
    assert_eq!(
        doc["process"]["process_graph"]["udf"]["process_id"],
        "run_udf"
    );

    // Rehydration (what a restart does): the persisted layer recompiles
    // through the store-backed resolution to the same plan.
    let modules = udf_app.publish.rehydrate(layer).await.expect("rehydrates");
    let template = swath_api::compile_service_layer(&dataset, layer, Some(&modules), &budget())
        .expect("recompiles");
    assert_eq!(template.plan.ops, expected_ops());
    let inputs: Vec<&str> = template
        .plan
        .inputs
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(inputs, ["b8a", "b04"]);
    // The unwired path cannot serve it — loudly.
    let err =
        swath_api::compile_service_layer(&dataset, layer, None, &budget()).expect_err("unwired");
    assert!(matches!(
        err,
        swath_render::CompileError::UdfUnavailable { .. }
    ));
    // The fetcher was never consulted for an inline module.
    // (CountingFetcher::default serves nothing; a call would have 404'd.)
}

#[tokio::test]
async fn remote_module_is_fetched_once_and_a_mutated_remote_cannot_change_the_service() {
    let fetcher = CountingFetcher::serving(NDVI);
    let udf_app = common::openeo_app_with_udf(fetcher.clone());
    let (status, id) = publish(&udf_app.app, udf_process(REMOTE, &["b8a", "b04"])).await;
    assert_eq!(status, StatusCode::CREATED);
    // One compile motion (which compiles the graph twice: validate, then
    // lower the persisted form) = exactly one fetch.
    assert_eq!(fetcher.calls(), 1);
    let dataset = udf_app.catalog.stored_dataset("hls-s30").expect("dataset");
    let layer = dataset
        .layers
        .iter()
        .find(|layer| layer.id == id)
        .expect("service layer persisted");
    assert_eq!(
        layer.plan,
        PlanKind::Udf {
            code_hash: code_hash(NDVI)
        }
    );
    assert_eq!(
        udf_app.store.get(&code_hash(NDVI)).await.expect("store"),
        Some(NDVI.to_vec())
    );

    // The remote changes under the published service.
    fetcher.mutate(HILLSHADE);
    // Rehydration resolves the persisted hash from the store: the same
    // module, the same plan, and no fetch.
    let modules = udf_app.publish.rehydrate(layer).await.expect("rehydrates");
    let template = swath_api::compile_service_layer(&dataset, layer, Some(&modules), &budget())
        .expect("recompiles");
    assert_eq!(template.plan.ops, expected_ops());
    assert_eq!(fetcher.calls(), 1, "rehydration never fetches");

    // A NEW publish of the same graph is a new compile motion: it fetches
    // (once more) and sees the mutated remote — and hillshade wants one
    // plane, so it is refused as a bad `udf` parameter rather than
    // silently replacing the service.
    let response = common::request_on(
        &udf_app.app,
        "POST",
        "/services",
        Some(service_request(&udf_process(REMOTE, &["b8a", "b04"]))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(fetcher.calls(), 2);
    let error = common::body_json(response).await;
    assert_eq!(error["code"], "ProcessParameterInvalid");
    // The published service is untouched.
    let dataset = udf_app.catalog.stored_dataset("hls-s30").expect("dataset");
    let layer = dataset
        .layers
        .iter()
        .find(|l| l.id == id)
        .expect("still there");
    assert_eq!(
        layer.plan,
        PlanKind::Udf {
            code_hash: code_hash(NDVI)
        }
    );
}

/// A missing module in the store is a loud rehydration failure, never a
/// fetch.
#[tokio::test]
async fn rehydration_without_the_stored_module_is_loud_and_never_fetches() {
    let fetcher = CountingFetcher::serving(NDVI);
    let udf_app = common::openeo_app_with_udf(fetcher.clone());
    let graph = udf_process(REMOTE, &["b8a", "b04"]);
    let layer = swath_core::catalog::Layer {
        id: "xyz-orphan".into(),
        title: "orphan".into(),
        description: String::new(),
        plan: PlanKind::Udf {
            code_hash: code_hash(b"never stored"),
        },
        rescale: swath_core::catalog::Rescale {
            min: -1.0,
            max: 1.0,
        },
        colormap: None,
        resampling: swath_core::catalog::Resampling::Bilinear,
        tile_size: 256,
        process: Some(graph),
    };
    let err = udf_app
        .publish
        .rehydrate(&layer)
        .await
        .expect_err("missing");
    assert_eq!(
        err,
        swath_api::RehydrateError::ModuleMissing {
            code_hash: code_hash(b"never stored")
        }
    );
    assert_eq!(fetcher.calls(), 0);
}

/// Every failure path in the standard's vocabulary: registry codes only,
/// the module problems naming the `run_udf` node and the `udf` parameter.
#[tokio::test]
async fn error_paths_name_the_node_and_parameter_with_registry_codes() {
    let fetcher = CountingFetcher::serving(NDVI);
    let udf_app = common::openeo_app_with_udf(fetcher.clone());
    let error_schema = common::openeo_schema("/components/schemas/error");
    let shape = |process: Value| {
        let app = udf_app.app.clone();
        async move {
            let response =
                common::request_on(&app, "POST", "/services", Some(service_request(&process)))
                    .await;
            let status = response.status().as_u16();
            let error = common::body_json(response).await;
            json!({ "status": status, "error": error })
        }
    };

    let garbage = shape(udf_process(
        &data_url(b"not a wasm module"),
        &["b8a", "b04"],
    ))
    .await;
    let wrong_arity = shape(udf_process(&data_url(NDVI), &["b8a", "b04", "b02"])).await;
    let bad_runtime = {
        let mut process = udf_process(&data_url(NDVI), &["b8a", "b04"]);
        process["process_graph"]["udf"]["arguments"]["runtime"] = json!("Python");
        shape(process).await
    };
    let bad_form = shape(udf_process("udf.py", &["b8a", "b04"])).await;
    let unreachable = shape(udf_process(
        "https://udf.example.org/missing.wasm",
        &["b8a", "b04"],
    ))
    .await;
    let oversized = {
        let over = "A".repeat(MODULE_MAX_BYTES.div_ceil(3) * 4 + 4);
        shape(udf_process(
            &format!("data:application/wasm;base64,{over}"),
            &["b8a", "b04"],
        ))
        .await
    };
    // Unwired deployment: the process simply is not offered.
    let (plain, _) = common::openeo_app();
    let unwired = {
        let response = common::request_on(
            &plain,
            "POST",
            "/services",
            Some(service_request(&udf_process(
                &data_url(NDVI),
                &["b8a", "b04"],
            ))),
        )
        .await;
        let status = response.status().as_u16();
        let error = common::body_json(response).await;
        json!({ "status": status, "error": error })
    };

    let shapes = json!({
        "module does not compile": garbage,
        "module refuses the input arity": wrong_arity,
        "unsupported runtime": bad_runtime,
        "udf argument in no supported form": bad_form,
        "remote module unreachable": unreachable,
        "inline module over the size limit": oversized,
        "run_udf on a deployment without UDF wiring": unwired,
    });
    for (name, shape) in shapes.as_object().expect("object") {
        common::assert_openeo_valid(&error_schema, name, &shape["error"]);
    }
    insta::assert_json_snapshot!("openeo_udf_error_shapes", shapes);
    // Nothing was published or persisted by any failure.
    let dataset = udf_app.catalog.stored_dataset("hls-s30").expect("dataset");
    assert!(dataset.layers.iter().all(|layer| layer.process.is_none()));
    assert_eq!(
        udf_app.store.get(&code_hash(NDVI)).await.expect("store"),
        None
    );
}

/// `GET /processes` offers `run_udf` exactly where it is wired, with the
/// profile's narrowing note; the unwired list is unchanged.
#[tokio::test]
async fn processes_list_run_udf_only_where_wired() {
    let udf_app = common::openeo_app_with_udf(CountingFetcher::default());
    let response = common::request_on(&udf_app.app, "GET", "/processes", None).await;
    let list = common::body_json(response).await;
    let schema = common::openeo_response_schema("/processes", "get", "200");
    common::assert_openeo_valid(&schema, "processes list (UDF wired)", &list);
    let processes = list["processes"].as_array().expect("processes");
    let run_udf = processes
        .iter()
        .find(|p| p["id"] == "run_udf")
        .expect("run_udf listed where wired");
    let description = run_udf["description"].as_str().expect("description");
    assert!(description.contains("**Swath profile:**"), "{description}");
    assert!(
        description.contains("data:application/wasm;base64"),
        "{description}"
    );
    assert!(description.contains("content hash"), "{description}");
    assert_eq!(processes.len(), 13);

    let (plain, _) = common::openeo_app();
    let response = common::request_on(&plain, "GET", "/processes", None).await;
    let list = common::body_json(response).await;
    assert!(
        list["processes"]
            .as_array()
            .expect("processes")
            .iter()
            .all(|p| p["id"] != "run_udf"),
        "unwired deployments do not offer run_udf"
    );
}
