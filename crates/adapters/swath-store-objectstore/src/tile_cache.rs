// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `TileCache` adapter over `object_store` (ARCHITECTURE.md §7: the
//! Phase-1 cache adapter — local fs and in-memory today, S3/MinIO via the
//! same trait).
//!
//! # Key → object path
//!
//! A [`TileKey`](swath_core::cache::TileKey) is 64 lowercase hex chars;
//! its entry lives at `tiles/<hh>/<hh>/<full-key>` — the first two byte
//! pairs shard the namespace (256 × 256 prefixes) so filesystem-backed
//! stores never accumulate millions of siblings in one directory, and
//! S3-style stores spread request load across key prefixes. The full key
//! stays in the leaf name so an operator can map an object back to its
//! key by inspection.
//!
//! # Entry framing
//!
//! Each object is a tiny self-describing frame:
//!
//! ```text
//! [1 byte: frame version = 1][1 byte: content-type length][content-type][payload]
//! ```
//!
//! Why not a bare payload with the content type in object metadata or a
//! file extension: `object_store`'s attribute support varies by backend
//! (the local filesystem store has none), and an extension cannot be
//! reconstructed from the key alone on `get`. The two-plus-N byte header
//! works identically on every backend and keeps `get` a single request.
//! The cost, stated honestly: cached objects are not directly servable
//! files; a CDN-pointable extension-keyed layout is future work for when
//! something other than the swath serve path reads the cache (deferral
//! tracked in `docs/ROADMAP.md`, alongside the GC sweep).
//!
//! # Failure semantics
//!
//! Errors map to [`CacheError`]: transport failures to `Io`, an object
//! that exists but does not parse as a frame (foreign bytes under our
//! prefix, truncation) to `Corrupt`. A missing object is a plain
//! `Ok(None)` miss. The *policy* for these errors (miss-and-render,
//! log-and-serve) belongs to the caller, per the port contract.

use std::sync::Arc;

use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _, PutPayload};
use swath_core::cache::{CacheError, CachedTile, TileCache, TileKey};

/// Frame version byte this adapter writes and accepts.
const FRAME_VERSION: u8 = 1;

/// Prefix all entries live under, so a cache can share a bucket with
/// other data (and a future GC sweep knows exactly what is its own).
const TILES_PREFIX: &str = "tiles";

/// The `object_store`-backed [`TileCache`]: local filesystem, in-memory,
/// or S3-compatible — whatever store the binary wires in.
#[derive(Debug, Clone)]
pub struct ObjectStoreTileCache {
    store: Arc<dyn ObjectStore>,
}

impl ObjectStoreTileCache {
    /// A cache over `store`. The store's root *is* the cache root;
    /// entries live under `tiles/`.
    #[must_use]
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    /// The sharded object path for `key` (module docs).
    fn path(key: &TileKey) -> Path {
        let hex = key.as_str();
        // Keys are 64 hex chars by construction; slicing 0..2/2..4 cannot
        // panic on any key `TileKey::compute` produces.
        Path::from(format!(
            "{TILES_PREFIX}/{}/{}/{hex}",
            &hex[..2.min(hex.len())],
            &hex[2.min(hex.len())..4.min(hex.len())],
        ))
    }

    /// Encodes the entry frame (module docs). Content types longer than
    /// 255 bytes cannot be framed; no real media type is — treated as a
    /// caller bug via `Corrupt` on the way in.
    fn encode(key: &TileKey, bytes: &[u8], content_type: &str) -> Result<Vec<u8>, CacheError> {
        let ct = content_type.as_bytes();
        let ct_len = u8::try_from(ct.len()).map_err(|_| CacheError::Corrupt {
            key: key.clone(),
            detail: format!("content type is {} bytes (max 255)", ct.len()),
        })?;
        let mut frame = Vec::with_capacity(2 + ct.len() + bytes.len());
        frame.push(FRAME_VERSION);
        frame.push(ct_len);
        frame.extend_from_slice(ct);
        frame.extend_from_slice(bytes);
        Ok(frame)
    }

