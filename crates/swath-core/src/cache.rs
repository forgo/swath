// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `TileCache` port and the content-derived cache key
//! (ARCHITECTURE.md §6/§10, issue #36).
//!
//! # The key model (§10)
//!
//! A [`TileKey`] is a SHA-256 digest over everything that determines a
//! tile's bytes: layer id, [`layer_version`], the render plan's canonical
//! JSON, the tile matrix set id, the tile coordinate, and the tile size.
//! SHA-256 (not `DefaultHasher`) because the key outlives the process —
//! it names objects in a store, so it must be stable across Rust releases
//! and architectures. Each input is length-prefixed before hashing, so no
//! concatenation of fields can collide with another field split
//! ("ab"+"c" ≠ "a"+"bc"), and the whole encoding is domain-separated by a
//! version tag ([`TILE_KEY_DOMAIN`]) so a future v2 scheme can never
//! collide with v1 keys. The exact digest for a known input is pinned by
//! a test — accidental input reordering is a visible diff.
//!
//! # `layer_version` v1 semantics (resolves §16.3 *for now*)
//!
//! `layer_version` is a **string derived from the serving inputs**, not a
//! persisted counter and not a vector clock:
//!
//! - **catalog-backed layers**: the latest granule id joined with the
//!   layer's plan hash ([`layer_version`] with `Some(granule)`) — a new
//!   granule or an edited layer definition each produce a new version;
//! - **static/fixture layers**: the plan hash alone (`None`).
//!
//! Content-derived means invalidation needs no machinery: a new granule
//! changes the version, every key under the old version simply stops
//! being asked for — a clean miss, nothing to delete synchronously. The
//! costs, stated honestly:
//!
//! - **orphaned entries**: superseded versions' objects linger until
//!   garbage-collected. GC is deliberate future operational work (a sweep
//!   by age, or by enumerating live versions) — content-keyed entries
//!   never go *stale*, only unreachable, which is why the port carries no
//!   TTL in v1.
//! - **whole-layer invalidation granularity**: a new granule in a future
//!   *mosaic* layer would today bump the version for every tile, not just
//!   the tiles the granule's footprint touches. Partial-mosaic
//!   invalidation (only affected tiles miss) is future work that lands
//!   with mosaics themselves; single-granule serving (`latest` wins) is
//!   exactly right with the whole-version bump.
//!
//! Both deferrals (GC, partial-mosaic invalidation) are tracked in
//! `docs/ROADMAP.md`'s deferral inventory, with revisit triggers.
//!
//! # The port
//!
//! [`TileCache`] follows the crate's native-AFIT port pattern (see
//! [`crate::source`] — `Send` futures, deliberately not dyn-compatible,
//! no runtime dependency in the core). No TTL parameter: entries are
//! immutable under their key (same key ⇒ same bytes), so time never makes
//! one wrong. Failure semantics are the *caller's* policy — the serve
//! path treats a failed `get` as a miss and a failed `put` as a logged
//! warning, never a failed response.

use core::fmt;
use core::future::Future;

use sha2::{Digest, Sha256};

use crate::tile::TileCoord;

/// Domain-separation tag mixed into every [`TileKey`] digest. Versioned:
/// a future scheme change bumps this string, guaranteeing v2 keys can
/// never collide with v1 objects in the same store.
pub const TILE_KEY_DOMAIN: &str = "swath tile-key v1";

/// Everything that determines one cached tile's identity — the hash
/// inputs of [`TileKey::compute`], spelled as a struct so call sites name
/// every field (silently swapping two `&str` arguments cannot happen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileKeyInputs<'a> {
    /// The layer id (the URL-facing name).
    pub layer: &'a str,
    /// The layer's serving version — see [`layer_version`].
    pub layer_version: &'a str,
    /// The render plan's canonical JSON: its compact `serde_json`
    /// serialization (struct field order is fixed by declaration, so the
    /// serialization is deterministic for a given plan value).
    pub plan_json: &'a str,
    /// Tile matrix set id (`"WebMercatorQuad"` today).
    pub tms: &'a str,
    /// The tile address within the TMS.
    pub coord: TileCoord,
    /// Tile side length in pixels (256 classic, 512 retina).
    pub tile_size: u32,
}

