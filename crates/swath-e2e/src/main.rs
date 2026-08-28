// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `swath-e2e` — the typed compose-stack e2e harness (issue #98).
//!
//! Replaces the ~120 lines of sed/grep assertions that lived in the
//! `just e2e` recipe (and the assertion half of `tests/e2e/stack-up.sh`)
//! with named Rust checks over the same live stack. The harness ASSUMES
//! the stack is already up and healthy (`SWATH_STACK_UP_ONLY=1
//! tests/e2e/stack-up.sh` — lifecycle only) and that teardown stays the
//! recipe's trap; it then owns the whole assertion story: the honest
//! pre-drop 404, the granule drop (via the shared
//! `tests/e2e/drop-granule.sh`), poll-to-live, trace-header and SSE
//! assertions against the typed [`swath_core::trace::Trace`] (no string
//! re-parsing), golden comparisons through `swath-testkit`'s pdiff
//! library, the openEO authoring round trip, and the declared-bounds
//! check. Every check has a name; every failure names the endpoint, the
//! expectation, and what was actually observed.
//!
//! The north-star ingest-to-pixel measurement is additionally emitted as
//! a machine-readable JSON line on stdout and written to
//! `target/e2e/metrics.json` (value, budget, git sha, timestamp) —
//! PERFORMANCE.md (M5) consumes it.

// A harness's contract is its stdout/stderr; the workspace-wide
// restriction lints against printing target library/server code.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod http;
mod sse;

use std::fmt;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use swath_core::trace::{Strategy, TemporalRule, Trace};
use swath_testkit::{DiffPolicy, diff, load_png};

/// The proven tile, OGC request order (`z/row/col`).
const TILE: &str = "/tilesets/truecolor/tiles/12/1561/848";
/// The same tile on the on-the-fly NDVI layer.
const NDVI: &str = "/tilesets/ndvi/tiles/12/1561/848";
/// The proven tile in the SSE envelope's XYZ order (`z/x/y`).
const TILE_XYZ: &str = "12/848/1561";
/// A point inside tile 12/848/1561 (lon, lat) — the declared-bounds probe.
const PROBE_LON_LAT: (f64, f64) = (-105.4248, 39.27);

const TRUECOLOR_GOLDEN: &str = "crates/swath-render/tests/data/truecolor-12-848-1561.png";
const NDVI_COLORMAPPED_GOLDEN: &str = "crates/swath-render/tests/data/ndvi-rdylgn-12-848-1561.png";
const NDVI_GRAYSCALE_GOLDEN: &str = "crates/swath-render/tests/data/ndvi-12-848-1561.png";

// --- The time dimension (ADR 0015, issue #180): the Park Fire series ---

/// The proven fire tile (z13, fully inside the T10TFK fixture window),
/// OGC request order (`z/row/col`).
const FIRE_TILE: &str = "/tilesets/park-fire-ndvi/tiles/13/3100/1326";
/// The same fire tile in the SSE envelope's XYZ order (`z/x/y`).
const FIRE_TILE_XYZ: &str = "13/1326/3100";
/// An instant between the July (2024-07-22) and August (2024-08-16)
/// acquisitions: latest-at-or-before must select pre-fire July.
const FIRE_PRE_INSTANT: &str = "2024-08-01T00:00:00Z";
/// An instant between August and September: the fresh burn scar.
const FIRE_POST_INSTANT: &str = "2024-08-20T00:00:00Z";
const FIRE_PRE_GRANULE: &str = "hlss30-t10tfk-2024204";
const FIRE_POST_GRANULE: &str = "hlss30-t10tfk-2024229";
const FIRE_PRE_GOLDEN: &str =
    "crates/swath-render/tests/data/fire-ndvi-rdylgn-13-1326-3100-2024204.png";
const FIRE_POST_GOLDEN: &str =
    "crates/swath-render/tests/data/fire-ndvi-rdylgn-13-1326-3100-2024229.png";
/// The derived temporal extent of the six dropped dates
/// (drop-fire-granules.sh) — what `/collections/hls-s30-fire` must serve.
const FIRE_EXTENT: [&str; 2] = ["2024-06-07T19:03:00Z", "2024-10-15T19:03:00Z"];

/// The north-star budget (issue #35): measured 297 ms and 801 ms locally,
/// 535 ms in CI, so 10000 ms is ~20x headroom over the CI number — tight
/// enough to catch a real regression (a sleep, a poll interval, an
/// accidental batch step), loose enough to shrug off runner noise.
/// Tightening it further is a deliberate, visible act: record new
/// measurements here when you do.
const I2P_BUDGET_MS: u64 = 10_000;

/// Where artifacts (fetched tiles, metrics.json) land, matching the
/// stack's own data-plane directory.
const ARTIFACT_DIR: &str = "target/e2e";

/// One failed check: which check, against what, expected vs observed.
struct Failure {
    check: &'static str,
    endpoint: String,
    expected: String,
    actual: String,
}

impl Failure {
    fn new(
        check: &'static str,
        endpoint: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            check,
            endpoint: endpoint.into(),
            expected: expected.into(),
            actual: actual.into(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "FAIL {}", self.check)?;
        writeln!(f, "  endpoint: {}", self.endpoint)?;
        writeln!(f, "  expected: {}", self.expected)?;
        write!(f, "  actual:   {}", self.actual)
    }
}

/// The `X-Swath-Trace` summary header the tile handler emits
/// (`swath-api/src/routes.rs`): decision + byte/timing digest of the
/// full [`Trace`] the SSE stream carries.
#[derive(serde::Deserialize)]
struct TraceHeader {
    decision: String,
    bytes_read: u64,
    total_ms: u64,
    ingest_to_pixel_ms: Option<u64>,
}

/// The SSE `trace` event's `data:` payload — the API envelope around the
/// pinned core [`Trace`]. Declared here as a deserialize mirror:
/// `swath_api::traces::TraceEvent` is serialize-only by design (the
/// server never parses its own stream), so the harness owns the reading
/// half; the payload itself deserializes into the shared core type.
#[derive(serde::Deserialize)]
struct TraceEnvelope {
    tile: String,
    layer: String,
    trace: Trace,
}

/// The subset of OGC tileset metadata the bounds check reads.
#[derive(serde::Deserialize)]
struct TilesetMeta {
    #[serde(rename = "boundingBox")]
    bounding_box: BoundingBox,
}

#[derive(serde::Deserialize)]
struct BoundingBox {
    #[serde(rename = "lowerLeft")]
    lower_left: [f64; 2],
    #[serde(rename = "upperRight")]
    upper_right: [f64; 2],
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("e2e OK");
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("{failure}");
            ExitCode::FAILURE
        }
    }
}

fn pass(name: &str, detail: impl fmt::Display) {
    println!("PASS {name}: {detail}");
}

/// GET with transport errors converted into a named failure.
fn get(check: &'static str, path: &str) -> Result<http::Response, Failure> {
    http::get(path).map_err(|e| Failure::new(check, path, "an HTTP response", e))
}

