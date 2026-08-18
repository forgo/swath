// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `RasterSource` adapter for virtual-reference manifests over
//! [`object_store`]: legacy granules (NetCDF4/HDF5) served as **byte
//! ranges into the original, untouched file** (ADR 0006, issue #39).
//!
//! A [`VirtualSource`] reads the `VirtualManifest` v1 documents the ingest
//! path stores next to each legacy granule (`<asset>.vmanifest.json`,
//! issue #40) and serves windowed pixel reads by fetching **only the chunk
//! ranges a window touches** from the original file — never the whole
//! granule, never a rewritten copy. The
//! [`Provenance`](swath_core::trace::Provenance) ranges in every
//! [`WindowData`] are exactly those fetches, pointing at the *original*
//! `.h5` — the legacy-granule x-ray payoff: a Trace shows tiles being
//! carved live out of a file nobody converted.
//!
//! # Asset addressing: `<manifest-key>#<array-name>`
//!
//! One manifest describes *many* arrays (a VNP09GA granule has 67), so an
//! [`AssetRef`] must select one. The adapter reads the URI fragment
//! convention:
//!
//! ```text
//! vnp09ga/granule.h5.vmanifest.json#HDFEOS/GRIDS/VIIRS_Grid_1km_2D/Data Fields/SurfReflect_M7_1
//! ```
//!
//! — everything before the first `#` is the manifest's object-store key,
//! everything after is the manifest array name (which never contains `#`:
//! HDF5 path components and cfgrib variable names are `#`-free in
//! practice, and the generator would have to invent one). The fragment is
//! **required**: a manifest key with no fragment is refused with a
//! [`SourceError::Format`] naming the convention, never resolved by guess.
//! Each addressed array behaves as a single-band raster (`describe`
//! reports `band_count: 1`); band vocabulary lives in the granule's asset
//! map, where each dataset band names its array via this fragment.
//!
//! # What the adapter serves
//!
//! Exactly what the manifest can georeference: 2-D arrays carrying a
//! [`Georef`](swath_manifest::Georef) (CRS — EPSG or proj-string —
//! geotransform, nodata, band). Arrays without a georef (coordinate
//! vectors, metadata blobs) and non-2-D shapes are an honest
//! [`SourceError::Unsupported`]. Virtual cubes have **no overview
//! pyramids** — `describe` reports `overview_levels: []` and the planner
//! treats every read as full-resolution.
//!
//! # Chunks, codecs, and missing chunks
//!
//! `read_window` intersects the (full-resolution pixel) window with the
//! manifest's chunk grid, fetches each intersecting chunk's byte range via
//! [`ObjectStore::get_range`], and decodes the manifest codec chain **in
//! reverse** (codecs are recorded in HDF5 filter-pipeline order — see the
//! manifest docs): `zlib:<level>` inflates via flate2, `shuffle` undoes
//! HDF5's byte-shuffle. Sample bytes are little-endian by construction —
//! the generator refuses big-endian datasets at referencing time
//! (`swath-referencer` docs), so every dtype that can reach a manifest is
//! native/LE. HDF5 stores edge chunks at full chunk shape, so a decoded
//! chunk is always `chunk_rows × chunk_cols` samples; the window copies
//! the overlap. Chunks **absent from the manifest** are unallocated in the
//! original file: their pixels fill with the array's nodata sentinel (or
//! zero when none is declared — libhdf5's own default fill), matching what
//! h5py returns for the same read, with no I/O and no provenance.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::Arc;

use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _};
use swath_core::raster::{AssetRef, DType, RasterInfo, WindowRequest};
use swath_core::source::{
    BandSelection, PixelBuffer, RasterSource, ReadLevel, SourceError, WindowData,
};
use swath_core::trace::Provenance;
use swath_manifest::{ChunkRef, Georef, VirtualArray, VirtualManifest};

