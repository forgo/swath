// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! openEO conformance smoke tests (issue #41, ADR 0010, the #27 pattern):
//! every JSON document the openEO surface serves is validated against the
//! response schemas of the pinned official openEO API 1.2.0 spec
//! (`tests/data/openeo/`, provenance in its README), and the profile's
//! honesty rules — endpoints list only what exists, no billing, no
//! over-claimed conformance, no `swath:` internals leaking — are asserted
//! directly.

#[allow(
    dead_code,
    reason = "shared between the API test targets; not every helper is used in each"
)]
mod common;

use axum::http::StatusCode;

async fn json_ok(app: &axum::Router, path: &str) -> serde_json::Value {
    let response = common::request_on(app, "GET", path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    common::body_json(response).await
}

// --- Capabilities (GET / doubles as the OGC landing page) ---

#[tokio::test]
async fn capabilities_are_valid_under_both_standards_and_list_only_what_exists() {
    let (app, _) = common::openeo_app();
    let capabilities = json_ok(&app, "/").await;

    // openEO: the capabilities response schema of the pinned spec.
    let schema = common::openeo_response_schema("/", "get", "200");
    common::assert_openeo_valid(&schema, "openEO capabilities (GET /)", &capabilities);
    // OGC: the same document is still a schema-valid landing page.
    common::assert_valid("common/landingPage.json", &capabilities);

    assert_eq!(capabilities["api_version"], "1.2.0");
    assert_eq!(capabilities["type"], "Catalog");
    assert_eq!(capabilities["production"], false);
    // Billing is deliberately absent (no paid plans exist — omitting the
    // optional object is the honest declaration).
    assert!(capabilities.get("billing").is_none());

    // The endpoints array lists exactly the implemented surface.
    let endpoints = capabilities["endpoints"].as_array().expect("endpoints");
    let listed: Vec<(String, Vec<String>)> = endpoints
        .iter()
        .map(|endpoint| {
            (
                endpoint["path"].as_str().expect("path").to_owned(),
                endpoint["methods"]
                    .as_array()
                    .expect("methods")
                    .iter()
                    .map(|m| m.as_str().expect("method").to_owned())
                    .collect(),
            )
        })
        .collect();
    let expected: Vec<(String, Vec<String>)> = swath_api::openeo::OPENEO_ENDPOINTS
        .iter()
        .map(|(path, methods)| {
            (
                (*path).to_owned(),
                methods.iter().map(|m| (*m).to_owned()).collect(),
            )
        })
        .collect();
    assert_eq!(listed, expected);
    // Honesty: nothing auth-, job-, or file-shaped is claimed.
    for (path, _) in &listed {
        assert!(
            !path.contains("credentials") && !path.contains("jobs") && !path.contains("files"),
            "unimplemented endpoint claimed: {path}"
        );
    }
    // POST /result exists (ADR 0014) — with its preview-grade narrowing
    // declared honestly in the capabilities description, and no general
    // sync-processing claim anywhere else.
    assert!(
        listed
            .iter()
            .any(|(path, methods)| path == "/result" && methods == &["POST"]),
        "POST /result must be listed"
    );
    let description = capabilities["description"].as_str().expect("description");
    assert!(
        description.contains("preview-bounded")
            && description.contains("not general synchronous processing")
            && description.contains("ProcessGraphComplexity"),
        "the capabilities description must state the POST /result narrowing: {description}"
    );

    // The OGC conformance declaration is untouched by the openEO merge:
    // only the Tiles classes actually met, no openEO class over-claimed.
    let conformance = json_ok(&app, "/conformance").await;
    common::assert_valid("common/confClasses.json", &conformance);
    let declared: Vec<&str> = conformance["conformsTo"]
        .as_array()
        .expect("conformsTo")
        .iter()
        .map(|class| class.as_str().expect("class"))
        .collect();
    assert_eq!(declared, swath_api::CONFORMANCE_CLASSES);
}

#[tokio::test]
async fn well_known_discovery_points_at_this_instance() {
    let (app, _) = common::openeo_app();
    let discovery = json_ok(&app, "/.well-known/openeo").await;
    let schema = common::openeo_response_schema("/.well-known/openeo", "get", "200");
    common::assert_openeo_valid(&schema, "well-known discovery", &discovery);
    assert_eq!(
        discovery["versions"],
        serde_json::json!([{
            "url": "http://localhost/",
            "api_version": "1.2.0",
            "production": false,
        }])
    );
}

/// Fixtures/static mode serves no openEO surface — and its landing page
/// stays the plain OGC document (no capabilities vocabulary).
#[tokio::test]
async fn static_mode_serves_no_openeo_surface() {
    for path in [
        "/.well-known/openeo",
        "/collections",
        "/processes",
        "/service_types",
        "/services",
    ] {
        let response = common::get(path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "GET {path}");
    }
    let landing = common::body_json(common::get("/").await).await;
    assert!(landing.get("api_version").is_none());
    assert!(landing.get("endpoints").is_none());
}

// --- Collections (catalog Datasets through the STAC converters) ---

#[tokio::test]
async fn collections_are_schema_valid_and_leak_no_swath_internals() {
    let (app, _) = common::openeo_app();
    let list = json_ok(&app, "/collections").await;
    let schema = common::openeo_response_schema("/collections", "get", "200");
    common::assert_openeo_valid(&schema, "collections list", &list);

    let collections = list["collections"].as_array().expect("collections");
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0]["id"], "hls-s30");

    let collection = json_ok(&app, "/collections/hls-s30").await;
    let schema = common::openeo_response_schema("/collections/{collection_id}", "get", "200");
    common::assert_openeo_valid(&schema, "collection", &collection);
    // The list entry and the full document agree (one representation).
    assert_eq!(&collection, &collections[0]);

    // Datacube view: the band vocabulary and CRS84 extent of the dataset.
    assert_eq!(
        collection["cube:dimensions"]["bands"]["values"],
        serde_json::json!(["b02", "b03", "b04", "b8a"])
    );
    assert_eq!(collection["cube:dimensions"]["x"]["axis"], "x");

    // R2 honesty: swath-internal fields stay behind the converters.
    let keys: Vec<&String> = collection.as_object().expect("object").keys().collect();
    assert!(
        keys.iter().all(|key| !key.starts_with("swath:")),
        "swath-internal fields leaked into the openEO collection: {keys:?}"
    );

    // Unknown collection: the standardized error shape and code.
    let response = common::request_on(&app, "GET", "/collections/nope", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error = common::body_json(response).await;
    let schema = common::openeo_schema("/components/schemas/error");
    common::assert_openeo_valid(&schema, "error", &error);
    assert_eq!(error["code"], "CollectionNotFound");
}

