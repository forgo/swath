// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `render_tile`: the tiler engine's orchestration (ARCHITECTURE.md §5) —
//! read → warp → pixel ops → encode, returning the encoded tile *and* the
//! [`Trace`] that explains it (REQUIREMENTS.md R4: the explanation is the
//! same data tests assert against).
//!
//! # What this module fixes, and what it defers
//!
//! - **Strategy is [`Strategy::Live`] or [`Strategy::Overview`]** — the
//!   materialization planner (issue #37) does not exist yet; until it
//!   lands, the tiler itself makes exactly one local choice (#38): when a
//!   band's source carries an embedded overview coarse enough for the
//!   target resolution (GDAL's selection rule — see
//!   `window::select_overview`), the read is served from that overview
//!   IFD and the Trace records `Overview { level }` (the decimation
//!   factor). The Trace reports `Overview` only when **every** band that
//!   actually read pixels read the same overview level; any mix (or a
//!   full-res read anywhere) stays `Live` — one tile, one honest
//!   decision. This is a deliberately minimal seam: #37's planner will
//!   own the strategy choice and subsume this per-render selection.
//! - **The target CRS is fixed to Web Mercator** ([`Crs::WEB_MERCATOR`]):
//!   `WebMercatorQuad` is the only TMS in Phase 1 (`TileCoord` is defined
//!   on it). Other target TMSs widen [`TileRequest`] later.
//! - **I/O is async, compute is synchronous** (ARCHITECTURE.md §11): source
//!   reads await; warp/eval/encode run inline on the calling task.
//!   `spawn_blocking`/`rayon` offload is an open question (§16.7) deferred
//!   until a server actually feels the latency — noted, not built.
//!
//! # Trace semantics
//!
//! - [`Trace::provenance`] is the concatenation of every band's real
//!   [`WindowData::provenance`] ranges, in band-declaration order (bands in
//!   the order [`RenderPlan::inputs`] declares them, ranges in fetch order
//!   within each band) — a deterministic ordering, asserted by tests. Each
//!   range's `path` names the object it was fetched from, which is the
//!   per-band attribution; no extra label field is needed while every band
//!   is its own asset.
//! - [`Trace::bytes_read`] is the sum of all bands'
//!   [`WindowData::bytes_read`] — window reads only. Header/metadata I/O
//!   during `describe` is not counted: the `RasterSource` port reports
//!   fetch provenance for pixel reads, not metadata (a future port
//!   extension if header accounting is ever wanted).
//! - [`Trace::timings`] are **best-effort wall-clock measurements**
//!   (`std::time::Instant`, taken here — swath-core stays clock-free) and
//!   are inherently non-deterministic: tests assert presence and sanity,
//!   never equality, and determinism tests must exclude them. `read_ms`
//!   covers all source I/O (`describe` + `read_window`); `total_ms` is
//!   recorded, not derived, so parts need not sum to it once stages overlap
//!   under future concurrency.
//! - [`Trace::ingest_to_pixel_ms`] is the north-star number (REQUIREMENTS.md
//!   §3): when the request carries [`TileRequest::ingested_at`] (its assets
//!   were resolved from a catalog granule), the Trace records
//!   `render_completed_wall_clock − ingested_at`, computed **here at Trace
//!   assembly** — this crate already owns the render's clocks (`Instant`
//!   timings above; wall clock via `SystemTime` for this one subtraction),
//!   the core stays clock-free (`Datetime::to_unix_millis` is pure calendar
//!   math), and the API layer stays translation-only. Every render of a
//!   granule-backed layer carries the number (elapsed-since-ingest keeps
//!   being true); the *first* render after ingest is the reported metric.
//!   Clock skew that would make it negative clamps to 0.
//!
//! A tile whose footprint misses every source raster is **not an error**:
//! it renders as a fully transparent tile with empty provenance and zero
//! `bytes_read` — a served empty tile is still explained (R4).

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use swath_core::cache::{TileCache, TileKey};
use swath_core::catalog::Datetime;
use swath_core::crs::Crs;
use swath_core::raster::{AssetRef, RasterInfo};
use swath_core::reproject::{CoordTransform, Reproject, ReprojectError};
use swath_core::source::{BandSelection, RasterSource, ReadLevel, SourceError};
use swath_core::tile::TileCoord;
use swath_core::trace::{Provenance, Strategy, Timings, Trace};

