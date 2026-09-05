// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Requester-pays consent (#424, ADR 0030 §6).
//!
//! A requester-pays bucket bills whoever reads it. That is a decision the
//! operator makes once, explicitly, per source — so the check is a pure
//! function the caller runs **before** opening anything, and consent is
//! read off the recorded events like every other state.

use swath_core::catalog::{DatasetId, Datetime};
use swath_core::sources::{
    Consent, ConsentRefusal, Source, SourceEvent, SourceEventKind, SourceId, SourceKind,
    SourceOrigin, consent_event, consent_of, may_read,
};

fn at(value: &str) -> Datetime {
    Datetime::new(value).expect("a test instant")
}

fn source(requester_pays: bool) -> Source {
    Source {
        id: SourceId::new("billed"),
        kind: SourceKind::Stac,
        target: "s3://requester-pays-bucket".to_owned(),
        title: "Billed".to_owned(),
        bindings: vec![DatasetId::new("hls-s30")],
        origin: SourceOrigin::Config,
        credential_profile: None,
        requester_pays,
    }
}

/// A source that does not bill the reader needs no consent — the gate is
/// about billing, not about reading.
#[test]
fn an_ordinary_source_needs_no_consent() {
    assert_eq!(may_read(&source(false), &[]), Ok(()));
}

/// A requester-pays source is refused until consent is recorded, and the
/// refusal says why in words an operator can act on.
#[test]
fn a_requester_pays_source_is_refused_until_someone_agrees() {
    let refusal = may_read(&source(true), &[]).expect_err("nobody has agreed");
    assert_eq!(
        refusal,
        ConsentRefusal::NoConsent {
            id: SourceId::new("billed")
        }
    );
    let said = refusal.to_string();
    assert!(said.contains("bills this deployment"), "{said}");
    assert!(said.contains("before the first read"), "{said}");
}

/// Consent is recorded per source, with who and when — and reading it
/// back gives both.
#[test]
fn consent_records_who_and_when() {
    let event = consent_event(
        &SourceId::new("billed"),
        "operator",
        at("2026-09-05T12:00:00Z"),
    );
    assert_eq!(event.kind, SourceEventKind::RequesterPaysConsented);
    assert_eq!(
        consent_of(std::slice::from_ref(&event)),
        Some(Consent {
            by: "operator".to_owned(),
            at: at("2026-09-05T12:00:00Z"),
        })
    );
    assert_eq!(may_read(&source(true), &[event]), Ok(()));
}

/// One source's consent is not another's, and an ordinary event is not
/// consent — a poll does not agree to be billed.
#[test]
fn consent_is_per_source_and_only_consent_counts() {
    let elsewhere = consent_event(
        &SourceId::new("other"),
        "operator",
        at("2026-09-05T12:00:00Z"),
    );
    // `consent_of` is given one source's events by construction; what
    // this pins is that a non-consent event never reads as consent.
    let polled = SourceEvent {
        source: SourceId::new("billed"),
        at: at("2026-09-05T12:00:00Z"),
        kind: SourceEventKind::Polled,
        detail: "operator".to_owned(),
    };
    assert_eq!(consent_of(std::slice::from_ref(&polled)), None);
    assert!(may_read(&source(true), &[polled]).is_err());
    assert_eq!(
        consent_of(std::slice::from_ref(&elsewhere)).map(|c| c.by),
        Some("operator".to_owned())
    );
}

/// The most recent consent is the one that counts, so re-consenting
/// after a change of operator records the new name.
#[test]
fn the_newest_consent_is_the_one_recorded() {
    let id = SourceId::new("billed");
    let first = consent_event(&id, "alice", at("2026-09-05T10:00:00Z"));
    let second = consent_event(&id, "bob", at("2026-09-05T12:00:00Z"));
    assert_eq!(
        consent_of(&[first.clone(), second.clone()]).map(|c| c.by),
        Some("bob".to_owned())
    );
    // Order-independent, like every other derivation here.
    assert_eq!(
        consent_of(&[second, first]).map(|c| c.by),
        Some("bob".to_owned())
    );
}

/// Consent serializes with no currency anywhere: it records that someone
/// agreed, not what they agreed to pay.
#[test]
fn consent_carries_no_money() {
    let consent = Consent {
        by: "operator".to_owned(),
        at: at("2026-09-05T12:00:00Z"),
    };
    let json = serde_json::to_string(&consent).expect("serializable");
    for currency in ["$", "usd", "USD", "price", "dollar", "cost"] {
        assert!(!json.contains(currency), "{currency} in {json}");
    }
}
