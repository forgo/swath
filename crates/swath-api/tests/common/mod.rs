// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// `mod common` is compiled once per test binary and each binary uses a
// subset of it — one allow here instead of one per binary (#348).
#![allow(
    dead_code,
    reason = "compiled once per test binary; each uses a subset"
)]

//! Shared plumbing for the API tests: the fixture-wired app (COG source +
//! proj4rs over the committed HLS fixtures), an in-process request
//! helper (`tower::ServiceExt::oneshot` — no network), and the OGC
//! schema validator over the committed official schemas.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Response;
use jsonschema::{Retrieve, Uri, Validator};
use object_store::local::LocalFileSystem;
use swath_api::{ApiState, LayerRegistry, router};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;

pub(crate) mod wasm;

// The shared plumbing (#348): one in-memory catalog, the committed fixtures
// in catalog form, the test-data paths and the in-process request helpers
// live in `swath-testsupport`; this module keeps only what builds the API
// itself (the routers, the states, the schema validators). `mod common` is
// compiled once per test binary and each uses a subset, hence the allow.
#[allow(
    unused_imports,
    reason = "shared between the API test binaries; not every one uses each"
)]
pub(crate) use swath_testsupport::{
    catalog::MemoryCatalog,
    fixtures::{hls_catalog_dataset, hls_catalog_granule, park_fire},
    http::{body_bytes, body_json, request_on},
    paths::{fixtures_dir, render_goldens_dir},
};
/// Base URL the test app mints links under.
pub(crate) const BASE_URL: &str = "http://localhost";

/// The committed reference NDVI module (`examples/udf/ndvi`, the #205
/// dual-implementation golden): 2 planes in, 1 out.
pub(crate) const NDVI_WASM: &[u8] =
    include_bytes!("../../../adapters/swath-udf-wasmtime/tests/fixtures/ndvi.wasm");

/// A module fetcher serving nothing: for suites whose every module is
/// inline.
#[derive(Clone, Default)]
pub(crate) struct NoFetch;

impl swath_core::udf::ModuleFetcher for NoFetch {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, swath_core::udf::ModuleFetchError> {
        Err(swath_core::udf::ModuleFetchError::NotFound {
            url: url.to_owned(),
        })
    }
}

/// `bytes` as the inline `data:application/wasm;base64,…` a `run_udf`
/// node's `udf` argument accepts.
pub(crate) fn wasm_data_url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "data:application/wasm;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The committed official OGC schemas (tests/data/ogc/README.md).
pub(crate) fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/ogc")
}

/// The API over the fixture registry, wired to the concrete Phase-1
/// adapters — the same wiring the binary (#29) will do.
pub(crate) fn app() -> Router {
    let store = LocalFileSystem::new_with_prefix(fixtures_dir()).expect("fixture dir exists");
    let state = ApiState::new(
        LayerRegistry::hls_fixtures(),
        CogSource::new(Arc::new(store)),
        Proj4rsReproject,
        BASE_URL,
    );
    router(Arc::new(state))
}

/// One in-process GET; returns the full response (status, headers,
/// extensions) with the body still unread.
pub(crate) async fn get(path: &str) -> Response<Body> {
    get_with_accept(path, None).await
}

/// One in-process GET with an optional `Accept` header.
pub(crate) async fn get_with_accept(path: &str, accept: Option<&str>) -> Response<Body> {
    swath_testsupport::http::get_with_accept(&app(), path, accept).await
}

/// Resolves external `$ref`s against the committed schema files: every
/// reference in the OGC schemas is a relative sibling file name, so the
/// URI's last path segment names the file (looked up in `tms/`, then
/// `common/`).
struct CommittedSchemas;

impl Retrieve for CommittedSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .path()
            .as_str()
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("no file name in $ref URI `{uri}`"))?;
        let dir = schemas_dir();
        let path = [dir.join("tms").join(name), dir.join("common").join(name)]
            .into_iter()
            .find(|p| p.exists())
            .ok_or_else(|| format!("`{name}` is not a committed OGC schema (from `{uri}`)"))?;
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }
}

/// Compiles a committed OGC schema (path relative to `tests/data/ogc/`,
/// e.g. `"tms/tileSet.json"`).
pub(crate) fn schema(relative: &str) -> Validator {
    let raw = std::fs::read_to_string(schemas_dir().join(relative)).expect("schema file exists");
    let schema: serde_json::Value = serde_json::from_str(&raw).expect("schema parses");
    jsonschema::options()
        .with_retriever(CommittedSchemas)
        .build(&schema)
        .expect("schema compiles")
}