use crate::encode::{EncodeError, encode_png};
use crate::error::RenderError;
use crate::grid::TargetGrid;
use crate::ir::{PlanError, RenderPlan, TileFormat, eval};
use crate::warp::{Resampling, WarpedBuffer, warp};
use crate::window::{clip_to_raster, select_overview, source_extent};

#[cfg(doc)]
use swath_core::source::WindowData;

/// Everything `render_tile` needs to render one tile of one layer: which
/// asset backs each named band input, the compiled [`RenderPlan`], the
/// target tile, its pixel size, and the resampling kernel.
///
/// This is the resolved-layer-lite request shape (ARCHITECTURE.md §5 calls
/// the full form `ResolvedLayer` + `RenderSpec`); the catalog service and
/// planner will construct richer forms of it later — the fields here are
/// only what the render path consumes today.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TileRequest {
    /// Asset backing each band name the plan's
    /// [`inputs`](RenderPlan::inputs) declare. Extra entries are ignored;
    /// a declared input with no entry is [`TileError::MissingBand`].
    pub bands: BTreeMap<String, AssetRef>,
    /// The pixel pipeline to run.
    pub plan: RenderPlan,
    /// The target tile (`WebMercatorQuad`; the target CRS is fixed to Web
    /// Mercator — module docs).
    pub coord: TileCoord,
    /// Tile side length in pixels (256 classic, 512 retina).
    pub tile_size: u32,
    /// Resampling kernel for every band's warp (bilinear for continuous
    /// data, nearest for categorical). Per-band kernels are a later
    /// widening if a mixed plan ever needs one.
    pub resampling: Resampling,
    /// When the granule backing this request's assets was ingested, if the
    /// caller resolved them from the catalog — the zero point of the
    /// ingest-to-pixel timer. `None` (static/fixture layers) leaves
    /// [`Trace::ingest_to_pixel_ms`] unset.
    pub ingested_at: Option<Datetime>,
}

impl TileRequest {
    /// A request rendering `plan` over `bands` for `coord` at `tile_size`
    /// pixels with `resampling`.
    #[must_use]
    pub fn new(
        bands: BTreeMap<String, AssetRef>,
        plan: RenderPlan,
        coord: TileCoord,
        tile_size: u32,
        resampling: Resampling,
    ) -> Self {
        Self {
            bands,
            plan,
            coord,
            tile_size,
            resampling,
            ingested_at: None,
        }
    }

    /// This request's assets came from a granule ingested at `ingested_at`:
    /// the rendered tile's Trace will carry the ingest-to-pixel latency.
    #[must_use]
    pub fn with_ingested_at(mut self, ingested_at: Datetime) -> Self {
        self.ingested_at = Some(ingested_at);
        self
    }
}

/// An encoded output tile: the bytes and the format they are encoded in.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodedTile {
    /// The encoding of [`bytes`](Self::bytes).
    pub format: TileFormat,
    /// The encoded image.
    pub bytes: Vec<u8>,
}