// --- Processes (the compiler subset, pinned definitions) ---

#[tokio::test]
async fn processes_are_schema_valid_and_exactly_the_compiler_subset() {
    let (app, _) = common::openeo_app();
    let list = json_ok(&app, "/processes").await;
    let schema = common::openeo_response_schema("/processes", "get", "200");
    common::assert_openeo_valid(&schema, "processes list", &list);

    let processes = list["processes"].as_array().expect("processes");
    let ids: Vec<&str> = processes
        .iter()
        .map(|process| process["id"].as_str().expect("id"))
        .collect();
    // Exactly the subset the compiler's conformance statement declares
    // (swath_render::process module docs), alphabetical — minus
    // `run_udf`, offered only where a UDF executor and module store are
    // wired (openeo_udf.rs covers the wired list).
    assert_eq!(
        ids,
        [
            "add",
            "array_element",
            "divide",
            "filter_temporal",
            "linear_scale_range",
            "load_collection",
            "merge_cubes",
            "multiply",
            "ndvi",
            "reduce_dimension",
            "save_result",
            "subtract",
        ]
    );
    // Every served definition declares its narrowing honestly.
    for process in processes {
        let description = process["description"].as_str().expect("description");
        assert!(
            description.contains("**Swath profile:**"),
            "{} lacks its profile note",
            process["id"]
        );
    }
    // The join (ADR 0022) states its narrowing, and the scalar processes
    // say where they are admitted: a reducer or a resolver.
    let note = |id: &str| -> String {
        processes
            .iter()
            .find(|p| p["id"] == id)
            .and_then(|p| p["description"].as_str())
            .map(|d| {
                d.rsplit("**Swath profile:**")
                    .next()
                    .unwrap_or("")
                    .to_owned()
            })
            .expect("served process")
    };
    let merge = note("merge_cubes");
    for phrase in [
        "gray",
        "two different `load_collection` nodes",
        "`overlap_resolver` is required",
        "`context` is not accepted",
    ] {
        assert!(
            merge.contains(phrase),
            "merge_cubes note lacks {phrase:?}: {merge}"
        );
    }
    for id in ["add", "subtract", "multiply", "divide"] {
        let note = note(id);
        assert!(
            note.contains("reducer") && note.contains("overlap_resolver"),
            "{id} note must admit both a reducer and a resolver: {note}"
        );
    }
}

