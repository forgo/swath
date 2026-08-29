// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `object_store`-backed adapters (#352): what Swath persists beside
//! the catalog — encoded tiles ([`ObjectStoreTileCache`], the `TileCache`
//! port) and `run_udf` module bytes ([`ObjectStoreModuleStore`], the
//! `ModuleStore` port; [`HttpModuleFetcher`], the once-per-publish fetch).
//! One crate because they are one shape: a hash-sharded key layout under a
//! prefix (`tiles/`, `udf/`) over whatever store the binary wires in —
//! local filesystem, in-memory, or S3-compatible. Each module documents
//! its own layout, framing and failure semantics.

mod module_store;
mod tile_cache;

pub use module_store::{HttpModuleFetcher, ObjectStoreModuleStore};
pub use tile_cache::ObjectStoreTileCache;
