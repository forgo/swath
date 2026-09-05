// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The sources domain (#422, ADR 0030). Two properties carry the whole
//! design and both are asserted here: **state is derived from the event
//! log**, so it cannot drift from reality, and **no field can hold a
//! secret**, so "no secret reaches the catalog" is a property of the type
//! rather than a rule someone has to remember.
//!
//! The in-memory store below is also the reference for what deleting a
//! source means: the origin and its events go, the granules stay.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use swath_core::catalog::{DatasetId, Datetime};
use swath_core::sources::{
    Source, SourceEvent, SourceEventKind, SourceId, SourceKind, SourceOrigin, SourceState,
    SourceStore, SourceStoreError, state_of, statuses,
};

/// The core carries no runtime by design (ARCHITECTURE §6), so these
/// tests drive the store's futures themselves. Every one of them is ready
/// on the first poll — the in-memory store never awaits — so a single
/// poll with a no-op waker is the whole executor this suite needs.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the in-memory store never awaits"),
    }
}

fn at(value: &str) -> Datetime {
    Datetime::new(value).expect("a test instant")
}

fn source(id: &str) -> Source {
    Source {
        id: SourceId::new(id),
        kind: SourceKind::Filedrop,
        target: "/srv/incoming".to_owned(),
        title: "Incoming".to_owned(),
        bindings: vec![DatasetId::new("hls-s30")],
        origin: SourceOrigin::Config,
        credential_profile: None,
        requester_pays: false,
    }
}

fn event(id: &str, kind: SourceEventKind, when: &str, detail: &str) -> SourceEvent {
    SourceEvent {
        source: SourceId::new(id),
        at: at(when),
        kind,
        detail: detail.to_owned(),
    }
}

// --- State is derived, never stored (ADR 0030 §2) ---

/// A source nothing has happened to says so. It does not say "healthy",
/// which is the default a stored field would have given it.
#[test]
fn no_events_is_unknown_not_healthy() {
    let status = state_of(&[]);
    assert_eq!(status.state, SourceState::Unknown);
    assert_eq!(status.ingested, 0);
    assert_eq!(status.failures, 0);
    assert!(status.last_event.is_none());
}

/// The newest event decides, and the counts are the whole history.
#[test]
fn the_newest_event_decides_the_state() {
    let events = [
        event("s", SourceEventKind::Started, "2026-09-01T00:00:00Z", ""),
        event(
            "s",
            SourceEventKind::Ingested,
            "2026-09-01T00:05:00Z",
            "a.tif",
        ),
        event(
            "s",
            SourceEventKind::Ingested,
            "2026-09-01T00:06:00Z",
            "b.tif",
        ),
        event("s", SourceEventKind::Polled, "2026-09-01T00:09:00Z", ""),
    ];
    let status = state_of(&events);
    assert_eq!(
        status.state,
        SourceState::Watching {
            since: at("2026-09-01T00:00:00Z"),
            last_event: at("2026-09-01T00:09:00Z"),
        }
    );
    assert_eq!(status.ingested, 2);
    assert_eq!(status.last_event, Some(at("2026-09-01T00:09:00Z")));
}

/// A failure is the state only while it is the last word. A source that
/// recovered is watching — saying otherwise would be as wrong as a stale
/// healthy field, in the other direction.
#[test]
fn a_failure_ends_when_the_source_recovers() {
    let failed = event(
        "s",
        SourceEventKind::Failed,
        "2026-09-01T01:00:00Z",
        "permission denied",
    );
    let failing = state_of(&[
        event("s", SourceEventKind::Started, "2026-09-01T00:00:00Z", ""),
        failed.clone(),
    ]);
    assert_eq!(
        failing.state,
        SourceState::Failing {
            since: at("2026-09-01T01:00:00Z"),
            detail: "permission denied".to_owned(),
        }
    );

    let recovered = state_of(&[
        event("s", SourceEventKind::Started, "2026-09-01T00:00:00Z", ""),
        failed,
        event("s", SourceEventKind::Polled, "2026-09-01T01:30:00Z", ""),
    ]);
    assert!(matches!(recovered.state, SourceState::Watching { .. }));
    // The failure is still counted: recovered is not the same as never
    // having failed.
    assert_eq!(recovered.failures, 1);
}