/// Asserts `instance` is valid under the committed schema, with a
/// readable failure listing every violation.
pub(crate) fn assert_valid(relative: &str, instance: &serde_json::Value) {
    let validator = schema(relative);
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|err| format!("  {} at {}", err, err.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "instance violates {relative}:\n{}\ninstance: {}",
        errors.join("\n"),
        serde_json::to_string_pretty(instance).expect("instance pretty-prints"),
    );
}

// --- openEO test plumbing (issue #41, ADR 0010) ---

use swath_core::catalog::{Dataset, DatasetId, Granule, TimeRange};

/// The pinned openEO API 1.2.0 spec (tests/data/openeo/README.md).
pub(crate) fn openeo_spec_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/openeo")
}

/// The pinned openEO `openapi.json`, parsed once — with `OpenAPI` 3.0's
/// `nullable: true` mechanically translated into JSON Schema's
/// null-in-type (`"type": [T, "null"]`, or `anyOf` around a bare `$ref`),
/// since a JSON Schema validator would otherwise ignore the keyword and
/// reject the nulls the spec explicitly allows. A translation, never a
/// loosening: only schemas the spec itself marks nullable admit null.
pub(crate) fn openeo_spec() -> &'static serde_json::Value {
    static SPEC: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    SPEC.get_or_init(|| {
        let raw = std::fs::read_to_string(openeo_spec_dir().join("openapi.json"))
            .expect("pinned openEO openapi.json exists");
        let mut spec = serde_json::from_str(&raw).expect("openapi.json parses");
        translate_nullable(&mut spec);
        // One more mechanical repair: `process_json_schema` discriminates
        // its variants (generic / process-graph / datacube) with `oneOf`,
        // but no variant *requires* its `subtype` discriminator, so a
        // generic JSON Schema object matches several branches at once and
        // exact-one semantics can never hold — the upstream intent is
        // clearly at-least-one. `anyOf` keeps every branch constraint
        // intact without the impossible exclusivity.
        let process_json_schema = spec
            .pointer_mut("/components/schemas/process_json_schema")
            .and_then(serde_json::Value::as_object_mut)
            .expect("process_json_schema is in the pinned spec");
        let variants = process_json_schema
            .remove("oneOf")
            .expect("process_json_schema has oneOf variants");
        process_json_schema.insert("anyOf".to_owned(), variants);
        spec
    })
}

/// See [`openeo_spec`]: rewrites `nullable: true` into draft-compatible
/// null admission, everywhere in the document.
fn translate_nullable(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(obj) => {
            if obj.get("nullable") == Some(&serde_json::Value::Bool(true)) {
                obj.remove("nullable");
                if let Some(reference) = obj.remove("$ref") {
                    // Draft 4 ignores $ref siblings — hoist into anyOf.
                    obj.insert(
                        "anyOf".to_owned(),
                        serde_json::json!([{ "$ref": reference }, { "type": "null" }]),
                    );
                } else if let Some(serde_json::Value::String(t)) = obj.get("type") {
                    obj.insert("type".to_owned(), serde_json::json!([t.clone(), "null"]));
                }
            }
            for (_, child) in obj.iter_mut() {
                translate_nullable(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                translate_nullable(item);
            }
        }
        _ => {}
    }
}

/// Compiles a schema out of the pinned openEO spec: `pointer` addresses a
/// schema object inside `openapi.json` (e.g. a response's
/// `content['application/json'].schema`, or a named component). The spec's
/// internal `#/components/…` references resolve because the compiled root
/// carries the spec's `components` alongside the target (`allOf`-wrapped —
/// draft-4 `$ref` siblings would otherwise be ignored).
pub(crate) fn openeo_schema(pointer: &str) -> Validator {
    let spec = openeo_spec();
    let target = spec
        .pointer(pointer)
        .unwrap_or_else(|| panic!("`{pointer}` is not in the pinned openEO spec"));
    let root = serde_json::json!({
        "allOf": [target],
        "components": spec["components"],
    });
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft4)
        // Formats are annotations here, not assertions: the spec stamps
        // `format: uri` on the service `url`, but an XYZ service's url is
        // a tile *template* (`…/{z}/{y}/{x}`) by ecosystem convention —
        // structural validation is the bar these tests hold.
        .should_validate_formats(false)
        .build(&root)
        .expect("openEO schema compiles")
}

/// The response schema of an openEO operation, by path/method/status.
pub(crate) fn openeo_response_schema(path: &str, method: &str, status: &str) -> Validator {
    // JSON-pointer escaping: `/` in a path segment becomes `~1`.
    let escaped = path.replace('~', "~0").replace('/', "~1");
    openeo_schema(&format!(
        "/paths/{escaped}/{method}/responses/{status}/content/application~1json/schema"
    ))
}

