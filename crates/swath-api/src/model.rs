// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! OGC JSON document shapes: the serialized forms of the landing page,
//! conformance declaration, tilesets list, and tileset metadata.
//!
//! Field names and structure follow the official OGC schemas the
//! conformance suite validates against (`tests/data/ogc/`): OGC API -
//! Common Part 1 for landing page / conformance / links, and the OGC Two
//! Dimensional Tile Matrix Set and Tile Set Metadata Standard 2.0 (OGC
//! 17-083r4) for tileset metadata — the model OGC API - Tiles 1.0
//! `/req/tileset/description` requires. Only the fields Swath actually
//! populates exist here; the standard's optional fields are added when a
//! real need arrives, not speculatively.

/// A typed web link (RFC 8288 shape, OGC link schema).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Link {
    /// Target URI (or URI template when [`templated`](Self::templated)).
    pub href: String,
    /// Relation type.
    pub rel: String,
    /// Media type hint for the target.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// Human-readable label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `true` when `href` is a URI template (tile link templates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub templated: Option<bool>,
}

impl Link {
    /// A plain (non-templated, untyped) link.
    #[must_use]
    pub fn new(href: impl Into<String>, rel: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            rel: rel.into(),
            media_type: None,
            title: None,
            templated: None,
        }
    }

    /// Sets the media-type hint.
    #[must_use]
    pub fn media_type(mut self, media_type: impl Into<String>) -> Self {
        self.media_type = Some(media_type.into());
        self
    }

    /// Sets the human-readable title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Marks the link as a URI template.
    #[must_use]
    pub fn templated(mut self) -> Self {
        self.templated = Some(true);
        self
    }
}

/// The landing page (OGC API - Common `landingPage.json`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LandingPage {
    /// API title.
    pub title: String,
    /// API description.
    pub description: String,
    /// Links to the resources this API exposes.
    pub links: Vec<Link>,
}

/// The conformance declaration (OGC API - Common `confClasses.json`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Conformance {
    /// URIs of the conformance classes this API instance implements.
    #[serde(rename = "conformsTo")]
    pub conforms_to: Vec<String>,
}

/// One element of the tilesets list — the required subset of tileset
/// metadata per OGC API - Tiles `/req/tilesets-list/tileset-links`:
/// `dataType`, `crs`, `tileMatrixSetURI`, and links (self + tiling
/// scheme).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TileSetItem {
    /// Tileset title.
    pub title: String,
    /// Type of data represented (always `"map"` here: rendered images).
    #[serde(rename = "dataType")]
    pub data_type: String,
    /// CRS of the tileset (URI form).
    pub crs: String,
    /// Registered tile matrix set URI (`WebMercatorQuad`).
    #[serde(rename = "tileMatrixSetURI")]
    pub tile_matrix_set_uri: String,
    /// Links: `self` to the full metadata, tiling scheme to the TMS
    /// definition.
    pub links: Vec<Link>,
}

/// The tilesets list: `{"tilesets": [...]}` per
/// `/req/tilesets-list/tileset-links`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TileSetList {
    /// Available tilesets, one per layer, in layer-id order.
    pub tilesets: Vec<TileSetItem>,
}

/// A 2D bounding box (OGC 17-083r4 `2DBoundingBox.json`): lower-left and
/// upper-right corners in the CRS named by `crs`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BoundingBox2D {
    /// `[west, south]` (CRS84 axis order: longitude, latitude).
    #[serde(rename = "lowerLeft")]
    pub lower_left: [f64; 2],
    /// `[east, north]`.
    #[serde(rename = "upperRight")]
    pub upper_right: [f64; 2],
    /// CRS the corners are expressed in.
    pub crs: String,
    /// Axis labels, in coordinate order.
    #[serde(rename = "orderedAxes")]
    pub ordered_axes: [String; 2],
}

/// Full tileset metadata (OGC 17-083r4 `tileSet.json`): the list-item
/// subset plus description, data bounds, and the templated tile link.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TileSetMetadata {
    /// Tileset title.
    pub title: String,
    /// Narrative description.
    pub description: String,
    /// Type of data represented (always `"map"` here).
    #[serde(rename = "dataType")]
    pub data_type: String,
    /// CRS of the tileset (URI form).
    pub crs: String,
    /// Registered tile matrix set URI (`WebMercatorQuad`).
    #[serde(rename = "tileMatrixSetURI")]
    pub tile_matrix_set_uri: String,
    /// Geographic extent of the layer's source data (CRS84), derived from
    /// the assets' described footprints.
    #[serde(rename = "boundingBox")]
    pub bounding_box: BoundingBox2D,
    /// Links: self, tiling scheme, and the templated `item` tile link.
    pub links: Vec<Link>,
    /// The layer's frame-selection window (ADR 0015; the hull of the
    /// branch windows for a two-source layer, ADR 0022) as
    /// `[start, end]`, either side `null` when open — what bounds the
    /// frames a client may ask for. Catalog-backed layers only.
    #[serde(
        rename = "swath:window",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub window: Option<[Option<String>; 2]>,
    /// How many `load_collection` branches the layer reads (ADR 0022):
    /// a two-source layer resolves one granule per branch, so its
    /// frames are not one dataset's granule listing. Catalog-backed
    /// layers only.
    #[serde(
        rename = "swath:sources",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub sources: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::Link;

    #[test]
    fn link_serializes_with_ogc_field_names_and_omits_absent_options() {
        let plain = serde_json::to_value(Link::new("/x", "self")).unwrap();
        assert_eq!(plain, serde_json::json!({"href": "/x", "rel": "self"}));

        let full = serde_json::to_value(
            Link::new("/t/{tileMatrix}", "item")
                .media_type("image/png")
                .title("tiles")
                .templated(),
        )
        .unwrap();
        assert_eq!(full["type"], "image/png");
        assert_eq!(full["templated"], true);
    }
}
