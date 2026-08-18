// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! HDF5/NetCDF4 generation via `hdf5-metno` (prototype 0001, productionized).
//!
//! Grouping model (proven byte-identical to the `VirtualiZarr` HDF parser on
//! VNP09GA): one array per dataset, named by its HDF5 path without the
//! leading slash; chunked datasets get one ref per **allocated** chunk from
//! the chunk index (`chunks_visit` / `H5Dchunk_iter`), keyed by dotted
//! chunk-grid position; contiguous datasets are a single whole-storage ref
//! (key `"0"` per rank, `""` for scalars, chunk shape = dataset shape);
//! datasets with no allocated storage keep an empty ref list.
//!
//! Over the prototype, this adds georeferencing: when the file is HDF-EOS5
//! (`HDFEOS INFORMATION/StructMetadata.0` present), the [`crate::eos`] grids
//! are attached to every 2-D data field living under
//! `HDFEOS/GRIDS/<grid>/…` whose shape matches the grid, with nodata from a
//! numeric `_FillValue` attribute and band semantics from the field name.
//! Plain (non-EOS) HDF5/NetCDF4 arrays carry no georef — CF coordinate
//! interpretation is future scope, recorded honestly rather than guessed
//! (deferral tracked in `docs/ROADMAP.md`).
//!
//! Exotic datatypes (big-endian, compound, vlen, …) are a deliberate
//! [`ReferencerError::Unsupported`]: the conformance sidecar remains the
//! fallback for containers this path rejects (ADR 0006).

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::ReferencerError;
use crate::manifest::{ChunkRef, Georef, ManifestVersion, VirtualArray, VirtualManifest};

use crate::eos::EosGrid;

/// The HDF-EOS5 metadata dataset holding the grid structure text.
const STRUCT_METADATA: &str = "/HDFEOS INFORMATION/StructMetadata.0";

/// Generates the manifest for an HDF5/NetCDF4 granule.
pub(crate) fn generate(granule: &Path) -> Result<VirtualManifest, ReferencerError> {
    let file = hdf5_metno::File::open(granule).map_err(|e| ReferencerError::Malformed {
        detail: format!("cannot open `{}` as HDF5: {e}", granule.display()),
    })?;
    let source = granule.display().to_string();

    let mut arrays = Vec::new();
    walk_group(&file, &source, &mut arrays)?;
    if arrays.is_empty() {
        return Err(ReferencerError::Malformed {
            detail: format!("no datasets found in `{source}`"),
        });
    }

    if let Some(grids) = eos_grids(&file, granule)? {
        attach_georefs(&file, &grids, &mut arrays);
    }

    Ok(VirtualManifest {
        manifest_version: ManifestVersion,
        generator: crate::GENERATOR.to_owned(),
        source,
        arrays,
    })
}

/// Depth-first over datasets then subgroups (mirrors the sidecar's
/// `ManifestGroup` traversal, so manifests agree on array order, not just
/// content — prototype 0001 §7).
fn walk_group(
    group: &hdf5_metno::Group,
    source: &str,
    arrays: &mut Vec<VirtualArray>,
) -> Result<(), ReferencerError> {
    let group_name = group.name();
    let backend = |what: &str, e: hdf5_metno::Error| ReferencerError::Backend {
        detail: format!("{what} of `{group_name}`"),
        source: Box::new(e),
    };
    for ds in group
        .datasets()
        .map_err(|e| backend("listing datasets", e))?
    {
        arrays.push(array_of(&ds, source)?);
    }
    for sub in group
        .groups()
        .map_err(|e| backend("listing subgroups", e))?
    {
        walk_group(&sub, source, arrays)?;
    }
    Ok(())
}