/// The derivation is a function of the *set*, not of the order it was
/// handed in — a store that returns rows in any order gets one answer.
#[test]
fn the_state_does_not_depend_on_the_order_of_the_events() {
    let mut events = vec![
        event("s", SourceEventKind::Started, "2026-09-01T00:00:00Z", ""),
        event("s", SourceEventKind::Ingested, "2026-09-01T00:05:00Z", "a"),
        event("s", SourceEventKind::Failed, "2026-09-01T02:00:00Z", "gone"),
    ];
    let forward = state_of(&events);
    events.reverse();
    assert_eq!(state_of(&events), forward);
    events.swap(0, 1);
    assert_eq!(state_of(&events), forward);
}

/// Two events at the same instant: the more decisive outcome wins, so a
/// failure is never hidden behind a poll recorded in the same
/// millisecond.
#[test]
fn a_tie_does_not_hide_a_failure() {
    let status = state_of(&[
        event("s", SourceEventKind::Polled, "2026-09-01T00:00:00Z", ""),
        event("s", SourceEventKind::Failed, "2026-09-01T00:00:00Z", "hung"),
    ]);
    assert!(matches!(status.state, SourceState::Failing { .. }));
}

/// One derivation per source: one source's events never colour another's
/// state.
#[test]
fn statuses_are_per_source() {
    let map = statuses(&[
        event(
            "healthy",
            SourceEventKind::Polled,
            "2026-09-01T00:00:00Z",
            "",
        ),
        event(
            "broken",
            SourceEventKind::Failed,
            "2026-09-01T00:00:00Z",
            "no",
        ),
    ]);
    assert_eq!(map.len(), 2);
    assert!(matches!(
        map[&SourceId::new("healthy")].state,
        SourceState::Watching { .. }
    ));
    assert!(matches!(
        map[&SourceId::new("broken")].state,
        SourceState::Failing { .. }
    ));
}

// --- No secret can be stored (ADR 0030 §4) ---

/// The serialized form of a source carries the credential profile's
/// *name* and nothing that could be a value. This is the milestone's
/// standing invariant, asserted on the type rather than on a code path:
/// there is no field to leak through.
#[test]
fn a_source_serializes_a_credential_name_and_never_a_value() {
    let mut with_profile = source("s3-imagery");
    with_profile.credential_profile = Some("imagery-reader".to_owned());
    let json = serde_json::to_value(&with_profile).expect("serializable");

    assert_eq!(json["credentialProfile"], serde_json::Value::Null);
    assert_eq!(json["credential_profile"], "imagery-reader");
    let text = json.to_string();
    for forbidden in ["secret", "token", "password", "access_key", "AKIA"] {
        assert!(!text.contains(forbidden), "{forbidden} in {text}");
    }

    // And a source without one omits the field entirely, rather than
    // serving an empty string that reads like a cleared secret.
    let json = serde_json::to_value(source("plain")).expect("serializable");
    assert!(json.get("credential_profile").is_none());
}

/// A round trip through the wire form is lossless, so a persisted source
/// is the source that was stored.
#[test]
fn a_source_round_trips_through_its_wire_form() {
    let mut original = source("s");
    original.kind = SourceKind::Stac;
    original.credential_profile = Some("reader".to_owned());
    original.bindings = vec![DatasetId::new("a"), DatasetId::new("b")];
    let json = serde_json::to_string(&original).expect("serializable");
    let back: Source = serde_json::from_str(&json).expect("deserializable");
    assert_eq!(back, original);
    assert_eq!(back.kind.as_str(), "stac");
}

// --- The store's contract, including what deletion means ---

/// An in-memory [`SourceStore`]: the reference implementation the API and
/// the adapters are checked against.
#[derive(Default)]
struct MemoryStore {
    sources: Mutex<BTreeMap<SourceId, Source>>,
    events: Mutex<Vec<SourceEvent>>,
}

