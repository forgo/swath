// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Golden tests: the kernel vs the GDAL/rio-tiler oracle, self-contained.
//!
//! Each case replays a committed, real warp of an HLS fixture band into a
//! 256-px Web Mercator tile — the same tiles the upstream oracle suite
//! renders — and requires **bit-identical** output:
//!
//! * `tests/data/tile-<z>-<x>-<y>.xf` — every point the kernel asks the
//!   transform for, in call order, as answered by the real proj4rs
//!   EPSG:3857 → UTM transform when the fixture was captured. The replay
//!   transform below plays the sequence back, so these tests need no
//!   projection library.
//! * `tests/data/<band>-<z>-<x>-<y>.src` — the source window a COG reader
//!   returned (placement, raw samples, nodata, source grid) plus the
//!   tile's target grid.
//! * `tests/data/<band>-<z>-<x>-<y>.out` — the expected values + validity.
//!
//! The expected bytes are oracle-anchored: they were produced by a
//! pipeline whose PNG tiles pass a perceptual diff against
//! GDAL/rio-tiler renders of the same fixtures (the swath workspace's
//! golden suite; ADR 0002 — GDAL lives only in test suites, as the
//! correctness bar). Regeneration: the ignored `warp_golden_capture` test
//! in that workspace's `swath-render` crate, run only while its PNG suite
//! is green.
//!
//! The cases pin the kernel's hardest behaviors: the z12 swath-edge tile
//! exercises nodata renormalization and the containing-pixel gate under
//! bilinear, and value-preserving nearest over a categorical mask; the
//! z11 parent tile decimates (kernel scales 256/320 and 256/381), pinning
//! the scaled-triangle anti-aliasing path and GDAL's reciprocal scale
//! snapping.

use std::cell::Cell;
use std::path::PathBuf;

use swath_warp::{
    CoordTransform, GeoTransform, GridBounds, NodataPolicy, PixelWindow, Resampling, SourceBuffer,
    SourceGrid, TargetGrid, TransformError, source_window, warp,
};

/// The resampling-support margin the captures were made with.
const WINDOW_MARGIN: u32 = 4;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// A little-endian cursor over a committed fixture file.
struct Reader {
    bytes: Vec<u8>,
    pos: usize,
}

impl Reader {
    fn open(name: &str) -> Self {
        let path = data_dir().join(name);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|err| panic!("cannot read golden fixture {}: {err}", path.display()));
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> &[u8] {
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        out
    }

    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().expect("4 bytes"))
    }

    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8).try_into().expect("8 bytes"))
    }

    fn f64(&mut self) -> f64 {
        f64::from_le_bytes(self.take(8).try_into().expect("8 bytes"))
    }

    fn done(&self) -> bool {
        self.pos == self.bytes.len()
    }
}

/// Plays back a recorded transform-output sequence in call order.
///
/// Order-based replay is sound because the kernel's call sequence is a
/// documented function of the target grid alone: the densified boundary
/// (per `source_window` and again per `warp`), then one batch per target
/// row. The final bit-identical comparison would fail loudly on any drift.
struct Replay {
    outputs: Vec<(f64, f64)>,
    cursor: Cell<usize>,
}

impl Replay {
    fn load(name: &str) -> Self {
        let mut r = Reader::open(name);
        let n = usize::try_from(r.u64()).expect("count fits usize");
        let outputs = (0..n).map(|_| (r.f64(), r.f64())).collect();
        assert!(r.done(), "{name}: trailing bytes");
        Self {
            outputs,
            cursor: Cell::new(0),
        }
    }

    fn next(&self) -> (f64, f64) {
        let i = self.cursor.get();
        self.cursor.set(i + 1);
        *self
            .outputs
            .get(i)
            .expect("replay exhausted: kernel asked for more points than were recorded")
    }

    fn assert_consumed(&self) {
        assert_eq!(
            self.cursor.get(),
            self.outputs.len(),
            "kernel asked for fewer points than were recorded"
        );
    }
}

impl CoordTransform for Replay {
    fn transform(&self, _x: f64, _y: f64) -> Result<(f64, f64), TransformError> {
        Ok(self.next())
    }

    fn transform_slice(&self, points: &mut [(f64, f64)]) -> Result<(), TransformError> {
        for p in points.iter_mut() {
            *p = self.next();
        }
        Ok(())
    }
}

