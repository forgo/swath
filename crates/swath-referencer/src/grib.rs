// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! GRIB2 generation via `gribberish` (prototype 0001, productionized).
//!
//! Grouping model (proven equivalent to kerchunk's `scan_grib` on a GFS
//! sample): one single-chunk array per message (key `"0.0"`), whose ref
//! spans the complete GRIB2 message — the reader decodes the message to get
//! the grid. Names are cfgrib-style (see [`cfgrib_variable_name`]); repeated
//! variables get `_1`, `_2`, … suffixes. Codecs record the section-5
//! packing template as `grib2:*` strings — the sidecar derives the same
//! vocabulary independently from eccodes' `packingType`.
//!
//! GRIB2 arrays carry **no georef yet**: the grid-definition-template →
//! CRS/transform mapping is real work with its own known-answer tests, and
//! no GRIB dataset is on the serving path today (VNP09GA, the legacy-primary
//! dataset, is HDF-EOS — ADR 0008). Recorded honestly rather than guessed;
//! deferral tracked in `docs/ROADMAP.md`.

use std::collections::HashMap;
use std::path::Path;

use gribberish::message::read_messages;
use gribberish::templates::product::tables::FixedSurfaceType;
use swath_core::ingest::ReferencerError;
use swath_core::manifest::{ChunkRef, ManifestVersion, VirtualArray, VirtualManifest};

/// Generates the manifest for a GRIB2 granule.
pub(crate) fn generate(granule: &Path) -> Result<VirtualManifest, ReferencerError> {
    let data = std::fs::read(granule).map_err(|e| ReferencerError::Backend {
        detail: format!("reading `{}`", granule.display()),
        source: Box::new(e),
    })?;
    let source = granule.display().to_string();

    let mut arrays: Vec<VirtualArray> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (index, message) in read_messages(&data).enumerate() {
        let malformed = |what: &str, detail: String| ReferencerError::Malformed {
            detail: format!("message {index} of `{source}`: {what}: {detail}"),
        };
        let offset = message.byte_offset() as u64;
        let length = message.len() as u64;
        if length == 0 {
            return Err(malformed(
                "indicator section",
                "zero total length".to_owned(),
            ));
        }
        let abbrev = message
            .variable_abbrev()
            .map_err(|e| malformed("variable abbrev", e.to_string()))?;
        let (surface, surface_value) = message
            .first_fixed_surface()
            .map_err(|e| malformed("first fixed surface", e.to_string()))?;
        let (nj, ni) = message
            .grid_dimensions()
            .map_err(|e| malformed("grid dimensions", e.to_string()))?;
        let template = message
            .data_template_number()
            .map_err(|e| malformed("data representation template", e.to_string()))?;

        let base = cfgrib_variable_name(&abbrev, &surface, surface_value);
        let n = seen.entry(base.clone()).or_insert(0);
        let name = if *n == 0 { base } else { format!("{base}_{n}") };
        *n += 1;

        arrays.push(VirtualArray {
            name,
            shape: vec![nj as u64, ni as u64],
            chunks: vec![nj as u64, ni as u64],
            // gribberish decodes GRIB2 grids to f64 (`Message::data` ->
            // Vec<f64>), matching the sidecar's kerchunk GRIB codec dtype.
            dtype: "float64".to_owned(),
            codecs: vec![packing_codec(template)],
            georef: None,
            refs: vec![ChunkRef {
                key: "0.0".to_owned(),
                path: source.clone(),
                offset,
                length,
            }],
        });
    }
    if arrays.is_empty() {
        return Err(ReferencerError::Malformed {
            detail: format!("no GRIB messages found in `{source}`"),
        });
    }
    Ok(VirtualManifest {
        manifest_version: ManifestVersion,
        generator: crate::GENERATOR.to_owned(),
        source,
        arrays,
    })
}