    /// Decodes an entry frame back into a [`CachedTile`].
    fn decode(key: &TileKey, frame: &[u8]) -> Result<CachedTile, CacheError> {
        let corrupt = |detail: String| CacheError::Corrupt {
            key: key.clone(),
            detail,
        };
        let [version, ct_len, rest @ ..] = frame else {
            return Err(corrupt(format!(
                "frame is {} byte(s), need >= 2",
                frame.len()
            )));
        };
        if *version != FRAME_VERSION {
            return Err(corrupt(format!(
                "frame version {version}, this adapter reads {FRAME_VERSION}"
            )));
        }
        let ct_len = usize::from(*ct_len);
        if rest.len() < ct_len {
            return Err(corrupt(format!(
                "content-type length {ct_len} exceeds remaining {} byte(s)",
                rest.len()
            )));
        }
        let (ct, payload) = rest.split_at(ct_len);
        let content_type = std::str::from_utf8(ct)
            .map_err(|_| corrupt("content type is not UTF-8".to_owned()))?
            .to_owned();
        Ok(CachedTile::new(payload.to_vec(), content_type))
    }

    fn io_error(key: &TileKey, source: object_store::Error) -> CacheError {
        CacheError::Io {
            key: key.clone(),
            source: Box::new(source),
        }
    }
}

impl TileCache for ObjectStoreTileCache {
    async fn get(&self, key: &TileKey) -> Result<Option<CachedTile>, CacheError> {
        let result = self.store.get(&Self::path(key)).await;
        let object = match result {
            Ok(object) => object,
            Err(object_store::Error::NotFound { .. }) => return Ok(None),
            Err(err) => return Err(Self::io_error(key, err)),
        };
        let frame = object
            .bytes()
            .await
            .map_err(|err| Self::io_error(key, err))?;
        Self::decode(key, &frame).map(Some)
    }

    async fn put(&self, key: &TileKey, bytes: &[u8], content_type: &str) -> Result<(), CacheError> {
        let frame = Self::encode(key, bytes, content_type)?;
        self.store
            .put(&Self::path(key), PutPayload::from(frame))
            .await
            .map(|_| ())
            .map_err(|err| Self::io_error(key, err))
    }
}

#[cfg(test)]
mod tests {
    use swath_core::cache::{TileKey, TileKeyInputs};
    use swath_core::tile::TileCoord;

    use super::ObjectStoreTileCache;

    fn key() -> TileKey {
        TileKey::compute(&TileKeyInputs {
            layer: "truecolor",
            layer_version: "v",
            plan_json: "{}",
            tms: "WebMercatorQuad",
            coord: TileCoord::new(12, 848, 1561).expect("valid tile"),
            tile_size: 256,
        })
    }

    #[test]
    fn paths_are_sharded_by_hash_prefix() {
        let key = key();
        let hex = key.as_str();
        let path = ObjectStoreTileCache::path(&key).to_string();
        assert_eq!(path, format!("tiles/{}/{}/{hex}", &hex[..2], &hex[2..4]));
    }

    #[test]
    fn frame_round_trips_and_rejects_garbage() {
        let key = key();
        let frame = ObjectStoreTileCache::encode(&key, b"PNG-BYTES", "image/png").expect("encodes");
        let entry = ObjectStoreTileCache::decode(&key, &frame).expect("decodes");
        assert_eq!(entry.bytes, b"PNG-BYTES");
        assert_eq!(entry.content_type, "image/png");

        // Truncations and foreign bytes are Corrupt, never a panic.
        assert!(ObjectStoreTileCache::decode(&key, &[]).is_err());
        assert!(ObjectStoreTileCache::decode(&key, &[1]).is_err());
        assert!(
            ObjectStoreTileCache::decode(&key, &[9, 0]).is_err(),
            "unknown frame version"
        );
        assert!(
            ObjectStoreTileCache::decode(&key, &[1, 200, 0]).is_err(),
            "ct length overrun"
        );
    }
}
