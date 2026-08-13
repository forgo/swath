// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The batch pyramid writer (crate docs): builds the missing decimation
//! ladder for one asset, level by level, chunk by chunk — idempotent
//! (existing chunks are probed and skipped, never rewritten) and
//! resumable (a killed run left no completed-factor record for its
//! half-built level; the rerun writes exactly the missing chunks and then
//! records it).

use object_store::path::Path;
use object_store::{ObjectStoreExt as _, PutPayload};
use swath_core::raster::{AssetRef, DType, RasterInfo, WindowRequest};
use swath_core::source::{BandSelection, PixelBuffer, RasterSource, ReadLevel, SourceError};

use crate::layout::{
    self, GroupAttrs, LAYOUT_VERSION, Multiscale, MultiscaleLevel, PyramidMeta, PyramidResampling,
    ZarrayMeta,
};
use crate::{PyramidSource, us};

/// What to materialize and how to aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterializeSpec {
    /// Chunk side length of every level array ([`layout::CHUNK`]).
    pub chunk: u32,
    /// Coarsest-level bound: the ladder stops at the first level whose
    /// larger axis fits this many pixels ([`layout::MIN_DIM`]).
    pub min_dim: u32,
    /// Block aggregation: `Average` for continuous data, `Nearest` for
    /// categorical/QA data.
    pub resampling: PyramidResampling,
}

impl MaterializeSpec {
    /// The default spec with the given aggregation.
    #[must_use]
    pub fn with_resampling(resampling: PyramidResampling) -> Self {
        Self {
            resampling,
            ..Self::default()
        }
    }
}

impl Default for MaterializeSpec {
    fn default() -> Self {
        Self {
            chunk: layout::CHUNK,
            min_dim: layout::MIN_DIM,
            resampling: PyramidResampling::Average,
        }
    }
}

/// What one materialization run did — every number observed, none
/// estimated (a rerun over a complete pyramid reports zero writes).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MaterializeReport {
    /// The pyramid root the run wrote under.
    pub root: String,
    /// Factors this run completed (written and recorded), ascending.
    pub factors_completed: Vec<u32>,
    /// Ladder factors that were already complete and were skipped whole.
    pub factors_already_complete: Vec<u32>,
    /// Chunk objects written by this run.
    pub chunks_written: u64,
    /// Chunk objects that already existed and were skipped.
    pub chunks_skipped: u64,
}

/// Why materialization refused or failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MaterializeError {
    /// Reading the source asset failed.
    #[error("source read failed")]
    Source(#[from] SourceError),

    /// Storage or transport failure writing or probing the pyramid.
    #[error("pyramid store i/o failure at `{path}`")]
    Store {
        /// The object being written or probed.
        path: String,
        /// The underlying storage error.
        #[source]
        source: object_store::Error,
    },

    /// A pyramid already exists under this asset's root but records a
    /// different identity (source grid, dtype, chunking, or resampling).
    /// Deliberate and loud: resuming into a mismatched pyramid would mix
    /// generations; remove the stale pyramid to regenerate.
    #[error("existing pyramid conflicts with this run: {detail}")]
    Conflict {
        /// What disagreed.
        detail: String,
    },

    /// The asset cannot be materialized by this layout (multi-band
    /// assets; every serving layer maps one band per asset today).
    #[error("cannot materialize {asset}: {detail}")]
    Unsupported {
        /// The refused asset.
        asset: AssetRef,
        /// Why.
        detail: String,
    },
}

