// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Materialized overview pyramids over [`object_store`] (issue #183 — the
//! batch-materialization path of ARCHITECTURE.md §10, closing the
//! "overview generation" deferral in `docs/ROADMAP.md`).
//!
//! The planner (issue #37) chooses among overviews that *exist*; until
//! this crate, only COG-embedded overviews existed, and sources without
//! them (virtual cubes; low zooms past a COG's coarsest embedded level)
//! could only render live. This adapter completes the third strategy:
//!
//! - [`PyramidSource`] wraps any inner [`RasterSource`] and **overlays**
//!   materialized levels onto it: `describe` reports the union of the
//!   asset's embedded overview factors and the pyramid's completed ones
//!   (so the planner needs zero changes), and `read_window` serves a
//!   materialized factor from stored chunks while delegating everything
//!   else — full-res reads and embedded factors — to the inner source
//!   untouched.
//! - [`PyramidSource::materialize`] is the batch writer: it builds the
//!   decimation ladder an asset is missing (embedded factors are never
//!   duplicated), reading each level from the best already-available
//!   coarser grid and aggregating nodata-aware.
//!
//! # Idempotent and resumable, by construction
//!
//! Chunk objects are written once and never rewritten: the writer probes
//! (`head`) before computing, so a rerun skips existing chunks, a killed
//! run resumes at the first missing chunk, and two identical runs write
//! identical bytes in any interleaving. A factor enters the group
//! `.zattrs` `completed` list only after every one of its chunks exists —
//! the serve path never sees a half-built level. The layout and its
//! documents are specified in [`layout`] (GeoZarr-shaped plain Zarr v2 —
//! external Zarr readers open the group directly).
//!
//! # Trust and staleness
//!
//! A pyramid names its source and full-resolution grid in its identity
//! document; `describe` merges completed factors only when that document
//! still [`matches`](layout::PyramidMeta::matches) the asset it just
//! described. A stale or foreign pyramid (the asset was replaced under
//! its URI) is silently *not* advertised — serving degrades to exactly
//! the pre-pyramid behavior, never to wrong pixels. Granule assets are
//! immutable by convention in this system (the fixtures policy, the
//! content-derived cache); regeneration after a deliberate asset swap is
//! `materialize` again (it detects the conflict and refuses until the
//! stale pyramid is removed).
//!
//! Like the tile-cache adapter, this crate reports real observed I/O:
//! chunk fetches land in [`WindowData::provenance`] as whole-object
//! reads; metadata fetches (`.zattrs`) are per-asset bookkeeping and are
//! not counted, matching the COG adapter's treatment of header walks.

pub mod layout;
mod materialize;

use std::sync::Arc;

use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _};
use swath_core::raster::{AssetRef, DType, RasterInfo, WindowRequest};
use swath_core::source::{
    BandSelection, PixelBuffer, RasterSource, ReadLevel, SourceError, WindowData,
};
use swath_core::trace::Provenance;

pub use layout::PyramidResampling;
pub use materialize::{MaterializeError, MaterializeReport, MaterializeSpec};

use layout::GroupAttrs;

/// A [`RasterSource`] overlay serving materialized pyramid levels from an
/// [`ObjectStore`], delegating everything else to the wrapped source
/// (crate docs).
#[derive(Debug, Clone)]
pub struct PyramidSource<S> {
    inner: S,
    store: Arc<dyn ObjectStore>,
}

impl<S> PyramidSource<S> {
    /// Wraps `inner`, overlaying pyramids stored under
    /// `pyramids/` in `store` (typically the same store the assets live
    /// in — one storage root).
    #[must_use]
    pub fn new(inner: S, store: Arc<dyn ObjectStore>) -> Self {
        Self { inner, store }
    }

    /// The wrapped source.
    #[must_use]
    pub fn inner(&self) -> &S {
        &self.inner
    }

    fn object_path(asset: &AssetRef, path: &str) -> Result<Path, SourceError> {
        Path::parse(path).map_err(|e| SourceError::Format {
            asset: asset.clone(),
            detail: format!("pyramid path `{path}` is not a valid object path: {e}"),
        })
    }

    /// Loads and parses the pyramid group attrs for `asset`, or `None`
    /// when no pyramid exists (or the stored document is unreadable —
    /// serving must degrade to the inner source, never fail, so foreign
    /// bytes under the pyramid root behave exactly like no pyramid).
    async fn load_attrs(&self, asset: &AssetRef) -> Result<Option<GroupAttrs>, SourceError> {
        let root = layout::pyramid_root(asset.as_str());
        let path = Self::object_path(asset, &layout::zattrs_path(&root))?;
        let object = match self.store.get(&path).await {
            Ok(object) => object,
            Err(object_store::Error::NotFound { .. }) => return Ok(None),
            Err(err) => {
                return Err(SourceError::Io {
                    asset: asset.clone(),
                    source: Box::new(err),
                });
            }
        };
        let bytes = object.bytes().await.map_err(|err| SourceError::Io {
            asset: asset.clone(),
            source: Box::new(err),
        })?;
        Ok(serde_json::from_slice::<GroupAttrs>(&bytes).ok())
    }

