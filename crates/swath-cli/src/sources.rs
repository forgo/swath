// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The running deployment's sources (ADR 0030, issue #415): the registry
//! the ingest tasks record into, and the read side the API serves.
//!
//! One in-process store, shared by `Arc`. Config-declared sources live in
//! the config file, so the registry holds their **definitions** only as a
//! convenience; what it really owns is the **event log**, which is the
//! only authority on what each source is doing (ADR 0030 §2).
//!
//! # What a restart does
//!
//! The event log is in memory and does not survive a restart, deliberately:
//! it describes what this process has observed, and a process that has
//! just started has observed nothing. A source therefore reads `Unknown`
//! until its watch reports, rather than replaying a claim from a previous
//! run that may no longer be true. Definitions survive because they live
//! in the config file.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use swath_core::catalog::Datetime;
use swath_core::sources::{
    Source, SourceEvent, SourceEventKind, SourceId, SourceStatus, SourceStore, SourceStoreError,
    state_of,
};

/// Sources and their events, in this process.
#[derive(Debug, Default)]
pub(crate) struct SourceRegistry {
    sources: Mutex<BTreeMap<SourceId, Source>>,
    events: Mutex<Vec<SourceEvent>>,
}

impl SourceRegistry {
    /// A registry holding `sources` and no events yet.
    pub(crate) fn with_sources(sources: impl IntoIterator<Item = Source>) -> Arc<Self> {
        let registry = Self::default();
        {
            let mut held = registry.sources.lock().expect("source registry");
            for source in sources {
                held.insert(source.id.clone(), source);
            }
        }
        Arc::new(registry)
    }

    /// Records `kind` against `id` now. Infallible by construction: an
    /// ingest task must never fail because its bookkeeping failed.
    pub(crate) fn record(&self, id: &SourceId, kind: SourceEventKind, detail: impl Into<String>) {
        let at = Datetime::from_unix_millis(now_unix_millis())
            .unwrap_or_else(|_| Datetime::new("1970-01-01T00:00:00Z").expect("the epoch"));
        self.events
            .lock()
            .expect("source registry")
            .push(SourceEvent {
                source: id.clone(),
                at,
                kind,
                detail: detail.into(),
            });
    }

    /// Every source with its derived status, in id order.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the HTTP read side lands with the sources resource (#417)"
        )
    )]
    pub(crate) fn statuses(&self) -> Vec<(Source, SourceStatus)> {
        let events = self.events.lock().expect("source registry").clone();
        self.sources
            .lock()
            .expect("source registry")
            .values()
            .map(|source| {
                let own: Vec<SourceEvent> = events
                    .iter()
                    .filter(|event| event.source == source.id)
                    .cloned()
                    .collect();
                (source.clone(), state_of(&own))
            })
            .collect()
    }
}

impl SourceStore for SourceRegistry {
    async fn upsert_source(&self, source: &Source) -> Result<(), SourceStoreError> {
        self.sources
            .lock()
            .expect("source registry")
            .insert(source.id.clone(), source.clone());
        Ok(())
    }

    async fn get_source(&self, id: &SourceId) -> Result<Option<Source>, SourceStoreError> {
        Ok(self
            .sources
            .lock()
            .expect("source registry")
            .get(id)
            .cloned())
    }

    async fn list_sources(&self) -> Result<Vec<Source>, SourceStoreError> {
        Ok(self
            .sources
            .lock()
            .expect("source registry")
            .values()
            .cloned()
            .collect())
    }

    async fn delete_source(&self, id: &SourceId) -> Result<(), SourceStoreError> {
        if self
            .sources
            .lock()
            .expect("source registry")
            .remove(id)
            .is_none()
        {
            return Err(SourceStoreError::NotFound { id: id.clone() });
        }
        // The origin and its history go; the granules it ingested stay
        // (ADR 0030 §3) — nothing here can reach them.
        self.events
            .lock()
            .expect("source registry")
            .retain(|event| &event.source != id);
        Ok(())
    }

    async fn record_event(&self, event: &SourceEvent) -> Result<(), SourceStoreError> {
        self.events
            .lock()
            .expect("source registry")
            .push(event.clone());
        Ok(())
    }

    async fn events(&self, id: &SourceId) -> Result<Vec<SourceEvent>, SourceStoreError> {
        Ok(self
            .events
            .lock()
            .expect("source registry")
            .iter()
            .filter(|event| &event.source == id)
            .cloned()
            .collect())
    }
}

/// Wall-clock milliseconds since the Unix epoch.
fn now_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