/// A content-derived cache key: the lowercase-hex SHA-256 digest of a
/// tile's identity (module docs). Same inputs ⇒ same key, on any machine,
/// forever; any changed input ⇒ a different key and therefore a clean
/// miss.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TileKey(String);

impl TileKey {
    /// Computes the key for `inputs` (length-prefixed SHA-256 — module
    /// docs; the digest for a known input is pinned by a test).
    #[must_use]
    pub fn compute(inputs: &TileKeyInputs<'_>) -> Self {
        let mut hasher = Sha256::new();
        let mut field = |bytes: &[u8]| {
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        };
        field(TILE_KEY_DOMAIN.as_bytes());
        field(inputs.layer.as_bytes());
        field(inputs.layer_version.as_bytes());
        field(inputs.plan_json.as_bytes());
        field(inputs.tms.as_bytes());
        field(&[inputs.coord.z]);
        field(&inputs.coord.x.to_le_bytes());
        field(&inputs.coord.y.to_le_bytes());
        field(&inputs.tile_size.to_le_bytes());
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use core::fmt::Write as _;
            write!(hex, "{byte:02x}").expect("writing hex to a String is infallible");
        }
        Self(hex)
    }

    /// The key as lowercase hex — the string form the Trace carries and
    /// adapters derive object paths from.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The v1 `layer_version` (module docs): the plan hash, prefixed by the
/// backing granule id when the layer is catalog-backed. `plan_json` is
/// the same canonical JSON [`TileKeyInputs::plan_json`] carries.
#[must_use]
pub fn layer_version(granule: Option<&str>, plan_json: &str) -> String {
    layer_version_over(granule.as_slice(), plan_json)
}

/// [`layer_version`] over every granule a frame resolved to, in branch
/// order (ADR 0022): a two-source layer keys under the **ordered pair**
/// `a+b@<plan hash>` — a new granule on either branch is a new version,
/// and the pair `(a, b)` never shares an entry with `(b, a)`. One granule
/// is byte-for-byte [`layer_version`]'s `Some` form; none is its `None`.
#[must_use]
pub fn layer_version_over(granules: &[&str], plan_json: &str) -> String {
    let digest = Sha256::digest(plan_json.as_bytes());
    let mut version = String::new();
    if !granules.is_empty() {
        version.push_str(&granules.join("+"));
        version.push('@');
    }
    for byte in digest {
        use core::fmt::Write as _;
        write!(version, "{byte:02x}").expect("writing hex to a String is infallible");
    }
    version
}

/// One cached entry: the encoded tile bytes and the content type they
/// were stored with (`image/png` today).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CachedTile {
    /// The encoded tile, byte-identical to what `put` stored.
    pub bytes: Vec<u8>,
    /// The IANA media type recorded at `put` time.
    pub content_type: String,
}

impl CachedTile {
    /// An entry holding `bytes` of `content_type`.
    #[must_use]
    pub fn new(bytes: Vec<u8>, content_type: impl Into<String>) -> Self {
        Self {
            bytes,
            content_type: content_type.into(),
        }
    }
}

/// What can go wrong at the cache boundary. Small on purpose: to the
/// serve path every failure means the same thing (treat a `get` as a
/// miss, log a `put`), so the taxonomy only separates storage transport
/// from a corrupt entry.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CacheError {
    /// Storage or transport failure reading or writing an entry.
    #[error("cache i/o failure for key {key}")]
    Io {
        /// The key being read or written.
        key: TileKey,
        /// The underlying storage/transport error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The stored object exists but is not a valid cache entry (foreign
    /// or corrupt bytes under our key).
    #[error("cache entry for key {key} is corrupt: {detail}")]
    Corrupt {
        /// The key whose entry failed to decode.
        key: TileKey,
        /// What failed to decode.
        detail: String,
    },
}

