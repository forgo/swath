// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! OGC conformance smoke tests (REQUIREMENTS.md R5): every JSON document
//! the API serves is validated against the committed **official** OGC
//! schemas (`tests/data/ogc/`, provenance in its README), and the
//! standard's structural requirements — required links, required list
//! fields, the honest conformance declaration — are asserted directly.

#[allow(
    dead_code,
    reason = "shared between the API test targets; not every helper is used in each"
)]
mod common;

use axum::http::StatusCode;
use swath_api::CONFORMANCE_CLASSES;

const TILES_CORE: &str = "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/core";
const TILING_SCHEME_REL: &str = "http://www.opengis.net/def/rel/ogc/1.0/tiling-scheme";
const TILESETS_MAP_REL: &str = "http://www.opengis.net/def/rel/ogc/1.0/tilesets-map";
const WEB_MERCATOR_QUAD_URI: &str =
    "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad";

fn links_of(document: &serde_json::Value) -> &Vec<serde_json::Value> {
    document["links"].as_array().expect("document has links")
}

fn link_with_rel<'a>(document: &'a serde_json::Value, rel: &str) -> &'a serde_json::Value {
    links_of(document)
        .iter()
        .find(|link| link["rel"] == rel)
        .unwrap_or_else(|| panic!("no link with rel `{rel}`"))
}

async fn json_ok(path: &str) -> serde_json::Value {
    let response = common::get(path).await;
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    assert_eq!(
        response.headers()["content-type"],
        "application/json",
        "GET {path} content type"
    );
    common::body_json(response).await
}

// --- Landing page (OGC API - Common shapes) ---

#[tokio::test]
async fn landing_page_is_schema_valid_and_links_the_required_resources() {
    let landing = json_ok("/").await;
    common::assert_valid("common/landingPage.json", &landing);

    // The official landingPage.json misspells its link ref ("$href" —
    // README), so validate every link against the link schema directly.
    for link in links_of(&landing) {
        common::assert_valid("common/link.json", link);
    }

    assert_eq!(link_with_rel(&landing, "self")["href"], "http://localhost/");
    assert_eq!(
        link_with_rel(&landing, "conformance")["href"],
        "http://localhost/conformance"
    );
    // Dataset tilesets advertised from the root (OGC 20-057
    // /req/dataset-tilesets/landingpage), at the …/tiles path the
    // standard requires (/req/dataset-tilesets/operation).
    assert_eq!(
        link_with_rel(&landing, TILESETS_MAP_REL)["href"],
        "http://localhost/tiles"
    );
}

// --- Conformance declaration ---

#[tokio::test]
async fn conformance_is_schema_valid_and_declares_exactly_what_is_implemented() {
    let conformance = json_ok("/conformance").await;
    common::assert_valid("common/confClasses.json", &conformance);

    let declared: Vec<&str> = conformance["conformsTo"]
        .as_array()
        .expect("conformsTo is an array")
        .iter()
        .map(|class| class.as_str().expect("class is a string"))
        .collect();

    // The declaration is the implemented set — nothing more (honesty:
    // classes we serve partial shapes of, like OGC API Common Core, are
    // not declared), nothing less (Tiles /req/core/conformance-success
    // requires listing the supported Tiles classes, Core included).
    assert_eq!(declared, CONFORMANCE_CLASSES);
    assert!(declared.contains(&TILES_CORE));
    assert!(
        !declared
            .iter()
            .any(|class| class.contains("ogcapi-common-1")),
        "OGC API Common Core must not be declared: no OpenAPI definition is served"
    );
}

// --- Tilesets list ---

