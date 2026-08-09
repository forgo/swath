// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `RasterSource` adapter for Cloud-Optimized `GeoTIFFs` over [`object_store`].
//!
//! COG *reading* is ADOPT, never build (ARCHITECTURE.md §3): TIFF/IFD/GeoKey
//! parsing and tile decoding come from [`async-tiff`] (developmentseed). This
//! crate owns the port mapping — [`CogSource`] implements
//! [`swath_core::source::RasterSource`] — and the storage boundary: all bytes
//! flow through a [`object_store::ObjectStore`], so local files, in-memory
//! stores (tests), and S3/MinIO are the same code path.
//!
//! # Provenance is observed, never inferred
//!
//! `read_window` drives async-tiff through a recording reader
//! ([`reader::StoreReader`]) that logs every `get_range` it actually issues
//! against the store. The [`Provenance`](swath_core::trace::Provenance)
//! ranges in a [`WindowData`](swath_core::source::WindowData) are therefore
//! the exact byte ranges fetched to decode the window's tiles — the Trace's
//! raw material (REQUIREMENTS.md R4) — not estimates derived from IFD offsets.
//! Metadata reads (header + IFD walks) are *not* part of a window's
//! provenance: they are per-asset bookkeeping, not pixel I/O, and will be
//! amortized away by metadata caching without changing observable results.
//!
//! # Scope
//!
//! Tiled COGs, chunky (pixel-interleaved) planar configuration, single-sample
//! or multi-sample bands, any compression async-tiff's default decoder
//! registry handles (deflate, LZW, zstd, JPEG, uncompressed). Reads are
//! served from the full-resolution IFD; overview *levels* are reported by
//! `describe` and overview reads land when the port grows a level selector
//! (planner work, ARCHITECTURE.md §5).
//!
//! [`async-tiff`]: https://docs.rs/async-tiff

mod meta;
mod reader;
mod window;

use std::sync::Arc;

use async_tiff::ImageFileDirectory;
use async_tiff::decoder::DecoderRegistry;
use async_tiff::error::AsyncTiffError;
use async_tiff::metadata::TiffMetadataReader;
use object_store::ObjectStore;
use object_store::path::Path;
use swath_core::raster::{AssetRef, RasterInfo, WindowRequest};
use swath_core::source::{BandSelection, RasterSource, SourceError, WindowData};

use crate::reader::StoreReader;

/// A [`RasterSource`] that reads Cloud-Optimized `GeoTIFFs` from an
/// [`ObjectStore`].
///
/// The store is fixed at construction; an [`AssetRef`] is interpreted as an
/// object **path within that store** (e.g. `granules/B04.tif`), so the same
/// asset naming works over local filesystem, in-memory, and S3 stores.
/// Stateless per call: every read re-fetches metadata (caching is a later,
/// observable-behavior-preserving optimization).
#[derive(Debug, Clone)]
pub struct CogSource {
    store: Arc<dyn ObjectStore>,
}

impl CogSource {
    /// Creates a source reading from `store`.
    #[must_use]
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    fn path_for(asset: &AssetRef) -> Result<Path, SourceError> {
        Path::parse(asset.as_str()).map_err(|e| SourceError::Format {
            asset: asset.clone(),
            detail: format!("not a valid object path: {e}"),
        })
    }

    /// Fetch and parse all IFDs of `asset` (no pixel I/O, unrecorded).
    async fn load_ifds(&self, asset: &AssetRef) -> Result<Vec<ImageFileDirectory>, SourceError> {
        let path = Self::path_for(asset)?;
        let fetch = StoreReader::new(Arc::clone(&self.store), path);
        let mut reader = TiffMetadataReader::try_open(&fetch)
            .await
            .map_err(|e| map_tiff_error(asset, e))?;
        reader
            .read_all_ifds(&fetch)
            .await
            .map_err(|e| map_tiff_error(asset, e))
    }
}

impl RasterSource for CogSource {
    async fn describe(&self, asset: &AssetRef) -> Result<RasterInfo, SourceError> {
        let ifds = self.load_ifds(asset).await?;
        meta::raster_info(asset, &ifds)
    }

    async fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
    ) -> Result<WindowData, SourceError> {
        let ifds = self.load_ifds(asset).await?;
        let info = meta::raster_info(asset, &ifds)?;
        // BandSelection is non_exhaustive: new selection kinds must be
        // adopted here explicitly, not silently misread.
        let BandSelection::Single(band_index) = band else {
            return Err(SourceError::Unsupported {
                asset: asset.clone(),
                detail: format!("band selection {band:?} not yet supported by this adapter"),
            });
        };
        if band_index >= info.band_count {
            return Err(SourceError::BandOutOfRange {
                asset: asset.clone(),
                band: band_index,
                band_count: info.band_count,
            });
        }

        let full = WindowRequest {
            col_off: 0,
            row_off: 0,
            width: info.width,
            height: info.height,
        };
        let Some(clip) = window.intersection(&full) else {
            // Nothing to read: an empty window clamped onto the grid, no I/O.
            let empty = WindowRequest {
                col_off: window.col_off.min(info.width),
                row_off: window.row_off.min(info.height),
                width: 0,
                height: 0,
            };
            let pixels = window::alloc_pixels(info.dtype, 0);
            return Ok(WindowData::new(empty, pixels, info.nodata, Vec::new()));
        };

        // The full-resolution image is always the first IFD (COG layout).
        let ifd = ifds.first().ok_or_else(|| SourceError::Format {
            asset: asset.clone(),
            detail: "TIFF contains no IFDs".to_string(),
        })?;
        let plan = window::TilePlan::for_window(asset, ifd, clip)?;

        // Recorded reader: provenance = exactly the ranges fetched below.
        let path = Self::path_for(asset)?;
        let recorder = StoreReader::recording(Arc::clone(&self.store), path);
        let tiles = ifd
            .fetch_tiles(plan.tiles(), &recorder)
            .await
            .map_err(|e| map_tiff_error(asset, e))?;

        let registry = DecoderRegistry::default();
        let mut pixels = window::alloc_pixels(info.dtype, plan.sample_count());
        for tile in tiles {
            let (x, y) = (tile.x(), tile.y());
            let array = tile
                .decode(&registry)
                .map_err(|e| map_tiff_error(asset, e))?;
            window::copy_tile(asset, &mut pixels, &array, &plan, (x, y), band_index)?;
        }

        Ok(WindowData::new(
            clip,
            pixels,
            info.nodata,
            recorder.take_provenance(),
        ))
    }
}

/// Translates async-tiff failures into the port's error contract.
fn map_tiff_error(asset: &AssetRef, err: AsyncTiffError) -> SourceError {
    match err {
        AsyncTiffError::External(inner) => {
            if matches!(
                inner.downcast_ref::<object_store::Error>(),
                Some(object_store::Error::NotFound { .. })
            ) {
                SourceError::NotFound {
                    asset: asset.clone(),
                }
            } else {
                SourceError::Io {
                    asset: asset.clone(),
                    source: inner,
                }
            }
        }
        AsyncTiffError::IOError(io) => SourceError::Io {
            asset: asset.clone(),
            source: Box::new(io),
        },
        AsyncTiffError::EndOfFile(..) => SourceError::Io {
            asset: asset.clone(),
            source: Box::new(err),
        },
        other => SourceError::Format {
            asset: asset.clone(),
            detail: other.to_string(),
        },
    }
}
