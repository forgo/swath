// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The binary's composite `RasterSource`: COG assets and virtual-cube
//! assets served through one port instance (issue #39).
//!
//! The API/render stack is generic over a single `S: RasterSource`
//! (deliberately not dyn-compatible — see the port's module docs), so the
//! binary supplies **one** concrete source that holds both compiled-in
//! adapters and dispatches per asset. This is exactly the "enum over the
//! compiled-in adapters" future the port docs recorded, realized here in
//! the wiring layer rather than the core: the core stays adapter-blind,
//! and adding a source kind is a binary-level change.
//!
//! Dispatch is on the asset's addressing shape, which the ingest path
//! guarantees: virtual-cube assets are `<key>.vmanifest.json[#<array>]`
//! (the filedrop legacy path writes them, `swath ingest reference` names
//! them) — [`VirtualSource::handles`] recognizes exactly that convention.
//! Everything else is a COG object path, as before #39.

use std::sync::Arc;

use object_store::ObjectStore;
use swath_core::raster::{AssetRef, RasterInfo, WindowRequest};
use swath_core::source::{BandSelection, RasterSource, ReadLevel, SourceError, WindowData};
use swath_source_cog::CogSource;
use swath_source_inmem::InMemSource;
use swath_source_virtual::VirtualSource;

/// COG + virtual-cube + in-memory-demo reads over one port instance,
/// dispatched per asset.
#[derive(Debug, Clone)]
pub(crate) struct CompositeSource {
    cog: CogSource,
    virtual_cube: VirtualSource,
    /// The docs/EXTENDING.md walkthrough toy (`inmem:` scheme, issue #125).
    inmem: InMemSource,
}

impl CompositeSource {
    /// Both storage adapters over the same store — one storage root, two
    /// formats — plus the storage-free in-memory demo.
    pub(crate) fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self {
            cog: CogSource::new(Arc::clone(&store)),
            virtual_cube: VirtualSource::new(store),
            inmem: InMemSource::demo(),
        }
    }
}

impl RasterSource for CompositeSource {
    async fn describe(&self, asset: &AssetRef) -> Result<RasterInfo, SourceError> {
        if InMemSource::handles(asset) {
            self.inmem.describe(asset).await
        } else if VirtualSource::handles(asset) {
            self.virtual_cube.describe(asset).await
        } else {
            self.cog.describe(asset).await
        }
    }

    async fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
        level: ReadLevel,
    ) -> Result<WindowData, SourceError> {
        if InMemSource::handles(asset) {
            self.inmem.read_window(asset, window, band, level).await
        } else if VirtualSource::handles(asset) {
            self.virtual_cube
                .read_window(asset, window, band, level)
                .await
        } else {
            self.cog.read_window(asset, window, band, level).await
        }
    }
}
