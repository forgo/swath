// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `run_udf` publish motion (ADR 0018, issue #204): what the openEO
//! surface needs beyond the compiler to accept a graph naming a WASM
//! module — the registrar that validates it, the store that persists it
//! by hash, and the fetcher that resolves a remote `udf` URL **once**.
//!
//! Two resolutions, deliberately distinct:
//!
//! - [`UdfPublish::resolve`] — a fresh compile (`POST /services`,
//!   `POST /result`): every remote `udf` argument is fetched now, one
//!   `GET` per node, and handed to the compiler; inline `data:` modules
//!   need nothing. After a successful publish the module bytes go into
//!   the store ([`UdfPublish::persist`]).
//! - [`UdfPublish::rehydrate`] — a persisted service coming back at
//!   startup: the layer's `PlanKind::Udf { code_hash }` names the bytes,
//!   the store answers them, and the compiler receives them under the
//!   graph's remote URL. **No fetch, ever**: a mutated remote cannot
//!   change what a published service renders.
//!
//! The ports are the core's native-AFIT traits; this module erases them
//! behind boxed futures so the openEO state stays generic over its three
//! render/catalog ports only.
//!
//! The registrar and the tile-path executor are **one object** (#205):
//! [`UdfPublish::new`] takes something that is both, so a module the
//! compile motion registered is runnable by exactly the executor
//! [`UdfPublish::executor`] hands the tile handlers and the preview —
//! the "a registration is a promise the tile path keeps" invariant is a
//! type, not a convention.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use swath_core::catalog::{Layer as DomainLayer, PlanKind};
use swath_core::udf::{ModuleFetchError, ModuleFetcher, ModuleStore, ModuleStoreError};
use swath_render::{CompileContext, CompileError, UdfExecutor, UdfRegistrar, UdfSource};

/// The shared tile-path executor handle: what [`ApiState::with_udf_executor`](crate::ApiState::with_udf_executor)
/// takes and [`UdfPublish::executor`] answers.
pub type SharedUdfExecutor = Arc<dyn UdfExecutor>;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Object-safe view of [`ModuleStore`].
trait DynModuleStore: Send + Sync {
    fn get<'a>(
        &'a self,
        code_hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<Vec<u8>>, ModuleStoreError>>;
    fn put<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<String, ModuleStoreError>>;
}

impl<M: ModuleStore> DynModuleStore for M {
    fn get<'a>(
        &'a self,
        code_hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<Vec<u8>>, ModuleStoreError>> {
        Box::pin(ModuleStore::get(self, code_hash))
    }

    fn put<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<String, ModuleStoreError>> {
        Box::pin(ModuleStore::put(self, bytes))
    }
}

/// Object-safe view of [`ModuleFetcher`].
trait DynModuleFetcher: Send + Sync {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ModuleFetchError>>;
}

impl<F: ModuleFetcher> DynModuleFetcher for F {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ModuleFetchError>> {
        Box::pin(ModuleFetcher::fetch(self, url))
    }
}

/// The `run_udf` wiring: registrar + tile-path executor (one object) +
/// module store + fetcher. Cloning shares all of them.
#[derive(Clone)]
pub struct UdfPublish {
    registrar: Arc<dyn UdfRegistrar>,
    executor: SharedUdfExecutor,
    store: Arc<dyn DynModuleStore>,
    fetcher: Arc<dyn DynModuleFetcher>,
}

impl std::fmt::Debug for UdfPublish {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UdfPublish { registrar, executor, store, fetcher }")
    }
}

/// Why a persisted UDF service could not be rehydrated from the store.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RehydrateError {
    /// The store holds nothing under the layer's `code_hash`: the module
    /// was never persisted, or the store root changed under the catalog.
    #[error("module `{code_hash}` is not in the module store")]
    ModuleMissing {
        /// The persisted hash.
        code_hash: String,
    },
    /// The store failed (I/O, or bytes that do not hash to the key).
    #[error(transparent)]
    Store(#[from] ModuleStoreError),
}

/// A compile motion's `run_udf` inputs: the registrar plus the remote
/// modules resolved for one graph (keyed by their `udf` URL). Built by
/// [`UdfPublish::resolve`] / [`UdfPublish::rehydrate`], applied to a
/// [`CompileContext`] by [`UdfModules::apply`].
#[derive(Clone)]
pub struct UdfModules {
    registrar: Arc<dyn UdfRegistrar>,
    remote: Vec<(String, Vec<u8>)>,
}

impl std::fmt::Debug for UdfModules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let remote: Vec<(&str, usize)> = self
            .remote
            .iter()
            .map(|(url, bytes)| (url.as_str(), bytes.len()))
            .collect();
        f.debug_struct("UdfModules")
            .field("remote", &remote)
            .finish_non_exhaustive()
    }
}

impl UdfModules {
    /// Wires the registrar and every resolved remote module into `ctx`.
    #[must_use]
    pub fn apply(&self, ctx: CompileContext) -> CompileContext {
        self.remote.iter().fold(
            ctx.with_udf_registrar(Arc::clone(&self.registrar)),
            |ctx, (url, bytes)| ctx.with_udf_module(url, bytes.clone()),
        )
    }
}