fn run() -> Result<(), Failure> {
    fs::create_dir_all(ARTIFACT_DIR).map_err(|e| {
        Failure::new(
            "artifact_dir_writable",
            ARTIFACT_DIR,
            "artifact directory created",
            e.to_string(),
        )
    })?;
    landing_page_serves_swath_document()?;
    catalog_registers_hls_s30_dataset()?;
    tile_404_before_ingest()?;
    drop_granule()?;
    let first = tile_live_within_60s_of_drop()?;
    first_render_checks(&first)?;
    second_fetch_is_cache_hit_with_identical_bytes(&first.body)?;
    truecolor_matches_oracle_golden()?;
    let ndvi_bytes = sse_and_ndvi_checks()?;
    openeo_checks(&ndvi_bytes)?;
    tileset_bounds_contain_proven_tile()?;
    fire_time_dimension_checks()
}

// --- The time dimension (ADR 0015 / issue #180) over the fire series ---

/// The whole temporal story against the live stack: honest 404 before
/// the fire drop, the six-granule drop, `datetime=` frame selection with
/// self-golden-pinned pixels per date (values oracle-pinned in process), granule-scoped cache identity, the
/// temporal decision on the SSE trace, the RFC 7807 / refusal taxonomy,
/// and the derived temporal extent on the collection document.
fn fire_time_dimension_checks() -> Result<(), Failure> {
    fire_tile_404_before_drop()?;
    drop_fire_granules()?;
    fire_series_fully_ingested_within_60s()?;

    let mut subscriber = sse::Subscriber::connect().map_err(|e| {
        Failure::new(
            "fire_sse_carries_temporal_decision",
            "/traces",
            "an SSE subscription",
            e,
        )
    })?;
    let latest = fire_absent_datetime_is_latest_and_cache_shared()?;
    let pre = fire_frame_matches_golden(
        "fire_pre_frame_matches_self_golden",
        FIRE_PRE_INSTANT,
        FIRE_PRE_GOLDEN,
        "fire-pre.png",
    )?;
    let post = fire_frame_matches_golden(
        "fire_post_frame_matches_self_golden",
        FIRE_POST_INSTANT,
        FIRE_POST_GOLDEN,
        "fire-post.png",
    )?;
    if pre == post || pre == latest {
        return Err(Failure::new(
            "fire_frames_differ_across_dates",
            FIRE_TILE,
            "different pixels for different resolved granules",
            "identical bytes across dates",
        ));
    }
    pass(
        "fire_frames_differ_across_dates",
        "same tile, two dates, different oracle-pinned pixels (the burn scar is visible)",
    );
    fire_sse_carries_temporal_decision(&mut subscriber)?;
    fire_change_layer_serves_two_sources(&mut subscriber, &pre, &post)?;
    fire_datetime_error_taxonomy()?;
    fire_collection_serves_derived_temporal_extent()?;
    qgis_xyz_template_serves_png()?;
    dataset_registration_checks()
}

