// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `run_udf` module store and fetcher ports (ADR 0018, issue #204):
//! where the bytes of a sandboxed WASM module live, and how they arrive.
//!
//! # Content addressing
//!
//! A module's identity is its [`code_hash`] — lowercase sha256 hex of the
//! bytes, the string every [`UdfStage`](crate::catalog::PlanKind::Udf)
//! carries. The [`ModuleStore`] is keyed by nothing else: `put` computes
//! the hash and answers it, `get` resolves it. Content addressing is what
//! makes a published service immutable — a graph's `udf` argument may be
//! a remote URL, but the service persists the hash, and serving resolves
//! the hash, so **a mutated remote can never change what a published
//! service renders**. The URL is fetched exactly once, at the compile
//! motion ([`ModuleFetcher`]); rehydration on restart goes to the store
//! by hash and never fetches.
//!
//! # Bounds
//!
//! A module is at most [`MODULE_MAX_BYTES`] (8 MiB), enforced before any
//! byte is buffered: inline `data:` payloads are refused by encoded
//! length, remote fetches by declared size. The bound is a wire/storage
//! bound, distinct from the 64 MiB *linear memory* cap ADR 0018 places on
//! a running instance.
//!
//! # Deferred: store GC
//!
//! Entries are never deleted: a hash a deleted service referenced stays
//! resolvable (a re-published identical module is a no-op `put`). A sweep
//! by reachability from the catalog's `swath:layers` is the recorded
//! deferral in `docs/ROADMAP.md`'s inventory (the ADR 0018 v2 list) —
//! like the tile cache's GC, it earns nothing until storage pressure is
//! measured.
//!
//! # The ports
//!
//! Both follow the crate's native-AFIT port pattern (see
//! [`crate::source`]): `Send` futures, no runtime dependency in the
//! core. The adapters live in `swath-store-objectstore`.

use core::future::Future;

use sha2::{Digest, Sha256};

/// The largest module the engine accepts on the wire and in the store:
/// 8 MiB. Larger modules are refused before buffering.
pub const MODULE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// The content-addressed identity of module `bytes`: lowercase sha256
/// hex — the `code_hash` a plan's UDF stage and the persisted layer carry,
/// and the key the [`ModuleStore`] resolves. The exact digest for a known
/// input is pinned by a test.
#[must_use]
pub fn code_hash(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(hex, "{byte:02x}").expect("writing hex to a String is infallible");
    }
    hex
}

/// Why a [`ModuleStore`] operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ModuleStoreError {
    /// The backing store failed (transport, filesystem).
    #[error("module store I/O: {detail}")]
    Io {
        /// The backend's explanation.
        detail: String,
    },
    /// The bytes stored under `code_hash` do not hash to it: foreign or
    /// damaged bytes under the store's prefix. Never served — a
    /// content-addressed store that answered them would let a tampered
    /// file change a published service.
    #[error("module `{code_hash}` is corrupt: stored bytes hash to `{actual}`")]
    Corrupt {
        /// The requested hash.
        code_hash: String,
        /// What the stored bytes actually hash to.
        actual: String,
    },
    /// A `put` over [`MODULE_MAX_BYTES`].
    #[error("module of {len} bytes exceeds the {MODULE_MAX_BYTES}-byte limit")]
    TooLarge {
        /// The offered length.
        len: usize,
    },
}

/// The content-addressed module store port: hash → bytes.
///
/// Entries are immutable under their key (same hash ⇒ same bytes), so a
/// `put` of already-stored bytes is a no-op, and a `get` can be cached
/// forever. A missing hash is a plain `Ok(None)`.
pub trait ModuleStore: Send + Sync {
    /// The bytes stored under `code_hash`, verified to hash to it; `None`
    /// when nothing is stored there.
    fn get(
        &self,
        code_hash: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, ModuleStoreError>> + Send;

    /// Stores `bytes` under their [`code_hash`], answering it. Refuses
    /// more than [`MODULE_MAX_BYTES`].
    fn put(&self, bytes: &[u8]) -> impl Future<Output = Result<String, ModuleStoreError>> + Send;
}

/// Why a [`ModuleFetcher`] could not deliver a remote module.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ModuleFetchError {
    /// The URL's scheme is not one the fetcher speaks (only `http`/`https`
    /// are ever offered to it).
    #[error("cannot fetch `{url}`: unsupported URL")]
    Unsupported {
        /// The offending URL.
        url: String,
    },
    /// The remote answered that nothing is there.
    #[error("cannot fetch `{url}`: not found")]
    NotFound {
        /// The URL.
        url: String,
    },
    /// The remote declares more than [`MODULE_MAX_BYTES`]; refused before
    /// the body is read.
    #[error("cannot fetch `{url}`: {size} bytes exceeds the {MODULE_MAX_BYTES}-byte limit")]
    TooLarge {
        /// The URL.
        url: String,
        /// The declared size.
        size: u64,
    },
    /// Any other transport failure.
    #[error("cannot fetch `{url}`: {detail}")]
    Transport {
        /// The URL.
        url: String,
        /// The transport's explanation.
        detail: String,
    },
}

/// The remote-module fetch port: called **once per compile motion** for a
/// graph whose `udf` argument is an `http(s)` URL — never at serve time,
/// never on rehydration (module docs).
pub trait ModuleFetcher: Send + Sync {
    /// The bytes at `url`, refusing bodies over [`MODULE_MAX_BYTES`]
    /// before buffering them.
    fn fetch(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, ModuleFetchError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::{MODULE_MAX_BYTES, code_hash};

    /// The identity function is pinned: sha256 of the empty input and of
    /// a known string, lowercase hex — a changed hashing scheme would
    /// orphan every stored module and persisted service.
    #[test]
    fn code_hash_is_lowercase_sha256_hex() {
        assert_eq!(
            code_hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            code_hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_wire_bound_is_eight_mebibytes() {
        assert_eq!(MODULE_MAX_BYTES, 8_388_608);
    }
}