impl<S: RasterSource> PyramidSource<S> {
    /// Materializes the missing overview ladder for `asset` (crate docs:
    /// idempotent, resumable, embedded factors never duplicated).
    ///
    /// Levels are built coarsest-from-finest: each level reads from the
    /// best already-available coarser grid — full resolution, an embedded
    /// overview, or a level this pyramid already completed — through this
    /// same source, so a resumed run and a fresh run take identical
    /// inputs.
    ///
    /// # Errors
    ///
    /// [`MaterializeError`]; see its variants. Single-writer discipline
    /// is assumed (concurrent materializers of the *same* asset would
    /// duplicate work, not corrupt: chunks are content-identical by
    /// construction).
    pub async fn materialize(
        &self,
        asset: &AssetRef,
        spec: &MaterializeSpec,
    ) -> Result<MaterializeReport, MaterializeError> {
        let info = self.inner.describe(asset).await?;
        if info.band_count != 1 {
            return Err(MaterializeError::Unsupported {
                asset: asset.clone(),
                detail: format!(
                    "{} bands; pyramids are single-band (layout v1)",
                    info.band_count
                ),
            });
        }
        let root = layout::pyramid_root(asset.as_str());
        let ladder = layout::ladder(info.width, info.height, &info.overview_levels, spec.min_dim);

        let mut meta = match self.load_attrs_strict(asset, &root).await? {
            Some(attrs) => {
                let meta = attrs.pyramid;
                if !meta.matches(asset.as_str(), &info) {
                    return Err(MaterializeError::Conflict {
                        detail: format!(
                            "stored pyramid records source `{}` ({}x{}, {}), this asset \
                             describes as {}x{} {}",
                            meta.source,
                            meta.width,
                            meta.height,
                            meta.dtype,
                            info.width,
                            info.height,
                            layout::zarr_dtype(info.dtype),
                        ),
                    });
                }
                if meta.chunk != spec.chunk {
                    return Err(MaterializeError::Conflict {
                        detail: format!("stored chunk {} != requested {}", meta.chunk, spec.chunk),
                    });
                }
                if meta.resampling != spec.resampling {
                    return Err(MaterializeError::Conflict {
                        detail: format!(
                            "stored resampling {:?} != requested {:?}",
                            meta.resampling, spec.resampling
                        ),
                    });
                }
                meta
            }
            None => PyramidMeta {
                layout_version: LAYOUT_VERSION,
                source: asset.as_str().to_owned(),
                width: info.width,
                height: info.height,
                dtype: layout::zarr_dtype(info.dtype).to_owned(),
                nodata: info.nodata,
                crs: info.crs.clone(),
                transform: info.transform,
                chunk: spec.chunk,
                resampling: spec.resampling,
                completed: Vec::new(),
            },
        };

        let mut report = MaterializeReport {
            root: root.clone(),
            factors_completed: Vec::new(),
            factors_already_complete: Vec::new(),
            chunks_written: 0,
            chunks_skipped: 0,
        };

        for &factor in &ladder {
            if meta.completed.contains(&factor) {
                report.factors_already_complete.push(factor);
                continue;
            }
            self.build_level(asset, &root, &info, &meta, factor, spec, &mut report)
                .await?;
            meta.completed.push(factor);
            meta.completed.sort_unstable();
            self.write_group_docs(&root, &meta).await?;
            report.factors_completed.push(factor);
        }

        Ok(report)
    }

    /// Loads the group attrs for materialization: unlike the serve path's
    /// tolerant loader, an unreadable document here is a loud
    /// [`MaterializeError::Conflict`] — resuming must never write into a
    /// pyramid it cannot verify.
    async fn load_attrs_strict(
        &self,
        asset: &AssetRef,
        root: &str,
    ) -> Result<Option<GroupAttrs>, MaterializeError> {
        let path_str = layout::zattrs_path(root);
        let path = parse_path(&path_str)?;
        let object = match self.store.get(&path).await {
            Ok(object) => object,
            Err(object_store::Error::NotFound { .. }) => return Ok(None),
            Err(source) => {
                return Err(MaterializeError::Store {
                    path: path_str,
                    source,
                });
            }
        };
        let bytes = object
            .bytes()
            .await
            .map_err(|source| MaterializeError::Store {
                path: path_str.clone(),
                source,
            })?;
        match serde_json::from_slice::<GroupAttrs>(&bytes) {
            Ok(attrs) => Ok(Some(attrs)),
            Err(err) => Err(MaterializeError::Conflict {
                detail: format!(
                    "`{path_str}` exists for {asset} but is not a readable pyramid document: {err}"
                ),
            }),
        }
    }