/// A [`RasterSource`] serving virtual-reference manifests from an
/// [`ObjectStore`].
///
/// The store is fixed at construction; an [`AssetRef`] is interpreted as
/// `<manifest-key>#<array-name>` **within that store** (crate docs), so
/// the same asset naming works over local filesystem, in-memory, and S3
/// stores. Stateless per call: every read re-fetches the manifest
/// (caching is a later, observable-behavior-preserving optimization —
/// manifests are a few hundred KB of JSON, dwarfed by pixel I/O).
#[derive(Debug, Clone)]
pub struct VirtualSource {
    store: Arc<dyn ObjectStore>,
}

impl VirtualSource {
    /// Creates a source reading from `store`.
    #[must_use]
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    /// Whether an asset URI names a virtual-reference manifest this
    /// adapter reads: its pre-fragment path ends in `.vmanifest.json`
    /// (the ingest path's storage convention, issue #40).
    #[must_use]
    pub fn handles(asset: &AssetRef) -> bool {
        let uri = asset.as_str();
        let key = uri.split_once('#').map_or(uri, |(key, _)| key);
        key.ends_with(".vmanifest.json")
    }

    /// Splits `<manifest-key>#<array-name>`, refusing fragment-less URIs.
    fn split(asset: &AssetRef) -> Result<(Path, &str), SourceError> {
        let Some((key, array)) = asset.as_str().split_once('#') else {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: "virtual-cube assets are addressed as \
                         `<manifest-key>#<array-name>` (one manifest holds many \
                         arrays); no `#<array-name>` fragment present"
                    .to_owned(),
            });
        };
        if array.is_empty() {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: "empty `#<array-name>` fragment".to_owned(),
            });
        }
        let path = Path::parse(key).map_err(|e| SourceError::Format {
            asset: asset.clone(),
            detail: format!("manifest key is not a valid object path: {e}"),
        })?;
        Ok((path, array))
    }

    /// Fetches and parses the manifest, then selects the addressed array.
    async fn load_array(&self, asset: &AssetRef) -> Result<(VirtualArray, Georef), SourceError> {
        let (path, array_name) = Self::split(asset)?;
        let payload = self
            .store
            .get(&path)
            .await
            .map_err(|e| map_store_error(asset, e))?
            .bytes()
            .await
            .map_err(|e| map_store_error(asset, e))?;
        let text = std::str::from_utf8(&payload).map_err(|e| SourceError::Format {
            asset: asset.clone(),
            detail: format!("manifest is not UTF-8: {e}"),
        })?;
        let manifest = VirtualManifest::from_json_str(text).map_err(|e| SourceError::Format {
            asset: asset.clone(),
            detail: e.to_string(),
        })?;
        let Some(array) = manifest.arrays.into_iter().find(|a| a.name == array_name) else {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: format!("manifest has no array `{array_name}`"),
            });
        };
        let Some(georef) = array.georef.clone() else {
            return Err(SourceError::Unsupported {
                asset: asset.clone(),
                detail: format!(
                    "array `{array_name}` carries no georeferencing; only \
                     georeferenced 2-D arrays are servable rasters"
                ),
            });
        };
        Ok((array, georef))
    }
}

impl RasterSource for VirtualSource {
    async fn describe(&self, asset: &AssetRef) -> Result<RasterInfo, SourceError> {
        let (array, georef) = self.load_array(asset).await?;
        raster_info(asset, &array, &georef)
    }