/// Asserts `instance` is valid under an openEO spec schema addressed by
/// JSON pointer, with a readable failure listing every violation.
pub(crate) fn assert_openeo_valid(
    pointer_schema: &Validator,
    what: &str,
    instance: &serde_json::Value,
) {
    let errors: Vec<String> = pointer_schema
        .iter_errors(instance)
        .map(|err| format!("  {} at {}", err, err.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "instance violates {what}:\n{}\ninstance: {}",
        errors.join("\n"),
        serde_json::to_string_pretty(instance).expect("instance pretty-prints"),
    );
}

/// The catalog-mode app with the openEO surface merged in — the wiring
/// `swath serve --catalog` does, over the in-memory catalog and the
/// committed fixtures. Returns the router (Clone; reuse one instance so
/// mutations through the services surface are visible across requests)
/// and the catalog for persistence assertions.
pub(crate) fn openeo_app() -> (Router, MemoryCatalog) {
    openeo_app_with_preview_ceiling(None)
}

/// The catalog-mode openEO app over an arbitrary seeded dataset and its
/// granules, with no config-defined layer templates — the temporal
/// conformance tests (issue #181, ADR 0015) seed the Park Fire series
/// here and author every layer through the openEO surface itself.
pub(crate) fn openeo_app_seeded(
    dataset: Dataset,
    granules: Vec<Granule>,
) -> (Router, MemoryCatalog) {
    use swath_api::{CatalogLayers, OpenEoState, openeo_router};

    let catalog = MemoryCatalog::default();
    catalog.seed(dataset, granules);
    let provider = CatalogLayers::new(catalog.clone(), Vec::new());
    let store: Arc<dyn object_store::ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(fixtures_dir()).expect("fixture dir exists"));
    let state = ApiState::new(
        provider.clone(),
        CogSource::new(Arc::clone(&store)),
        Proj4rsReproject,
        BASE_URL,
    )
    .with_openeo();
    let openeo_state =
        OpenEoState::new(provider, CogSource::new(store), Proj4rsReproject, BASE_URL);
    let app = router(Arc::new(state)).merge(openeo_router(Arc::new(openeo_state)));
    (app, catalog)
}

/// [`openeo_app_seeded`] plus an in-memory write-through tile cache — the
/// cache-identity tests over published layers (ADR 0022's granule pair).
pub(crate) fn openeo_app_seeded_cached(
    dataset: Dataset,
    granules: Vec<Granule>,
) -> (Router, MemoryCatalog) {
    use swath_api::{CatalogLayers, OpenEoState, openeo_router};
    use swath_cache_objectstore::ObjectStoreTileCache;

    let catalog = MemoryCatalog::default();
    catalog.seed(dataset, granules);
    let provider = CatalogLayers::new(catalog.clone(), Vec::new());
    let store: Arc<dyn object_store::ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(fixtures_dir()).expect("fixture dir exists"));
    let state = ApiState::new(
        provider.clone(),
        CogSource::new(Arc::clone(&store)),
        Proj4rsReproject,
        BASE_URL,
    )
    .with_openeo()
    .with_cache(ObjectStoreTileCache::new(Arc::new(
        object_store::memory::InMemory::new(),
    )));
    let openeo_state =
        OpenEoState::new(provider, CogSource::new(store), Proj4rsReproject, BASE_URL);
    let app = router(Arc::new(state)).merge(openeo_router(Arc::new(openeo_state)));
    (app, catalog)
}

/// [`openeo_app`] with the preview budget's `max_estimated_live_bytes`
/// ceiling overridden — the refusal-path tests force the planner over
/// budget with a tiny ceiling (the default admits every fixture render).
pub(crate) fn openeo_app_with_preview_ceiling(ceiling: Option<u64>) -> (Router, MemoryCatalog) {
    openeo_app_with_budget(ceiling, swath_core::planner::Budget::default())
}