    /// Writes one level: probes every chunk, reads only the strips that
    /// still have missing chunks, aggregates, writes the missing chunks.
    #[allow(
        clippy::too_many_lines,
        clippy::too_many_arguments,
        clippy::similar_names,
        reason = "one coherent strip loop over deliberately parallel \
                  row/column block bounds; splitting would scatter the \
                  window bookkeeping the comments walk through"
    )]
    async fn build_level(
        &self,
        asset: &AssetRef,
        root: &str,
        info: &RasterInfo,
        meta: &PyramidMeta,
        factor: u32,
        spec: &MaterializeSpec,
        report: &mut MaterializeReport,
    ) -> Result<(), MaterializeError> {
        let (lw, lh) = layout::level_dims(info.width, info.height, factor);
        // The level's .zarray first (idempotent: same bytes every run) —
        // external Zarr readers need it, and a resumed run rewrites it
        // harmlessly.
        let zarray = serde_json::to_vec(&ZarrayMeta::new(lw, lh, info.dtype, info.nodata))
            .expect("ZarrayMeta serializes");
        self.put_object(&layout::zarray_path(root, factor), zarray)
            .await?;

        // The base grid this level decimates from: the coarsest grid
        // already available — full resolution (1), an embedded overview,
        // or a completed pyramid level — whose factor divides this one.
        let base = std::iter::once(1)
            .chain(info.overview_levels.iter().copied())
            .chain(meta.completed.iter().copied())
            .filter(|&s| s < factor && factor.is_multiple_of(s))
            .max()
            .expect("1 always qualifies");
        let q = u64::from(factor / base);

        let chunk = u64::from(spec.chunk);
        let chunks_x = lw.div_ceil(chunk);
        let chunks_y = lh.div_ceil(chunk);
        let fill = info.nodata.unwrap_or(0.0);

        for cy in 0..chunks_y {
            // Which chunks of this row are missing? Existing ones are
            // never rewritten (idempotence), and a fully-present row
            // skips its read entirely (resumption cost is probes, not
            // pixels).
            let mut missing = Vec::new();
            for cx in 0..chunks_x {
                let path_str = layout::chunk_path(root, factor, cy, cx);
                let path = parse_path(&path_str)?;
                match self.store.head(&path).await {
                    Ok(_) => report.chunks_skipped += 1,
                    Err(object_store::Error::NotFound { .. }) => missing.push(cx),
                    Err(source) => {
                        return Err(MaterializeError::Store {
                            path: path_str,
                            source,
                        });
                    }
                }
            }
            if missing.is_empty() {
                continue;
            }

            // One full-width strip read per chunk row. The needed base
            // rows are the strip's level rows times `q`; the request is
            // phrased in full-resolution coordinates (the ReadLevel
            // contract) sized to *cover* those base rows under the base
            // grid's exact ratio, so the covering read is guaranteed to
            // include every sample the aggregation below indexes.
            let r0 = cy * chunk;
            let r1 = (r0 + chunk).min(lh);
            let (_, bh) = layout::level_dims(info.width, info.height, base);
            let b0 = r0 * q;
            let b1 = (r1 * q).min(bh);
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "raster dims far below 2^52; results clamped to the grid"
            )]
            let (full_row_off, full_row_end) = {
                let ry = info.height as f64 / bh as f64;
                (
                    ((b0 as f64 * ry).floor().max(0.0) as u64).min(info.height),
                    ((b1 as f64 * ry).ceil() as u64).min(info.height),
                )
            };
            let window = WindowRequest {
                col_off: 0,
                row_off: full_row_off,
                width: info.width,
                height: full_row_end - full_row_off,
            };
            let level = if base == 1 {
                ReadLevel::FullRes
            } else {
                ReadLevel::Overview { factor: base }
            };
            let data = self
                .read_window(asset, window, BandSelection::Single(0), level)
                .await?;
            let src = as_f64(&data.pixels);
            let src_w = us(data.window.width);
            let src_h = us(data.window.height);
            let (src_col0, src_row0) = (data.window.col_off, data.window.row_off);
            let (base_w, base_h) = (data.grid.width, data.grid.height);
            let nodata = data.nodata;

            // Aggregate the strip: out pixel (row, col) at this level
            // covers base-grid rows [row*q, min((row+1)*q, base_h)) and
            // the same for columns, indexed relative to the window
            // actually read.
            let strip_rows = r1 - r0;
            let mut strip = vec![fill; us(strip_rows * lw)];
            for r in 0..strip_rows {
                let out_row = r0 + r;
                let b_r0 = out_row * q;
                let b_r1 = ((out_row + 1) * q).min(base_h);
                for out_col in 0..lw {
                    let b_c0 = out_col * q;
                    let b_c1 = ((out_col + 1) * q).min(base_w);
                    strip[us(r * lw + out_col)] = aggregate(
                        &src,
                        src_w,
                        src_h,
                        (src_col0, src_row0),
                        (b_c0, b_r0, b_c1, b_r1),
                        nodata,
                        fill,
                        spec.resampling,
                    );
                }
            }

            // Cut and write the missing chunks (padded to full size with
            // the fill value — the Zarr contract for edge chunks).
            for cx in missing {
                let c0 = cx * chunk;
                let c1 = ((cx + 1) * chunk).min(lw);
                let mut block = vec![fill; us(chunk * chunk)];
                for r in 0..strip_rows {
                    block[us(r * chunk)..us(r * chunk + (c1 - c0))]
                        .copy_from_slice(&strip[us(r * lw + c0)..us(r * lw + c1)]);
                }
                let bytes = encode(&block, info.dtype);
                self.put_object(&layout::chunk_path(root, factor, cy, cx), bytes)
                    .await?;
                report.chunks_written += 1;
            }
        }
        Ok(())
    }

    /// Writes the group documents: `.zgroup` and the `.zattrs` carrying
    /// the updated completion record (last-write-wins; single-writer
    /// discipline documented on [`Self::materialize`]).
    async fn write_group_docs(
        &self,
        root: &str,
        meta: &PyramidMeta,
    ) -> Result<(), MaterializeError> {
        self.put_object(
            &layout::zgroup_path(root),
            serde_json::to_vec(&serde_json::json!({"zarr_format": 2})).expect("literal serializes"),
        )
        .await?;
        let attrs = GroupAttrs {
            multiscales: vec![Multiscale {
                version: "0.1".to_owned(),
                name: meta.source.clone(),
                datasets: meta
                    .completed
                    .iter()
                    .map(|&factor| MultiscaleLevel {
                        path: factor.to_string(),
                        factor,
                    })
                    .collect(),
                resampling: meta.resampling,
            }],
            pyramid: meta.clone(),
        };
        self.put_object(
            &layout::zattrs_path(root),
            serde_json::to_vec(&attrs).expect("GroupAttrs serializes"),
        )
        .await
    }

    async fn put_object(&self, path_str: &str, bytes: Vec<u8>) -> Result<(), MaterializeError> {
        let path = parse_path(path_str)?;
        self.store
            .put(&path, PutPayload::from(bytes))
            .await
            .map(|_| ())
            .map_err(|source| MaterializeError::Store {
                path: path_str.to_owned(),
                source,
            })
    }
}