/// Section 5 (data representation) template number → codec string. The
/// manifest records HOW the chunk bytes decode; the sidecar derives the same
/// strings independently from eccodes' `packingType`, so exact agreement is
/// part of the conformance contract.
fn packing_codec(template: u16) -> String {
    match template {
        0 => "grib2:simple".to_owned(),
        2 => "grib2:complex".to_owned(),
        3 => "grib2:complex-spatial-diff".to_owned(),
        4 => "grib2:ieee-float".to_owned(),
        40 => "grib2:jpeg2000".to_owned(),
        41 => "grib2:png".to_owned(),
        42 => "grib2:aec".to_owned(),
        n => format!("grib2:template{n}"),
    }
}

/// cfgrib-compatible variable name from gribberish's NCEP-style abbreviation.
///
/// The reference sidecar (kerchunk `scan_grib`, backed by cfgrib) names
/// arrays with eccodes `shortName`s made into identifiers (`t`, `10u` →
/// `u10`, `prmsl`). gribberish speaks NCEP abbreviations (`TMP`, `UGRD`,
/// `PRMSL`), so we translate: an abbrev → shortName table (the variables the
/// harness exercises plus obvious neighbors — the full WMO/eccodes table is
/// deliberate later work the conformance harness will drive), a level prefix
/// for height-above-ground fields (`2t` → `t2m`, `10u` → `u10`), then
/// cfgrib's identifier rule (leading digits rotate to the end). Unknown
/// abbreviations fall back to the lowercased abbreviation, which the
/// equivalence harness flags.
fn cfgrib_variable_name(
    abbrev: &str,
    surface: &FixedSurfaceType,
    surface_value: Option<f64>,
) -> String {
    let base = match abbrev {
        "TMP" => "t",
        "UGRD" => "u",
        "VGRD" => "v",
        "RH" => "r",
        "HGT" => "gh",
        "SPFH" => "q",
        "DPT" => "dpt",
        "PRMSL" => "prmsl",
        other => return other.to_lowercase(),
    };

    // eccodes gives height-above-ground surface fields level-qualified
    // shortNames (2t, 10u); cfgrib rewrites them into identifiers by moving
    // the digits to the end (t2m carries an extra 'm' by eccodes convention;
    // wind components are plain u10/v10).
    if matches!(surface, FixedSurfaceType::SpecifiedHeightLevelAboveGround)
        && let Some(level) = surface_value
    {
        let suffix = if base == "t" { "m" } else { "" };
        return format!("{base}{level:.0}{suffix}");
    }
    base.to_owned()
}

#[cfg(test)]
mod tests {
    use super::{FixedSurfaceType, cfgrib_variable_name, packing_codec};

    #[test]
    fn packing_codec_covers_the_wmo_templates() {
        assert_eq!(packing_codec(0), "grib2:simple");
        assert_eq!(packing_codec(3), "grib2:complex-spatial-diff");
        assert_eq!(packing_codec(42), "grib2:aec");
        assert_eq!(packing_codec(199), "grib2:template199");
    }

    #[test]
    fn cfgrib_names_match_the_reference_vocabulary() {
        // The GFS sample's three variables, exactly as kerchunk/cfgrib named
        // them in the bake-off (prototype 0001 §7).
        assert_eq!(
            cfgrib_variable_name("TMP", &FixedSurfaceType::IsobaricSurface, Some(850.0)),
            "t"
        );
        assert_eq!(
            cfgrib_variable_name(
                "UGRD",
                &FixedSurfaceType::SpecifiedHeightLevelAboveGround,
                Some(10.0)
            ),
            "u10"
        );
        assert_eq!(
            cfgrib_variable_name("PRMSL", &FixedSurfaceType::MeanSeaLevel, None),
            "prmsl"
        );
        // The eccodes 't2m' quirk and the unknown-abbrev fallback.
        assert_eq!(
            cfgrib_variable_name(
                "TMP",
                &FixedSurfaceType::SpecifiedHeightLevelAboveGround,
                Some(2.0)
            ),
            "t2m"
        );
        assert_eq!(
            cfgrib_variable_name("WEIRD", &FixedSurfaceType::GroundOrWater, None),
            "weird"
        );
    }
}
