// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `RasterSource` port: async windowed reads from source assets.
//!
//! This is the first port trait to land (ARCHITECTURE.md §6): adapters
//! (`swath-source-cog` first; Zarr and virtual-reference later) implement
//! [`RasterSource`], and the core consumes it generically. The vocabulary the
//! trait speaks — [`RasterInfo`], [`WindowRequest`], [`AssetRef`] — lives in
//! [`crate::raster`]; this module adds the request/response types specific to
//! the port ([`BandSelection`], [`WindowData`], [`PixelBuffer`]) and its error
//! taxonomy ([`SourceError`]).
//!
//! # Async without I/O in the core
//!
//! The trait uses **native async-in-trait**, desugared to
//! `-> impl Future<…> + Send` so callers can spawn the returned futures onto
//! multithreaded executors. This keeps swath-core free of both `async-trait`
//! and any runtime dependency (no tokio here — adapters choose their own
//! runtime). The trade-off, made deliberately: the trait is **not
//! dyn-compatible** (`dyn RasterSource` will not compile). Today every
//! consumer is generic (`S: RasterSource`) and adapters are the only
//! implementors; if dynamic dispatch is ever needed, the revisit options are
//! `async-trait` boxing or an enum over the compiled-in adapters — recorded
//! here so the future decision starts from this note.
//!
//! Implementors write plain `async fn` — it satisfies the desugared
//! signature, and the compiler enforces the `Send` bound at the impl site.

use core::future::Future;

use crate::raster::{AssetRef, DType, RasterInfo, WindowRequest};
use crate::trace::Provenance;

/// Which band(s) of an asset a read targets.
///
/// Band indices are **zero-based** (band 0 is the first band), unlike GDAL's
/// 1-based convention — this is a Rust-native API and every index in it is
/// zero-based; adapters translate at their boundary.
///
/// Single-band today; a multi-band variant is an additive, non-breaking
/// extension later (the enum is `#[non_exhaustive]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BandSelection {
    /// One band, by zero-based index.
    Single(u32),
}

/// A dtype-tagged, densely packed pixel buffer in row-major order
/// (row by row, top to bottom; within a row, left to right).
///
/// The variant *is* the dtype tag — there is no way to hold pixels whose
/// static type disagrees with their declared [`DType`]. `#[non_exhaustive]`
/// mirrors [`DType`]: both widen together as real sources demand.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PixelBuffer {
    /// Unsigned 8-bit samples.
    UInt8(Vec<u8>),
    /// Signed 16-bit samples.
    Int16(Vec<i16>),
    /// Unsigned 16-bit samples.
    UInt16(Vec<u16>),
    /// Signed 32-bit samples.
    Int32(Vec<i32>),
    /// IEEE 754 single-precision samples.
    Float32(Vec<f32>),
    /// IEEE 754 double-precision samples.
    Float64(Vec<f64>),
}

impl PixelBuffer {
    /// The sample data type of this buffer.
    #[must_use]
    pub const fn dtype(&self) -> DType {
        match self {
            Self::UInt8(_) => DType::UInt8,
            Self::Int16(_) => DType::Int16,
            Self::UInt16(_) => DType::UInt16,
            Self::Int32(_) => DType::Int32,
            Self::Float32(_) => DType::Float32,
            Self::Float64(_) => DType::Float64,
        }
    }

    /// Number of samples in the buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::UInt8(v) => v.len(),
            Self::Int16(v) => v.len(),
            Self::UInt16(v) => v.len(),
            Self::Int32(v) => v.len(),
            Self::Float32(v) => v.len(),
            Self::Float64(v) => v.len(),
        }
    }

    /// Whether the buffer holds zero samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The samples as raw **little-endian** bytes, sample order preserved.
    ///
    /// This is the canonical byte form the test oracle hashes (SHA-256 of
    /// exactly these bytes); it is byte-for-byte identical to a `numpy`
    /// `array.astype('<dtype').tobytes()` of the same pixels.
    #[must_use]
    pub fn to_le_bytes(&self) -> Vec<u8> {
        fn bytes_of<T: Copy, const N: usize>(v: &[T], f: impl Fn(T) -> [u8; N]) -> Vec<u8> {
            let mut out = Vec::with_capacity(v.len() * N);
            for &s in v {
                out.extend_from_slice(&f(s));
            }
            out
        }
        match self {
            Self::UInt8(v) => v.clone(),
            Self::Int16(v) => bytes_of(v, i16::to_le_bytes),
            Self::UInt16(v) => bytes_of(v, u16::to_le_bytes),
            Self::Int32(v) => bytes_of(v, i32::to_le_bytes),
            Self::Float32(v) => bytes_of(v, f32::to_le_bytes),
            Self::Float64(v) => bytes_of(v, f64::to_le_bytes),
        }
    }
}

/// The result of a windowed read: pixels plus everything the Trace needs.
///
/// Reads **clip**: [`window`](Self::window) is the region actually read (the
/// intersection of the request with the raster grid), which may be smaller
/// than requested or empty. The pixel count in
/// [`pixels`](Self::pixels) is always `window.width * window.height` (times
/// one band — multi-band layout is defined when [`BandSelection`] grows).
///
/// [`provenance`](Self::provenance) records the **actual byte ranges
/// fetched** from storage to satisfy this read, in fetch order, and
/// [`bytes_read`](Self::bytes_read) their total length — the raw material of
/// the glass-box Trace (REQUIREMENTS.md R4). Adapters must report real,
/// observed fetches, not estimates.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct WindowData {
    /// The window actually read, in full-resolution pixel coordinates.
    pub window: WindowRequest,
    /// The pixel samples, row-major, dtype-tagged.
    pub pixels: PixelBuffer,
    /// Nodata sentinel (GDAL convention, widened to `f64`), if declared.
    pub nodata: Option<f64>,
    /// Every byte range fetched from storage for this read, in fetch order.
    pub provenance: Vec<Provenance>,
    /// Total bytes fetched — always the sum of `provenance` range lengths.
    pub bytes_read: u64,
}