    async fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
        level: ReadLevel,
    ) -> Result<WindowData, SourceError> {
        let (array, georef) = self.load_array(asset).await?;
        let info = raster_info(asset, &array, &georef)?;
        // Virtual cubes carry no overview pyramids (`describe` reports the
        // empty list); an overview request is a caller bug surfaced with
        // the port's own error, never silently served at full resolution.
        if let ReadLevel::Overview { factor } = level {
            return Err(SourceError::OverviewNotFound {
                asset: asset.clone(),
                factor,
                available: Vec::new(),
            });
        }
        // BandSelection is non_exhaustive: new selection kinds must be
        // adopted here explicitly, not silently misread.
        let BandSelection::Single(band_index) = band else {
            return Err(SourceError::Unsupported {
                asset: asset.clone(),
                detail: format!("band selection {band:?} not yet supported by this adapter"),
            });
        };
        if band_index >= info.band_count {
            return Err(SourceError::BandOutOfRange {
                asset: asset.clone(),
                band: band_index,
                band_count: info.band_count,
            });
        }

        let full = WindowRequest {
            col_off: 0,
            row_off: 0,
            width: info.width,
            height: info.height,
        };
        let Some(clip) = window.intersection(&full) else {
            // Nothing to read: an empty window clamped onto the grid, no I/O.
            let empty = WindowRequest {
                col_off: window.col_off.min(info.width),
                row_off: window.row_off.min(info.height),
                width: 0,
                height: 0,
            };
            return Ok(WindowData::new(
                empty,
                info.clone(),
                pixels_from_le_bytes(info.dtype, &[]),
                info.nodata,
                Vec::new(),
            ));
        };

        let dtype = info.dtype;
        let reader = ChunkReader::new(asset, &array, dtype)?;
        let mut out = WindowBytes::new(&clip, dtype, info.nodata);
        let mut provenance = Vec::new();
        for (chunk_row, chunk_col) in reader.chunks_touching(&clip) {
            let Some(chunk_ref) = reader.chunk_ref(chunk_row, chunk_col) else {
                // Unallocated in the original file: stays at fill value.
                continue;
            };
            let raw = self
                .store
                .get_range(
                    &source_path(asset, chunk_ref)?,
                    chunk_ref.offset..chunk_ref.offset + chunk_ref.length,
                )
                .await
                .map_err(|e| map_store_error(asset, e))?;
            provenance.push(Provenance {
                path: chunk_ref.path.clone(),
                offset: chunk_ref.offset,
                length: chunk_ref.length,
            });
            let decoded = reader.decode(asset, &raw)?;
            out.copy_chunk(&decoded, &reader, chunk_row, chunk_col);
        }

        let nodata = info.nodata;
        Ok(WindowData::new(
            clip,
            info,
            pixels_from_le_bytes(dtype, &out.bytes),
            nodata,
            provenance,
        ))
    }
}

/// Builds the port's `RasterInfo` from a manifest array + georef.
fn raster_info(
    asset: &AssetRef,
    array: &VirtualArray,
    georef: &Georef,
) -> Result<RasterInfo, SourceError> {
    let [height, width] = array.shape[..] else {
        return Err(SourceError::Unsupported {
            asset: asset.clone(),
            detail: format!(
                "array `{}` has shape {:?}; only 2-D (rows, cols) arrays are \
                 servable rasters",
                array.name, array.shape
            ),
        });
    };
    let dtype = dtype_of(&array.dtype).ok_or_else(|| SourceError::Unsupported {
        asset: asset.clone(),
        detail: format!(
            "array `{}` dtype `{}` has no raster mapping",
            array.name, array.dtype
        ),
    })?;
    Ok(RasterInfo {
        crs: (&georef.crs).into(),
        width,
        height,
        transform: georef.transform.into(),
        band_count: 1,
        dtype,
        nodata: georef.nodata,
        // Virtual cubes carry no overview pyramids — reported honestly
        // empty, never synthesized.
        overview_levels: Vec::new(),
    })
}

/// Maps the manifest's numpy-style dtype strings onto the port vocabulary.
/// The generator emits only native little-endian scalars (its own
/// documented boundary), so the plain names below are the complete
/// servable set.
fn dtype_of(dtype: &str) -> Option<DType> {
    match dtype {
        "uint8" => Some(DType::UInt8),
        "int16" => Some(DType::Int16),
        "uint16" => Some(DType::UInt16),
        "int32" => Some(DType::Int32),
        "float32" => Some(DType::Float32),
        "float64" => Some(DType::Float64),
        _ => None,
    }
}

