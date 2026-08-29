// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `ModuleStore` and `ModuleFetcher` adapters over `object_store`
//! (ADR 0018, issue #204): where `run_udf` module bytes persist, and how
//! a remote module is fetched — once — at the publish motion.
//!
//! # Store layout
//!
//! A module's `code_hash` is 64 lowercase hex chars; its bytes live at
//! `udf/<hh>/<hh>/<full-hash>.wasm` — the same two-level shard the tile
//! cache uses (the sibling [`tile_cache`](super::tile_cache) module), so a filesystem root never
//! accumulates thousands of siblings and an operator can map an object
//! back to its hash by inspection. The object is the raw module: no
//! framing, no metadata — the hash IS the integrity check, verified on
//! every `get` (foreign or damaged bytes under the prefix are
//! [`ModuleStoreError::Corrupt`], never served).
//!
//! # Fetch
//!
//! [`HttpModuleFetcher`] resolves `http(s)` URLs through `object_store`'s
//! HTTP store: one `GET`, with the declared size checked against
//! [`MODULE_MAX_BYTES`] **before** the body is buffered. It is called by
//! the compile motion only; serving and rehydration go to the store by
//! hash (see `swath_core::udf`).
//!
//! # Deferred
//!
//! No GC: entries are never deleted (a deleted service's module stays
//! resolvable; a re-published identical module is a no-op). The sweep by
//! reachability from `swath:layers` is the recorded deferral in
//! `docs/ROADMAP.md`'s inventory.

use std::sync::Arc;

use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _, PutPayload};
use swath_core::udf::{
    MODULE_MAX_BYTES, ModuleFetchError, ModuleFetcher, ModuleStore, ModuleStoreError, code_hash,
};

/// Prefix all entries live under, so a store can share a root with other
/// data (and a future GC sweep knows exactly what is its own).
const UDF_PREFIX: &str = "udf";

/// The `object_store`-backed [`ModuleStore`]: local filesystem,
/// in-memory, or S3-compatible — whatever store the binary wires in.
#[derive(Debug, Clone)]
pub struct ObjectStoreModuleStore {
    store: Arc<dyn ObjectStore>,
}

impl ObjectStoreModuleStore {
    /// A module store over `store`, under the `udf/` prefix.
    #[must_use]
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    /// The object path of `code_hash`: `udf/<hh>/<hh>/<hash>.wasm`.
    fn path(code_hash: &str) -> Path {
        let (first, second) = (
            code_hash.get(0..2).unwrap_or(code_hash),
            code_hash.get(2..4).unwrap_or_default(),
        );
        Path::from(format!("{UDF_PREFIX}/{first}/{second}/{code_hash}.wasm"))
    }
}

fn io(err: &object_store::Error) -> ModuleStoreError {
    ModuleStoreError::Io {
        detail: err.to_string(),
    }
}

impl ModuleStore for ObjectStoreModuleStore {
    async fn get(&self, code_hash: &str) -> Result<Option<Vec<u8>>, ModuleStoreError> {
        let result = match self.store.get(&Self::path(code_hash)).await {
            Ok(result) => result,
            Err(object_store::Error::NotFound { .. }) => return Ok(None),
            Err(err) => return Err(io(&err)),
        };
        let bytes = result.bytes().await.map_err(|err| io(&err))?.to_vec();
        let actual = swath_core::udf::code_hash(&bytes);
        if actual != code_hash {
            return Err(ModuleStoreError::Corrupt {
                code_hash: code_hash.to_owned(),
                actual,
            });
        }
        Ok(Some(bytes))
    }

    async fn put(&self, bytes: &[u8]) -> Result<String, ModuleStoreError> {
        if bytes.len() > MODULE_MAX_BYTES {
            return Err(ModuleStoreError::TooLarge { len: bytes.len() });
        }
        let hash = code_hash(bytes);
        let path = Self::path(&hash);
        // Content-addressed: an existing object under this hash already
        // holds these bytes — a re-put is a no-op, never a rewrite.
        match self.store.head(&path).await {
            Ok(_) => return Ok(hash),
            Err(object_store::Error::NotFound { .. }) => {}
            Err(err) => return Err(io(&err)),
        }
        self.store
            .put(&path, PutPayload::from(bytes.to_vec()))
            .await
            .map_err(|err| io(&err))?;
        Ok(hash)
    }
}

/// The [`ModuleFetcher`] over `object_store`'s HTTP store: `http` and
/// `https` URLs only, size-checked before buffering.
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpModuleFetcher;

