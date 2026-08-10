// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The vendored colormap lookup tables behind [`Colormap`]'s palette
//! variants.
//!
//! The tables are matplotlib's published 256-entry byte LUTs, committed
//! verbatim as `colormaps/luts.json` (provenance, regeneration recipe, and
//! data licenses: `colormaps/README.md`) and embedded via `include_str!` —
//! the pinned-oracle pattern the golden suites use, applied to palettes.
//! The JSON is parsed once, lazily; a malformed fixture is a build defect
//! and panics on first use (the colormap golden tests exercise every
//! table, so it cannot ship).
//!
//! # No interpolation, by design
//!
//! [`lut`] hands back the raw 256-entry table; [`crate::ir::eval`] indexes
//! it by the **quantized** gray value — `clamp(0.0, 255.0)` then truncate
//! toward zero, exactly the numpy `astype(uint8)` arithmetic the final
//! quantization applies (`ir` module docs). Linear interpolation between
//! entries is deliberately off: a colormapped pixel is `lut[q(gray)]`, the
//! same `q` that would have produced the grayscale pixel, so the palette
//! path stays bit-relatable to the oracle-validated gray path (the
//! two-level golden scheme in `tests/golden_ir.rs` asserts exactly that
//! relation).

use std::sync::LazyLock;

use serde::Deserialize;

use crate::ir::Colormap;

/// One 256-entry RGB lookup table.
pub type Lut = [[u8; 3]; 256];

/// The committed fixture: matplotlib's byte LUTs (see the module docs).
const LUTS_JSON: &str = include_str!("colormaps/luts.json");

/// The JSON shape of `colormaps/luts.json`.
#[derive(Deserialize)]
struct LutFile {
    maps: LutMaps,
}

/// The three vendored palettes, by their matplotlib names.
#[derive(Deserialize)]
struct LutMaps {
    viridis: Vec<[u8; 3]>,
    magma: Vec<[u8; 3]>,
    #[serde(rename = "RdYlGn")]
    rdylgn: Vec<[u8; 3]>,
}

/// The parsed tables, one allocation-free array per palette.
struct Tables {
    viridis: Lut,
    magma: Lut,
    rdylgn: Lut,
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let file: LutFile =
        serde_json::from_str(LUTS_JSON).expect("colormaps/luts.json is valid LUT JSON");
    let fixed = |name: &str, entries: Vec<[u8; 3]>| -> Lut {
        entries.try_into().unwrap_or_else(|entries: Vec<[u8; 3]>| {
            panic!(
                "colormaps/luts.json: `{name}` has {} entries, expected 256",
                entries.len()
            )
        })
    };
    Tables {
        viridis: fixed("viridis", file.maps.viridis),
        magma: fixed("magma", file.maps.magma),
        rdylgn: fixed("RdYlGn", file.maps.rdylgn),
    }
});

/// The 256-entry RGB table for `map`, or `None` for
/// [`Colormap::Grayscale`] — the identity map has no table; gray planes
/// pass through untouched.
#[must_use]
pub fn lut(map: Colormap) -> Option<&'static Lut> {
    match map {
        Colormap::Grayscale => None,
        Colormap::Viridis => Some(&TABLES.viridis),
        Colormap::Magma => Some(&TABLES.magma),
        Colormap::RdYlGn => Some(&TABLES.rdylgn),
    }
}