/// Per-array read machinery: chunk grid geometry, refs by grid position,
/// and the codec chain.
struct ChunkReader<'a> {
    /// Chunk shape (`rows`, `cols`).
    chunk_rows: u64,
    chunk_cols: u64,
    /// Sample size in bytes.
    sample_bytes: usize,
    /// Codec chain in manifest (filter-pipeline) order.
    codecs: &'a [String],
    /// Refs by (`chunk_row`, `chunk_col`).
    refs: BTreeMap<(u64, u64), &'a ChunkRef>,
}

impl<'a> ChunkReader<'a> {
    fn new(asset: &AssetRef, array: &'a VirtualArray, dtype: DType) -> Result<Self, SourceError> {
        let [chunk_rows, chunk_cols] = array.chunks[..] else {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: format!(
                    "array `{}`: chunk shape {:?} does not match its 2-D shape",
                    array.name, array.chunks
                ),
            });
        };
        if chunk_rows == 0 || chunk_cols == 0 {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: format!("array `{}`: zero-sized chunk shape", array.name),
            });
        }
        let mut refs = BTreeMap::new();
        for chunk_ref in &array.refs {
            let Some((row, col)) = parse_key(&chunk_ref.key) else {
                return Err(SourceError::Format {
                    asset: asset.clone(),
                    detail: format!(
                        "array `{}`: chunk key `{}` is not a 2-D grid position",
                        array.name, chunk_ref.key
                    ),
                });
            };
            refs.insert((row, col), chunk_ref);
        }
        Ok(Self {
            chunk_rows,
            chunk_cols,
            sample_bytes: dtype.size_bytes(),
            codecs: &array.codecs,
            refs,
        })
    }

    /// The chunk-grid positions a clipped window touches, row-major.
    fn chunks_touching(&self, clip: &WindowRequest) -> Vec<(u64, u64)> {
        let row0 = clip.row_off / self.chunk_rows;
        let row1 = (clip.end_row() - 1) / self.chunk_rows;
        let col0 = clip.col_off / self.chunk_cols;
        let col1 = (clip.end_col() - 1) / self.chunk_cols;
        let mut chunks = Vec::new();
        for row in row0..=row1 {
            for col in col0..=col1 {
                chunks.push((row, col));
            }
        }
        chunks
    }

    fn chunk_ref(&self, row: u64, col: u64) -> Option<&ChunkRef> {
        self.refs.get(&(row, col)).copied()
    }

    /// Decodes one stored chunk: the manifest codec chain applied in
    /// reverse (module docs), then a hard size check against the chunk
    /// shape — a short chunk is corruption, never padded over.
    fn decode(&self, asset: &AssetRef, raw: &[u8]) -> Result<Vec<u8>, SourceError> {
        let mut bytes = raw.to_vec();
        for codec in self.codecs.iter().rev() {
            bytes = match codec
                .split_once(':')
                .map_or(codec.as_str(), |(name, _)| name)
            {
                "zlib" => {
                    let mut decoded = Vec::new();
                    flate2::read::ZlibDecoder::new(bytes.as_slice())
                        .read_to_end(&mut decoded)
                        .map_err(|e| SourceError::Format {
                            asset: asset.clone(),
                            detail: format!("zlib chunk decode failed: {e}"),
                        })?;
                    decoded
                }
                "shuffle" => unshuffle(&bytes, self.sample_bytes),
                other => {
                    return Err(SourceError::Unsupported {
                        asset: asset.clone(),
                        detail: format!("codec `{other}` is not supported by this adapter"),
                    });
                }
            };
        }
        let expected = usize::try_from(self.chunk_rows * self.chunk_cols)
            .expect("chunk sample count fits usize")
            * self.sample_bytes;
        if bytes.len() != expected {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: format!(
                    "decoded chunk is {} bytes, expected {expected} \
                     ({} x {} samples of {} byte(s))",
                    bytes.len(),
                    self.chunk_rows,
                    self.chunk_cols,
                    self.sample_bytes
                ),
            });
        }
        Ok(bytes)
    }
}