/// Encoded-tile cache (ARCHITECTURE.md §6), keyed by [`TileKey`].
///
/// Entries are immutable under their key: `put` for an existing key may
/// overwrite or keep the old object — both hold the same bytes by
/// construction, so implementations need no compare-and-swap. No TTL in
/// v1 (module docs: content-keyed entries never go stale, they get
/// orphaned; GC is future operational work).
///
/// Same async-in-trait design as [`crate::source::RasterSource`]: native
/// AFIT with `Send` futures, not dyn-compatible, no runtime in the core.
pub trait TileCache: Send + Sync {
    /// The entry under `key`, or `None` on a miss.
    fn get(
        &self,
        key: &TileKey,
    ) -> impl Future<Output = Result<Option<CachedTile>, CacheError>> + Send;

    /// Stores `bytes` (of `content_type`) under `key`.
    fn put(
        &self,
        key: &TileKey,
        bytes: &[u8],
        content_type: &str,
    ) -> impl Future<Output = Result<(), CacheError>> + Send;
}

/// The cache that never hits: `get` always misses, `put` discards. The
/// default cache slot of servers running without one configured — serving
/// through `NoCache` is behaviorally identical to serving with no cache
/// code at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoCache;

impl TileCache for NoCache {
    async fn get(&self, _key: &TileKey) -> Result<Option<CachedTile>, CacheError> {
        Ok(None)
    }

    async fn put(
        &self,
        _key: &TileKey,
        _bytes: &[u8],
        _content_type: &str,
    ) -> Result<(), CacheError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::future::Future;

    use super::{
        CachedTile, NoCache, TileCache, TileKey, TileKeyInputs, layer_version, layer_version_over,
    };
    use crate::tile::TileCoord;