/// [`openeo_app_with_preview_ceiling`] under an operator budget (#272):
/// what `swath serve` hands the openEO surface from its resolved
/// `[budget]` → flags/env layering, so published services and previews
/// serve under it exactly as declared layers do.
pub(crate) fn openeo_app_with_budget(
    ceiling: Option<u64>,
    budget: swath_core::planner::Budget,
) -> (Router, MemoryCatalog) {
    use swath_api::{CatalogLayer, CatalogLayers, OpenEoState, openeo_router};

    let catalog = MemoryCatalog::default();
    catalog.seed(hls_catalog_dataset(), vec![hls_catalog_granule()]);

    // The serving templates for the config-defined layers: identical
    // plans to the fixture registry (whose inputs already name dataset
    // bands), so a catalog-served tile is byte-comparable to a
    // registry-served one.
    let registry = LayerRegistry::hls_fixtures();
    let templates: Vec<CatalogLayer> = registry
        .iter()
        .map(|layer| CatalogLayer {
            id: layer.id.clone(),
            title: layer.title.clone(),
            description: layer.description.clone(),
            dataset: DatasetId::new("hls-s30"),
            plan: layer.plan.clone(),
            resampling: layer.resampling,
            tile_size: layer.tile_size,
            budget: layer.budget.clone(),
            window: TimeRange::default(),
            sources: Vec::new(),
        })
        .collect();
    let provider = CatalogLayers::new(catalog.clone(), templates);

    let store: Arc<dyn object_store::ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(fixtures_dir()).expect("fixture dir exists"));
    let state = ApiState::new(
        provider.clone(),
        CogSource::new(Arc::clone(&store)),
        Proj4rsReproject,
        BASE_URL,
    )
    .with_openeo();
    // The openEO surface renders previews (ADR 0014) through its own
    // clone of the same adapters — exactly the binary's wiring.
    let mut openeo_state =
        OpenEoState::new(provider, CogSource::new(store), Proj4rsReproject, BASE_URL)
            .with_budget(budget);
    if let Some(ceiling) = ceiling {
        openeo_state = openeo_state.with_preview_ceiling(ceiling);
    }
    let app = router(Arc::new(state)).merge(openeo_router(Arc::new(openeo_state)));
    (app, catalog)
}

/// The openEO app with `run_udf` wired (ADR 0018, #204): the real
/// wasmtime registrar, an in-memory `object_store` module store, and the
/// caller's fetcher (a counting double in the UDF suite). Returns the
/// publish wiring and the store so tests can rehydrate and inspect.
pub(crate) fn openeo_app_with_udf<F>(fetcher: F) -> UdfApp
where
    F: swath_core::udf::ModuleFetcher + 'static,
{
    openeo_app_with_udf_budget(fetcher, swath_core::planner::Budget::default())
}

/// [`openeo_app_with_udf`] under an operator budget (#272) — the fuel
/// axis's regression harness: `[budget] max-udf-fuel-per-tile` must
/// bind a published `run_udf` service and its preview.
pub(crate) fn openeo_app_with_udf_budget<F>(
    fetcher: F,
    budget: swath_core::planner::Budget,
) -> UdfApp
where
    F: swath_core::udf::ModuleFetcher + 'static,
{
    use swath_api::{CatalogLayers, OpenEoState, UdfPublish, openeo_router};
    use swath_modulestore_objectstore::ObjectStoreModuleStore;
    use swath_udf_wasmtime::WasmtimeUdf;

    let catalog = MemoryCatalog::default();
    catalog.seed(hls_catalog_dataset(), vec![hls_catalog_granule()]);
    let provider = CatalogLayers::new(catalog.clone(), Vec::new());
    let store: Arc<dyn object_store::ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(fixtures_dir()).expect("fixture dir exists"));
    let executor = Arc::new(WasmtimeUdf::new().expect("engine builds on this host"));
    let modules = ObjectStoreModuleStore::new(Arc::new(object_store::memory::InMemory::new()));
    let publish = UdfPublish::new(executor, modules.clone(), fetcher);
    // The tile handlers run UDF stages through the very executor the
    // publish motion registered them with (#205) — the binary's wiring.
    let state = ApiState::new(
        provider.clone(),
        CogSource::new(Arc::clone(&store)),
        Proj4rsReproject,
        BASE_URL,
    )
    .with_openeo()
    .with_udf_executor(publish.executor());
    let openeo_state =
        OpenEoState::new(provider, CogSource::new(store), Proj4rsReproject, BASE_URL)
            .with_udf(publish.clone())
            .with_budget(budget);
    let app = router(Arc::new(state)).merge(openeo_router(Arc::new(openeo_state)));
    UdfApp {
        app,
        catalog,
        publish,
        store: modules,
    }
}

/// What [`openeo_app_with_udf`] hands back.
pub(crate) struct UdfApp {
    pub(crate) app: Router,
    pub(crate) catalog: MemoryCatalog,
    pub(crate) publish: swath_api::UdfPublish,
    pub(crate) store: swath_modulestore_objectstore::ObjectStoreModuleStore,
}