    /// Reads a window of a **completed** materialized level: maps the
    /// full-resolution request onto the level grid (covering, then
    /// clipped), fetches exactly the chunk objects it touches, and blits
    /// their overlaps into a dense buffer.
    async fn read_level(
        &self,
        asset: &AssetRef,
        meta: &layout::PyramidMeta,
        factor: u32,
        window: WindowRequest,
    ) -> Result<WindowData, SourceError> {
        let full = meta.full_info().ok_or_else(|| SourceError::Format {
            asset: asset.clone(),
            detail: format!("pyramid dtype `{}` is not readable", meta.dtype),
        })?;
        let grid = layout::level_info(&full, factor);
        let request = layout::to_grid(&window, &full, &grid);
        let bounds = WindowRequest {
            col_off: 0,
            row_off: 0,
            width: grid.width,
            height: grid.height,
        };
        let Some(clip) = request.intersection(&bounds) else {
            let empty = WindowRequest {
                col_off: request.col_off.min(grid.width),
                row_off: request.row_off.min(grid.height),
                width: 0,
                height: 0,
            };
            let pixels = alloc_pixels(grid.dtype, 0);
            return Ok(WindowData::new(
                empty,
                grid,
                pixels,
                meta.nodata,
                Vec::new(),
            ));
        };

        let chunk = u64::from(meta.chunk);
        let root = layout::pyramid_root(asset.as_str());
        let sample_bytes = grid.dtype.size_bytes() as u64;
        let chunk_len = chunk * chunk * sample_bytes;
        let mut pixels = alloc_pixels(grid.dtype, us(clip.width * clip.height));
        let mut provenance = Vec::new();

        let cy0 = clip.row_off / chunk;
        let cy1 = (clip.end_row() - 1) / chunk;
        let cx0 = clip.col_off / chunk;
        let cx1 = (clip.end_col() - 1) / chunk;
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let path_str = layout::chunk_path(&root, factor, cy, cx);
                let path = Self::object_path(asset, &path_str)?;
                let object = match self.store.get(&path).await {
                    Ok(object) => object,
                    Err(object_store::Error::NotFound { .. }) => {
                        return Err(SourceError::Format {
                            asset: asset.clone(),
                            detail: format!(
                                "pyramid chunk `{path_str}` is missing from a completed level"
                            ),
                        });
                    }
                    Err(err) => {
                        return Err(SourceError::Io {
                            asset: asset.clone(),
                            source: Box::new(err),
                        });
                    }
                };
                let bytes = object.bytes().await.map_err(|err| SourceError::Io {
                    asset: asset.clone(),
                    source: Box::new(err),
                })?;
                if bytes.len() as u64 != chunk_len {
                    return Err(SourceError::Format {
                        asset: asset.clone(),
                        detail: format!(
                            "pyramid chunk `{path_str}` is {} byte(s), expected {chunk_len}",
                            bytes.len()
                        ),
                    });
                }
                provenance.push(Provenance {
                    path: path_str,
                    offset: 0,
                    length: bytes.len() as u64,
                });
                let chunk_rect = WindowRequest {
                    col_off: cx * chunk,
                    row_off: cy * chunk,
                    width: chunk,
                    height: chunk,
                };
                blit_chunk(asset, &mut pixels, &bytes, &clip, &chunk_rect)?;
            }
        }

        Ok(WindowData::new(clip, grid, pixels, meta.nodata, provenance))
    }
}

impl<S: RasterSource> RasterSource for PyramidSource<S> {
    async fn describe(&self, asset: &AssetRef) -> Result<RasterInfo, SourceError> {
        let mut info = self.inner.describe(asset).await?;
        if let Some(attrs) = self.load_attrs(asset).await?
            && attrs.pyramid.matches(asset.as_str(), &info)
        {
            for factor in attrs.pyramid.completed {
                if factor > 1 && !info.overview_levels.contains(&factor) {
                    info.overview_levels.push(factor);
                }
            }
            info.overview_levels.sort_unstable();
        }
        Ok(info)
    }