/// Why `render_tile` failed, with the band/stage where it happened.
///
/// Per-pixel data conditions (nodata, out-of-domain points, a tile off the
/// raster entirely) are never errors — they land in the validity mask and
/// the Trace. Only structural problems surface here.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TileError {
    /// The plan declares no input bands, so there is nothing to render.
    #[error("render plan declares no input bands")]
    NoBands,

    /// The plan declares a band input the request maps to no asset.
    #[error("band `{band}` has no asset in the request")]
    MissingBand {
        /// The unmapped band name.
        band: String,
    },

    /// A source read (`describe` or `read_window`) failed for a band.
    #[error("source read failed for band `{band}`")]
    Source {
        /// The band whose asset was being read.
        band: String,
        /// The underlying source error.
        #[source]
        source: SourceError,
    },

    /// No transform from the target CRS to the source CRS could be built.
    #[error("cannot reproject {crs_to} -> {crs_from}")]
    Reproject {
        /// The target (tile) CRS.
        crs_to: Crs,
        /// The source (asset) CRS.
        crs_from: Crs,
        /// The underlying reprojection error.
        #[source]
        source: ReprojectError,
    },

    /// Two input assets sit in different CRSs. Unsupported by design for
    /// now: rendering would need a per-band transform and the Trace a
    /// per-band `crs_from`; refusing loudly beats silently picking one.
    #[error(
        "mixed source CRSs are unsupported: band `{band}` ({asset}) is in \
         {found}, but band `{first_band}` ({first_asset}) established {expected}"
    )]
    MixedCrs {
        /// The CRS the first band established.
        expected: Crs,
        /// The first band's name.
        first_band: String,
        /// The first band's asset.
        first_asset: AssetRef,
        /// The mismatched CRS.
        found: Crs,
        /// The band that mismatched.
        band: String,
        /// Its asset.
        asset: AssetRef,
    },

    /// Source-window computation or the warp kernel failed for a band.
    #[error("warp stage failed for band `{band}`")]
    Warp {
        /// The band being warped.
        band: String,
        /// The underlying kernel error.
        #[source]
        source: RenderError,
    },

    /// The render plan could not be evaluated.
    #[error("pixel ops failed")]
    Plan(#[from] PlanError),

    /// The evaluated tile could not be encoded.
    #[error("encode failed")]
    Encode(#[from] EncodeError),
}

/// Pixel margin added around each band's source window for resampling
/// support: bilinear needs 1 source pixel at scale ≥ 1 and up to
/// `ceil(1/scale) + 1` when the warp decimates; 4 covers every tile a
/// z ≥ 11 pyramid asks of 30 m HLS sources with headroom (the golden
/// suite's worst case is radius 2). Nearest sampling needs no support but
/// keeps 1 pixel of slack against boundary rounding.
const fn window_margin(resampling: Resampling) -> u32 {
    match resampling {
        Resampling::Nearest => 1,
        Resampling::Bilinear(_) => 4,
    }
}

/// Milliseconds of a duration, saturating (a render is never near
/// `u64::MAX` ms; the conversion is total anyway).
fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Wall-clock milliseconds elapsed since `ingested` — the ingest-to-pixel
/// number, taken at Trace assembly (module docs). Clamped at 0: clock skew
/// must never produce a nonsensical negative latency.
fn ingest_to_pixel_ms(ingested: &Datetime) -> u64 {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    u64::try_from(now_ms.saturating_sub(ingested.to_unix_millis())).unwrap_or(0)
}

/// One band's resolved inputs after the describe phase.
struct BandAsset<'a> {
    name: &'a str,
    asset: &'a AssetRef,
    info: RasterInfo,
}

/// The per-render geometry shared by every band's read/warp.
struct RenderGeometry {
    grid: TargetGrid,
    resampling: Resampling,
    margin: u32,
}

/// The running I/O and compute accounting a render accumulates band by
/// band — the raw material of [`Trace::provenance`], [`Trace::bytes_read`],
/// and the read/warp [`Timings`].
#[derive(Default)]
struct Accounting {
    read_time: Duration,
    warp_time: Duration,
    provenance: Vec<Provenance>,
    bytes_read: u64,
}

