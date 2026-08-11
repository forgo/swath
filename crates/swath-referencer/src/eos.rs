// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! HDF-EOS5 `StructMetadata.0` grid parsing: the georeferencing source for
//! gridded HDF-EOS products (VNP09GA, ADR 0008).
//!
//! # Parsing scope (honest and narrow)
//!
//! `StructMetadata.0` is an ODL-ish text block. This parser reads exactly
//! the fields the VNP09GA product line actually uses to place a grid —
//! nothing more:
//!
//! - `GROUP=GRID_n` … `END_GROUP=GRID_n` blocks under `GridStructure`;
//! - per grid: `GridName`, `XDim`, `YDim` (the grid-level scalars, not the
//!   `Dimension` objects), `UpperLeftPointMtrs`, `LowerRightMtrs`,
//!   `Projection`, `ProjParams`, and `GridOrigin`;
//! - `Projection=HE5_GCTP_SNSOID` (the MODIS-heritage sinusoidal projection
//!   on a sphere of radius `ProjParams[0]` — **no EPSG code exists** for it,
//!   hence the manifest's proj-string CRS vocabulary) and
//!   `GridOrigin=HE5_HDFE_GD_UL` (upper-left anchored, the only origin the
//!   product line emits).
//!
//! Other projections/origins are a loud [`ReferencerError::Unsupported`],
//! never a guessed georef; swath/point structures (`SwathStructure`,
//! `PointStructure`) are ignored. Widening this scope is deliberate work
//! with new known-answer tests, not a parser tweak (deferral tracked in
//! `docs/ROADMAP.md`).

use swath_core::ingest::ReferencerError;
use swath_core::manifest::GeorefCrs;
use swath_core::raster::GeoTransform;

/// One grid definition parsed out of `StructMetadata.0`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EosGrid {
    /// The grid's name — the `HDFEOS/GRIDS/<name>/…` path segment its data
    /// fields live under.
    pub name: String,
    /// Grid width in cells.
    pub xdim: u64,
    /// Grid height in cells.
    pub ydim: u64,
    /// The grid's CRS (a sinusoidal proj string, given the supported scope).
    pub crs: GeorefCrs,
    /// Pixel↔CRS mapping (upper-left anchored, north-up).
    pub transform: GeoTransform,
}

/// Parses every `GRID_n` block of a `StructMetadata.0` text.
///
/// # Errors
///
/// [`ReferencerError::Malformed`] when a grid block lacks/mangles a required
/// field; [`ReferencerError::Unsupported`] for projections or grid origins
/// outside the documented scope (module docs).
pub(crate) fn parse_grids(text: &str) -> Result<Vec<EosGrid>, ReferencerError> {
    let mut grids = Vec::new();
    let mut block: Option<(String, Vec<(String, String)>)> = None;

    for line in text.lines() {
        let line = line.trim();
        if let Some(id) = line.strip_prefix("GROUP=GRID_") {
            if block.is_none() {
                block = Some((format!("GRID_{id}"), Vec::new()));
            }
            continue;
        }
        if let Some(id) = line.strip_prefix("END_GROUP=GRID_") {
            if let Some((open_id, fields)) = block.take() {
                if open_id == format!("GRID_{id}") {
                    grids.push(grid_from_fields(&open_id, &fields)?);
                } else {
                    // An inner GRID_-prefixed group would be new territory.
                    return Err(malformed(&open_id, "mismatched END_GROUP nesting"));
                }
            }
            continue;
        }
        if let Some((_, fields)) = &mut block
            && let Some((key, value)) = line.split_once('=')
        {
            // First occurrence wins: the grid-level scalars precede the
            // Dimension/DataField objects, whose keys never collide with
            // the ones we read anyway.
            if !fields.iter().any(|(k, _)| k == key) {
                fields.push((key.to_owned(), value.to_owned()));
            }
        }
    }
    Ok(grids)
}