/// The object-store path of a chunk's source file.
fn source_path(asset: &AssetRef, chunk_ref: &ChunkRef) -> Result<Path, SourceError> {
    Path::parse(&chunk_ref.path).map_err(|e| SourceError::Format {
        asset: asset.clone(),
        detail: format!(
            "chunk ref path `{}` is not a valid object path: {e}",
            chunk_ref.path
        ),
    })
}

/// Parses a dotted 2-D chunk key (`"1.2"`) into (`row`, `col`).
fn parse_key(key: &str) -> Option<(u64, u64)> {
    let (row, col) = key.split_once('.')?;
    Some((row.parse().ok()?, col.parse().ok()?))
}

/// Undoes HDF5's byte-shuffle filter: shuffled storage groups byte 0 of
/// every sample first, then byte 1, … — the inverse gathers each sample's
/// bytes back together. (~n·size moves; allocation-for-allocation the
/// same shape as libhdf5's own `H5Z_filter_shuffle` decode path.)
fn unshuffle(bytes: &[u8], sample_bytes: usize) -> Vec<u8> {
    if sample_bytes <= 1 || !bytes.len().is_multiple_of(sample_bytes) {
        return bytes.to_vec();
    }
    let samples = bytes.len() / sample_bytes;
    let mut out = vec![0_u8; bytes.len()];
    for (byte_index, plane) in bytes.chunks_exact(samples).enumerate() {
        for (sample_index, &byte) in plane.iter().enumerate() {
            out[sample_index * sample_bytes + byte_index] = byte;
        }
    }
    out
}

/// The output window as raw little-endian bytes, prefilled with the
/// array's fill value so unallocated chunks read back as nodata.
struct WindowBytes {
    clip: WindowRequest,
    sample_bytes: usize,
    bytes: Vec<u8>,
}

impl WindowBytes {
    fn new(clip: &WindowRequest, dtype: DType, nodata: Option<f64>) -> Self {
        let sample_bytes = dtype.size_bytes();
        let samples = usize::try_from(clip.width * clip.height).expect("window fits usize");
        let fill = fill_bytes(dtype, nodata);
        let mut bytes = Vec::with_capacity(samples * sample_bytes);
        for _ in 0..samples {
            bytes.extend_from_slice(&fill);
        }
        Self {
            clip: *clip,
            sample_bytes,
            bytes,
        }
    }

    /// Copies the overlap of a decoded (full-shape) chunk into the window.
    fn copy_chunk(&mut self, decoded: &[u8], reader: &ChunkReader<'_>, row: u64, col: u64) {
        let chunk_row0 = row * reader.chunk_rows;
        let chunk_col0 = col * reader.chunk_cols;
        let row_start = self.clip.row_off.max(chunk_row0);
        let row_end = self.clip.end_row().min(chunk_row0 + reader.chunk_rows);
        let col_start = self.clip.col_off.max(chunk_col0);
        let col_end = self.clip.end_col().min(chunk_col0 + reader.chunk_cols);
        if row_start >= row_end || col_start >= col_end {
            return;
        }
        let span = usize::try_from(col_end - col_start).expect("row span fits usize");
        for grid_row in row_start..row_end {
            let src_offset = usize::try_from(
                (grid_row - chunk_row0) * reader.chunk_cols + (col_start - chunk_col0),
            )
            .expect("chunk offset fits usize")
                * self.sample_bytes;
            let dst_offset = usize::try_from(
                (grid_row - self.clip.row_off) * self.clip.width + (col_start - self.clip.col_off),
            )
            .expect("window offset fits usize")
                * self.sample_bytes;
            let len = span * self.sample_bytes;
            self.bytes[dst_offset..dst_offset + len]
                .copy_from_slice(&decoded[src_offset..src_offset + len]);
        }
    }
}