    fn inputs() -> TileKeyInputs<'static> {
        TileKeyInputs {
            layer: "truecolor",
            layer_version: "g-2024158@abc123",
            plan_json: r#"{"inputs":[{"name":"b04"}]}"#,
            tms: "WebMercatorQuad",
            coord: TileCoord {
                z: 12,
                x: 848,
                y: 1561,
            },
            tile_size: 256,
        }
    }

    /// The known-answer pin: this exact digest for this exact input,
    /// forever (computed independently in Python — sha256 over the
    /// length-prefixed fields — and cross-checked against this
    /// implementation before committing). Protects the field encoding
    /// (order, length prefixes, domain tag) against accidental
    /// reordering — any change to the scheme must consciously rewrite
    /// this constant (and bump `TILE_KEY_DOMAIN`).
    #[test]
    fn key_is_pinned_to_a_known_answer() {
        assert_eq!(
            TileKey::compute(&inputs()).as_str(),
            "1d31e53806985ca6ed44e8fe79cc8fc9b9c5b4676bafbf8a4090e5f33fb07b2a",
        );
    }

    #[test]
    fn same_inputs_same_key() {
        assert_eq!(TileKey::compute(&inputs()), TileKey::compute(&inputs()));
    }

    #[test]
    fn every_single_input_change_changes_the_key() {
        let base = TileKey::compute(&inputs());
        let variants = [
            TileKeyInputs {
                layer: "ndvi",
                ..inputs()
            },
            TileKeyInputs {
                layer_version: "g-2024159@abc123",
                ..inputs()
            },
            TileKeyInputs {
                plan_json: r#"{"inputs":[{"name":"b03"}]}"#,
                ..inputs()
            },
            TileKeyInputs {
                tms: "WorldCRS84Quad",
                ..inputs()
            },
            TileKeyInputs {
                coord: TileCoord {
                    z: 12,
                    x: 848,
                    y: 1562,
                },
                ..inputs()
            },
            TileKeyInputs {
                tile_size: 512,
                ..inputs()
            },
        ];
        for variant in variants {
            assert_ne!(
                TileKey::compute(&variant),
                base,
                "changing {variant:?} must change the key"
            );
        }
    }

    /// Length-prefixing makes field boundaries part of the hash: moving a
    /// byte across a boundary (same concatenation, different split) must
    /// change the key.
    #[test]
    fn field_boundaries_are_hashed() {
        let a = TileKey::compute(&TileKeyInputs {
            layer: "ab",
            layer_version: "c",
            ..inputs()
        });
        let b = TileKey::compute(&TileKeyInputs {
            layer: "a",
            layer_version: "bc",
            ..inputs()
        });
        assert_ne!(a, b);
    }

    #[test]
    fn layer_version_is_content_derived() {
        let plan = r#"{"inputs":[]}"#;
        // Static: plan hash only, stable.
        assert_eq!(layer_version(None, plan), layer_version(None, plan));
        // Catalog: granule id is visible in the version (operator-legible)
        // and a new granule is a new version.
        let v1 = layer_version(Some("g-2024158"), plan);
        let v2 = layer_version(Some("g-2024159"), plan);
        assert!(v1.starts_with("g-2024158@"));
        assert_ne!(v1, v2);
        // An edited plan is a new version too, granule unchanged.
        assert_ne!(v1, layer_version(Some("g-2024158"), r#"{"inputs":[{}]}"#));
    }

    /// The ADR 0015 cache-identity pin: a time-parameterized frame keys
    /// under the version of the granule it *resolved to*. Two requests
    /// whose `datetime`s resolve to different granules get distinct
    /// versions and therefore distinct keys; two requests resolving to
    /// the same granule share one key — no `datetime` string is (or may
    /// ever be) an input to the key, so how the granule was asked for
    /// cannot fragment the cache.
    #[test]
    fn temporal_frames_key_by_resolved_granule_only() {
        let plan = r#"{"inputs":[{"name":"b04"}]}"#;
        let key_for = |granule: &str| {
            let version = layer_version(Some(granule), plan);
            TileKey::compute(&TileKeyInputs {
                layer_version: &version,
                ..inputs()
            })
        };
        // Distinct resolved granules (e.g. datetime=pre-fire vs
        // datetime=post-fire) → distinct keys.
        assert_ne!(
            key_for("hlss30-t10tfk-2024204"),
            key_for("hlss30-t10tfk-2024229")
        );
        // Same resolved granule — whether via an instant, an interval,
        // or an absent datetime — → the same key, computed twice.
        assert_eq!(
            key_for("hlss30-t10tfk-2024229"),
            key_for("hlss30-t10tfk-2024229")
        );
    }

    #[test]
    fn key_serializes_as_its_hex_string() {
        let key = TileKey::compute(&inputs());
        let json = serde_json::to_value(&key).unwrap();
        assert_eq!(json, serde_json::Value::String(key.as_str().to_owned()));
        let back: TileKey = serde_json::from_value(json).unwrap();
        assert_eq!(back, key);
    }

    /// Drives a ready future without a runtime (the core has none):
    /// `NoCache`'s futures resolve on the first poll by construction.
    fn poll_ready<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(output) => output,
            std::task::Poll::Pending => unreachable!("NoCache futures are immediately ready"),
        }
    }

    /// `NoCache` misses and discards — the do-nothing baseline.
    #[test]
    fn no_cache_always_misses() {
        let key = TileKey::compute(&inputs());
        assert!(matches!(poll_ready(NoCache.get(&key)), Ok(None)));
        assert!(poll_ready(NoCache.put(&key, b"png", "image/png")).is_ok());
        let entry = CachedTile::new(vec![1, 2, 3], "image/png");
        assert_eq!(entry.content_type, "image/png");
    }

    #[test]
    fn layer_version_over_binds_every_branch_in_order() {
        let plan = r#"{"inputs":[]}"#;
        assert_eq!(
            layer_version_over(&["a"], plan),
            layer_version(Some("a"), plan)
        );
        assert_eq!(layer_version_over(&[], plan), layer_version(None, plan));
        let pair = layer_version_over(&["a", "b"], plan);
        assert!(pair.starts_with("a+b@"));
        assert_ne!(pair, layer_version_over(&["a", "c"], plan));
        assert_ne!(pair, layer_version_over(&["b", "a"], plan));
        assert_ne!(pair, layer_version_over(&["a"], plan));
    }
}
