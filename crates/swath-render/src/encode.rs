// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Tile encoding: [`RgbaTile`] to bytes.
//!
//! PNG is the Phase-1 tile format ([`TileFormat`] documents the WebP
//! deferral, tracked in `docs/ROADMAP.md`). Encoding is deterministic — fixed encoder settings, and PNG
//! carries no timestamps or ancillary metadata through this path — so the
//! same tile always yields the same bytes; a test double-encodes and
//! compares hashes to keep that contract honest.

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder as _};

use crate::ir::RgbaTile;

#[cfg(doc)]
use crate::ir::TileFormat;

/// Why a tile failed to encode.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EncodeError {
    /// The tile's `pixels` length disagrees with `width * height * 4`.
    #[error("tile buffer holds {actual} bytes, dimensions require {expected}")]
    ShapeMismatch {
        /// `width * height * 4`.
        expected: u64,
        /// `pixels.len()`.
        actual: u64,
    },
    /// The PNG encoder rejected the image.
    #[error("png encoding failed")]
    Png(#[from] image::ImageError),
}

/// Encodes `tile` as a lossless RGBA PNG.
///
/// Settings are pinned (default compression, adaptive filtering) so output
/// bytes are a pure function of the pixel data — cache keys and goldens
/// both rely on that.
///
/// # Errors
///
/// [`EncodeError::ShapeMismatch`] when the buffer length disagrees with the
/// dimensions; [`EncodeError::Png`] when the encoder itself fails (e.g. a
/// zero-sized image, which PNG cannot represent).
pub fn encode_png(tile: &RgbaTile) -> Result<Vec<u8>, EncodeError> {
    let expected = u64::from(tile.width) * u64::from(tile.height) * 4;
    let actual = tile.pixels.len() as u64;
    if expected != actual {
        return Err(EncodeError::ShapeMismatch { expected, actual });
    }
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Default, FilterType::Adaptive)
        .write_image(
            &tile.pixels,
            tile.width,
            tile.height,
            ExtendedColorType::Rgba8,
        )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{EncodeError, encode_png};
    use crate::ir::RgbaTile;

    fn gradient_tile(width: u32, height: u32) -> RgbaTile {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                #[allow(clippy::cast_possible_truncation, reason = "folded into u8 range")]
                pixels.extend_from_slice(&[
                    (x % 256) as u8,
                    (y % 256) as u8,
                    ((x + y) % 256) as u8,
                    255,
                ]);
            }
        }
        RgbaTile {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn png_roundtrips_pixels_exactly() {
        let tile = gradient_tile(16, 8);
        let png = encode_png(&tile).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(decoded.dimensions(), (16, 8));
        assert_eq!(decoded.into_raw(), tile.pixels);
    }

    #[test]
    fn encoding_is_deterministic() {
        let tile = gradient_tile(64, 64);
        assert_eq!(encode_png(&tile).unwrap(), encode_png(&tile).unwrap());
    }

    #[test]
    fn shape_mismatch_is_an_error() {
        let tile = RgbaTile {
            width: 2,
            height: 2,
            pixels: vec![0; 15],
        };
        assert!(matches!(
            encode_png(&tile),
            Err(EncodeError::ShapeMismatch {
                expected: 16,
                actual: 15
            })
        ));
    }
}
