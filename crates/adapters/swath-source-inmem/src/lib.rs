// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `RasterSource` adapter for a deterministic in-memory raster — the
//! **docs/EXTENDING.md walkthrough toy** (issue #125), built by following
//! the guide's "new source adapter" steps verbatim to prove them complete.
//! This crate lives on an unmerged evidence branch and is never shipped.
//!
//! # Asset addressing: `inmem:<name>`
//!
//! [`InMemSource::handles`] recognizes the `inmem:` scheme, mirroring how
//! the binary's `CompositeSource` dispatches per asset (its module docs);
//! [`InMemSource::demo`] serves exactly one asset, `inmem:demo`.
//!
//! # The demo raster (the truth table's fixture)
//!
//! A 6×4 `UInt8` grid in Web Mercator, north-up unit pixels anchored at
//! the origin, nodata sentinel `255`. Sample values are the pure function
//!
//! ```text
//! v(row, col) = row * 16 + col * 3,   except (1, 2) and (2, 4) = 255
//! ```
//!
//! — the same formula `tests/oracle/inmem_truth.py` evaluates with numpy
//! to generate `tests/data/window_truth.json`, so the committed truth is
//! derived independently of this Rust code (the oracle pattern,
//! ADR 0002).
//!
//! # Port-contract obligations exercised here
//!
//! * Reads **clip** to the grid (out-of-bounds requests return the
//!   intersection, possibly empty).
//! * [`Provenance`] reports the byte ranges actually copied out of the
//!   in-memory buffer — one contiguous range per touched row, offsets in
//!   sample bytes — and `bytes_read` is their sum.
//! * No overviews: `describe` reports `overview_levels: []` and an
//!   overview read is a typed [`SourceError::OverviewNotFound`].
//! * The error taxonomy is the port's: unknown assets are
//!   [`SourceError::NotFound`], bad bands [`SourceError::BandOutOfRange`].

use std::collections::BTreeMap;

use swath_core::crs::Crs;
use swath_core::raster::{AssetRef, DType, GeoTransform, RasterInfo, WindowRequest};
use swath_core::source::{
    BandSelection, PixelBuffer, RasterSource, ReadLevel, SourceError, WindowData,
};
use swath_core::trace::Provenance;

/// The demo grid's nodata sentinel.
const NODATA: u8 = 255;

/// One in-memory raster: its grid description plus row-major samples.
#[derive(Debug, Clone)]
struct Raster {
    info: RasterInfo,
    samples: Vec<u8>,
}

/// An in-memory `RasterSource`: named synthetic rasters, `inmem:` scheme.
#[derive(Debug, Clone, Default)]
pub struct InMemSource {
    rasters: BTreeMap<String, Raster>,
}

impl InMemSource {
    /// Whether `asset` is addressed to this adapter (the `inmem:` scheme) —
    /// the dispatch hook the binary's composite source calls.
    #[must_use]
    pub fn handles(asset: &AssetRef) -> bool {
        asset.as_str().starts_with("inmem:")
    }

    /// The source holding the single documented demo raster, `inmem:demo`
    /// (grid and value formula: module docs).
    #[must_use]
    pub fn demo() -> Self {
        const WIDTH: u64 = 6;
        const HEIGHT: u64 = 4;
        let mut samples = Vec::with_capacity(usize::try_from(WIDTH * HEIGHT).expect("tiny grid"));
        for row in 0..HEIGHT {
            for col in 0..WIDTH {
                let v = if (row, col) == (1, 2) || (row, col) == (2, 4) {
                    u64::from(NODATA)
                } else {
                    row * 16 + col * 3
                };
                samples.push(u8::try_from(v).expect("demo values fit u8"));
            }
        }
        let info = RasterInfo {
            crs: Crs::WEB_MERCATOR,
            width: WIDTH,
            height: HEIGHT,
            transform: GeoTransform::north_up(0.0, 0.0, 1.0, -1.0),
            band_count: 1,
            dtype: DType::UInt8,
            nodata: Some(f64::from(NODATA)),
            overview_levels: vec![],
        };
        let mut rasters = BTreeMap::new();
        rasters.insert("inmem:demo".to_owned(), Raster { info, samples });
        Self { rasters }
    }

    /// Looks up an asset or produces the port's `NotFound`.
    fn raster(&self, asset: &AssetRef) -> Result<&Raster, SourceError> {
        self.rasters
            .get(asset.as_str())
            .ok_or_else(|| SourceError::NotFound {
                asset: asset.clone(),
            })
    }
}

impl RasterSource for InMemSource {
    async fn describe(&self, asset: &AssetRef) -> Result<RasterInfo, SourceError> {
        Ok(self.raster(asset)?.info.clone())
    }

    async fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
        level: ReadLevel,
    ) -> Result<WindowData, SourceError> {
        let raster = self.raster(asset)?;

        match band {
            BandSelection::Single(0) => {}
            BandSelection::Single(b) => {
                return Err(SourceError::BandOutOfRange {
                    asset: asset.clone(),
                    band: b,
                    band_count: raster.info.band_count,
                });
            }
            _ => {
                return Err(SourceError::Unsupported {
                    asset: asset.clone(),
                    detail: "only single-band selection is supported".to_owned(),
                });
            }
        }
        if let ReadLevel::Overview { factor } = level {
            return Err(SourceError::OverviewNotFound {
                asset: asset.clone(),
                factor,
                available: raster.info.overview_levels.clone(),
            });
        }

        // Clip the request to the grid (the port's clipping contract).
        let col0 = window.col_off.min(raster.info.width);
        let row0 = window.row_off.min(raster.info.height);
        let col1 = window
            .col_off
            .saturating_add(window.width)
            .min(raster.info.width);
        let row1 = window
            .row_off
            .saturating_add(window.height)
            .min(raster.info.height);
        let clipped = WindowRequest {
            col_off: col0,
            row_off: row0,
            width: col1 - col0,
            height: row1 - row0,
        };

        // Copy row by row, recording each copy as a real observed "fetch".
        let mut pixels =
            Vec::with_capacity(usize::try_from(clipped.width * clipped.height).expect("tiny grid"));
        let mut provenance = Vec::new();
        for row in row0..row1 {
            let start = usize::try_from(row * raster.info.width + col0).expect("tiny grid");
            let len = usize::try_from(clipped.width).expect("tiny grid");
            pixels.extend_from_slice(&raster.samples[start..start + len]);
            if len > 0 {
                provenance.push(Provenance {
                    path: asset.as_str().to_owned(),
                    offset: u64::try_from(start).expect("tiny grid"),
                    length: u64::try_from(len).expect("tiny grid"),
                });
            }
        }

        Ok(WindowData::new(
            clipped,
            raster.info.clone(),
            PixelBuffer::UInt8(pixels),
            raster.info.nodata,
            provenance,
        ))
    }
}