/// Every `run_udf` node's `(node id, udf argument)` in a graph — the
/// pre-pass that decides what to fetch (the compiler re-validates each
/// one, and enforces one per graph).
fn udf_arguments(process: &Value) -> Vec<(&str, &str)> {
    let Some(nodes) = process.get("process_graph").unwrap_or(process).as_object() else {
        return Vec::new();
    };
    nodes
        .iter()
        .filter_map(|(id, node)| {
            (node.get("process_id")?.as_str()? == "run_udf")
                .then(|| node.get("arguments")?.get("udf")?.as_str())
                .flatten()
                .map(|udf| (id.as_str(), udf))
        })
        .collect()
}

impl UdfPublish {
    /// The wiring over concrete adapters. `runtime` is both the
    /// compile-motion registrar and the tile-path executor (the wasmtime
    /// adapter is both over one module LRU), so what registers is what
    /// runs.
    pub fn new<U, M, F>(runtime: Arc<U>, store: M, fetcher: F) -> Self
    where
        U: UdfRegistrar + UdfExecutor + 'static,
        M: ModuleStore + 'static,
        F: ModuleFetcher + 'static,
    {
        Self {
            registrar: Arc::clone(&runtime) as Arc<dyn UdfRegistrar>,
            executor: runtime,
            store: Arc::new(store),
            fetcher: Arc::new(fetcher),
        }
    }

    /// The tile-path executor (#205): hand it to
    /// [`ApiState::with_udf_executor`](crate::ApiState::with_udf_executor);
    /// the preview (`POST /result`) renders through it too.
    #[must_use]
    pub fn executor(&self) -> SharedUdfExecutor {
        Arc::clone(&self.executor)
    }

    /// Resolves `process`'s remote modules for a fresh compile: each
    /// `run_udf` node whose `udf` is an `http(s)` URL is fetched exactly
    /// once, here. Inline modules and malformed arguments are left to the
    /// compiler's own diagnostics.
    ///
    /// # Errors
    ///
    /// A fetch failure as [`CompileError::InvalidArgument`] naming the
    /// node and the `udf` argument (the openEO `ProcessParameterInvalid`).
    pub async fn resolve(&self, process: &Value) -> Result<UdfModules, CompileError> {
        let mut remote = Vec::new();
        for (node, udf) in udf_arguments(process) {
            let Ok(UdfSource::Remote(url)) = UdfSource::parse(udf) else {
                continue;
            };
            if remote.iter().any(|(seen, _)| *seen == url) {
                continue;
            }
            let bytes =
                self.fetcher
                    .fetch(&url)
                    .await
                    .map_err(|err| CompileError::InvalidArgument {
                        node: node.to_owned(),
                        process: "run_udf".into(),
                        argument: "udf".into(),
                        detail: err.to_string(),
                    })?;
            remote.push((url, bytes));
        }
        Ok(UdfModules {
            registrar: Arc::clone(&self.registrar),
            remote,
        })
    }

    /// Resolves a persisted service's modules for rehydration: a layer
    /// whose plan is `PlanKind::Udf { code_hash }` gets its bytes from the
    /// store by that hash, offered to the compiler under every remote
    /// `udf` URL its graph names. Never fetches. Layers of any other kind
    /// resolve to the bare registrar.
    ///
    /// # Errors
    ///
    /// [`RehydrateError`] when the store cannot answer the hash.
    pub async fn rehydrate(&self, layer: &DomainLayer) -> Result<UdfModules, RehydrateError> {
        let registrar = Arc::clone(&self.registrar);
        let PlanKind::Udf { code_hash } = &layer.plan else {
            return Ok(UdfModules {
                registrar,
                remote: Vec::new(),
            });
        };
        let urls: Vec<String> = layer
            .process
            .as_ref()
            .map(udf_arguments)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, udf)| match UdfSource::parse(udf) {
                Ok(UdfSource::Remote(url)) => Some(url),
                _ => None,
            })
            .collect();
        if urls.is_empty() {
            // Inline module: the graph carries the bytes verbatim.
            return Ok(UdfModules {
                registrar,
                remote: Vec::new(),
            });
        }
        let bytes =
            self.store
                .get(code_hash)
                .await?
                .ok_or_else(|| RehydrateError::ModuleMissing {
                    code_hash: code_hash.clone(),
                })?;
        Ok(UdfModules {
            registrar,
            remote: urls.into_iter().map(|url| (url, bytes.clone())).collect(),
        })
    }

    /// Persists a compiled product's module bytes, answering the hash the
    /// stage carries.
    ///
    /// # Errors
    ///
    /// [`ModuleStoreError`] from the store.
    pub async fn persist(&self, bytes: &[u8]) -> Result<String, ModuleStoreError> {
        self.store.put(bytes).await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::udf_arguments;

    #[test]
    fn udf_arguments_reads_every_run_udf_node_wrapped_or_bare() {
        let graph = json!({ "process_graph": {
            "load": { "process_id": "load_collection", "arguments": { "id": "x" } },
            "a": { "process_id": "run_udf", "arguments": { "udf": "https://h/a.wasm" } },
            "b": { "process_id": "run_udf", "arguments": { "udf": "data:application/wasm;base64,AA==" } },
            "c": { "process_id": "run_udf", "arguments": {} },
        }});
        assert_eq!(
            udf_arguments(&graph),
            [
                ("a", "https://h/a.wasm"),
                ("b", "data:application/wasm;base64,AA=="),
            ]
        );
        assert!(udf_arguments(&json!({ "process_graph": {} })).is_empty());
        assert!(udf_arguments(&json!("nope")).is_empty());
    }
}