impl HttpModuleFetcher {
    /// A fetcher; it holds no connection state.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ModuleFetcher for HttpModuleFetcher {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, ModuleFetchError> {
        let unsupported = || ModuleFetchError::Unsupported {
            url: url.to_owned(),
        };
        let parsed = url::Url::parse(url).map_err(|_| unsupported())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(unsupported());
        }
        let transport = |detail: String| ModuleFetchError::Transport {
            url: url.to_owned(),
            detail,
        };
        // The store's base is the URL's origin; the object is its path.
        // Plain `http` is allowed: the profile admits both schemes, and
        // content addressing — not transport — is what pins a published
        // module.
        let base = &parsed[..url::Position::BeforePath];
        let store = object_store::http::HttpBuilder::new()
            .with_url(base)
            .with_client_options(object_store::ClientOptions::new().with_allow_http(true))
            .build()
            .map_err(|err| transport(err.to_string()))?;
        let path = Path::parse(parsed.path()).map_err(|err| transport(err.to_string()))?;
        let result = match store.get(&path).await {
            Ok(result) => result,
            Err(object_store::Error::NotFound { .. }) => {
                return Err(ModuleFetchError::NotFound {
                    url: url.to_owned(),
                });
            }
            Err(err) => {
                return Err(ModuleFetchError::Transport {
                    url: url.to_owned(),
                    detail: err.to_string(),
                });
            }
        };
        // The declared size gates the read: an oversized body is never
        // buffered.
        let size = result.meta.size;
        if size > MODULE_MAX_BYTES as u64 {
            return Err(ModuleFetchError::TooLarge {
                url: url.to_owned(),
                size,
            });
        }
        let bytes = result
            .bytes()
            .await
            .map_err(|err| ModuleFetchError::Transport {
                url: url.to_owned(),
                detail: err.to_string(),
            })?;
        if bytes.len() > MODULE_MAX_BYTES {
            return Err(ModuleFetchError::TooLarge {
                url: url.to_owned(),
                size: bytes.len() as u64,
            });
        }
        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use object_store::ObjectStoreExt as _;
    use object_store::memory::InMemory;
    use swath_core::udf::{MODULE_MAX_BYTES, ModuleStore, ModuleStoreError, code_hash};

    use super::ObjectStoreModuleStore;

    #[tokio::test]
    async fn put_then_get_round_trips_by_hash_and_misses_are_none() {
        let store = ObjectStoreModuleStore::new(Arc::new(InMemory::new()));
        let bytes = b"\0asm\x01\0\0\0module";
        let hash = store.put(bytes).await.expect("put");
        assert_eq!(hash, code_hash(bytes));
        assert_eq!(store.get(&hash).await.expect("get"), Some(bytes.to_vec()));
        assert_eq!(store.get(&code_hash(b"other")).await.expect("get"), None);
        // Re-putting identical bytes is a no-op answering the same hash.
        assert_eq!(store.put(bytes).await.expect("re-put"), hash);
    }

    #[tokio::test]
    async fn objects_are_sharded_under_the_udf_prefix() {
        let hash = "cafe0000000000000000000000000000000000000000000000000000000000ff";
        assert_eq!(
            ObjectStoreModuleStore::path(hash).as_ref(),
            format!("udf/ca/fe/{hash}.wasm")
        );
    }

    /// Foreign bytes under a hash's path are refused, never served: the
    /// hash is the integrity check.
    #[tokio::test]
    async fn tampered_objects_are_corrupt_not_served() {
        let inner = Arc::new(InMemory::new());
        let store = ObjectStoreModuleStore::new(inner.clone());
        let hash = code_hash(b"published module");
        inner
            .put(
                &ObjectStoreModuleStore::path(&hash),
                object_store::PutPayload::from_static(b"tampered"),
            )
            .await
            .expect("tamper");
        assert_eq!(
            store.get(&hash).await,
            Err(ModuleStoreError::Corrupt {
                code_hash: hash,
                actual: code_hash(b"tampered"),
            })
        );
    }

    #[tokio::test]
    async fn oversized_modules_are_refused() {
        let store = ObjectStoreModuleStore::new(Arc::new(InMemory::new()));
        let big = vec![0u8; MODULE_MAX_BYTES + 1];
        assert_eq!(
            store.put(&big).await,
            Err(ModuleStoreError::TooLarge {
                len: MODULE_MAX_BYTES + 1
            })
        );
    }
}