/// Resolves and describes every declared band, in declaration order, with
/// one `describe` per distinct asset. Describe I/O counts toward `read`.
async fn describe_bands<'a, S: RasterSource>(
    source: &S,
    request: &'a TileRequest,
    acc: &mut Accounting,
) -> Result<Vec<BandAsset<'a>>, TileError> {
    let mut described: Vec<(&AssetRef, RasterInfo)> = Vec::new();
    let mut bands = Vec::with_capacity(request.plan.inputs.len());
    for input in &request.plan.inputs {
        let asset = request
            .bands
            .get(&input.name)
            .ok_or_else(|| TileError::MissingBand {
                band: input.name.clone(),
            })?;
        let cached = described.iter().find(|(a, _)| *a == asset);
        let info = if let Some((_, info)) = cached {
            info.clone()
        } else {
            let read_started = Instant::now();
            let info = source
                .describe(asset)
                .await
                .map_err(|source| TileError::Source {
                    band: input.name.clone(),
                    source,
                })?;
            acc.read_time += read_started.elapsed();
            described.push((asset, info.clone()));
            info
        };
        bands.push(BandAsset {
            name: &input.name,
            asset,
            info,
        });
    }
    Ok(bands)
}

/// Windows, reads, and warps one band. A band whose source window misses
/// the raster entirely yields an all-invalid plane — nothing read, nothing
/// to explain but the absence itself.
///
/// Returns the warped plane plus this band's **level vote** for the
/// Trace's decision: `None` when nothing was read, `Some(None)` for a
/// full-resolution read, `Some(Some(factor))` for an overview read.
///
/// Overview selection (#38, the pre-planner mini-decision — module docs):
/// the source extent is traced once; if the source's `overview_levels`
/// contain a factor eligible under GDAL's rule (`select_overview`), the
/// read targets that level. The request stays in **full-resolution**
/// coordinates (the `ReadLevel` port contract) with the resampling margin
/// scaled by the factor (a margin of N overview pixels spans N x factor
/// full-res pixels); the adapter returns the overview grid it actually
/// read inside `WindowData::grid`, and the warp runs off that grid
/// unchanged — no overview arithmetic here.
async fn read_and_warp<S: RasterSource>(
    source: &S,
    band: &BandAsset<'_>,
    geometry: &RenderGeometry,
    to_source: &dyn CoordTransform,
    acc: &mut Accounting,
) -> Result<(WarpedBuffer, Option<Option<u32>>), TileError> {
    let grid = &geometry.grid;
    let stage = |source| TileError::Warp {
        band: band.name.to_owned(),
        source,
    };
    let nothing = || {
        let len = grid.width() as usize * grid.height() as usize;
        WarpedBuffer {
            width: grid.width(),
            height: grid.height(),
            values: vec![0.0; len],
            valid: vec![false; len],
        }
    };
    let Some(extent) = source_extent(grid, &band.info, to_source).map_err(stage)? else {
        return Ok((nothing(), None));
    };
    let factor = select_overview(
        &band.info.overview_levels,
        &extent,
        grid.width(),
        grid.height(),
    );
    let (level, margin) = match factor {
        Some(f) => (
            ReadLevel::Overview { factor: f },
            geometry.margin.saturating_mul(f),
        ),
        None => (ReadLevel::FullRes, geometry.margin),
    };
    let Some(window) = clip_to_raster(&extent, margin, &band.info) else {
        return Ok((nothing(), None));
    };

    let read_started = Instant::now();
    let data = source
        .read_window(band.asset, window, BandSelection::Single(0), level)
        .await
        .map_err(|source| TileError::Source {
            band: band.name.to_owned(),
            source,
        })?;
    acc.read_time += read_started.elapsed();
    acc.provenance.extend(data.provenance.iter().cloned());
    acc.bytes_read += data.bytes_read;

    let warp_started = Instant::now();
    let buffer = warp(&data, to_source, grid, geometry.resampling).map_err(stage)?;
    acc.warp_time += warp_started.elapsed();
    Ok((buffer, Some(factor)))
}

