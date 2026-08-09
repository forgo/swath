// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! IFD → [`RasterInfo`] mapping: `GeoKeys` → CRS, `ModelPixelScale` +
//! `ModelTiepoint` → [`GeoTransform`], `SampleFormat`/`BitsPerSample` →
//! [`DType`], `GDAL_NODATA` → nodata, reduced-resolution IFDs → overviews.

use async_tiff::ImageFileDirectory;
use async_tiff::tags::SampleFormat;
use swath_core::crs::Crs;
use swath_core::raster::{AssetRef, DType, GeoTransform, RasterInfo};
use swath_core::source::SourceError;

/// TIFF `NewSubfileType` bit 0: this IFD is a reduced-resolution image.
const FILETYPE_REDUCED_IMAGE: u32 = 1;

/// Builds the port's [`RasterInfo`] from a COG's IFD chain (full-resolution
/// image first, then overviews — the COG layout).
pub(crate) fn raster_info(
    asset: &AssetRef,
    ifds: &[ImageFileDirectory],
) -> Result<RasterInfo, SourceError> {
    let primary = ifds
        .first()
        .ok_or_else(|| format_err(asset, "TIFF contains no IFDs"))?;

    let width = u64::from(primary.image_width());
    let height = u64::from(primary.image_height());
    let dtype = dtype(asset, primary)?;
    let band_count = u32::from(primary.samples_per_pixel());
    let crs = crs(asset, primary)?;
    let transform = geo_transform(asset, primary)?;
    let nodata = nodata(asset, primary)?;

    // Overviews: subsequent reduced-resolution IFDs, reported as decimation
    // factors of the full-resolution grid (e.g. a 256-wide overview of a
    // 512-wide image is level 2), ascending.
    let mut overview_levels: Vec<u32> = ifds[1..]
        .iter()
        .filter(|ifd| {
            ifd.new_subfile_type()
                .is_some_and(|t| t & FILETYPE_REDUCED_IMAGE != 0)
        })
        .map(|ifd| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                clippy::cast_sign_loss,
                reason = "decimation factors are tiny (powers of two in practice)"
            )]
            {
                (width as f64 / f64::from(ifd.image_width())).round() as u32
            }
        })
        .collect();
    overview_levels.sort_unstable();

    Ok(RasterInfo {
        crs,
        width,
        height,
        transform,
        band_count,
        dtype,
        nodata,
        overview_levels,
    })
}

fn dtype(asset: &AssetRef, ifd: &ImageFileDirectory) -> Result<DType, SourceError> {
    let bits = ifd.bits_per_sample();
    let formats = ifd.sample_format();
    let (Some(&first_bits), Some(&first_format)) = (bits.first(), formats.first()) else {
        return Err(format_err(asset, "missing BitsPerSample/SampleFormat"));
    };
    if !bits.iter().all(|&b| b == first_bits) || !formats.iter().all(|&f| f == first_format) {
        return Err(unsupported(asset, "mixed per-sample dtypes"));
    }
    match (first_format, first_bits) {
        (SampleFormat::Uint, 8) => Ok(DType::UInt8),
        (SampleFormat::Uint, 16) => Ok(DType::UInt16),
        (SampleFormat::Int, 16) => Ok(DType::Int16),
        (SampleFormat::Int, 32) => Ok(DType::Int32),
        (SampleFormat::Float, 32) => Ok(DType::Float32),
        (SampleFormat::Float, 64) => Ok(DType::Float64),
        (format, bits) => Err(unsupported(
            asset,
            &format!("sample format {format:?} with {bits} bits per sample"),
        )),
    }
}

fn crs(asset: &AssetRef, ifd: &ImageFileDirectory) -> Result<Crs, SourceError> {
    ifd.geo_key_directory()
        .and_then(async_tiff::geo::GeoKeyDirectory::epsg_code)
        .map(|code| Crs::from_epsg(u32::from(code)))
        .ok_or_else(|| unsupported(asset, "no EPSG code in GeoTIFF GeoKeys"))
}

fn geo_transform(asset: &AssetRef, ifd: &ImageFileDirectory) -> Result<GeoTransform, SourceError> {
    let scale = ifd
        .model_pixel_scale()
        .ok_or_else(|| unsupported(asset, "no ModelPixelScale tag"))?;
    let tiepoint = ifd
        .model_tiepoint()
        .ok_or_else(|| unsupported(asset, "no ModelTiepoint tag"))?;
    let ([sx, sy, ..], [i, j, _, x, y, ..]) = (scale, tiepoint) else {
        return Err(format_err(asset, "short ModelPixelScale/ModelTiepoint"));
    };
    // GeoTIFF raster-space tiepoint (i, j) maps to model-space (x, y) with a
    // north-up pixel scale: origin is the tiepoint walked back to pixel (0,0).
    Ok(GeoTransform::north_up(x - i * sx, y + j * sy, *sx, -sy))
}

fn nodata(asset: &AssetRef, ifd: &ImageFileDirectory) -> Result<Option<f64>, SourceError> {
    ifd.gdal_nodata()
        .map(|raw| {
            raw.trim()
                .trim_end_matches('\0')
                .trim()
                .parse::<f64>()
                .map_err(|_| format_err(asset, &format!("unparseable GDAL_NODATA value {raw:?}")))
        })
        .transpose()
}

fn format_err(asset: &AssetRef, detail: &str) -> SourceError {
    SourceError::Format {
        asset: asset.clone(),
        detail: detail.to_string(),
    }
}

fn unsupported(asset: &AssetRef, detail: &str) -> SourceError {
    SourceError::Unsupported {
        asset: asset.clone(),
        detail: detail.to_string(),
    }
}