/// Parses an object path, which cannot fail for the paths this layout
/// generates (hex + factors + row.col), but is mapped honestly anyway.
fn parse_path(path: &str) -> Result<Path, MaterializeError> {
    Path::parse(path).map_err(|e| MaterializeError::Store {
        path: path.to_owned(),
        source: object_store::Error::Generic {
            store: "pyramid",
            source: Box::new(e),
        },
    })
}

/// The samples of `pixels` widened to `f64`, in place order.
fn as_f64(pixels: &PixelBuffer) -> Vec<f64> {
    match pixels {
        PixelBuffer::UInt8(v) => v.iter().map(|&s| f64::from(s)).collect(),
        PixelBuffer::Int16(v) => v.iter().map(|&s| f64::from(s)).collect(),
        PixelBuffer::UInt16(v) => v.iter().map(|&s| f64::from(s)).collect(),
        PixelBuffer::Int32(v) => v.iter().map(|&s| f64::from(s)).collect(),
        PixelBuffer::Float32(v) => v.iter().map(|&s| f64::from(s)).collect(),
        PixelBuffer::Float64(v) => v.clone(),
        // PixelBuffer is non_exhaustive; source adapters only produce the
        // variants above.
        _ => unreachable!("pixel buffer variant not produced by any source adapter"),
    }
}

