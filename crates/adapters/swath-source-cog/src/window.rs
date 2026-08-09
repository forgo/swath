// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Window assembly: intersect the (clipped) request with the tile grid,
//! then blit each decoded tile's overlap into the output buffer.

use async_tiff::tags::PlanarConfiguration;
use async_tiff::{Array, ImageFileDirectory, TypedArray};
use swath_core::raster::{AssetRef, DType, WindowRequest};
use swath_core::source::{PixelBuffer, SourceError};

/// Which tiles a clipped window touches, and the geometry needed to place
/// their pixels.
pub(crate) struct TilePlan {
    clip: WindowRequest,
    tile_width: u64,
    tile_height: u64,
    tiles: Vec<(usize, usize)>,
    samples_per_pixel: usize,
}

impl TilePlan {
    /// Plans a read of `clip` (already intersected with the raster grid,
    /// non-empty) against `ifd`'s tile grid.
    pub(crate) fn for_window(
        asset: &AssetRef,
        ifd: &ImageFileDirectory,
        clip: WindowRequest,
    ) -> Result<Self, SourceError> {
        if ifd.planar_configuration() != PlanarConfiguration::Chunky {
            return Err(SourceError::Unsupported {
                asset: asset.clone(),
                detail: "planar (band-separate) configuration".to_string(),
            });
        }
        let (Some(tile_width), Some(tile_height)) = (ifd.tile_width(), ifd.tile_height()) else {
            return Err(SourceError::Unsupported {
                asset: asset.clone(),
                detail: "striped TIFF (no tile grid); only tiled COGs are supported".to_string(),
            });
        };
        let (tile_width, tile_height) = (u64::from(tile_width), u64::from(tile_height));

        let tx0 = clip.col_off / tile_width;
        let tx1 = (clip.end_col() - 1) / tile_width;
        let ty0 = clip.row_off / tile_height;
        let ty1 = (clip.end_row() - 1) / tile_height;
        let mut tiles = Vec::with_capacity(us((tx1 - tx0 + 1) * (ty1 - ty0 + 1)));
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                tiles.push((us(tx), us(ty)));
            }
        }

        Ok(Self {
            clip,
            tile_width,
            tile_height,
            tiles,
            samples_per_pixel: usize::from(ifd.samples_per_pixel()),
        })
    }

    /// The tiles to fetch, row-major.
    pub(crate) fn tiles(&self) -> &[(usize, usize)] {
        &self.tiles
    }

    /// Samples in the output buffer (`clip.width * clip.height`, one band).
    pub(crate) fn sample_count(&self) -> usize {
        us(self.clip.width * self.clip.height)
    }
}

/// Allocates a zeroed buffer of `len` samples for `dtype`.
pub(crate) fn alloc_pixels(dtype: DType, len: usize) -> PixelBuffer {
    match dtype {
        DType::UInt8 => PixelBuffer::UInt8(vec![0; len]),
        DType::Int16 => PixelBuffer::Int16(vec![0; len]),
        DType::UInt16 => PixelBuffer::UInt16(vec![0; len]),
        DType::Int32 => PixelBuffer::Int32(vec![0; len]),
        DType::Float32 => PixelBuffer::Float32(vec![0.0; len]),
        DType::Float64 => PixelBuffer::Float64(vec![0.0; len]),
        // DType is non_exhaustive; meta::dtype only ever produces the
        // variants above, and it widens in lockstep with PixelBuffer.
        _ => unreachable!("dtype not produced by this adapter"),
    }
}

/// Copies the overlap between the plan's window and the decoded tile at grid
/// position `(tile_x, tile_y)` into `pixels`, selecting `band`.
pub(crate) fn copy_tile(
    asset: &AssetRef,
    pixels: &mut PixelBuffer,
    array: &Array,
    plan: &TilePlan,
    (tile_x, tile_y): (usize, usize),
    band: u32,
) -> Result<(), SourceError> {
    // Chunky shape is (height, width, samples). Using the *decoded* shape
    // (not the nominal tile size) stays correct if a reader crops edge tiles.
    let [tile_h, tile_w, samples] = array.shape();
    if samples != plan.samples_per_pixel {
        return Err(SourceError::Format {
            asset: asset.clone(),
            detail: format!(
                "decoded tile has {samples} samples/pixel, IFD declares {}",
                plan.samples_per_pixel
            ),
        });
    }
    let tile_rect = WindowRequest {
        col_off: tile_x as u64 * plan.tile_width,
        row_off: tile_y as u64 * plan.tile_height,
        width: tile_w as u64,
        height: tile_h as u64,
    };
    let Some(overlap) = plan.clip.intersection(&tile_rect) else {
        return Ok(()); // a fetched tile that contributes nothing; harmless
    };

    let geom = Blit {
        dst_width: us(plan.clip.width),
        dst_col: us(overlap.col_off - plan.clip.col_off),
        dst_row: us(overlap.row_off - plan.clip.row_off),
        src_width: tile_w,
        src_col: us(overlap.col_off - tile_rect.col_off),
        src_row: us(overlap.row_off - tile_rect.row_off),
        rows: us(overlap.height),
        cols: us(overlap.width),
        samples,
        band: us(u64::from(band)),
    };

    match (pixels, array.data()) {
        (PixelBuffer::UInt8(dst), TypedArray::UInt8(src)) => geom.blit(dst, src),
        (PixelBuffer::Int16(dst), TypedArray::Int16(src)) => geom.blit(dst, src),
        (PixelBuffer::UInt16(dst), TypedArray::UInt16(src)) => geom.blit(dst, src),
        (PixelBuffer::Int32(dst), TypedArray::Int32(src)) => geom.blit(dst, src),
        (PixelBuffer::Float32(dst), TypedArray::Float32(src)) => geom.blit(dst, src),
        (PixelBuffer::Float64(dst), TypedArray::Float64(src)) => geom.blit(dst, src),
        _ => {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: "decoded tile dtype disagrees with IFD dtype".to_string(),
            });
        }
    }
    Ok(())
}

/// Row-copy geometry for one tile-overlap blit.
struct Blit {
    dst_width: usize,
    dst_col: usize,
    dst_row: usize,
    src_width: usize,
    src_col: usize,
    src_row: usize,
    rows: usize,
    cols: usize,
    samples: usize,
    band: usize,
}

impl Blit {
    fn blit<T: Copy>(&self, dst: &mut [T], src: &[T]) {
        for row in 0..self.rows {
            let dst_start = (self.dst_row + row) * self.dst_width + self.dst_col;
            let src_start = ((self.src_row + row) * self.src_width + self.src_col) * self.samples;
            if self.samples == 1 {
                dst[dst_start..dst_start + self.cols]
                    .copy_from_slice(&src[src_start..src_start + self.cols]);
            } else {
                for col in 0..self.cols {
                    dst[dst_start + col] = src[src_start + col * self.samples + self.band];
                }
            }
        }
    }
}

/// `u64` → `usize`; window/tile dimensions always fit (buffers of this size
/// are allocated).
fn us(v: u64) -> usize {
    usize::try_from(v).expect("dimension exceeds usize")
}
