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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use swath_api::SourcePublisher;
use swath_core::catalog::Datetime;
use swath_core::sources::{
    Source, SourceEvent, SourceEventKind, SourceId, SourceStatus, SourceStore, SourceStoreError,
    state_of,
};

/// Sources and their events, in this process.
///
/// The registry also fans each recorded event onto the trace bus (#416)
/// when one is wired, so the Sources screen is live without polling. The
/// bus is a live view rather than a log: publishing is throttled per
/// source, and a suppressed event is still recorded here — the registry
/// is the count, the bus is the news.
#[derive(Debug, Default)]
pub(crate) struct SourceRegistry {
    sources: Mutex<BTreeMap<SourceId, Source>>,
    events: Mutex<Vec<SourceEvent>>,
    publisher: Mutex<Option<SourcePublisher>>,
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

    /// Fans recorded events onto `publisher` from now on (#416).
    pub(crate) fn publishing_to(&self, publisher: SourcePublisher) {
        *self.publisher.lock().expect("source registry") = Some(publisher);
    }

    /// Records `kind` against `id` now. Infallible by construction: an
    /// ingest task must never fail because its bookkeeping failed.
    pub(crate) fn record(&self, id: &SourceId, kind: SourceEventKind, detail: impl Into<String>) {
        let at = Datetime::from_unix_millis(now_unix_millis())
            .unwrap_or_else(|_| Datetime::new("1970-01-01T00:00:00Z").expect("the epoch"));
        let event = SourceEvent {
            source: id.clone(),
            at,
            kind,
            detail: detail.into(),
        };
        if let Some(publisher) = self.publisher.lock().expect("source registry").as_ref() {
            // Best-effort, exactly as the render path's publish is: an
            // ingest task must never stall on telemetry. The bus may
            // throttle this away; the registry below always counts it.
            publisher.publish(
                id.as_str(),
                wire_name(kind),
                event.at.as_str(),
                &event.detail,
            );
        }
        self.events.lock().expect("source registry").push(event);
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

/// The stable wire name of an event kind — what the bus envelope's
/// `event` field carries.
fn wire_name(kind: SourceEventKind) -> &'static str {
    match kind {
        SourceEventKind::Started => "started",
        SourceEventKind::Ingested => "ingested",
        SourceEventKind::Polled => "polled",
        SourceEventKind::Failed => "failed",
        SourceEventKind::Stopped => "stopped",
        // `SourceEventKind` is non_exhaustive: a kind added later names
        // itself here rather than being published as something it is not.
        _ => "unknown",
    }
}

/// How long a reachability probe may take before the source is called
/// unreachable. Stated rather than assumed (#417): a directory that does
/// not answer in this long is not one an ingest task can use, and a probe
/// that waited forever would report health it never measured.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How often each source is probed. Slow on purpose: this is a
/// heartbeat behind a live event stream, not the stream itself.
pub(crate) const PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// Probes `dir` forever, recording what it finds against `id` (#417).
///
/// The probe is what makes "watching" a measured fact: it either reads
/// the directory within [`PROBE_TIMEOUT`] or records the reason it could
/// not. It never records a success it did not observe.
pub(crate) async fn probe_loop(registry: Arc<SourceRegistry>, id: SourceId, dir: PathBuf) {
    loop {
        tokio::time::sleep(PROBE_INTERVAL).await;
        match probe_once(&dir).await {
            Ok(()) => registry.record(&id, SourceEventKind::Polled, ""),
            Err(detail) => registry.record(&id, SourceEventKind::Failed, detail),
        }
    }
}

/// One probe: the directory is readable, within the timeout.
pub(crate) async fn probe_once(dir: &std::path::Path) -> Result<(), String> {
    let target = dir.to_path_buf();
    let read = tokio::task::spawn_blocking(move || std::fs::read_dir(&target).map(|_| ()));
    match tokio::time::timeout(PROBE_TIMEOUT, read).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(err))) => Err(err.to_string()),
        Ok(Err(err)) => Err(format!("probe task failed: {err}")),
        Err(_) => Err(format!("no answer within {}s", PROBE_TIMEOUT.as_secs())),
    }
}

