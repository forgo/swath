// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Shared plumbing for the API tests: the fixture-wired app (COG source +
//! proj4rs over the committed HLS fixtures), an in-process request
//! helper (`tower::ServiceExt::oneshot` — no network), and the OGC
//! schema validator over the committed official schemas.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response};
use http_body_util::BodyExt as _;
use jsonschema::{Retrieve, Uri, Validator};
use object_store::local::LocalFileSystem;
use swath_api::{ApiState, LayerRegistry, router};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use tower::ServiceExt as _;

/// Base URL the test app mints links under.
pub(crate) const BASE_URL: &str = "http://localhost";

/// The committed HLS fixture directory (tests/fixtures/README.md, ADR 0004).
pub(crate) fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// The committed official OGC schemas (tests/data/ogc/README.md).
pub(crate) fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/ogc")
}

/// swath-render's committed oracle goldens (the #25/#26 suite) — the API
/// tile tests compare served tiles against the very same references.
pub(crate) fn render_goldens_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../swath-render/tests/data")
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
    let mut request = Request::builder().uri(path).method("GET");
    if let Some(accept) = accept {
        request = request.header("accept", accept);
    }
    app()
        .oneshot(request.body(Body::empty()).expect("request builds"))
        .await
        .expect("infallible service")
}

/// Collects a response body to bytes.
pub(crate) async fn body_bytes(response: Response<Body>) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec()
}