/// Renders one tile end-to-end: for each declared band — describe (once
/// per distinct asset), compute the source window, read, warp — then run
/// the plan's pixel ops, encode, and assemble the [`Trace`].
///
/// Generic over the two ports it consumes; no dynamic dispatch is needed
/// while every caller knows its adapters at compile time.
///
/// # Errors
///
/// Any [`TileError`]; see its variants for the taxonomy. A tile that
/// simply has no source data where it falls is **not** an error (module
/// docs).
pub async fn render_tile<S: RasterSource, R: Reproject + ?Sized>(
    source: &S,
    reproject: &R,
    request: &TileRequest,
) -> Result<(EncodedTile, Trace), TileError> {
    let started = Instant::now();
    let geometry = RenderGeometry {
        grid: TargetGrid::for_tile(request.coord, request.tile_size),
        resampling: request.resampling,
        margin: window_margin(request.resampling),
    };
    let mut acc = Accounting::default();

    // Phase 1: resolve and describe every declared band.
    let bands = describe_bands(source, request, &mut acc).await?;
    let Some(first) = bands.first() else {
        return Err(TileError::NoBands);
    };

    // Phase 2: one source CRS for all bands, one transform for the render.
    let crs_from = first.info.crs;
    if let Some(mismatch) = bands.iter().find(|b| b.info.crs != crs_from) {
        return Err(TileError::MixedCrs {
            expected: crs_from,
            first_band: first.name.to_owned(),
            first_asset: first.asset.clone(),
            found: mismatch.info.crs,
            band: mismatch.name.to_owned(),
            asset: mismatch.asset.clone(),
        });
    }
    let to_source = reproject
        .transformer(Crs::WEB_MERCATOR, crs_from)
        .map_err(|source| TileError::Reproject {
            crs_to: Crs::WEB_MERCATOR,
            crs_from,
            source,
        })?;

    // Phase 3: window → read → warp, per band in declaration order,
    // collecting each band's level vote for the decision below.
    let mut warped: Vec<WarpedBuffer> = Vec::with_capacity(bands.len());
    let mut votes: Vec<Option<u32>> = Vec::with_capacity(bands.len());
    for band in &bands {
        let (buffer, vote) =
            read_and_warp(source, band, &geometry, to_source.as_ref(), &mut acc).await?;
        warped.push(buffer);
        if let Some(vote) = vote {
            votes.push(vote);
        }
    }

    // Phase 4: pixel ops, then encode.
    let pixel_started = Instant::now();
    let tile = eval(&request.plan, &warped)?;
    let pixel_time = pixel_started.elapsed();

    let encode_started = Instant::now();
    let bytes = match request.plan.output.format {
        TileFormat::Png => encode_png(&tile)?,
    };
    let encode_time = encode_started.elapsed();

    // Phase 5: the Trace — every field real (R4).
    let mut sources: Vec<AssetRef> = Vec::new();
    for band in &bands {
        if !sources.contains(band.asset) {
            sources.push(band.asset.clone());
        }
    }
    // Overview only when every band that read pixels read the same
    // overview level; anything else is honestly Live (module docs; #37's
    // planner subsumes this).
    let decision = match votes.first() {
        Some(&Some(level)) if votes.iter().all(|&v| v == Some(level)) => {
            Strategy::Overview { level }
        }
        _ => Strategy::Live,
    };
    let trace = Trace {
        decision,
        source: first.asset.clone(),
        sources,
        crs_from,
        crs_to: Crs::WEB_MERCATOR,
        bytes_read: acc.bytes_read,
        provenance: acc.provenance,
        timings: Timings {
            read_ms: millis(acc.read_time),
            warp_ms: millis(acc.warp_time),
            pixel_ops_ms: millis(pixel_time),
            encode_ms: millis(encode_time),
            total_ms: millis(started.elapsed()),
        },
        ingest_to_pixel_ms: request.ingested_at.as_ref().map(ingest_to_pixel_ms),
    };

    Ok((
        EncodedTile {
            format: request.plan.output.format,
            bytes,
        },
        trace,
    ))
}

