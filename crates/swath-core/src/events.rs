// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `EventSource` port: where new granules announce themselves
//! (ARCHITECTURE.md §6; adapters: file-drop first, S3 notifications / CMR
//! polling later).
//!
//! # Shape: pull, not stream
//!
//! The ARCHITECTURE.md sketch draws `subscribe() -> BoxStream<GranuleEvent>`.
//! This port is the pull form instead — `next_event(&mut self)`, one event
//! per await — for two deliberate reasons:
//!
//! 1. **No stream dependency in the core.** A `BoxStream` return type would
//!    put `futures-core` (and boxing) into the crate that owns no runtime;
//!    native AFIT (the same pattern as [`Catalog`](crate::catalog::Catalog)
//!    and `RasterSource`) needs nothing beyond `core::future`.
//! 2. **The consumer is a single sequential loop.** The ingest orchestrator
//!    processes events one at a time (each becomes a catalog upsert); a pull
//!    loop is exactly that shape, and adapters that buffer internally (a
//!    directory scan yielding several manifests) drain their buffer across
//!    successive calls. Fan-out to multiple consumers, if ever needed, is an
//!    adapter concern layered on top — not port surface today.
//!
//! `&mut self` is honest about that single-consumer contract: an event, once
//! yielded, is consumed.
//!
//! # Clocks
//!
//! The core stays clock-free: [`GranuleEvent::arrived_at`] is stamped by the
//! **adapter** (whose job is to observe the outside world, wall clocks
//! included) and merely carried here. It is the zero point of the
//! ingest-to-pixel metric (REQUIREMENTS.md §3): "a new granule arrives" is
//! defined as "the event source observed it".

use core::future::Future;

use crate::catalog::{Datetime, Granule};

/// A granule's arrival: the full domain [`Granule`] the announcement
/// described (id, dataset, footprint, acquisition time, band → asset map)
/// plus when the event source observed it.
///
/// The event carries a whole `Granule` rather than just ids: registration
/// needs footprint, acquisition time, and the asset map anyway, and every
/// planned source (file-drop manifest, S3 notification + metadata fetch, CMR
/// record) can supply them at announcement time. `ingested_at` on the carried
/// granule is adapter-irrelevant and ignored: the orchestrator stamps it from
/// [`arrived_at`](Self::arrived_at).
#[derive(Debug, Clone, PartialEq)]
pub struct GranuleEvent {
    /// The granule as announced.
    pub granule: Granule,
    /// When the event source observed the arrival (adapter's wall clock,
    /// UTC) — the ingest-to-pixel zero point.
    pub arrived_at: Datetime,
}

/// What can go wrong at the event-source boundary.
///
/// The port's error contract, defined in the core so consumers match on
/// semantics, not adapter internals (same pattern as
/// [`CatalogError`](crate::catalog::CatalogError)).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EventError {
    /// An announcement was observed but could not be understood (a malformed
    /// manifest, an unparsable notification). The source remains usable:
    /// consumers log and keep pulling.
    #[error("malformed granule announcement: {detail}")]
    Malformed {
        /// What was wrong, naming the offending announcement.
        detail: String,
    },

    /// The underlying transport/filesystem/service failed.
    #[error("event source backend failure: {detail}")]
    Backend {
        /// What was being attempted.
        detail: String,
        /// The underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// The ingest-trigger port (ARCHITECTURE.md §6): a sequential source of
/// granule arrivals. Implemented by adapter crates (file-drop first);
/// consumed by the ingest orchestrator's loop.
///
/// See the [module docs](self) for why this is pull-shaped, and the
/// [`crate::source`] module docs for the recorded AFIT trade-off (native
/// async-in-trait, `Send` futures, not dyn-compatible).
pub trait EventSource: Send {
    /// The next arrival. `Ok(None)` means the source is exhausted (a finite
    /// replay source, say); live watchers pend until an event arrives. An
    /// `Err` reports one bad announcement or a transient backend failure —
    /// the source stays pollable either way.
    fn next_event(
        &mut self,
    ) -> impl Future<Output = Result<Option<GranuleEvent>, EventError>> + Send;
}