    async fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
        level: ReadLevel,
    ) -> Result<WindowData, SourceError> {
        let ReadLevel::Overview { factor } = level else {
            return self.inner.read_window(asset, window, band, level).await;
        };
        let materialized = match self.load_attrs(asset).await? {
            Some(attrs) if attrs.pyramid.completed.contains(&factor) => Some(attrs.pyramid),
            _ => None,
        };
        let Some(meta) = materialized else {
            return self.inner.read_window(asset, window, band, level).await;
        };
        // Materialized levels are single-band by construction; any other
        // selection is refused explicitly, mirroring the inner adapters.
        let BandSelection::Single(0) = band else {
            return Err(SourceError::Unsupported {
                asset: asset.clone(),
                detail: format!("band selection {band:?} not supported by materialized pyramids"),
            });
        };
        self.read_level(asset, &meta, factor, window).await
    }
}

/// Allocates a zeroed buffer of `len` samples for `dtype`.
fn alloc_pixels(dtype: DType, len: usize) -> PixelBuffer {
    match dtype {
        DType::UInt8 => PixelBuffer::UInt8(vec![0; len]),
        DType::Int16 => PixelBuffer::Int16(vec![0; len]),
        DType::UInt16 => PixelBuffer::UInt16(vec![0; len]),
        DType::Int32 => PixelBuffer::Int32(vec![0; len]),
        DType::Float32 => PixelBuffer::Float32(vec![0.0; len]),
        DType::Float64 => PixelBuffer::Float64(vec![0.0; len]),
        // DType is non_exhaustive; dtype_from_zarr only ever produces the
        // variants above, and it widens in lockstep with PixelBuffer.
        _ => unreachable!("dtype not produced by this adapter"),
    }
}

/// Copies the overlap between `clip` and the (padded, full-size) chunk at
/// `chunk_rect` from raw little-endian `bytes` into `pixels`.
fn blit_chunk(
    asset: &AssetRef,
    pixels: &mut PixelBuffer,
    bytes: &[u8],
    clip: &WindowRequest,
    chunk_rect: &WindowRequest,
) -> Result<(), SourceError> {
    let Some(overlap) = clip.intersection(chunk_rect) else {
        return Ok(());
    };
    let geom = Blit {
        dst_width: us(clip.width),
        dst_col: us(overlap.col_off - clip.col_off),
        dst_row: us(overlap.row_off - clip.row_off),
        src_width: us(chunk_rect.width),
        src_col: us(overlap.col_off - chunk_rect.col_off),
        src_row: us(overlap.row_off - chunk_rect.row_off),
        rows: us(overlap.height),
        cols: us(overlap.width),
    };
    match pixels {
        PixelBuffer::UInt8(dst) => geom.blit(dst, bytes),
        PixelBuffer::Int16(dst) => geom.blit(dst, &decode(bytes, i16::from_le_bytes)),
        PixelBuffer::UInt16(dst) => geom.blit(dst, &decode(bytes, u16::from_le_bytes)),
        PixelBuffer::Int32(dst) => geom.blit(dst, &decode(bytes, i32::from_le_bytes)),
        PixelBuffer::Float32(dst) => geom.blit(dst, &decode(bytes, f32::from_le_bytes)),
        PixelBuffer::Float64(dst) => geom.blit(dst, &decode(bytes, f64::from_le_bytes)),
        // PixelBuffer is non_exhaustive; alloc_pixels above is the only
        // constructor on this path.
        _ => {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: "pyramid buffer dtype not supported".to_owned(),
            });
        }
    }
    Ok(())
}

/// Decodes raw little-endian chunk bytes into typed samples.
fn decode<T: Copy, const N: usize>(bytes: &[u8], f: impl Fn([u8; N]) -> T) -> Vec<T> {
    bytes
        .chunks_exact(N)
        .map(|c| f(c.try_into().expect("chunks_exact yields N bytes")))
        .collect()
}

/// Row-copy geometry for one chunk-overlap blit.
struct Blit {
    dst_width: usize,
    dst_col: usize,
    dst_row: usize,
    src_width: usize,
    src_col: usize,
    src_row: usize,
    rows: usize,
    cols: usize,
}

impl Blit {
    fn blit<T: Copy>(&self, dst: &mut [T], src: &[T]) {
        for row in 0..self.rows {
            let dst_start = (self.dst_row + row) * self.dst_width + self.dst_col;
            let src_start = (self.src_row + row) * self.src_width + self.src_col;
            dst[dst_start..dst_start + self.cols]
                .copy_from_slice(&src[src_start..src_start + self.cols]);
        }
    }
}

/// `u64` → `usize`; window/chunk dimensions always fit (buffers of this
/// size are allocated).
pub(crate) fn us(v: u64) -> usize {
    usize::try_from(v).expect("dimension exceeds usize")
}