/// [`render_tile`] behind a write-through [`TileCache`] (#36,
/// ARCHITECTURE.md §8's `write-through cache?` box): consult the cache
/// under `key` first — a hit serves the stored bytes with a
/// [`Strategy::CacheHit`] Trace; a miss renders live exactly as
/// [`render_tile`] does, then writes the encoded tile through.
///
/// # Cache-failure policy (the port leaves it to this caller)
///
/// The cache can never fail a response: a failed or corrupt `get` is
/// logged and treated as a miss, and a failed write-through `put` is
/// logged and the freshly rendered tile served anyway. The write is
/// awaited inline (this crate owns no executor to detach it onto); it is
/// "non-blocking" in the sense that matters — its *failure* never blocks
/// the response. Detaching the write onto a runtime is a caller-level
/// optimization for when a profile shows the put latency.
///
/// # The cache-hit Trace (documented decisions)
///
/// Every field stays honest to its definition:
///
/// - `decision` is [`Strategy::CacheHit`] with the key — the one field
///   that makes a hit unmistakable;
/// - `bytes_read` is **0** and `provenance` empty: they count *source*
///   reads, and a hit touches no source (the payload size is on the wire
///   as `Content-Length`; no new Trace field is added — see the field
///   docs in swath-core);
/// - `source`/`sources` name the cache entry (`cache://<key>`): where
///   the bytes actually came from;
/// - `crs_from == crs_to` (Web Mercator): the cached tile is already in
///   the tile CRS, no reprojection was consulted;
/// - `timings` are zero except `total_ms` (the cache fetch is the whole
///   render);
/// - `ingest_to_pixel_ms` is `None`: the north-star number belongs to
///   the *first* render after ingest, which is by construction a miss.
///
/// # Errors
///
/// Exactly [`render_tile`]'s errors — cache failures never surface.
pub async fn render_tile_cached<S, R, C>(
    source: &S,
    reproject: &R,
    cache: &C,
    key: &TileKey,
    request: &TileRequest,
) -> Result<(EncodedTile, Trace), TileError>
where
    S: RasterSource,
    R: Reproject + ?Sized,
    C: TileCache,
{
    let started = Instant::now();
    let content_type = request.plan.output.format.content_type();
    match cache.get(key).await {
        Ok(Some(entry)) if entry.content_type == content_type => {
            return Ok(cache_hit(key, request, entry.bytes, started.elapsed()));
        }
        Ok(Some(entry)) => {
            // The key binds the plan and the plan fixes the format, so a
            // mismatched content type is a foreign/corrupt entry: an
            // honest miss, logged.
            tracing::warn!(
                key = %key,
                stored = %entry.content_type,
                expected = %content_type,
                "cache entry content type mismatches the plan output; rendering live"
            );
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(key = %key, error = %err, "cache get failed; rendering live");
        }
    }

    let (encoded, trace) = render_tile(source, reproject, request).await?;
    if let Err(err) = cache.put(key, &encoded.bytes, content_type).await {
        tracing::warn!(
            key = %key,
            error = %err,
            "cache write-through failed; serving the rendered tile"
        );
    }
    Ok((encoded, trace))
}

/// The served-from-cache result: the stored bytes and the hit Trace
/// (field semantics documented on [`render_tile_cached`]).
fn cache_hit(
    key: &TileKey,
    request: &TileRequest,
    bytes: Vec<u8>,
    total: Duration,
) -> (EncodedTile, Trace) {
    let entry = AssetRef::new(format!("cache://{key}"));
    let trace = Trace {
        decision: Strategy::CacheHit {
            key: key.as_str().to_owned(),
        },
        source: entry.clone(),
        sources: vec![entry],
        crs_from: Crs::WEB_MERCATOR,
        crs_to: Crs::WEB_MERCATOR,
        bytes_read: 0,
        provenance: Vec::new(),
        timings: Timings {
            total_ms: millis(total),
            ..Timings::default()
        },
        ingest_to_pixel_ms: None,
    };
    (
        EncodedTile {
            format: request.plan.output.format,
            bytes,
        },
        trace,
    )
}