/// One sample's little-endian bytes of the fill value: the nodata sentinel
/// cast to the dtype (GDAL's widened-f64 convention read back down), or
/// zero when none is declared — libhdf5's own default fill.
fn fill_bytes(dtype: DType, nodata: Option<f64>) -> Vec<u8> {
    let value = nodata.unwrap_or(0.0);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "nodata sentinels are exact values of the array's own dtype, \
                  round-tripped through the manifest's f64 field"
    )]
    match dtype {
        DType::UInt8 => vec![value as u8],
        DType::Int16 => (value as i16).to_le_bytes().to_vec(),
        DType::UInt16 => (value as u16).to_le_bytes().to_vec(),
        DType::Int32 => (value as i32).to_le_bytes().to_vec(),
        DType::Float32 => (value as f32).to_le_bytes().to_vec(),
        DType::Float64 => value.to_le_bytes().to_vec(),
        // DType is #[non_exhaustive]; dtype_of() is this adapter's only
        // producer and yields exactly the variants above.
        _ => unreachable!("dtype_of never yields unhandled DType variants"),
    }
}

/// Reassembles a raw little-endian byte buffer into the dtype-tagged
/// pixel buffer (the exact inverse of `PixelBuffer::to_le_bytes`).
fn pixels_from_le_bytes(dtype: DType, bytes: &[u8]) -> PixelBuffer {
    fn samples<T, const N: usize>(bytes: &[u8], decode: impl Fn([u8; N]) -> T) -> Vec<T> {
        bytes
            .chunks_exact(N)
            .map(|chunk| decode(chunk.try_into().expect("exact chunk")))
            .collect()
    }
    match dtype {
        DType::UInt8 => PixelBuffer::UInt8(bytes.to_vec()),
        DType::Int16 => PixelBuffer::Int16(samples(bytes, i16::from_le_bytes)),
        DType::UInt16 => PixelBuffer::UInt16(samples(bytes, u16::from_le_bytes)),
        DType::Int32 => PixelBuffer::Int32(samples(bytes, i32::from_le_bytes)),
        DType::Float32 => PixelBuffer::Float32(samples(bytes, f32::from_le_bytes)),
        DType::Float64 => PixelBuffer::Float64(samples(bytes, f64::from_le_bytes)),
        // DType is #[non_exhaustive]; dtype_of() is this adapter's only
        // producer and yields exactly the variants above.
        _ => unreachable!("dtype_of never yields unhandled DType variants"),
    }
}

/// Translates `object_store` failures into the port's error contract.
fn map_store_error(asset: &AssetRef, err: object_store::Error) -> SourceError {
    match err {
        object_store::Error::NotFound { .. } => SourceError::NotFound {
            asset: asset.clone(),
        },
        other => SourceError::Io {
            asset: asset.clone(),
            source: Box::new(other),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{VirtualSource, dtype_of, parse_key, unshuffle};
    use swath_core::raster::{AssetRef, DType};

    #[test]
    fn handles_matches_the_manifest_suffix_before_the_fragment() {
        assert!(VirtualSource::handles(&AssetRef::new(
            "vnp/g.h5.vmanifest.json#HDFEOS/GRIDS/x"
        )));
        assert!(VirtualSource::handles(&AssetRef::new(
            "g.h5.vmanifest.json"
        )));
        assert!(!VirtualSource::handles(&AssetRef::new("granule/B04.tif")));
        assert!(!VirtualSource::handles(&AssetRef::new("g.h5#array")));
    }

    #[test]
    fn unshuffle_regroups_sample_bytes() {
        // Two int16 samples shuffled: [lo0, lo1, hi0, hi1] planes.
        let shuffled = [0x01, 0x02, 0x10, 0x20];
        assert_eq!(unshuffle(&shuffled, 2), vec![0x01, 0x10, 0x02, 0x20]);
        // Sample size 1 is the identity.
        assert_eq!(unshuffle(&shuffled, 1), shuffled.to_vec());
    }

    #[test]
    fn chunk_keys_and_dtypes_parse() {
        assert_eq!(parse_key("1.2"), Some((1, 2)));
        assert_eq!(parse_key("0"), None); // 1-D key: not a 2-D raster
        assert_eq!(parse_key(""), None);
        assert_eq!(dtype_of("int16"), Some(DType::Int16));
        assert_eq!(dtype_of("|S18"), None);
        assert_eq!(dtype_of("int64"), None);
    }
}
