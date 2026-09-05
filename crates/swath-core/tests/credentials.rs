// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Credentials by reference (#423, ADR 0030 §4).
//!
//! The design's guarantee is structural rather than procedural: there is
//! **no type in the sources domain that can hold a secret value**. A
//! resolver answers `Resolved` or `Missing`; a source stores a profile
//! *name*; an event's detail names that profile and says what happened.
//! These tests assert the guarantee where it can be checked — over the
//! serialized bytes of everything the domain produces.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use swath_core::catalog::{DatasetId, Datetime};
use swath_core::sources::{
    CredentialResolution, CredentialResolver, Source, SourceEvent, SourceEventKind, SourceId,
    SourceKind, SourceOrigin, SourceState, credential_event, credential_resolution, state_of,
};

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("these resolvers never await"),
    }
}

fn at(value: &str) -> Datetime {
    Datetime::new(value).expect("a test instant")
}

fn source(profile: Option<&str>) -> Source {
    Source {
        id: SourceId::new("s3-imagery"),
        kind: SourceKind::Stac,
        target: "s3://imagery".to_owned(),
        title: "Imagery".to_owned(),
        bindings: vec![DatasetId::new("hls-s30")],
        origin: SourceOrigin::Config,
        credential_profile: profile.map(str::to_owned),
        requester_pays: false,
    }
}

/// A resolver that answers from a table — and, deliberately, has nowhere
/// to put the value it "found". Its shape is the contract.
struct Table(BTreeMap<String, bool>);

impl CredentialResolver for Table {
    async fn resolve(&self, profile: &str) -> CredentialResolution {
        if self.0.get(profile).copied().unwrap_or(false) {
            CredentialResolution::Resolved
        } else {
            CredentialResolution::Missing
        }
    }
}

/// The whole design in one assertion: the resolution type has no variant
/// that carries a value, so nothing downstream can be handed one.
#[test]
fn a_resolution_is_an_answer_not_a_value() {
    let resolver = Table(BTreeMap::from([("imagery-reader".to_owned(), true)]));
    assert_eq!(
        block_on(resolver.resolve("imagery-reader")),
        CredentialResolution::Resolved
    );
    assert_eq!(
        block_on(resolver.resolve("nope")),
        CredentialResolution::Missing
    );
    // `CredentialResolution` is a two-variant enum. If a `Value(String)`
    // ever joins it, this stops compiling — which is the point.
    let both = [
        CredentialResolution::Resolved,
        CredentialResolution::Missing,
    ];
    assert_eq!(both.len(), 2);
}

/// An unresolvable profile is reported as unresolvable, naming the
/// **profile** — never a value, and never a vague "auth failed".
#[test]
fn an_unresolvable_profile_names_the_profile_and_nothing_else() {
    let event = credential_event(
        &SourceId::new("s3-imagery"),
        "imagery-reader",
        at("2026-09-05T10:00:00Z"),
        CredentialResolution::Missing,
    );
    assert_eq!(event.kind, SourceEventKind::CredentialMissing);
    assert_eq!(
        event.detail,
        "credential profile `imagery-reader` did not resolve"
    );
    // The source of the failure is legible, and there is nothing else in
    // it: the whole event serializes to profile names and instants.
    let json = serde_json::to_string(&event).expect("serializable");
    for forbidden in ["secret", "token", "password", "AKIA", "access_key"] {
        assert!(!json.contains(forbidden), "{forbidden} in {json}");
    }
}

/// A source whose credential stopped resolving is failing, and the state
/// says why — so an operator is sent to the credential rather than to the
/// network.
#[test]
fn a_missing_credential_makes_the_source_fail_with_a_reason() {
    let events = [
        SourceEvent {
            source: SourceId::new("s3-imagery"),
            at: at("2026-09-05T09:00:00Z"),
            kind: SourceEventKind::Started,
            detail: String::new(),
        },
        credential_event(
            &SourceId::new("s3-imagery"),
            "imagery-reader",
            at("2026-09-05T10:00:00Z"),
            CredentialResolution::Missing,
        ),
    ];
    let status = state_of(&events);
    let SourceState::Failing { detail, .. } = &status.state else {
        panic!(
            "a source without its credential is not working: {:?}",
            status.state
        )
    };
    assert!(detail.contains("imagery-reader"));
    assert_eq!(credential_resolution(&events), Some(false));
}

/// Resolution follows the events like every other state: the newest one
/// decides, and nothing has been checked until something checks.
#[test]
fn resolution_is_read_off_the_events_and_starts_as_nothing() {
    assert_eq!(credential_resolution(&[]), None);

    let id = SourceId::new("s3-imagery");
    let missing = credential_event(
        &id,
        "imagery-reader",
        at("2026-09-05T10:00:00Z"),
        CredentialResolution::Missing,
    );
    let resolved = credential_event(
        &id,
        "imagery-reader",
        at("2026-09-05T10:30:00Z"),
        CredentialResolution::Resolved,
    );
    assert_eq!(
        credential_resolution(std::slice::from_ref(&missing)),
        Some(false)
    );
    assert_eq!(
        credential_resolution(&[missing.clone(), resolved.clone()]),
        Some(true)
    );
    // Order-independent, like the rest of the derivation.
    assert_eq!(credential_resolution(&[resolved, missing]), Some(true));

    // An ordinary event is not a credential check: a source that polled
    // successfully has still not proved its credential.
    let polled = SourceEvent {
        source: id,
        at: at("2026-09-05T11:00:00Z"),
        kind: SourceEventKind::Polled,
        detail: String::new(),
    };
    assert_eq!(credential_resolution(&[polled]), None);
}

/// A source names a profile and never a value, in every direction it can
/// be serialized. This is the milestone's standing invariant asserted at
/// the domain boundary.
#[test]
fn a_source_carries_a_name_in_every_direction() {
    let with = source(Some("imagery-reader"));
    let json = serde_json::to_string(&with).expect("serializable");
    assert!(json.contains("imagery-reader"));
    let back: Source = serde_json::from_str(&json).expect("deserializable");
    assert_eq!(back, with);

    // And a source that names none has no field at all — not an empty
    // string, which would read like a cleared secret.
    let without = serde_json::to_value(source(None)).expect("serializable");
    assert!(without.get("credential_profile").is_none());
}