/// One captured source window + target grid, parsed from a `.src` file.
struct Captured {
    window: PixelWindow,
    samples: Vec<f64>,
    nodata: Option<f64>,
    source_grid: SourceGrid,
    target: TargetGrid,
}

impl Captured {
    fn load(name: &str) -> Self {
        let mut r = Reader::open(name);
        let window = PixelWindow {
            col_off: r.u64(),
            row_off: r.u64(),
            width: r.u64(),
            height: r.u64(),
        };
        let dtype = r.u8();
        let has_nodata = r.u8() != 0;
        let nodata_value = r.f64();
        let source_grid = SourceGrid {
            width: r.u64(),
            height: r.u64(),
            transform: GeoTransform {
                origin_x: r.f64(),
                pixel_width: r.f64(),
                row_rotation: r.f64(),
                origin_y: r.f64(),
                col_rotation: r.f64(),
                pixel_height: r.f64(),
            },
        };
        let bounds = GridBounds {
            min_x: r.f64(),
            min_y: r.f64(),
            max_x: r.f64(),
            max_y: r.f64(),
        };
        let target = TargetGrid::new(bounds, r.u32(), r.u32());
        let len = usize::try_from(window.width * window.height).expect("window fits memory");
        // Exact widening to f64, per the `SourceBuffer::samples` contract.
        let samples: Vec<f64> = match dtype {
            1 => (0..len).map(|_| f64::from(r.u8())).collect(),
            2 => (0..len)
                .map(|_| f64::from(i16::from_le_bytes(r.take(2).try_into().expect("2 bytes"))))
                .collect(),
            other => panic!("{name}: unknown dtype tag {other}"),
        };
        assert!(r.done(), "{name}: trailing bytes");
        Self {
            window,
            samples,
            nodata: has_nodata.then_some(nodata_value),
            source_grid,
            target,
        }
    }
}

/// The expected warp result, parsed from a `.out` file.
fn load_expected(name: &str) -> (u32, u32, Vec<f64>, Vec<bool>) {
    let mut r = Reader::open(name);
    let (w, h) = (r.u32(), r.u32());
    let len = w as usize * h as usize;
    let values = (0..len).map(|_| r.f64()).collect();
    let valid = (0..len).map(|_| r.u8() != 0).collect();
    assert!(r.done(), "{name}: trailing bytes");
    (w, h, values, valid)
}

/// Replays one committed warp and requires bit-identical output.
fn assert_reproduces_golden(band: &str, tile: &str, resampling: Resampling) {
    let replay = Replay::load(&format!("tile-{tile}.xf"));
    let captured = Captured::load(&format!("{band}-{tile}.src"));
    let (w, h, values, valid) = load_expected(&format!("{band}-{tile}.out"));

    // The window computation reproduces the captured read request…
    let window = source_window(
        &captured.target,
        &captured.source_grid,
        &replay,
        WINDOW_MARGIN,
    )
    .expect("window computation")
    .expect("tile intersects the raster");
    assert_eq!(window, captured.window, "{band}-{tile}: source window");

    // …and the kernel reproduces the oracle-anchored output, bit for bit.
    let out = warp(
        &SourceBuffer {
            grid: captured.source_grid,
            window: captured.window,
            samples: &captured.samples,
            nodata: captured.nodata,
        },
        &replay,
        &captured.target,
        resampling,
    )
    .expect("warp");
    replay.assert_consumed();

    assert_eq!((out.width, out.height), (w, h), "{band}-{tile}: dimensions");
    assert_eq!(out.valid, valid, "{band}-{tile}: validity mask");
    let ours: Vec<u64> = out.values.iter().map(|v| v.to_bits()).collect();
    let expected: Vec<u64> = values.iter().map(|v| v.to_bits()).collect();
    assert_eq!(ours, expected, "{band}-{tile}: values (bitwise)");
}

#[test]
fn b04_swath_edge_bilinear_z12_is_bit_identical() {
    assert_reproduces_golden(
        "b04",
        "12-848-1562",
        Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
    );
}

#[test]
fn fmask_swath_edge_nearest_z12_is_bit_identical() {
    assert_reproduces_golden("fmask", "12-848-1562", Resampling::Nearest);
}

#[test]
fn b04_parent_bilinear_z11_decimates_bit_identically() {
    assert_reproduces_golden(
        "b04",
        "11-424-780",
        Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
    );
}
