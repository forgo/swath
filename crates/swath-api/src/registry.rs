// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The layer registry: which named layers this API serves, and how each
//! one renders.
//!
//! A [`Layer`] is a [`TileRequest`] template — band assets + compiled
//! plan + resampling — plus the human-facing identity (id, title,
//! description) the OGC documents expose. Users see layer ids and nothing
//! else (REQUIREMENTS.md R2).
//!
//! **Scope:** the registry is in-memory and immutable, constructed once at
//! startup — the `--fixtures`/config-file serving mode. Catalog-backed
//! serving (issue #31) lives beside it as
//! [`CatalogLayers`](crate::provider::CatalogLayers); both plug into the
//! handlers through the [`LayerProvider`](crate::provider::LayerProvider)
//! seam.

use std::collections::BTreeMap;

use swath_core::raster::AssetRef;
use swath_core::tile::TileCoord;
use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{NodataPolicy, Resampling, TileRequest};

/// One servable layer: identity for the OGC documents, and the render
/// template `render_tile` consumes.
#[derive(Debug, Clone)]
pub struct Layer {
    /// URL-safe identifier — the `{layerId}` path segment.
    pub id: String,
    /// Human-readable title (tileset metadata `title`).
    pub title: String,
    /// Short narrative description (tileset metadata `description`).
    pub description: String,
    /// Asset backing each band name the plan declares.
    pub bands: BTreeMap<String, AssetRef>,
    /// The pixel pipeline rendering this layer.
    pub plan: RenderPlan,
    /// Resampling kernel for every band's warp.
    pub resampling: Resampling,
    /// Tile side length in pixels.
    pub tile_size: u32,
}

impl Layer {
    /// The [`TileRequest`] rendering `coord` of this layer.
    #[must_use]
    pub fn tile_request(&self, coord: TileCoord) -> TileRequest {
        TileRequest::new(
            self.bands.clone(),
            self.plan.clone(),
            coord,
            self.tile_size,
            self.resampling,
        )
    }
}

/// The set of layers this API instance serves, keyed by layer id.
///
/// `BTreeMap` for deterministic iteration order — the tilesets list is
/// stable across requests and across runs.
#[derive(Debug, Clone, Default)]
pub struct LayerRegistry {
    layers: BTreeMap<String, Layer>,
}

impl LayerRegistry {
    /// A registry over `layers`, keyed by their ids. A duplicate id keeps
    /// the last layer (construction is programmatic; duplicates are a
    /// caller bug the catalog adapter will reject upstream).
    #[must_use]
    pub fn new(layers: impl IntoIterator<Item = Layer>) -> Self {
        Self {
            layers: layers
                .into_iter()
                .map(|layer| (layer.id.clone(), layer))
                .collect(),
        }
    }

    /// The layer with this id, if registered.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Layer> {
        self.layers.get(id)
    }

    /// All layers, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Layer> {
        self.layers.values()
    }

    /// The built-in demo registry over the committed HLS fixture COGs:
    /// `truecolor` (B04/B03/B02 composite, rescale 0..3000) and `ndvi`
    /// (`(b8a - b04) / (b8a + b04)`, rescale -1..1, grayscale) — the same
    /// plans the render golden suites pin against the GDAL/rio-tiler
    /// oracle (issues #25/#26), so an API-served tile is byte-comparable
    /// to a direct render.
    ///
    /// Asset refs are the bare fixture file names; the caller decides
    /// where they live by choosing the `RasterSource`'s store root (the
    /// tests point a local store at `tests/fixtures/`).
    #[must_use]
    pub fn hls_fixtures() -> Self {
        /// The reflectance-band kernel of the golden suites.
        const BILINEAR: Resampling = Resampling::Bilinear(NodataPolicy::ExcludeRenormalize);
        let asset = |name: &str| AssetRef::new(format!("hlss30-t13sdd-2024158-{name}.tif"));

        let truecolor = Layer {
            id: "truecolor".to_owned(),
            title: "HLS true color".to_owned(),
            description: "HLS S30 T13SDD 2024-158 B04/B03/B02 composite, \
                          reflectance rescaled 0..3000."
                .to_owned(),
            bands: [
                ("b04".to_owned(), asset("b04")),
                ("b03".to_owned(), asset("b03")),
                ("b02".to_owned(), asset("b02")),
            ]
            .into(),
            plan: RenderPlan::new(
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
            ),
            resampling: BILINEAR,
            tile_size: 256,
        };

        let ndvi = Layer {
            id: "ndvi".to_owned(),
            title: "HLS NDVI".to_owned(),
            description: "HLS S30 T13SDD 2024-158 NDVI ((B8A - B04) / (B8A + B04)), \
                          rescaled -1..1, grayscale."
                .to_owned(),
            bands: [
                ("b8a".to_owned(), asset("b8a")),
                ("b04".to_owned(), asset("b04")),
            ]
            .into(),
            plan: RenderPlan::new(
                vec![BandInput::new("b8a"), BandInput::new("b04")],
                vec![
                    PixelOp::BandMath(
                        (Expr::band("b8a") - Expr::band("b04"))
                            / (Expr::band("b8a") + Expr::band("b04")),
                    ),
                    PixelOp::Rescale {
                        min: -1.0,
                        max: 1.0,
                    },
                    PixelOp::Colormap(Colormap::Grayscale),
                ],
                OutputSpec::new(TileFormat::Png),
            ),
            resampling: BILINEAR,
            tile_size: 256,
        };

        Self::new([truecolor, ndvi])
    }
}

#[cfg(test)]
mod tests {
    use super::LayerRegistry;

    #[test]
    fn fixture_registry_serves_both_demo_layers_in_id_order() {
        let registry = LayerRegistry::hls_fixtures();
        let ids: Vec<&str> = registry.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["ndvi", "truecolor"]);
        assert!(registry.get("truecolor").is_some());
        assert!(registry.get("nope").is_none());
    }

    #[test]
    fn layer_template_produces_a_render_request() {
        let registry = LayerRegistry::hls_fixtures();
        let layer = registry.get("truecolor").unwrap();
        let coord = swath_core::tile::TileCoord::new(12, 848, 1561).unwrap();
        let request = layer.tile_request(coord);
        assert_eq!(request.coord, coord);
        assert_eq!(request.tile_size, 256);
        assert_eq!(request.bands.len(), 3);
    }
}