/// One dataset → one manifest array (module docs for the grouping model).
fn array_of(ds: &hdf5_metno::Dataset, source: &str) -> Result<VirtualArray, ReferencerError> {
    let path = ds.name(); // full HDF5 path, e.g. "/HDFEOS/GRIDS/…/SurfReflect_M1_1"
    let name = path.trim_start_matches('/').to_owned();

    let shape: Vec<u64> = ds.shape().iter().map(|&d| d as u64).collect();
    let descriptor =
        ds.dtype()
            .and_then(|t| t.to_descriptor())
            .map_err(|e| ReferencerError::Backend {
                detail: format!("reading datatype of `{name}`"),
                source: Box::new(e),
            })?;
    let dtype = numpy_dtype(&descriptor).ok_or_else(|| ReferencerError::Unsupported {
        detail: format!("dataset `{name}`: HDF5 datatype {descriptor} has no manifest mapping"),
    })?;
    let codecs: Vec<String> = ds.filters().iter().map(filter_codec).collect();

    let (chunks, refs) = if let Some(chunk) = ds.chunk() {
        // Chunked layout: walk the chunk index collecting each allocated
        // chunk's (logical offset, file address, stored size). Unallocated
        // chunks are simply absent — the reference sidecar omits them too.
        let chunk: Vec<u64> = chunk.iter().map(|&d| d as u64).collect();
        let mut refs = Vec::new();
        ds.chunks_visit(|c| {
            let key = c
                .offset
                .iter()
                .zip(&chunk)
                .map(|(elem, dim)| (elem / dim).to_string())
                .collect::<Vec<_>>()
                .join(".");
            refs.push(ChunkRef {
                key,
                path: source.to_owned(),
                offset: c.addr,
                length: c.size,
            });
            0 // continue iteration
        })
        .map_err(|e| ReferencerError::Backend {
            detail: format!("walking the chunk index of `{name}`"),
            source: Box::new(e),
        })?;
        (chunk, refs)
    } else {
        // Contiguous (or compact/no-storage) layout: one ref spanning the
        // whole allocation, chunk shape = dataset shape, exactly as the
        // sidecar represents it. `offset()` is None when no storage is
        // allocated (e.g. HDF-EOS "Projection" stubs) — keep the array,
        // drop the ref.
        let refs = match ds.offset() {
            Some(addr) => vec![ChunkRef {
                key: vec!["0"; shape.len()].join("."),
                path: source.to_owned(),
                offset: addr,
                length: ds.storage_size(),
            }],
            None => Vec::new(),
        };
        (shape.clone(), refs)
    };

    Ok(VirtualArray {
        name,
        shape,
        chunks,
        dtype,
        codecs,
        georef: None,
        refs,
    })
}

/// HDF5 datatype → numpy-style dtype string, matching what the sidecar
/// derives from the zarr dtype: plain names for native-endian scalars,
/// `|S<n>` for fixed-length strings. Exotic types are `None` — an honest
/// [`ReferencerError::Unsupported`] upstream, never a guess.
fn numpy_dtype(td: &hdf5_metno::types::TypeDescriptor) -> Option<String> {
    use hdf5_metno::types::{FloatSize, IntSize, TypeDescriptor as TD};
    Some(match td {
        TD::Integer(IntSize::U1) => "int8".to_owned(),
        TD::Integer(IntSize::U2) => "int16".to_owned(),
        TD::Integer(IntSize::U4) => "int32".to_owned(),
        TD::Integer(IntSize::U8) => "int64".to_owned(),
        TD::Unsigned(IntSize::U1) => "uint8".to_owned(),
        TD::Unsigned(IntSize::U2) => "uint16".to_owned(),
        TD::Unsigned(IntSize::U4) => "uint32".to_owned(),
        TD::Unsigned(IntSize::U8) => "uint64".to_owned(),
        TD::Float(FloatSize::U4) => "float32".to_owned(),
        TD::Float(FloatSize::U8) => "float64".to_owned(),
        TD::Boolean => "bool".to_owned(),
        TD::FixedAscii(n) | TD::FixedUnicode(n) => format!("|S{n}"),
        _ => return None,
    })
}

/// Filter pipeline entry → codec string: the vocabulary shared with the
/// sidecar, which derives the same strings independently from the zarr
/// codecs `VirtualiZarr` reports. Exact agreement is contractual (the
/// conformance harness compares codecs).
fn filter_codec(f: &hdf5_metno::filters::Filter) -> String {
    use hdf5_metno::filters::Filter;
    match f {
        Filter::Deflate(level) => format!("zlib:{level}"),
        Filter::Shuffle => "shuffle".to_owned(),
        Filter::Fletcher32 => "fletcher32".to_owned(),
        Filter::SZip(_, _) => "szip".to_owned(),
        Filter::NBit => "nbit".to_owned(),
        Filter::ScaleOffset(_) => "scaleoffset".to_owned(),
        Filter::User(id, _) => format!("hdf5:filter{id}"),
    }
}