impl WindowData {
    /// Creates a `WindowData`, deriving [`bytes_read`](Self::bytes_read)
    /// from the provenance ranges so the two can never disagree.
    #[must_use]
    pub fn new(
        window: WindowRequest,
        pixels: PixelBuffer,
        nodata: Option<f64>,
        provenance: Vec<Provenance>,
    ) -> Self {
        let bytes_read = provenance.iter().map(|p| p.length).sum();
        Self {
            window,
            pixels,
            nodata,
            provenance,
            bytes_read,
        }
    }

    /// The sample data type of the pixels.
    #[must_use]
    pub const fn dtype(&self) -> DType {
        self.pixels.dtype()
    }
}

/// What can go wrong at the source boundary.
///
/// This is the port's error contract, defined in the core so consumers match
/// on semantics, not on adapter internals. Adapters translate their library
/// and storage errors into these variants, carrying the underlying error as
/// [`source`](std::error::Error::source) where one exists.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SourceError {
    /// The asset does not exist in the underlying store.
    #[error("asset not found: {asset}")]
    NotFound {
        /// The asset that was requested.
        asset: AssetRef,
    },

    /// The asset exists but cannot be understood as the format this adapter
    /// reads (corrupt file, wrong magic, unparseable metadata).
    #[error("asset {asset} is not readable as this source's format: {detail}")]
    Format {
        /// The asset that was rejected.
        asset: AssetRef,
        /// What failed to parse.
        detail: String,
    },

    /// The asset parses, but uses a feature this adapter does not support
    /// (e.g. an exotic compression or sample layout).
    #[error("asset {asset} uses an unsupported feature: {detail}")]
    Unsupported {
        /// The asset that was rejected.
        asset: AssetRef,
        /// The unsupported feature.
        detail: String,
    },

    /// A band index outside the asset's band range.
    #[error("band {band} out of range for {asset} ({band_count} band(s))")]
    BandOutOfRange {
        /// The asset that was read.
        asset: AssetRef,
        /// The requested zero-based band index.
        band: u32,
        /// The asset's band count.
        band_count: u32,
    },

    /// Storage or transport failure while fetching bytes.
    #[error("i/o failure reading {asset}")]
    Io {
        /// The asset being read when the failure occurred.
        asset: AssetRef,
        /// The underlying storage/transport error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Read windowed samples from a source asset (ARCHITECTURE.md §6).
///
/// The port the tiler and planner consume; implemented by adapter crates
/// (COG first). Implementations are stateless per call from the trait's
/// perspective — any caching is an adapter concern and must not change
/// observable results.
///
/// See the [module docs](self) for the async-in-trait design (native AFIT,
/// `Send` futures, deliberately not dyn-compatible).
pub trait RasterSource: Send + Sync {
    /// Describes an asset — CRS, grid, geotransform, dtype, nodata, bands,
    /// overview levels — without reading pixels.
    fn describe(
        &self,
        asset: &AssetRef,
    ) -> impl Future<Output = Result<RasterInfo, SourceError>> + Send;

    /// Reads a pixel window from one band of an asset, **clipping** the
    /// request to the raster grid, and reports the exact byte ranges fetched.
    fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
    ) -> impl Future<Output = Result<WindowData, SourceError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::{BandSelection, PixelBuffer, WindowData};
    use crate::raster::{DType, WindowRequest};
    use crate::trace::Provenance;

    #[test]
    fn pixel_buffer_dtype_and_len() {
        let b = PixelBuffer::Int16(vec![-9999, 0, 1234]);
        assert_eq!(b.dtype(), DType::Int16);
        assert_eq!(b.len(), 3);
        assert!(!b.is_empty());
        assert!(PixelBuffer::UInt8(Vec::new()).is_empty());
    }

    #[test]
    fn le_bytes_are_little_endian_in_sample_order() {
        let b = PixelBuffer::Int16(vec![0x0102, -2]);
        assert_eq!(b.to_le_bytes(), vec![0x02, 0x01, 0xFE, 0xFF]);
        let f = PixelBuffer::Float32(vec![1.0]);
        assert_eq!(f.to_le_bytes(), 1.0_f32.to_le_bytes().to_vec());
        let u = PixelBuffer::UInt8(vec![7, 255]);
        assert_eq!(u.to_le_bytes(), vec![7, 255]);
    }

    #[test]
    fn window_data_derives_bytes_read_from_provenance() {
        let wd = WindowData::new(
            WindowRequest {
                col_off: 0,
                row_off: 0,
                width: 1,
                height: 1,
            },
            PixelBuffer::UInt8(vec![0]),
            Some(255.0),
            vec![
                Provenance {
                    path: "a.tif".into(),
                    offset: 100,
                    length: 40,
                },
                Provenance {
                    path: "a.tif".into(),
                    offset: 900,
                    length: 60,
                },
            ],
        );
        assert_eq!(wd.bytes_read, 100);
        assert_eq!(wd.dtype(), DType::UInt8);
    }

    #[test]
    fn band_selection_serializes_snake_case() {
        let json = serde_json::to_string(&BandSelection::Single(0)).unwrap();
        assert_eq!(json, r#"{"single":0}"#);
    }
}