/// Collects a response body as JSON.
pub(crate) async fn body_json(response: Response<Body>) -> serde_json::Value {
    serde_json::from_slice(&body_bytes(response).await).expect("body is JSON")
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

use std::collections::BTreeMap;
use std::sync::Mutex;

use swath_core::catalog::{
    Bbox, Catalog, CatalogError, Dataset, DatasetId, Datetime, Extent, Granule, GranuleQuery,
    TimeRange,
};

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

/// A minimal in-memory [`Catalog`] for the openEO surface tests: datasets
/// and granules behind a mutex, shared by clones (the provider and the
/// services handlers must see one store, like pgstac in production).
#[derive(Debug, Clone, Default)]
pub(crate) struct MemoryCatalog {
    datasets: Arc<Mutex<BTreeMap<String, Dataset>>>,
    granules: Arc<Mutex<Vec<Granule>>>,
}

impl MemoryCatalog {
    /// Seeds the store synchronously (test setup).
    pub(crate) fn seed(&self, dataset: Dataset, granules: Vec<Granule>) {
        self.datasets
            .lock()
            .unwrap()
            .insert(dataset.id.as_str().to_owned(), dataset);
        self.granules.lock().unwrap().extend(granules);
    }

    /// The stored dataset, for post-mutation assertions.
    pub(crate) fn stored_dataset(&self, id: &str) -> Option<Dataset> {
        self.datasets.lock().unwrap().get(id).cloned()
    }
}

impl Catalog for MemoryCatalog {
    async fn upsert_dataset(&self, dataset: &Dataset) -> Result<(), CatalogError> {
        self.datasets
            .lock()
            .unwrap()
            .insert(dataset.id.as_str().to_owned(), dataset.clone());
        Ok(())
    }

    async fn upsert_granules(&self, granules: &[Granule]) -> Result<(), CatalogError> {
        self.granules.lock().unwrap().extend_from_slice(granules);
        Ok(())
    }

    async fn get_dataset(&self, id: &DatasetId) -> Result<Option<Dataset>, CatalogError> {
        Ok(self.datasets.lock().unwrap().get(id.as_str()).cloned())
    }

    async fn list_datasets(&self) -> Result<Vec<Dataset>, CatalogError> {
        Ok(self.datasets.lock().unwrap().values().cloned().collect())
    }

    async fn find_granules(
        &self,
        dataset: &DatasetId,
        query: &GranuleQuery,
    ) -> Result<Vec<Granule>, CatalogError> {
        Ok(self
            .granules
            .lock()
            .unwrap()
            .iter()
            .filter(|granule| granule.dataset == *dataset && matches_query(query, granule))
            .cloned()
            .collect())
    }
}

/// Whether `granule` satisfies `query` — the filter semantics the pgstac
/// adapter delegates to STAC search: bbox intersection (inclusive edges;
/// no antimeridian handling — the fixture footprints don't cross it) and
/// inclusive datetime bounds.
fn matches_query(query: &GranuleQuery, granule: &Granule) -> bool {
    if let Some(bbox) = query.bbox {
        let g = granule.bbox;
        if bbox.west > g.east || g.west > bbox.east || bbox.south > g.north || g.south > bbox.north
        {
            return false;
        }
    }
    if let Some(range) = &query.datetime {
        let t = granule.datetime.to_unix_millis();
        if range.start.as_ref().is_some_and(|s| t < s.to_unix_millis())
            || range.end.as_ref().is_some_and(|e| t > e.to_unix_millis())
        {
            return false;
        }
    }
    true
}

/// The HLS fixture dataset in catalog form: the same band vocabulary and
/// serving layers as `LayerRegistry::hls_fixtures`, persisted the way
/// `[[datasets]]` config would persist them (`PlanKind` + rescale).
pub(crate) fn hls_catalog_dataset() -> Dataset {
    use swath_core::catalog::{Colormap, Layer, PlanKind, Resampling, Rescale};
    Dataset {
        id: DatasetId::new("hls-s30"),
        title: "HLS Sentinel-2 (S30)".to_owned(),
        description: "Harmonized Landsat Sentinel-2, S30 product.".to_owned(),
        license: "CC0-1.0".to_owned(),
        extent: Extent {
            bbox: Bbox {
                west: -105.537,
                south: 39.1954,
                east: -105.3581,
                north: 39.3345,
            },
            interval: TimeRange {
                start: Some(Datetime::new("2024-06-01T00:00:00Z").unwrap()),
                end: None,
            },
        },
        bands: ["b02", "b03", "b04", "b8a"]
            .map(str::to_owned)
            .into_iter()
            .collect(),
        layers: vec![
            Layer {
                id: "ndvi".to_owned(),
                title: "HLS NDVI".to_owned(),
                description: "(B8A - B04) / (B8A + B04), grayscale.".to_owned(),
                plan: PlanKind::BandMath {
                    expression: "(b8a - b04) / (b8a + b04)".to_owned(),
                },
                rescale: Rescale {
                    min: -1.0,
                    max: 1.0,
                },
                colormap: Some(Colormap::Grayscale),
                resampling: Resampling::Bilinear,
                tile_size: 256,
                process: None,
            },
            Layer {
                id: "truecolor".to_owned(),
                title: "HLS true color".to_owned(),
                description: "B04/B03/B02 composite.".to_owned(),
                plan: PlanKind::Composite {
                    r: "b04".to_owned(),
                    g: "b03".to_owned(),
                    b: "b02".to_owned(),
                },
                rescale: Rescale {
                    min: 0.0,
                    max: 3000.0,
                },
                colormap: None,
                resampling: Resampling::Bilinear,
                tile_size: 256,
                process: None,
            },
        ],
    }
}

/// The committed HLS fixture granule, catalog form: band assets are the
/// bare fixture file names the local store root resolves.
pub(crate) fn hls_catalog_granule() -> Granule {
    use swath_core::catalog::{GranuleAsset, GranuleId};
    let asset = |name: &str| GranuleAsset::raster(format!("hlss30-t13sdd-2024158-{name}.tif"));
    Granule {
        id: GranuleId::new("hlss30-t13sdd-2024158"),
        dataset: DatasetId::new("hls-s30"),
        bbox: Bbox {
            west: -105.537,
            south: 39.1954,
            east: -105.3581,
            north: 39.3345,
        },
        datetime: Datetime::new("2024-06-06T17:54:00Z").unwrap(),
        assets: [
            ("b02".to_owned(), asset("b02")),
            ("b03".to_owned(), asset("b03")),
            ("b04".to_owned(), asset("b04")),
            ("b8a".to_owned(), asset("b8a")),
        ]
        .into(),
        ingested_at: Some(Datetime::new("2024-06-06T18:00:00Z").unwrap()),
    }
}

/// The catalog-mode app with the openEO surface merged in — the wiring
/// `swath serve --catalog` does, over the in-memory catalog and the
/// committed fixtures. Returns the router (Clone; reuse one instance so
/// mutations through the services surface are visible across requests)
/// and the catalog for persistence assertions.
pub(crate) fn openeo_app() -> (Router, MemoryCatalog) {
    openeo_app_with_preview_ceiling(None)
}

/// [`openeo_app`] with the preview budget's `max_estimated_live_bytes`
/// ceiling overridden — the refusal-path tests force the planner over
/// budget with a tiny ceiling (the default admits every fixture render).
pub(crate) fn openeo_app_with_preview_ceiling(ceiling: Option<u64>) -> (Router, MemoryCatalog) {
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
        OpenEoState::new(provider, CogSource::new(store), Proj4rsReproject, BASE_URL);
    if let Some(ceiling) = ceiling {
        openeo_state = openeo_state.with_preview_ceiling(ceiling);
    }
    let app = router(Arc::new(state)).merge(openeo_router(Arc::new(openeo_state)));
    (app, catalog)
}

/// One in-process request against a specific router instance.
pub(crate) async fn request_on(
    app: &Router,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Response<Body> {
    let mut request = Request::builder().uri(path).method(method);
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&json).expect("body serializes"))
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(request.body(body).expect("request builds"))
        .await
        .expect("infallible service")
}