/// Reads and parses the HDF-EOS grid structure, when this file has one.
/// The metadata text is read as raw bytes at the dataset's storage range
/// (it is a fixed-length string scalar whose size varies by product;
/// byte-range + NUL-trim sidesteps const-generic string typing).
fn eos_grids(
    file: &hdf5_metno::File,
    granule: &Path,
) -> Result<Option<Vec<EosGrid>>, ReferencerError> {
    let Ok(ds) = file.dataset(STRUCT_METADATA) else {
        return Ok(None); // plain HDF5/NetCDF4, not HDF-EOS
    };
    let (Some(offset), length) = (ds.offset(), ds.storage_size()) else {
        return Ok(None); // present but unallocated: nothing to parse
    };
    let read = || -> std::io::Result<Vec<u8>> {
        let mut f = std::fs::File::open(granule)?;
        f.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0_u8; usize::try_from(length).unwrap_or(usize::MAX)];
        f.read_exact(&mut buf)?;
        Ok(buf)
    };
    let bytes = read().map_err(|e| ReferencerError::Backend {
        detail: format!("reading StructMetadata.0 bytes of `{}`", granule.display()),
        source: Box::new(e),
    })?;
    let text = String::from_utf8_lossy(&bytes);
    let grids = crate::eos::parse_grids(text.trim_end_matches('\0'))?;
    Ok(Some(grids))
}

/// Attaches a [`Georef`] to every array that is a 2-D data field of a parsed
/// grid: path under `HDFEOS/GRIDS/<grid>/` and shape = (`YDim`, `XDim`).
/// Coordinate vectors, projection stubs, and root-level datasets stay bare.
fn attach_georefs(file: &hdf5_metno::File, grids: &[EosGrid], arrays: &mut [VirtualArray]) {
    for array in arrays.iter_mut() {
        let Some(grid) = grids.iter().find(|g| {
            array.name.starts_with(&format!("HDFEOS/GRIDS/{}/", g.name))
                && array.shape == [g.ydim, g.xdim]
        }) else {
            continue;
        };
        let band = array
            .name
            .rsplit('/')
            .next()
            .map(str::to_owned)
            .filter(|b| !b.is_empty());
        array.georef = Some(Georef {
            crs: grid.crs.clone(),
            transform: grid.transform,
            nodata: numeric_fill_value(file, &array.name),
            band,
        });
    }
}

/// The dataset's `_FillValue` attribute widened to `f64`, when present,
/// scalar-ish, and numeric. Non-numeric fills (VNP09GA QF bands carry
/// `b"N/A"` on uint8) are honestly no-nodata — fill semantics beyond a
/// numeric sentinel are outside the manifest vocabulary.
fn numeric_fill_value(file: &hdf5_metno::File, name: &str) -> Option<f64> {
    use hdf5_metno::types::{FloatSize, IntSize, TypeDescriptor as TD};

    fn first<T: hdf5_metno::H5Type + Copy>(attr: &hdf5_metno::Attribute) -> Option<T> {
        attr.read_raw::<T>().ok()?.first().copied()
    }

    let attr = file.dataset(name).ok()?.attr("_FillValue").ok()?;
    let descriptor = attr.dtype().and_then(|t| t.to_descriptor()).ok()?;

    #[allow(clippy::cast_precision_loss)] // sentinels are small integers
    match descriptor {
        TD::Integer(IntSize::U1) => first::<i8>(&attr).map(f64::from),
        TD::Integer(IntSize::U2) => first::<i16>(&attr).map(f64::from),
        TD::Integer(IntSize::U4) => first::<i32>(&attr).map(f64::from),
        TD::Integer(IntSize::U8) => first::<i64>(&attr).map(|v| v as f64),
        TD::Unsigned(IntSize::U1) => first::<u8>(&attr).map(f64::from),
        TD::Unsigned(IntSize::U2) => first::<u16>(&attr).map(f64::from),
        TD::Unsigned(IntSize::U4) => first::<u32>(&attr).map(f64::from),
        TD::Unsigned(IntSize::U8) => first::<u64>(&attr).map(|v| v as f64),
        TD::Float(FloatSize::U4) => first::<f32>(&attr).map(f64::from),
        TD::Float(FloatSize::U8) => first::<f64>(&attr),
        _ => None,
    }
}