#[tokio::test]
async fn tilesets_list_is_schema_valid_with_the_required_subset_per_element() {
    let list = json_ok("/tilesets").await;

    // Same representation at the standard's dataset path (…/tiles).
    assert_eq!(list, json_ok("/tiles").await);

    let tilesets = list["tilesets"].as_array().expect("tilesets array");
    assert_eq!(tilesets.len(), 2, "one tileset per fixture layer");

    for item in tilesets {
        // Each element carries a subset of tileset metadata; ours also
        // satisfies the full official tileset schema (dataType + crs are
        // present), so hold it to that bar.
        common::assert_valid("tms/tileSet.json", item);

        // The required subset (/req/tilesets-list/tileset-links):
        // dataType, crs, tileMatrixSetURI, self + tiling-scheme links.
        assert_eq!(item["dataType"], "map");
        assert!(item["crs"].is_string());
        assert_eq!(item["tileMatrixSetURI"], WEB_MERCATOR_QUAD_URI);
        let self_link = link_with_rel(item, "self");
        assert_eq!(self_link["type"], "application/json");
        assert!(
            self_link["href"]
                .as_str()
                .expect("self href")
                .starts_with("http://localhost/tilesets/"),
            "self links point at the full tileset metadata"
        );
        assert_eq!(
            link_with_rel(item, TILING_SCHEME_REL)["href"],
            WEB_MERCATOR_QUAD_URI
        );
    }
}

// --- Tileset metadata ---

#[tokio::test]
async fn tileset_metadata_is_schema_valid_with_tms_uri_bounds_and_tile_template() {
    for layer in ["truecolor", "ndvi"] {
        let tileset = json_ok(&format!("/tilesets/{layer}")).await;
        common::assert_valid("tms/tileSet.json", &tileset);

        assert_eq!(tileset["dataType"], "map");
        // Registered TMS ⇒ tileMatrixSetURI required
        // (/req/tileset/description C) and a tiling-scheme link (D).
        assert_eq!(tileset["tileMatrixSetURI"], WEB_MERCATOR_QUAD_URI);
        assert_eq!(
            link_with_rel(&tileset, TILING_SCHEME_REL)["href"],
            WEB_MERCATOR_QUAD_URI
        );

        // Templated tile link with rel item and the three OGC variables
        // (E), typed image/png (F).
        let item = link_with_rel(&tileset, "item");
        assert_eq!(item["templated"], true);
        assert_eq!(item["type"], "image/png");
        assert_eq!(
            item["href"],
            format!(
                "http://localhost/tilesets/{layer}/tiles/{{tileMatrix}}/{{tileRow}}/{{tileCol}}"
            )
        );

        // Bounds derived from the described fixture assets, in CRS84:
        // the HLS T13SDD granule window sits in UTM 13N over Colorado.
        let bbox = &tileset["boundingBox"];
        let (west, south) = (
            bbox["lowerLeft"][0].as_f64().expect("west"),
            bbox["lowerLeft"][1].as_f64().expect("south"),
        );
        let (east, north) = (
            bbox["upperRight"][0].as_f64().expect("east"),
            bbox["upperRight"][1].as_f64().expect("north"),
        );
        assert!(
            (-107.0..-104.0).contains(&west) && west < east && east < -104.0,
            "fixture longitudes: west {west}, east {east}"
        );
        assert!(
            (38.0..40.0).contains(&south) && south < north && north < 40.0,
            "fixture latitudes: south {south}, north {north}"
        );
    }
}

// --- Exception shapes ---

#[tokio::test]
async fn unknown_layer_is_a_schema_valid_404_exception() {
    for path in ["/tilesets/missing", "/tilesets/missing/tiles/12/1561/848"] {
        let response = common::get(path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "GET {path}");
        let exception = common::body_json(response).await;
        common::assert_valid("common/exception.json", &exception);
        assert_eq!(exception["status"], 404);
    }
}

#[tokio::test]
async fn malformed_tile_coordinates_are_schema_valid_400_exceptions() {
    let response = common::get("/tilesets/truecolor/tiles/12/abc/848").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let exception = common::body_json(response).await;
    common::assert_valid("common/exception.json", &exception);
    assert_eq!(exception["status"], 400);
}

// --- Operational endpoints (non-OGC) ---

/// The liveness probe (#29): plain 200 `ok`, no JSON, no data-plane I/O —
/// the contract container healthchecks depend on.
#[tokio::test]
async fn healthz_is_plain_200_ok() {
    let response = common::get("/healthz").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::body_bytes(response).await, b"ok");
}