/// The runtime copies `GET /processes` serves are byte-identical to the
/// compiler's pinned oracle copies — one re-pin must update both.
#[test]
fn runtime_process_definitions_match_the_pinned_oracle_copies() {
    let runtime =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/openeo-processes");
    let oracle = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../swath-render/tests/data/openeo");
    let mut checked = 0;
    for entry in std::fs::read_dir(&runtime).expect("runtime dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let name = path.file_name().expect("file name");
        let ours = std::fs::read(&path).expect("runtime copy reads");
        let pinned = std::fs::read(oracle.join(name)).expect("oracle copy reads");
        assert_eq!(
            ours,
            pinned,
            "{} diverged from the pinned oracle copy",
            name.to_string_lossy()
        );
        checked += 1;
    }
    assert_eq!(checked, 13, "all thirteen pinned definitions are mirrored");
}

// --- Service types ---

#[tokio::test]
async fn service_types_declare_xyz_with_its_configuration_schema() {
    let (app, _) = common::openeo_app();
    let types = json_ok(&app, "/service_types").await;
    let schema = common::openeo_response_schema("/service_types", "get", "200");
    common::assert_openeo_valid(&schema, "service types", &types);

    let xyz = types.get("xyz").expect("xyz service type");
    assert_eq!(xyz["configuration"]["tile_size"]["default"], 256);
    assert_eq!(xyz["process_parameters"], serde_json::json!([]));
    // The single supported type, nothing over-claimed.
    assert_eq!(types.as_object().expect("object").len(), 1);
}

// --- Services (list/describe shapes; the loop lives in openeo_services.rs) ---

#[tokio::test]
async fn services_list_and_description_are_schema_valid() {
    let (app, _) = common::openeo_app();
    let list = json_ok(&app, "/services").await;
    let schema = common::openeo_response_schema("/services", "get", "200");
    common::assert_openeo_valid(&schema, "services list (empty)", &list);
    assert_eq!(list["services"], serde_json::json!([]));

    // Publish one service, then hold both representations to the spec.
    let graph = serde_json::json!({
        "type": "xyz",
        "title": "NDVI",
        "process": { "process_graph": {
            "load": { "process_id": "load_collection", "arguments": {
                "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
                "bands": ["b8a", "b04"],
            }},
            "ndvi": { "process_id": "ndvi", "arguments": {
                "data": { "from_node": "load" }, "nir": "b8a", "red": "b04",
            }},
            "save": { "process_id": "save_result", "arguments": {
                "data": { "from_node": "ndvi" }, "format": "png",
            }, "result": true },
        }},
    });
    let response = common::request_on(&app, "POST", "/services", Some(graph)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let id = response.headers()["openeo-identifier"]
        .to_str()
        .expect("identifier header")
        .to_owned();

    let list = json_ok(&app, "/services").await;
    let schema = common::openeo_response_schema("/services", "get", "200");
    common::assert_openeo_valid(&schema, "services list", &list);
    assert_eq!(list["services"][0]["id"], serde_json::json!(id));

    let service = json_ok(&app, &format!("/services/{id}")).await;
    let schema = common::openeo_response_schema("/services/{service_id}", "get", "200");
    common::assert_openeo_valid(&schema, "service description", &service);
    assert_eq!(service["type"], "xyz");
    assert_eq!(service["enabled"], true);
    assert!(service["process"]["process_graph"].is_object());
    assert_eq!(service["configuration"]["tile_size"], 256);
    assert_eq!(
        service["url"],
        serde_json::json!(format!(
            "http://localhost/tilesets/{id}/tiles/{{z}}/{{y}}/{{x}}"
        ))
    );
}

// --- Error registry honesty ---

/// Every openEO error code this surface emits exists in the pinned spec
/// registry (`errors.json`) with a matching HTTP status.
#[test]
fn emitted_error_codes_exist_in_the_pinned_registry_with_matching_status() {
    let raw = std::fs::read_to_string(common::openeo_spec_dir().join("errors.json"))
        .expect("pinned errors.json");
    let registry: serde_json::Value = serde_json::from_str(&raw).expect("errors.json parses");

    for (code, status) in [
        ("Internal", 500),
        ("CollectionNotFound", 404),
        ("ServiceNotFound", 404),
        ("ServiceUnsupported", 400),
        ("ServiceConfigInvalid", 400),
        ("ServiceConfigUnsupported", 400),
        ("FeatureUnsupported", 501),
        ("ProcessGraphMissing", 400),
        ("ProcessGraphInvalid", 400),
        ("ProcessUnsupported", 400),
        ("ProcessParameterRequired", 400),
        ("ProcessParameterInvalid", 400),
        // The preview surface (ADR 0014, POST /result).
        ("ProcessGraphComplexity", 400),
        ("NotFound", 404),
    ] {
        let entry = registry
            .get(code)
            .unwrap_or_else(|| panic!("`{code}` is not a standardized openEO error code"));
        assert_eq!(
            entry["http"], status,
            "`{code}` is emitted with status {status} but the registry says {}",
            entry["http"]
        );
    }
}