/// Assembles one [`EosGrid`] from a block's first-occurrence key/value list.
fn grid_from_fields(block: &str, fields: &[(String, String)]) -> Result<EosGrid, ReferencerError> {
    let get = |key: &str| -> Result<&str, ReferencerError> {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .ok_or_else(|| malformed(block, &format!("missing `{key}`")))
    };

    let name = get("GridName")?.trim_matches('"').to_owned();
    let xdim: u64 = get("XDim")?
        .parse()
        .map_err(|_| malformed(block, "unparsable `XDim`"))?;
    let ydim: u64 = get("YDim")?
        .parse()
        .map_err(|_| malformed(block, "unparsable `YDim`"))?;
    if xdim == 0 || ydim == 0 {
        return Err(malformed(block, "zero-sized grid"));
    }
    let (ulx, uly) = point_pair(block, get("UpperLeftPointMtrs")?)?;
    let (lrx, lry) = point_pair(block, get("LowerRightMtrs")?)?;

    let origin = get("GridOrigin")?;
    if origin != "HE5_HDFE_GD_UL" {
        return Err(ReferencerError::Unsupported {
            detail: format!("StructMetadata {block}: GridOrigin `{origin}` (only HE5_HDFE_GD_UL)"),
        });
    }

    let projection = get("Projection")?;
    if projection != "HE5_GCTP_SNSOID" {
        return Err(ReferencerError::Unsupported {
            detail: format!(
                "StructMetadata {block}: projection `{projection}` (only HE5_GCTP_SNSOID)"
            ),
        });
    }
    let params = get("ProjParams")?;
    let radius = params
        .trim_start_matches('(')
        .split(',')
        .next()
        .and_then(|r| r.trim().parse::<f64>().ok())
        .filter(|r| r.is_finite() && *r > 0.0)
        .ok_or_else(|| malformed(block, "unparsable sphere radius in `ProjParams`"))?;

    // The MODIS/VIIRS sinusoidal proj string (the vocabulary proj4rs and
    // GDAL both accept). `{radius}` via plain f64 Display: 6371007.181, no
    // trailing zeros.
    let crs = GeorefCrs::Proj4(format!(
        "+proj=sinu +lon_0=0 +x_0=0 +y_0=0 +R={radius} +units=m +no_defs"
    ));

    // Cell size from the corner points; LowerRight is south of UpperLeft,
    // so pixel_height comes out negative (north-up), as GeoTransform
    // documents.
    #[allow(clippy::cast_precision_loss)] // grid dims are ~10^3-10^4
    let transform = GeoTransform::north_up(
        ulx,
        uly,
        (lrx - ulx) / xdim as f64,
        (lry - uly) / ydim as f64,
    );
    if transform.determinant() == 0.0 {
        return Err(malformed(
            block,
            "degenerate corner points (zero-area grid)",
        ));
    }

    Ok(EosGrid {
        name,
        xdim,
        ydim,
        crs,
        transform,
    })
}

/// Parses `(x,y)` out of e.g. `(16679257.795000,-3335851.559000)`.
fn point_pair(block: &str, value: &str) -> Result<(f64, f64), ReferencerError> {
    let inner = value
        .trim()
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .ok_or_else(|| malformed(block, "corner point is not `(x,y)`"))?;
    let mut parts = inner.split(',').map(str::trim);
    let (Some(x), Some(y), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(malformed(block, "corner point is not two coordinates"));
    };
    match (x.parse::<f64>(), y.parse::<f64>()) {
        (Ok(x), Ok(y)) if x.is_finite() && y.is_finite() => Ok((x, y)),
        _ => Err(malformed(block, "unparsable corner coordinates")),
    }
}