/// Wall-clock milliseconds since the Unix epoch.
fn now_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use swath_api::{BusEvent, TraceBus};
    use swath_core::catalog::Datetime;
    use swath_core::sources::SourceStore as _;
    use swath_core::sources::{Source, SourceEventKind, SourceId, SourceKind, SourceOrigin};

    use super::SourceRegistry;

    fn source(id: &str) -> Source {
        Source {
            id: SourceId::new(id),
            kind: SourceKind::Filedrop,
            target: "/srv/incoming".to_owned(),
            title: id.to_owned(),
            bindings: Vec::new(),
            origin: SourceOrigin::Config,
            credential_profile: None,
        }
    }

    /// Recorded events reach the bus with the **server's** timestamp, so
    /// a client computes freshness from when the thing happened rather
    /// than from its own clock (#416).
    #[test]
    fn recorded_events_reach_the_bus_with_the_servers_instant() {
        let bus = TraceBus::default();
        let mut receiver = bus.subscribe_for_test();
        let registry = SourceRegistry::with_sources([source("fire")]);
        registry.publishing_to(bus.publisher());

        registry.record(
            &SourceId::new("fire"),
            SourceEventKind::Started,
            "/data/fire",
        );
        let BusEvent::Ingest(event) = receiver.try_recv().expect("an event") else {
            panic!("a recorded source event is an ingest event")
        };
        assert_eq!(event.source, "fire");
        assert_eq!(event.kind, "started");
        assert_eq!(event.detail, "/data/fire");
        // The instant is the one the registry recorded, and it is the one
        // the derived state reads — not two clocks disagreeing.
        let recorded = registry.statuses();
        assert_eq!(
            recorded[0].1.last_event.as_ref().map(Datetime::as_str),
            Some(event.at.as_str())
        );
    }

    /// The probe measures rather than assumes (#417): a readable
    /// directory answers, a missing one records why it could not, and
    /// the timeout is a stated number rather than an unbounded wait.
    #[tokio::test]
    async fn the_probe_reports_what_it_found() {
        let dir = swath_testsupport::TempDir::new("cli-source-probe");
        assert_eq!(super::probe_once(dir.path()).await, Ok(()));

        let missing = dir.path().join("not-here");
        let err = super::probe_once(&missing)
            .await
            .expect_err("a missing directory is not reachable");
        assert!(!err.is_empty(), "the reason is the origin's own words");

        // Stated, not assumed: the timeout is a number this module owns
        // and the API's docs quote.
        assert_eq!(super::PROBE_TIMEOUT.as_secs(), 5);
        assert!(super::PROBE_INTERVAL > super::PROBE_TIMEOUT);
    }

    /// A probe failure and a later recovery move the derived state,
    /// which is what makes `reachable` on the API a measured fact rather
    /// than a stored one.
    #[tokio::test]
    async fn probe_events_move_the_derived_state() {
        let registry = SourceRegistry::with_sources([source("fire")]);
        let at = |value: &str| Datetime::new(value).expect("a test instant");
        let record = async |kind, when: &str| {
            registry
                .record_event(&swath_core::sources::SourceEvent {
                    source: SourceId::new("fire"),
                    at: at(when),
                    kind,
                    detail: String::new(),
                })
                .await
                .expect("recorded");
        };

        record(SourceEventKind::Failed, "2026-09-04T10:00:00Z").await;
        assert_eq!(registry.statuses()[0].1.failures, 1);
        assert!(matches!(
            registry.statuses()[0].1.state,
            swath_core::sources::SourceState::Failing { .. }
        ));

        // One probe interval later the source answers again: reachable,
        // and the failure still counted.
        record(SourceEventKind::Polled, "2026-09-04T10:00:30Z").await;
        assert!(matches!(
            registry.statuses()[0].1.state,
            swath_core::sources::SourceState::Watching { .. }
        ));
        assert_eq!(registry.statuses()[0].1.failures, 1);
    }

    /// A registry with no bus is a registry: recording still works, which
    /// is what keeps the ingest path independent of telemetry.
    #[test]
    fn recording_without_a_bus_is_not_an_error() {
        let registry = SourceRegistry::with_sources([source("fire")]);
        registry.record(&SourceId::new("fire"), SourceEventKind::Ingested, "g1");
        assert_eq!(registry.statuses()[0].1.ingested, 1);
    }

    /// The bus throttles a busy source; the registry does not. The count
    /// on the screen is therefore complete even when the live feed is
    /// deliberately quiet (#416).
    #[test]
    fn the_registry_counts_what_the_bus_coalesces() {
        let bus = TraceBus::default();
        let mut receiver = bus.subscribe_for_test();
        let registry = SourceRegistry::with_sources([source("fire")]);
        registry.publishing_to(bus.publisher());

        for i in 0..25 {
            registry.record(
                &SourceId::new("fire"),
                SourceEventKind::Ingested,
                format!("g{i}"),
            );
        }
        let mut published = 0;
        while receiver.try_recv().is_ok() {
            published += 1;
        }
        assert!(published < 25, "the bus is a live view, not a log");
        assert_eq!(
            registry.statuses()[0].1.ingested,
            25,
            "the registry counts every one"
        );
    }
}