/// The dataset-creation surface (#196) against the live stack: register a
/// dataset + the fixture granule by API (asset headers validated by the
/// server, bbox and extents DERIVED), see it in /collections, author an
/// NDVI service on it, and serve a traced tile — register → author →
/// serve, end to end, with a refusal check on a bad asset.
#[allow(
    clippy::too_many_lines,
    reason = "one linear e2e scenario: register, refuse, register, discover, \
              author, serve — the harness style throughout this file"
)]
fn dataset_registration_checks() -> Result<(), Failure> {
    const CHECK: &str = "api_registered_dataset_serves_traced_tiles";
    let fail = |path: &str, expected: &str, got: String| -> Failure {
        Failure::new(CHECK, path, expected, got)
    };

    let body = serde_json::json!({
        "id": "api-hls", "title": "HLS, registered by API",
        "bands": ["b04", "b8a"],
    });
    let resp = http::post_json("/datasets", &body)
        .map_err(|e| fail("/datasets", "an HTTP response", e))?;
    if resp.status != 201 {
        return Err(fail("/datasets", "201 Created", format!("{}", resp.status)));
    }

    // A bad asset is refused with problem details naming it.
    let bad = serde_json::json!({
        "id": "bad", "datetime": "2024-06-06T17:54:00Z",
        "assets": {"b04": "no-such-file.tif"},
    });
    let resp = http::post_json("/datasets/api-hls/granules", &bad)
        .map_err(|e| fail("/datasets/api-hls/granules", "an HTTP response", e))?;
    let problem = String::from_utf8_lossy(&resp.body).into_owned();
    if resp.status != 400 || !problem.contains("no-such-file.tif") {
        return Err(fail(
            "/datasets/api-hls/granules",
            "400 naming the failing asset",
            format!("{} with {problem}", resp.status),
        ));
    }

    // The real fixture granule: same store keys the drop registered, no
    // bbox (derived from the asset header server-side).
    let granule = serde_json::json!({
        "id": "hlss30-t13sdd-2024158", "datetime": "2024-06-06T17:54:00Z",
        "assets": {
            "b04": "hlss30-t13sdd-2024158-b04.tif",
            "b8a": "hlss30-t13sdd-2024158-b8a.tif",
        },
    });
    let resp = http::post_json("/datasets/api-hls/granules", &granule)
        .map_err(|e| fail("/datasets/api-hls/granules", "an HTTP response", e))?;
    if resp.status != 201 {
        return Err(fail(
            "/datasets/api-hls/granules",
            "201 Created",
            format!(
                "{} with {}",
                resp.status,
                String::from_utf8_lossy(&resp.body)
            ),
        ));
    }

    // In /collections, with the DERIVED extent (the fixture footprint).
    let resp = get(CHECK, "/collections/api-hls")?;
    let doc: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| fail("/collections/api-hls", "a JSON document", e.to_string()))?;
    let west = doc["extent"]["spatial"]["bbox"][0][0]
        .as_f64()
        .unwrap_or(0.0);
    if resp.status != 200 || (west - -105.54).abs() > 0.02 {
        return Err(fail(
            "/collections/api-hls",
            "200 with the derived fixture footprint (west ~ -105.54)",
            format!("{} with west {west}", resp.status),
        ));
    }

    // Author NDVI on it through the services surface, then serve, traced.
    let service = serde_json::json!({
        "type": "xyz", "title": "NDVI (API-registered)",
        "process": {"process_graph": {
            "load": {"process_id": "load_collection", "arguments": {
                "id": "api-hls", "spatial_extent": null, "temporal_extent": null,
                "bands": ["b8a", "b04"]}},
            "ndvi": {"process_id": "ndvi", "arguments": {
                "data": {"from_node": "load"}, "nir": "b8a", "red": "b04"}},
            "scale": {"process_id": "linear_scale_range", "arguments": {
                "x": {"from_node": "ndvi"},
                "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255}},
            "save": {"process_id": "save_result", "arguments": {
                "data": {"from_node": "scale"}, "format": "png"}, "result": true},
        }},
    });
    let resp = http::post_json("/services", &service)
        .map_err(|e| fail("/services", "an HTTP response", e))?;
    let sid = resp
        .header("openeo-identifier")
        .unwrap_or_default()
        .to_owned();
    if resp.status != 201 || sid.is_empty() {
        return Err(fail(
            "/services",
            "201 with openeo-identifier",
            format!("{}", resp.status),
        ));
    }
    let tile = format!("/tilesets/{sid}/tiles/12/1561/848");
    let resp = get(CHECK, &tile)?;
    let traced = resp.header("x-swath-trace").is_some();
    let is_png = resp.body.starts_with(&[0x89, b'P', b'N', b'G']);
    if resp.status != 200 || !traced || !is_png {
        return Err(fail(
            &tile,
            "200 image/png with x-swath-trace (registered data, through the engine)",
            format!("status {}, traced: {traced}, png: {is_png}", resp.status),
        ));
    }
    pass(
        CHECK,
        format_args!("register -> author -> serve, traced ({sid})"),
    );
    Ok(())
}

/// The QGIS recipe's smoke (#194, docs/RECIPES.md): the documented XYZ
/// template — placeholders expanded exactly as QGIS expands them
/// (`{z}/{y}/{x}` in path position, `datetime=` passed through) — returns
/// 200 + `image/png` + PNG magic, for both the plain and the dated form.
/// A doc whose URL stops working fails this build.
fn qgis_xyz_template_serves_png() -> Result<(), Failure> {
    const CHECK: &str = "qgis_xyz_template_serves_png";
    // docs/RECIPES.md, expanded at the proven tiles: truecolor plain,
    // park-fire-ndvi with the documented `datetime=` query.
    let expanded = [
        TILE.to_owned(),
        format!("{FIRE_TILE}?datetime={FIRE_PRE_INSTANT}"),
    ];
    for path in &expanded {
        let resp = get(CHECK, path)?;
        let content_type = resp.header("content-type").unwrap_or("").to_owned();
        let is_png = resp.body.starts_with(&[0x89, b'P', b'N', b'G']);
        if resp.status != 200 || !content_type.starts_with("image/png") || !is_png {
            return Err(Failure::new(
                CHECK,
                path,
                "200 with image/png bytes (the documented QGIS template)",
                format!(
                    "status {status}, content-type `{content_type}`, png magic: {is_png}",
                    status = resp.status,
                ),
            ));
        }
    }
    pass(
        CHECK,
        "documented XYZ template serves PNG (plain + datetime=)",
    );
    Ok(())
}

/// The fire layer exists (it is in the tilesets list) but its dataset has
/// no granules yet: exactly 404 — the same honest shape the main drop
/// path proves for `truecolor`.
fn fire_tile_404_before_drop() -> Result<(), Failure> {
    const CHECK: &str = "fire_tile_404_before_drop";
    let resp = get(CHECK, FIRE_TILE)?;
    if resp.status != 404 {
        return Err(Failure::new(
            CHECK,
            FIRE_TILE,
            "404 before the fire drop (dataset empty)",
            format!("{}", resp.status),
        ));
    }
    pass(CHECK, "fire tile is 404 before its drop (dataset empty)");
    Ok(())
}

/// Lifecycle stimulus: the six-date Park Fire drop, via the shared script.
fn drop_fire_granules() -> Result<(), Failure> {
    const SCRIPT: &str = "tests/e2e/drop-fire-granules.sh";
    let status = Command::new("bash")
        .arg(SCRIPT)
        .status()
        .map_err(|e| Failure::new("fire_drop", SCRIPT, "drop script runs", e.to_string()))?;
    if !status.success() {
        return Err(Failure::new(
            "fire_drop",
            SCRIPT,
            "exit 0",
            format!("exit {:?}", status.code()),
        ));
    }
    Ok(())
}

/// Polls the granules route until all six fire dates are cataloged, so
/// every later frame request resolves against the complete series
/// ("latest" must mean 2024-10-15, not whichever granule ingested first).
fn fire_series_fully_ingested_within_60s() -> Result<(), Failure> {
    const CHECK: &str = "fire_series_fully_ingested_within_60s";
    const ENDPOINT: &str = "/datasets/hls-s30-fire/granules?limit=1";
    let deadline = Instant::now() + Duration::from_mins(1);
    let matched = loop {
        let last = match http::get(ENDPOINT) {
            Ok(resp) if resp.status == 200 => {
                let doc: serde_json::Value =
                    serde_json::from_slice(&resp.body).unwrap_or(serde_json::Value::Null);
                let matched = doc["numberMatched"].as_u64().unwrap_or(0);
                if matched >= 6 {
                    break matched;
                }
                format!("200 with numberMatched={matched}")
            }
            Ok(resp) => resp.status.to_string(),
            Err(e) => e,
        };
        if Instant::now() >= deadline {
            return Err(Failure::new(
                CHECK,
                ENDPOINT,
                "numberMatched >= 6 within 60s of the fire drop",
                format!("last: {last}"),
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    pass(CHECK, format_args!("all {matched} fire granules cataloged"));
    Ok(())
}

/// Absent `datetime` = latest, and cache identity is granule-scoped: an
/// explicit instant resolving to the same latest granule serves the very
/// bytes the absent-parameter render cached (decision `cache_hit`) — the
/// datetime string is provably not in the key. Returns the latest bytes.
fn fire_absent_datetime_is_latest_and_cache_shared() -> Result<Vec<u8>, Failure> {
    const CHECK: &str = "fire_absent_datetime_is_latest_and_cache_shared";
    let absent = get(CHECK, FIRE_TILE)?;
    if absent.status != 200 {
        return Err(Failure::new(
            CHECK,
            FIRE_TILE,
            "200 for the parameterless (latest) frame",
            format!("{}", absent.status),
        ));
    }
    let explicit_path = format!("{FIRE_TILE}?datetime=2030-01-01T00:00:00Z");
    let explicit = get(CHECK, &explicit_path)?;
    if explicit.status != 200 {
        return Err(Failure::new(
            CHECK,
            &explicit_path,
            "200",
            format!("{}", explicit.status),
        ));
    }
    if explicit.body != absent.body {
        return Err(Failure::new(
            CHECK,
            &explicit_path,
            "bytes identical to the absent-datetime frame (same resolved granule)",
            "differing payload",
        ));
    }
    let header = parse_trace_header(CHECK, &explicit_path, &explicit)?;
    if header.decision != "cache_hit" {
        return Err(Failure::new(
            CHECK,
            &explicit_path,
            "decision \"cache_hit\" (same granule -> same key -> shared entry)",
            format!("decision {:?}", header.decision),
        ));
    }
    pass(
        CHECK,
        "absent datetime = latest; an explicit instant resolving to the same granule \
         hits the same cache entry with identical bytes",
    );
    Ok(absent.body)
}

/// Fetches one dated frame, writes it as an artifact, and pins it
/// byte-for-byte against the committed `RdYlGn` self-golden — level 2 of
/// the #94 scheme, exactly as `ndvi_matches_colormapped_self_golden`:
/// the frame's VALUES are pinned to the grayscale rio-tiler oracle in
/// process (`swath-render`'s `golden_ir`, `swath-api`'s `tiles_datetime`),
/// the colors proven `lut[q(gray)]` there, and this freezes the served bytes.
/// Returns the served bytes.
fn fire_frame_matches_golden(
    check: &'static str,
    instant: &str,
    golden: &str,
    artifact: &str,
) -> Result<Vec<u8>, Failure> {
    let path = format!("{FIRE_TILE}?datetime={instant}");
    let resp = get(check, &path)?;
    if resp.status != 200 {
        return Err(Failure::new(
            check,
            &path,
            "200",
            format!("{}", resp.status),
        ));
    }
    let served = format!("{ARTIFACT_DIR}/{artifact}");
    fs::write(&served, &resp.body)
        .map_err(|e| Failure::new(check, &path, "artifact written", e.to_string()))?;
    let committed = fs::read(golden).map_err(|e| {
        Failure::new(
            check,
            &path,
            "committed self-golden readable",
            e.to_string(),
        )
    })?;
    if resp.body != committed {
        return Err(Failure::new(
            check,
            &path,
            format!("bytes identical to {golden} ({} bytes)", committed.len()),
            format!("{} bytes, differing payload", resp.body.len()),
        ));
    }
    pass(
        check,
        format_args!("datetime={instant} is byte-identical to the RdYlGn self-golden {golden}"),
    );
    Ok(resp.body)
}

/// The two-cube join against the live stack (ADR 0022, issue #296): a
/// `merge_cubes` change layer — NDVI(August) − NDVI(July) over the fire
/// collection — published through `/services` serves one tile from two
/// granules (pixels unlike either single-date frame; the values are
/// oracle-pinned in the swath-api suite), and its trace on the stream
/// names both granules, the `after` branch first.
#[allow(
    clippy::too_many_lines,
    reason = "one linear e2e scenario: publish the join, serve, read the stream — \
              the harness style throughout this file"
)]
fn fire_change_layer_serves_two_sources(
    subscriber: &mut sse::Subscriber,
    pre: &[u8],
    post: &[u8],
) -> Result<(), Failure> {
    const CHECK: &str = "fire_change_layer_serves_two_sources";
    let fail = |path: &str, expected: &str, got: String| -> Failure {
        Failure::new(CHECK, path, expected, got)
    };
    let load = |extent: [&str; 2]| {
        serde_json::json!({"process_id": "load_collection", "arguments": {
            "id": "hls-s30-fire", "spatial_extent": null,
            "temporal_extent": extent, "bands": ["b8a", "b04"]}})
    };
    let ndvi = |from: &str| {
        serde_json::json!({"process_id": "ndvi", "arguments": {
            "data": {"from_node": from}, "nir": "b8a", "red": "b04"}})
    };
    let service = serde_json::json!({
        "type": "xyz", "title": "Fire change (August − July)",
        "process": {"process_graph": {
            "before": load(["2024-07-01T00:00:00Z", "2024-08-01T00:00:00Z"]),
            "after": load(["2024-08-01T00:00:00Z", "2024-09-01T00:00:00Z"]),
            "ndvi_before": ndvi("before"),
            "ndvi_after": ndvi("after"),
            "change": {"process_id": "merge_cubes", "arguments": {
                "cube1": {"from_node": "ndvi_after"},
                "cube2": {"from_node": "ndvi_before"},
                "overlap_resolver": {"process_graph": {
                    "diff": {"process_id": "subtract", "arguments": {
                        "x": {"from_parameter": "x"}, "y": {"from_parameter": "y"}},
                        "result": true}}}}},
            "scale": {"process_id": "linear_scale_range", "arguments": {
                "x": {"from_node": "change"},
                "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255}},
            "save": {"process_id": "save_result", "arguments": {
                "data": {"from_node": "scale"}, "format": "png"}, "result": true},
        }},
    });
    let resp = http::post_json("/services", &service)
        .map_err(|e| fail("/services", "an HTTP response", e))?;
    let sid = resp
        .header("openeo-identifier")
        .unwrap_or_default()
        .to_owned();
    if resp.status != 201 || sid.is_empty() {
        return Err(fail(
            "/services",
            "201 with openeo-identifier",
            format!("{}", resp.status),
        ));
    }
    let path = format!("/tilesets/{sid}/tiles/13/3100/1326");
    let resp = get(CHECK, &path)?;
    if resp.status != 200 || !resp.body.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Err(fail(
            &path,
            "200 with PNG bytes",
            format!("{}", resp.status),
        ));
    }
    if resp.body == pre || resp.body == post {
        return Err(fail(
            &path,
            "a change tile unlike either single-date frame",
            "bytes identical to one of the dated frames".to_owned(),
        ));
    }
    // The stream: one temporal record per branch.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let frame = subscriber
            .next_frame(deadline)
            .map_err(|e| fail("/traces", "the change tile's trace event within 15s", e))?;
        if frame.is_keepalive() || frame.event.as_deref() == Some("lagged") {
            continue;
        }
        let data = frame.data.join("\n");
        let envelope: TraceEnvelope = serde_json::from_str(&data).map_err(|e| {
            fail(
                "/traces",
                "trace data deserializes as the envelope around a core Trace",
                format!("{e}; data: {data}"),
            )
        })?;
        if envelope.layer != sid || envelope.tile != FIRE_TILE_XYZ {
            continue;
        }
        let sources: Vec<(String, String)> = envelope
            .trace
            .temporal
            .as_ref()
            .map(|t| {
                t.sources
                    .iter()
                    .map(|s| (s.node.clone(), s.granule_id.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let expected = vec![
            ("after".to_owned(), FIRE_POST_GRANULE.to_owned()),
            ("before".to_owned(), FIRE_PRE_GRANULE.to_owned()),
        ];
        if sources != expected {
            return Err(fail(
                "/traces",
                "temporal.sources = [(after, 2024229), (before, 2024204)]",
                format!("{sources:?}"),
            ));
        }
        break;
    }
    pass(
        CHECK,
        "a merge_cubes change layer serves one tile from two granules; the trace names both",
    );
    Ok(())
}

/// The temporal decision is on the trace stream: both dated frames'
/// envelopes carry `temporal` with the resolved granule id and the
/// `latest_at_or_before` rule (typed via the shared core `Trace`).
fn fire_sse_carries_temporal_decision(subscriber: &mut sse::Subscriber) -> Result<(), Failure> {
    const CHECK: &str = "fire_sse_carries_temporal_decision";
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut seen: Vec<(String, String)> = Vec::new();
    loop {
        let granule_seen = |granule: &str| seen.iter().any(|(g, _)| g == granule);
        if granule_seen(FIRE_PRE_GRANULE) && granule_seen(FIRE_POST_GRANULE) {
            break;
        }
        let frame = subscriber.next_frame(deadline).map_err(|e| {
            Failure::new(
                CHECK,
                "/traces",
                "trace events for both dated fire frames within 15s",
                format!("{e}; temporal decisions so far: {seen:?}"),
            )
        })?;
        if frame.is_keepalive() || frame.event.as_deref() == Some("lagged") {
            continue;
        }
        let data = frame.data.join("\n");
        let envelope: TraceEnvelope = serde_json::from_str(&data).map_err(|e| {
            Failure::new(
                CHECK,
                "/traces",
                "trace data deserializes as the envelope around a core Trace",
                format!("{e}; data: {data}"),
            )
        })?;
        if envelope.layer != "park-fire-ndvi" || envelope.tile != FIRE_TILE_XYZ {
            continue;
        }
        let Some(temporal) = envelope.trace.temporal else {
            return Err(Failure::new(
                CHECK,
                "/traces",
                "a temporal decision on every catalog-backed fire render",
                format!(
                    "trace without `temporal` (decision {:?})",
                    envelope.trace.decision
                ),
            ));
        };
        seen.push((temporal.granule_id.clone(), format!("{:?}", temporal.rule)));
        if matches!(
            temporal.granule_id.as_str(),
            FIRE_PRE_GRANULE | FIRE_POST_GRANULE
        ) && temporal.rule != TemporalRule::LatestAtOrBefore
        {
            return Err(Failure::new(
                CHECK,
                "/traces",
                "rule latest_at_or_before for an instant datetime",
                format!("{:?} for {}", temporal.rule, temporal.granule_id),
            ));
        }
    }
    pass(
        CHECK,
        format_args!("SSE traces record the temporal decision (granule + rule): {seen:?}"),
    );
    Ok(())
}

/// Malformed `datetime` → 400 RFC 7807 naming the parameter; a window
/// before the first acquisition → the established 404 refusal shape.
fn fire_datetime_error_taxonomy() -> Result<(), Failure> {
    const CHECK: &str = "fire_datetime_error_taxonomy";
    let bad_path = format!("{FIRE_TILE}?datetime=yesterday");
    let resp = get(CHECK, &bad_path)?;
    let body = String::from_utf8_lossy(&resp.body).into_owned();
    if resp.status != 400 || !body.contains("\"status\":400") || !body.contains("datetime") {
        return Err(Failure::new(
            CHECK,
            &bad_path,
            "400 with an RFC 7807 body naming `datetime`",
            format!("{} with body {body:?}", resp.status),
        ));
    }
    let empty_path = format!("{FIRE_TILE}?datetime=2020-01-01T00:00:00Z");
    let resp = get(CHECK, &empty_path)?;
    let body = String::from_utf8_lossy(&resp.body).into_owned();
    if resp.status != 404 || !body.contains("acquisition datetime within") {
        return Err(Failure::new(
            CHECK,
            &empty_path,
            "404 with the empty-window refusal naming the window",
            format!("{} with body {body:?}", resp.status),
        ));
    }
    pass(
        CHECK,
        "malformed datetime is an RFC 7807 400; an empty window the honest 404 refusal",
    );
    Ok(())
}

/// The derived temporal extent is served where clients look for it: the
/// collection document's temporal dimension spans exactly the six
/// dropped acquisitions (deferral row 15's temporal half, made real).
fn fire_collection_serves_derived_temporal_extent() -> Result<(), Failure> {
    const CHECK: &str = "fire_collection_serves_derived_temporal_extent";
    const ENDPOINT: &str = "/collections/hls-s30-fire";
    let resp = get(CHECK, ENDPOINT)?;
    if resp.status != 200 {
        return Err(Failure::new(
            CHECK,
            ENDPOINT,
            "200",
            format!("{}", resp.status),
        ));
    }
    let doc: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| Failure::new(CHECK, ENDPOINT, "a JSON collection document", e.to_string()))?;
    let extent = &doc["cube:dimensions"]["t"]["extent"];
    let expected = serde_json::json!(FIRE_EXTENT);
    if extent != &expected {
        return Err(Failure::new(
            CHECK,
            ENDPOINT,
            format!("temporal extent {expected} derived from the ingested granules"),
            format!("{extent}"),
        ));
    }
    pass(
        CHECK,
        format_args!("collection serves the derived temporal extent {expected}"),
    );
    Ok(())
}

/// Was: `curl -sf "$base/" | grep -q '"title":"Swath"'` (stack-up.sh).
fn landing_page_serves_swath_document() -> Result<(), Failure> {
    const CHECK: &str = "landing_page_serves_swath_document";
    let resp = get(CHECK, "/")?;
    let doc: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|e| {
        Failure::new(
            CHECK,
            "/",
            "a JSON landing document",
            format!("unparseable body: {e}"),
        )
    })?;
    if resp.status != 200 || doc["title"] != "Swath" {
        return Err(Failure::new(
            CHECK,
            "/",
            "200 with title \"Swath\"",
            format!("{} with title {}", resp.status, doc["title"]),
        ));
    }
    pass(CHECK, "landing page OK");
    Ok(())
}

/// Was: `docker compose exec -T pgstac psql ... get_collection('hls-s30')
/// is not null | grep -qx t` (stack-up.sh) — plain STAC visibility (R5)
/// before any granule exists.
fn catalog_registers_hls_s30_dataset() -> Result<(), Failure> {
    const CHECK: &str = "catalog_registers_hls_s30_dataset";
    const SQL: &str = "select pgstac.get_collection('hls-s30') is not null;";
    let endpoint = "pgstac: psql (docker compose exec)";
    let output = Command::new("docker")
        .args(["compose", "exec", "-T", "pgstac", "psql", "-qtA", "-c", SQL])
        .output()
        .map_err(|e| Failure::new(CHECK, endpoint, "psql runs", e.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || stdout.trim() != "t" {
        return Err(Failure::new(
            CHECK,
            endpoint,
            "`t` (hls-s30 collection registered)",
            format!(
                "exit {:?}, stdout {:?}, stderr {:?}",
                output.status.code(),
                stdout.trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    pass(CHECK, "hls-s30 dataset registered (plain STAC visibility)");
    Ok(())
}

/// Was: the pre-drop `[ "$code" = "404" ]` in stack-up.sh — R1's honest
/// 404: the layer exists, its pixels don't.
fn tile_404_before_ingest() -> Result<(), Failure> {
    const CHECK: &str = "tile_404_before_ingest";
    let resp = get(CHECK, TILE)?;
    if resp.status != 404 {
        return Err(Failure::new(
            CHECK,
            TILE,
            "404 before any granule (catalog empty)",
            format!("{}", resp.status),
        ));
    }
    pass(CHECK, "tile is 404 before ingest (catalog empty)");
    Ok(())
}

/// Lifecycle stimulus, not an assertion: THE DROP, via the shared script
/// (single source of truth for the filedrop convention — band COGs
/// first, manifest renamed into place last).
fn drop_granule() -> Result<(), Failure> {
    const SCRIPT: &str = "tests/e2e/drop-granule.sh";
    let status = Command::new("bash")
        .arg(SCRIPT)
        .status()
        .map_err(|e| Failure::new("granule_drop", SCRIPT, "drop script runs", e.to_string()))?;
    if !status.success() {
        return Err(Failure::new(
            "granule_drop",
            SCRIPT,
            "exit 0",
            format!("exit {:?}", status.code()),
        ));
    }
    Ok(())
}

/// The first successful render: PNG bytes + parsed summary header.
struct FirstRender {
    body: Vec<u8>,
    header: TraceHeader,
}

/// Was: the poll loop in stack-up.sh (`120 × 0.5s` until 200) — arrive ->
/// catalog -> serve, automatically (R1).
fn tile_live_within_60s_of_drop() -> Result<FirstRender, Failure> {
    const CHECK: &str = "tile_live_within_60s_of_drop";
    let deadline = Instant::now() + Duration::from_mins(1);
    let resp = loop {
        let last = match http::get(TILE) {
            Ok(resp) if resp.status == 200 => break resp,
            Ok(resp) => resp.status.to_string(),
            Err(e) => e,
        };
        if Instant::now() >= deadline {
            return Err(Failure::new(
                CHECK,
                TILE,
                "200 within 60s of the drop",
                format!("last: {last}"),
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    fs::write(format!("{ARTIFACT_DIR}/tile.png"), &resp.body)
        .map_err(|e| Failure::new(CHECK, TILE, "tile artifact written", e.to_string()))?;
    let header = parse_trace_header(CHECK, TILE, &resp)?;
    pass(CHECK, "tile went live with zero manual steps (R1)");
    Ok(FirstRender {
        body: resp.body,
        header,
    })
}

/// Parses the `X-Swath-Trace` summary header into its typed form.
fn parse_trace_header(
    check: &'static str,
    endpoint: &str,
    resp: &http::Response,
) -> Result<TraceHeader, Failure> {
    let value = resp.header("x-swath-trace").ok_or_else(|| {
        Failure::new(
            check,
            endpoint,
            "an X-Swath-Trace header on the response",
            "header absent",
        )
    })?;
    serde_json::from_str(value).map_err(|e| {
        Failure::new(
            check,
            endpoint,
            "X-Swath-Trace parses as the summary JSON",
            format!("{value:?}: {e}"),
        )
    })
}

/// Was, in the justfile recipe: the trace-header grep, `bytes_read` sed,
/// `ingest_to_pixel_ms` sed + budget compare, and the `decision:"live"`
/// grep — four named checks over one typed header, plus the metrics
/// emission PERFORMANCE.md consumes.
fn first_render_checks(first: &FirstRender) -> Result<(), Failure> {
    pass(
        "first_render_carries_trace_header",
        format_args!(
            "X-Swath-Trace present and typed (total_ms={})",
            first.header.total_ms
        ),
    );

    if first.header.bytes_read == 0 {
        return Err(Failure::new(
            "first_render_reads_source_bytes",
            TILE,
            "trace bytes_read > 0 (real bytes read for this granule's pixels)",
            "bytes_read=0",
        ));
    }
    pass(
        "first_render_reads_source_bytes",
        format_args!(
            "trace provenance is non-empty (bytes_read={})",
            first.header.bytes_read
        ),
    );

    // THE NORTH-STAR NUMBER (REQUIREMENTS.md §3): the first tile
    // reflecting the just-ingested granule carries ingest-to-pixel
    // latency. Honest by construction: the first 200 is a fresh, uncached
    // render (the cache is empty until that render writes through), so
    // ingest-to-pixel is never measured on a hit — and its decision must
    // say so (checked below).
    let Some(i2p) = first.header.ingest_to_pixel_ms else {
        return Err(Failure::new(
            "ingest_to_pixel_under_budget",
            TILE,
            "a numeric ingest_to_pixel_ms in the trace header",
            "field absent",
        ));
    };
    emit_metrics(i2p)?;
    println!();
    println!("==========================================");
    println!("   INGEST-TO-PIXEL: {i2p} ms (budget {I2P_BUDGET_MS} ms)");
    println!("==========================================");
    println!();
    if i2p >= I2P_BUDGET_MS {
        return Err(Failure::new(
            "ingest_to_pixel_under_budget",
            TILE,
            format!("ingest_to_pixel_ms < {I2P_BUDGET_MS}"),
            format!("{i2p} ms"),
        ));
    }
    pass(
        "ingest_to_pixel_under_budget",
        format_args!("{i2p} ms is under the {I2P_BUDGET_MS} ms north-star budget"),
    );

    if first.header.decision != "live" {
        return Err(Failure::new(
            "first_render_decision_is_live",
            TILE,
            "decision \"live\" (fresh, uncached render)",
            format!("decision {:?}", first.header.decision),
        ));
    }
    pass(
        "first_render_decision_is_live",
        "north-star tile was a fresh live render (decision: live)",
    );
    Ok(())
}

/// The measured north-star number, machine-readable: one JSON line on
/// stdout and the same object at `target/e2e/metrics.json` — the
/// PERFORMANCE.md (M5) input.
fn emit_metrics(i2p: u64) -> Result<(), Failure> {
    let line = serde_json::json!({
        "metric": "ingest_to_pixel_ms",
        "value": i2p,
        "budget_ms": I2P_BUDGET_MS,
        "git_sha": git_sha(),
        "timestamp": iso8601_utc(SystemTime::now()),
    });
    println!("{line}");
    let path = format!("{ARTIFACT_DIR}/metrics.json");
    fs::write(&path, format!("{line}\n")).map_err(|e| {
        Failure::new(
            "metrics_artifact_written",
            path,
            "metrics.json written",
            e.to_string(),
        )
    })
}

/// The commit under test: CI's `GITHUB_SHA` when present, else the local
/// `git rev-parse HEAD`, else `"unknown"`.
fn git_sha() -> String {
    if let Ok(sha) = std::env::var("GITHUB_SHA")
        && !sha.is_empty()
    {
        return sha;
    }
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map_or_else(
            || "unknown".to_owned(),
            |out| String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        )
}

/// RFC 3339 / ISO 8601 UTC timestamp, seconds precision (civil-from-days,
/// no date dependency).
#[allow(
    clippy::many_single_char_names,
    reason = "the civil-from-days algorithm's conventional variable names"
)]
fn iso8601_utc(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = i64::try_from(days).expect("epoch days fit i64") + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Was: the second `curl` + `cmp` + `decision:"cache_hit"` grep — same
/// request, same bytes, now served from the write-through cache (#36).
fn second_fetch_is_cache_hit_with_identical_bytes(first_body: &[u8]) -> Result<(), Failure> {
    const CHECK: &str = "second_fetch_is_cache_hit_with_identical_bytes";
    let resp = get(CHECK, TILE)?;
    if resp.status != 200 {
        return Err(Failure::new(CHECK, TILE, "200", format!("{}", resp.status)));
    }
    if resp.body != first_body {
        return Err(Failure::new(
            CHECK,
            TILE,
            format!(
                "bytes identical to the first fetch ({} bytes)",
                first_body.len()
            ),
            format!("{} bytes, differing payload", resp.body.len()),
        ));
    }
    let header = parse_trace_header(CHECK, TILE, &resp)?;
    if header.decision != "cache_hit" {
        return Err(Failure::new(
            CHECK,
            TILE,
            "decision \"cache_hit\" on the repeat fetch",
            format!("decision {:?}", header.decision),
        ));
    }
    pass(
        CHECK,
        "second fetch served from the tile cache (decision: cache_hit, identical bytes)",
    );
    Ok(())
}

/// Perceptual comparison through the `swath-testkit` library (same
/// policy the pdiff bin defaults to).
fn pdiff_check(
    check: &'static str,
    endpoint: &str,
    served: &Path,
    golden: &str,
) -> Result<(), Failure> {
    let policy = DiffPolicy::default();
    let a = load_png(served)
        .map_err(|e| Failure::new(check, endpoint, "served tile decodes", e.to_string()))?;
    let b = load_png(Path::new(golden))
        .map_err(|e| Failure::new(check, endpoint, "golden decodes", e.to_string()))?;
    let report = diff(&a, &b)
        .map_err(|e| Failure::new(check, endpoint, "comparable dimensions", e.to_string()))?;
    if !report.passes(&policy) {
        return Err(Failure::new(
            check,
            endpoint,
            format!(
                "perceptual match vs {golden} (tolerance {}, max bad fraction {})",
                policy.per_channel_tolerance, policy.max_bad_pixel_fraction
            ),
            format!(
                "max |channel diff| {}, {} of {} pixels over tolerance ({:.4}%)",
                report.max_abs_channel_diff,
                report.pixels_exceeding_tolerance(policy.per_channel_tolerance),
                report.total_pixels(),
                report.pct_pixels_exceeding_tolerance(policy.per_channel_tolerance) * 100.0
            ),
        ));
    }
    Ok(())
}

/// Was: `cargo run -p swath-testkit --bin pdiff -- tile.png <golden>` —
/// the correctness oracle: a CORRECT tile is visible (§3), not just any
/// tile.
fn truecolor_matches_oracle_golden() -> Result<(), Failure> {
    const CHECK: &str = "truecolor_matches_oracle_golden";
    let served = format!("{ARTIFACT_DIR}/tile.png");
    pdiff_check(CHECK, TILE, Path::new(&served), TRUECOLOR_GOLDEN)?;
    pass(
        CHECK,
        "tile matches the rio-tiler/GDAL golden (default pdiff policy)",
    );
    Ok(())
}

/// The SSE window: subscribe first, then fetch the never-yet-rendered
/// NDVI tile (CHARTER.md §10 Phase 1: computed on the fly, not
/// pre-baked) and refetch the now-cached truecolor tile; every event is
/// deserialized into the typed envelope + core `Trace` and asserted by
/// variant, not by grep. Returns the served NDVI bytes for the openEO
/// round trip.
fn sse_and_ndvi_checks() -> Result<Vec<u8>, Failure> {
    const REFETCH: &str = "sse_reports_truecolor_keyed_cache_hit";
    let mut subscriber = sse::Subscriber::connect().map_err(|e| {
        Failure::new(
            "sse_delivers_typed_trace_events",
            "/traces",
            "an SSE subscription",
            e,
        )
    })?;

    let ndvi_bytes = ndvi_tile_renders_with_provenance()?;
    ndvi_matches_colormapped_self_golden()?;

    // Stimulus for the keyed cache-hit trace (#36): the truecolor tile is
    // cached by now, so this fetch must surface a cache_hit on the stream.
    let resp = get(REFETCH, TILE)?;
    if resp.status != 200 {
        return Err(Failure::new(
            REFETCH,
            TILE,
            "200 on the repeat fetch",
            format!("{}", resp.status),
        ));
    }

    let envelopes = collect_trace_envelopes(&mut subscriber)?;
    pass(
        "sse_delivers_typed_trace_events",
        format_args!("{} trace event(s) captured and typed", envelopes.len()),
    );

    let ndvi = envelopes
        .iter()
        .find(|e| e.layer == "ndvi" && e.tile == TILE_XYZ)
        .expect("collect loop returned only once the ndvi envelope was seen");
    if ndvi.trace.decision != Strategy::Live {
        return Err(Failure::new(
            "sse_reports_ndvi_live_render",
            "/traces",
            "ndvi trace decision Strategy::Live (computed on the fly)",
            format!("{:?}", ndvi.trace.decision),
        ));
    }
    pass(
        "sse_reports_ndvi_live_render",
        "SSE trace proves ndvi was computed on the fly (decision: live)",
    );

    let truecolor = envelopes
        .iter()
        .find(|e| e.layer == "truecolor" && e.tile == TILE_XYZ)
        .expect("collect loop returned only once the truecolor envelope was seen");
    match &truecolor.trace.decision {
        Strategy::CacheHit { key } if !key.is_empty() => pass(
            "sse_reports_truecolor_keyed_cache_hit",
            format_args!(
                "repeated tile was a keyed cache_hit (key {}…)",
                &key[..key.len().min(12)]
            ),
        ),
        other => {
            return Err(Failure::new(
                "sse_reports_truecolor_keyed_cache_hit",
                "/traces",
                "truecolor trace decision Strategy::CacheHit with a non-empty key",
                format!("{other:?}"),
            ));
        }
    }

    if !envelopes
        .iter()
        .any(|e| e.trace.ingest_to_pixel_ms.is_some())
    {
        return Err(Failure::new(
            "sse_carries_ingest_to_pixel",
            "/traces",
            "at least one captured trace with ingest_to_pixel_ms",
            "no captured trace carries the field",
        ));
    }
    pass(
        "sse_carries_ingest_to_pixel",
        "SSE trace carries ingest_to_pixel_ms",
    );

    Ok(ndvi_bytes)
}

/// Reads typed `trace` envelopes until both expected renders (ndvi live
/// window + truecolor refetch) have arrived. `lagged` events are
/// tolerated (as the bash capture was); keepalives are skipped.
fn collect_trace_envelopes(
    subscriber: &mut sse::Subscriber,
) -> Result<Vec<TraceEnvelope>, Failure> {
    const CHECK: &str = "sse_delivers_typed_trace_events";
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut envelopes: Vec<TraceEnvelope> = Vec::new();
    loop {
        let seen = |layer: &str| {
            envelopes
                .iter()
                .any(|e| e.layer == layer && e.tile == TILE_XYZ)
        };
        if seen("ndvi") && seen("truecolor") {
            return Ok(envelopes);
        }
        let frame = subscriber.next_frame(deadline).map_err(|e| {
            Failure::new(
                CHECK,
                "/traces",
                "trace events for the ndvi render and the truecolor cache hit within 15s",
                format!(
                    "{e}; captured so far: {:?}",
                    envelopes
                        .iter()
                        .map(|e| format!("{}/{:?}", e.layer, e.trace.decision))
                        .collect::<Vec<_>>()
                ),
            )
        })?;
        if frame.is_keepalive() || frame.event.as_deref() == Some("lagged") {
            continue;
        }
        if frame.event.as_deref() != Some("trace") {
            return Err(Failure::new(
                CHECK,
                "/traces",
                "only trace/lagged/keepalive frames",
                format!("event {:?}", frame.event),
            ));
        }
        let data = frame.data.join("\n");
        let envelope: TraceEnvelope = serde_json::from_str(&data).map_err(|e| {
            Failure::new(
                CHECK,
                "/traces",
                "trace data deserializes as the envelope around a core Trace",
                format!("{e}; data: {data}"),
            )
        })?;
        envelopes.push(envelope);
    }
}

/// Was: the ndvi `curl` (`200`), trace-header grep, and `bytes_read` sed
/// — HTTP side of the on-the-fly proof. Returns the served bytes.
fn ndvi_tile_renders_with_provenance() -> Result<Vec<u8>, Failure> {
    const CHECK: &str = "ndvi_tile_renders_with_provenance";
    let resp = get(CHECK, NDVI)?;
    if resp.status != 200 {
        return Err(Failure::new(CHECK, NDVI, "200", format!("{}", resp.status)));
    }
    let header = parse_trace_header(CHECK, NDVI, &resp)?;
    if header.bytes_read == 0 {
        return Err(Failure::new(
            CHECK,
            NDVI,
            "trace bytes_read > 0 on the ndvi render",
            "bytes_read=0",
        ));
    }
    fs::write(format!("{ARTIFACT_DIR}/ndvi.png"), &resp.body)
        .map_err(|e| Failure::new(CHECK, NDVI, "ndvi artifact written", e.to_string()))?;
    pass(
        CHECK,
        format_args!(
            "ndvi 200 with non-empty provenance (bytes_read={})",
            header.bytes_read
        ),
    );
    Ok(resp.body)
}

/// Was: `cmp ndvi.png <colormapped golden>` — level 2 of the two-level
/// NDVI golden scheme (issue #94): the served (colormapped) tile is
/// byte-identical to the committed self-golden; level 1 (values vs the
/// grayscale GDAL oracle) is asserted via the authored grayscale service.
fn ndvi_matches_colormapped_self_golden() -> Result<(), Failure> {
    const CHECK: &str = "ndvi_matches_colormapped_self_golden";
    let served = fs::read(format!("{ARTIFACT_DIR}/ndvi.png"))
        .map_err(|e| Failure::new(CHECK, NDVI, "served ndvi artifact readable", e.to_string()))?;
    let golden = fs::read(NDVI_COLORMAPPED_GOLDEN).map_err(|e| {
        Failure::new(
            CHECK,
            NDVI,
            "committed colormapped golden readable",
            e.to_string(),
        )
    })?;
    if served != golden {
        return Err(Failure::new(
            CHECK,
            NDVI,
            format!(
                "bytes identical to {NDVI_COLORMAPPED_GOLDEN} ({} bytes)",
                golden.len()
            ),
            format!("{} bytes, differing payload", served.len()),
        ));
    }
    pass(
        CHECK,
        "ndvi tile is byte-identical to the committed colormapped golden (#94, level 2)",
    );
    Ok(())
}

/// The openEO NDVI process graph (issue #41, ADR 0010, R3), with the
/// colormap named through the openEO representation (`save_result`
/// options, issue #94).
fn ndvi_graph(title: &str, colormap: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "xyz",
        "title": title,
        "process": {"process_graph": {
            "load": {"process_id": "load_collection", "arguments": {
                "id": "hls-s30", "spatial_extent": null, "temporal_extent": null,
                "bands": ["b8a", "b04"]}},
            "ndvi": {"process_id": "ndvi", "arguments": {
                "data": {"from_node": "load"}, "nir": "b8a", "red": "b04"}},
            "scale": {"process_id": "linear_scale_range", "arguments": {
                "x": {"from_node": "ndvi"},
                "inputMin": -1, "inputMax": 1, "outputMin": 0, "outputMax": 255}},
            "save": {"process_id": "save_result", "arguments": {
                "data": {"from_node": "scale"}, "format": "png",
                "options": {"colormap": colormap}}, "result": true}
        }}
    })
}

/// Publishes a service and returns its id — was: `POST /services` +
/// `201` check + `OpenEO-Identifier` sed.
fn publish_service(check: &'static str, title: &str, colormap: &str) -> Result<String, Failure> {
    let resp = http::post_json("/services", &ndvi_graph(title, colormap))
        .map_err(|e| Failure::new(check, "/services", "an HTTP response", e))?;
    if resp.status != 201 {
        return Err(Failure::new(
            check,
            "/services",
            "201 Created",
            format!(
                "{} with body {:?}",
                resp.status,
                String::from_utf8_lossy(&resp.body)
            ),
        ));
    }
    let sid = resp
        .header("openeo-identifier")
        .unwrap_or_default()
        .to_owned();
    if sid.is_empty() {
        return Err(Failure::new(
            check,
            "/services",
            "a non-empty OpenEO-Identifier header on the 201",
            "header absent or empty",
        ));
    }
    Ok(sid)
}

/// Was: the two service publications + tile fetches + `cmp`/pdiff — the
/// openEO authoring loop: graph in, live XYZ out, same compiler, same
/// serve path, zero manual steps; plus level 1 of the #94 scheme (the
/// colormapped tile is a palette over oracle-validated VALUES).
fn openeo_checks(ndvi_bytes: &[u8]) -> Result<(), Failure> {
    const PUBLISH: &str = "openeo_service_publishes_ndvi_graph";
    const MATCH: &str = "authored_service_tile_matches_builtin_ndvi";
    const GRAY: &str = "grayscale_service_matches_oracle_golden";
    let sid = publish_service(PUBLISH, "NDVI (authored)", "rdylgn")?;
    pass(PUBLISH, format_args!("openEO service published ({sid})"));

    let path = format!("/tilesets/{sid}/tiles/12/1561/848");
    let resp = get(MATCH, &path)?;
    if resp.status != 200 {
        return Err(Failure::new(
            MATCH,
            &path,
            "200",
            format!("{}", resp.status),
        ));
    }
    if resp.body != ndvi_bytes {
        return Err(Failure::new(
            MATCH,
            &path,
            format!(
                "bytes identical to the built-in NDVI tile ({} bytes)",
                ndvi_bytes.len()
            ),
            format!("{} bytes, differing payload", resp.body.len()),
        ));
    }
    pass(
        MATCH,
        "authored service tile is byte-identical to the built-in NDVI (graph in, live XYZ out)",
    );

    let gsid = publish_service(GRAY, "NDVI (authored, grayscale)", "grayscale")?;
    let gpath = format!("/tilesets/{gsid}/tiles/12/1561/848");
    let resp = get(GRAY, &gpath)?;
    if resp.status != 200 {
        return Err(Failure::new(
            GRAY,
            &gpath,
            "200",
            format!("{}", resp.status),
        ));
    }
    let served = format!("{ARTIFACT_DIR}/gray-service.png");
    fs::write(&served, &resp.body)
        .map_err(|e| Failure::new(GRAY, &gpath, "artifact written", e.to_string()))?;
    pdiff_check(GRAY, &gpath, Path::new(&served), NDVI_GRAYSCALE_GOLDEN)?;
    pass(
        GRAY,
        "ndvi values match the rio-tiler/GDAL golden (#94, level 1: grayscale service, default pdiff policy)",
    );
    Ok(())
}

/// Was: the `python3 -c` one-liner over `/tilesets/ndvi` — the DECLARED
/// bounds must contain the tile just proven correct (a wrong granule
/// bbox once put the demo viewport 48 km from the imagery while every
/// pixel test stayed green).
fn tileset_bounds_contain_proven_tile() -> Result<(), Failure> {
    const CHECK: &str = "tileset_bounds_contain_proven_tile";
    const ENDPOINT: &str = "/tilesets/ndvi";
    let resp = get(CHECK, ENDPOINT)?;
    if resp.status != 200 {
        return Err(Failure::new(
            CHECK,
            ENDPOINT,
            "200",
            format!("{}", resp.status),
        ));
    }
    let meta: TilesetMeta = serde_json::from_slice(&resp.body).map_err(|e| {
        Failure::new(
            CHECK,
            ENDPOINT,
            "tileset metadata with a boundingBox",
            e.to_string(),
        )
    })?;
    let [west, south] = meta.bounding_box.lower_left;
    let [east, north] = meta.bounding_box.upper_right;
    let (lon, lat) = PROBE_LON_LAT;
    if !(west <= lon && lon <= east && south <= lat && lat <= north) {
        return Err(Failure::new(
            CHECK,
            ENDPOINT,
            format!("declared bbox containing the proven tile's center ({lon}, {lat})"),
            format!("bbox [{west}, {south}, {east}, {north}]"),
        ));
    }
    pass(
        CHECK,
        format_args!(
            "declared tileset bounds contain the proven tile ([{west}, {south}, {east}, {north}])"
        ),
    );
    Ok(())
}