fn malformed(block: &str, detail: &str) -> ReferencerError {
    ReferencerError::Malformed {
        detail: format!("StructMetadata {block}: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{EosGrid, parse_grids};
    use swath_core::ingest::ReferencerError;
    use swath_core::manifest::GeorefCrs;

    /// The real VNP09GA StructMetadata.0 text, committed verbatim from the
    /// bake-off granule (`VNP09GA.A2012019.h33v12.002.2023122182434.h5`).
    const VNP09GA: &str = include_str!("../tests/data/structmetadata_vnp09ga.txt");

    #[test]
    #[allow(clippy::float_cmp)] // exact-copy fields legitimately compare equal
    fn vnp09ga_grids_parse_to_known_values() {
        let grids = parse_grids(VNP09GA).unwrap();
        assert_eq!(grids.len(), 2);

        let expected_proj = "+proj=sinu +lon_0=0 +x_0=0 +y_0=0 +R=6371007.181 +units=m +no_defs";
        let km = &grids[0];
        assert_eq!(km.name, "VIIRS_Grid_1km_2D");
        assert_eq!((km.xdim, km.ydim), (1200, 1200));
        assert_eq!(km.crs, GeorefCrs::Proj4(expected_proj.to_owned()));
        // Corner points straight from the text; cell size derived.
        assert!((km.transform.origin_x - 16_679_257.795).abs() < 1e-6);
        assert!((km.transform.origin_y - -3_335_851.559).abs() < 1e-6);
        assert!((km.transform.pixel_width - 926.625_433_055_833).abs() < 1e-6);
        assert!((km.transform.pixel_height + 926.625_433_055_833).abs() < 1e-6);
        assert_eq!(km.transform.row_rotation, 0.0);

        let m500 = &grids[1];
        assert_eq!(m500.name, "VIIRS_Grid_500m_2D");
        assert_eq!((m500.xdim, m500.ydim), (2400, 2400));
        assert!((m500.transform.pixel_width - 463.312_716_527_916).abs() < 1e-6);
        // Same footprint, half the cell size, same CRS.
        assert_eq!(m500.transform.origin_x, km.transform.origin_x);
        assert_eq!(m500.crs, km.crs);
    }

    #[test]
    fn grid_corners_close_at_the_lower_right() {
        // UL + dims * cell must land exactly on LowerRightMtrs — the
        // internal consistency GDAL relies on for these products.
        let grids = parse_grids(VNP09GA).unwrap();
        for grid in &grids {
            #[allow(clippy::cast_precision_loss)]
            let (lrx, lry) = grid
                .transform
                .pixel_to_crs(grid.xdim as f64, grid.ydim as f64);
            assert!((lrx - 17_791_208.314_667).abs() < 1e-3, "{}", grid.name);
            assert!((lry - -4_447_802.078_667).abs() < 1e-3, "{}", grid.name);
        }
    }

    #[test]
    fn text_without_grids_yields_none_found() {
        assert_eq!(
            parse_grids("GROUP=SwathStructure\nEND_GROUP=SwathStructure\n").unwrap(),
            Vec::<EosGrid>::new()
        );
    }

    #[test]
    fn unsupported_projection_and_origin_error_loudly() {
        let ps = VNP09GA.replacen("HE5_GCTP_SNSOID", "HE5_GCTP_UTM", 1);
        let err = parse_grids(&ps).unwrap_err();
        assert!(
            matches!(&err, ReferencerError::Unsupported { detail } if detail.contains("HE5_GCTP_UTM")),
            "{err}"
        );

        let origin = VNP09GA.replacen("HE5_HDFE_GD_UL", "HE5_HDFE_GD_LR", 1);
        let err = parse_grids(&origin).unwrap_err();
        assert!(matches!(err, ReferencerError::Unsupported { .. }), "{err}");
    }

    #[test]
    fn missing_required_fields_are_malformed() {
        let broken = VNP09GA.replacen("UpperLeftPointMtrs", "SomewhereElseMtrs", 1);
        let err = parse_grids(&broken).unwrap_err();
        assert!(
            matches!(&err, ReferencerError::Malformed { detail } if detail.contains("UpperLeftPointMtrs")),
            "{err}"
        );

        let bad_point = VNP09GA.replacen("(16679257.795000,", "(sixteen-million,", 1);
        assert!(parse_grids(&bad_point).is_err());
    }
}