/// Aggregates one base-grid block into one level pixel. `Average` is the
/// nodata-aware mean (all-nodata blocks stay `fill`); `Nearest` takes the
/// block's top-left sample verbatim (categorical data — nodata included,
/// so QA masks decimate without inventing classes).
#[allow(
    clippy::too_many_arguments,
    clippy::similar_names,
    reason = "a tight per-pixel kernel over deliberately parallel block \
              bounds; bundling these into a struct per call would obscure \
              the hot loop for no clarity gain"
)]
fn aggregate(
    src: &[f64],
    src_w: usize,
    src_h: usize,
    (src_col0, src_row0): (u64, u64),
    (b_c0, b_r0, b_c1, b_r1): (u64, u64, u64, u64),
    nodata: Option<f64>,
    fill: f64,
    resampling: PyramidResampling,
) -> f64 {
    #[allow(
        clippy::float_cmp,
        reason = "nodata sentinels compare by exact identity by convention \
                  (GDAL semantics); a margin would misclassify real data"
    )]
    let is_valid = |v: f64| match nodata {
        Some(nd) if nd.is_nan() => !v.is_nan(),
        Some(nd) => v != nd,
        None => true,
    };
    let at = |col: u64, row: u64| -> Option<f64> {
        let c = col.checked_sub(src_col0)?;
        let r = row.checked_sub(src_row0)?;
        let (c, r) = (us(c), us(r));
        (c < src_w && r < src_h).then(|| src[r * src_w + c])
    };
    match resampling {
        PyramidResampling::Nearest => at(b_c0, b_r0).unwrap_or(fill),
        PyramidResampling::Average => {
            let mut sum = 0.0;
            let mut count = 0u64;
            for row in b_r0..b_r1 {
                for col in b_c0..b_c1 {
                    if let Some(v) = at(col, row)
                        && is_valid(v)
                    {
                        sum += v;
                        count += 1;
                    }
                }
            }
            if count == 0 {
                fill
            } else {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "block sample counts are tiny (q^2)"
                )]
                {
                    sum / count as f64
                }
            }
        }
    }
}

/// Encodes `values` as little-endian samples of `dtype`, rounding and
/// clamping integers to their range.
fn encode(values: &[f64], dtype: DType) -> Vec<u8> {
    fn ints<T, const N: usize>(
        values: &[f64],
        min: f64,
        max: f64,
        to: impl Fn(f64) -> T,
        bytes: impl Fn(T) -> [u8; N],
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(values.len() * N);
        for &v in values {
            let clamped = v.round().clamp(min, max);
            out.extend_from_slice(&bytes(to(clamped)));
        }
        out
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "values are rounded and clamped to the target range first"
    )]
    match dtype {
        DType::UInt8 => ints(values, 0.0, f64::from(u8::MAX), |v| v as u8, |v| [v]),
        DType::Int16 => ints(
            values,
            f64::from(i16::MIN),
            f64::from(i16::MAX),
            |v| v as i16,
            i16::to_le_bytes,
        ),
        DType::UInt16 => ints(
            values,
            0.0,
            f64::from(u16::MAX),
            |v| v as u16,
            u16::to_le_bytes,
        ),
        DType::Int32 => ints(
            values,
            f64::from(i32::MIN),
            f64::from(i32::MAX),
            |v| v as i32,
            i32::to_le_bytes,
        ),
        DType::Float32 => {
            let mut out = Vec::with_capacity(values.len() * 4);
            for &v in values {
                out.extend_from_slice(&(v as f32).to_le_bytes());
            }
            out
        }
        DType::Float64 => {
            let mut out = Vec::with_capacity(values.len() * 8);
            for &v in values {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out
        }
        // DType is non_exhaustive; widens with PixelBuffer.
        _ => unreachable!("dtype not produced by any source adapter"),
    }
}
