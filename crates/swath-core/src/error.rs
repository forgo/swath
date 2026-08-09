// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Crate-level error taxonomy.
//!
//! Deliberately small: this crate is pure logic, so the only failures are
//! domain-invariant violations — there is no I/O to fail. Adapter crates wrap
//! their own error types around their port implementations; they do not extend
//! this enum.
//!
//! Errors derive [`thiserror::Error`] (on the ENGINEERING.md adopt list) so
//! `Display`/`source` stay declarative and in sync with the variants.

/// An invariant violation in a core domain type.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A tile coordinate outside the valid range for its zoom level
    /// (`x, y < 2^z`) or beyond [`MAX_ZOOM`](crate::tile::MAX_ZOOM).
    #[error("invalid tile coordinate {z}/{x}/{y}: x and y must be < 2^z and z <= {max_zoom}", max_zoom = crate::tile::MAX_ZOOM)]
    InvalidTileCoord {
        /// Zoom level of the rejected coordinate.
        z: u8,
        /// Column of the rejected coordinate.
        x: u32,
        /// Row of the rejected coordinate.
        y: u32,
    },

    /// A timestamp that is not RFC 3339 UTC (`Z`-suffixed) — the only
    /// datetime form the catalog domain accepts
    /// (see [`Datetime`](crate::catalog::Datetime)).
    #[error("invalid datetime `{value}`: expected RFC 3339 UTC, e.g. 2024-06-06T17:54:00Z")]
    InvalidDatetime {
        /// The rejected string.
        value: String,
    },

    /// A [`GeoTransform`](crate::raster::GeoTransform) whose linear part is
    /// singular (determinant ~0), so CRS→pixel inversion is undefined.
    #[error("geotransform is not invertible (determinant {determinant})")]
    NonInvertibleTransform {
        /// The (near-zero) determinant of the transform's 2×2 linear part.
        determinant: f64,
    },
}