impl SourceStore for MemoryStore {
    async fn upsert_source(&self, source: &Source) -> Result<(), SourceStoreError> {
        self.sources
            .lock()
            .expect("lock")
            .insert(source.id.clone(), source.clone());
        Ok(())
    }

    async fn get_source(&self, id: &SourceId) -> Result<Option<Source>, SourceStoreError> {
        Ok(self.sources.lock().expect("lock").get(id).cloned())
    }

    async fn list_sources(&self) -> Result<Vec<Source>, SourceStoreError> {
        Ok(self
            .sources
            .lock()
            .expect("lock")
            .values()
            .cloned()
            .collect())
    }

    async fn delete_source(&self, id: &SourceId) -> Result<(), SourceStoreError> {
        let removed = self.sources.lock().expect("lock").remove(id);
        if removed.is_none() {
            return Err(SourceStoreError::NotFound { id: id.clone() });
        }
        // The origin and its history go. The granules do not: this store
        // has no way to touch them, which is the point.
        self.events
            .lock()
            .expect("lock")
            .retain(|event| &event.source != id);
        Ok(())
    }

    async fn record_event(&self, event: &SourceEvent) -> Result<(), SourceStoreError> {
        self.events.lock().expect("lock").push(event.clone());
        Ok(())
    }

    async fn events(&self, id: &SourceId) -> Result<Vec<SourceEvent>, SourceStoreError> {
        Ok(self
            .events
            .lock()
            .expect("lock")
            .iter()
            .filter(|event| &event.source == id)
            .cloned()
            .collect())
    }
}

#[test]
fn the_store_round_trips_sources_and_their_events() {
    let store = MemoryStore::default();
    block_on(store.upsert_source(&source("a"))).unwrap();
    block_on(store.upsert_source(&source("b"))).unwrap();
    assert_eq!(block_on(store.list_sources()).unwrap().len(), 2);
    assert_eq!(
        block_on(store.get_source(&SourceId::new("a"))).unwrap(),
        Some(source("a"))
    );
    assert!(
        block_on(store.get_source(&SourceId::new("nope")))
            .unwrap()
            .is_none()
    );

    block_on(store.record_event(&event(
        "a",
        SourceEventKind::Started,
        "2026-09-01T00:00:00Z",
        "",
    )))
    .unwrap();
    block_on(store.record_event(&event(
        "b",
        SourceEventKind::Failed,
        "2026-09-01T00:00:00Z",
        "x",
    )))
    .unwrap();
    // Each source's events are its own.
    assert_eq!(
        block_on(store.events(&SourceId::new("a"))).unwrap().len(),
        1
    );
}

/// Deleting a source removes the origin and its history, and the store
/// has no reach into the data at all (ADR 0030 §3).
#[test]
fn deleting_a_source_takes_its_events_and_nothing_else() {
    let store = MemoryStore::default();
    block_on(store.upsert_source(&source("a"))).unwrap();
    block_on(store.upsert_source(&source("b"))).unwrap();
    for id in ["a", "b"] {
        block_on(store.record_event(&event(
            id,
            SourceEventKind::Ingested,
            "2026-09-01T00:00:00Z",
            "g",
        )))
        .unwrap();
    }

    block_on(store.delete_source(&SourceId::new("a"))).unwrap();
    assert!(
        block_on(store.get_source(&SourceId::new("a")))
            .unwrap()
            .is_none()
    );
    assert!(
        block_on(store.events(&SourceId::new("a")))
            .unwrap()
            .is_empty()
    );
    // The other source is untouched, and so is everything the deleted
    // source ingested — nothing here can reach a granule.
    assert_eq!(
        block_on(store.events(&SourceId::new("b"))).unwrap().len(),
        1
    );

    assert_eq!(
        block_on(store.delete_source(&SourceId::new("a"))),
        Err(SourceStoreError::NotFound {
            id: SourceId::new("a")
        })
    );
}
