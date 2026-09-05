// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The read-only sources resource (#417, ADR 0030).
//!
//! Three properties carry it, and each has a test that would fail if the
//! property broke: **origin is explicit**, so a config-owned source
//! cannot look editable; **every state is measured**, derived from the
//! event log with `null` where nothing has looked yet; and **nothing in
//! the response can carry a secret or a host path** — asserted by reading
//! the serialized bytes, not by reviewing the struct.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::StatusCode;
use swath_api::{SourcesState, sources_router};
use swath_core::catalog::{DatasetId, Datetime};
use swath_core::sources::{
    Source, SourceEvent, SourceEventKind, SourceId, SourceKind, SourceOrigin, SourceStore,
    SourceStoreError,
};

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
        self.sources.lock().expect("lock").remove(id);
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

fn source(id: &str, origin: SourceOrigin, target: &str) -> Source {
    Source {
        id: SourceId::new(id),
        kind: SourceKind::Filedrop,
        target: target.to_owned(),
        title: id.to_owned(),
        bindings: vec![DatasetId::new("hls-s30")],
        origin,
        credential_profile: None,
    }
}

fn event(id: &str, kind: SourceEventKind, at: &str, detail: &str) -> SourceEvent {
    SourceEvent {
        source: SourceId::new(id),
        at: Datetime::new(at).expect("a test instant"),
        kind,
        detail: detail.to_owned(),
    }
}

async fn app(sources: Vec<Source>, events: Vec<SourceEvent>) -> Router {
    let store = MemoryStore::default();
    for source in &sources {
        store.upsert_source(source).await.expect("seed");
    }
    for event in &events {
        store.record_event(event).await.expect("seed");
    }
    sources_router(Arc::new(SourcesState::new(store, common::BASE_URL)))
}

async fn get_json(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
    let response = common::request_on(app, "GET", path, None).await;
    let status = response.status();
    (status, common::body_json(response).await)
}

/// A config-owned source says so, so an operator can see at a glance
/// which sources their file owns — and a UI cannot offer to edit one it
/// cannot change.
#[tokio::test]
async fn origin_is_explicit() {
    let app = app(
        vec![
            source("declared", SourceOrigin::Config, "/srv/incoming"),
            source("created", SourceOrigin::Api, "s3://bucket/incoming"),
        ],
        Vec::new(),
    )
    .await;
    let (status, body) = get_json(&app, "/sources").await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["sources"].as_array().expect("sources");
    assert_eq!(rows[0]["id"], "created");
    assert_eq!(rows[0]["origin"], "api");
    assert_eq!(rows[1]["id"], "declared");
    assert_eq!(rows[1]["origin"], "config");
}

/// Every state is read off the events. Nothing has looked at a fresh
/// source, so `reachable` is `null` — not `false`, which would be a
/// claim, and not `true`, which would be a lie.
#[tokio::test]
async fn a_source_nothing_has_looked_at_claims_nothing() {
    let app = app(
        vec![source("fresh", SourceOrigin::Config, "/srv/incoming")],
        Vec::new(),
    )
    .await;
    let (_, body) = get_json(&app, "/sources/fresh").await;
    assert_eq!(body["status"]["state"], "unknown");
    assert_eq!(body["status"]["reachable"], serde_json::Value::Null);
    assert!(body["status"].get("lastEvent").is_none());
    assert!(body["status"].get("lastError").is_none());
    assert_eq!(body["status"]["ingested"], 0);
}

/// A watching source is reachable, a failing one is not, and the failure
/// carries the origin's own words. Recovery clears the error rather than
/// leaving it to look current.
#[tokio::test]
async fn reachability_and_the_last_error_follow_the_events() {
    let app = app(
        vec![
            source("healthy", SourceOrigin::Config, "/srv/a"),
            source("broken", SourceOrigin::Config, "/srv/b"),
            source("recovered", SourceOrigin::Config, "/srv/c"),
        ],
        vec![
            event(
                "healthy",
                SourceEventKind::Polled,
                "2026-09-04T10:00:00Z",
                "",
            ),
            event(
                "healthy",
                SourceEventKind::Ingested,
                "2026-09-04T10:01:00Z",
                "g",
            ),
            event(
                "broken",
                SourceEventKind::Failed,
                "2026-09-04T10:00:00Z",
                "permission denied",
            ),
            event(
                "recovered",
                SourceEventKind::Failed,
                "2026-09-04T10:00:00Z",
                "gone",
            ),
            event(
                "recovered",
                SourceEventKind::Polled,
                "2026-09-04T10:05:00Z",
                "",
            ),
        ],
    )
    .await;
    let (_, body) = get_json(&app, "/sources").await;
    let by_id: BTreeMap<&str, &serde_json::Value> = body["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|row| (row["id"].as_str().expect("id"), row))
        .collect();

    assert_eq!(by_id["healthy"]["status"]["state"], "watching");
    assert_eq!(by_id["healthy"]["status"]["reachable"], true);
    assert_eq!(
        by_id["healthy"]["status"]["lastEvent"],
        "2026-09-04T10:01:00Z"
    );
    assert_eq!(by_id["healthy"]["status"]["ingested"], 1);

    assert_eq!(by_id["broken"]["status"]["state"], "failing");
    assert_eq!(by_id["broken"]["status"]["reachable"], false);
    assert_eq!(by_id["broken"]["status"]["lastError"], "permission denied");

    // Recovered: reachable again, the stale error gone, the failure still
    // counted.
    assert_eq!(by_id["recovered"]["status"]["reachable"], true);
    assert!(by_id["recovered"]["status"].get("lastError").is_none());
    assert_eq!(by_id["recovered"]["status"]["failures"], 1);
}

/// The response carries the target's **scheme** and never its path: a
/// filedrop source watches a directory on the serving host, and host
/// paths do not leave this process — the same rule the granules route
/// follows for asset hrefs.
#[tokio::test]
async fn the_response_carries_a_scheme_and_never_a_host_path() {
    let app = app(
        vec![
            source("local", SourceOrigin::Config, "/srv/secret-mount/incoming"),
            source("remote", SourceOrigin::Api, "s3://bucket/incoming"),
        ],
        Vec::new(),
    )
    .await;
    let (_, body) = get_json(&app, "/sources").await;
    let text = body.to_string();
    assert!(!text.contains("/srv/"), "no host path in {text}");
    assert!(!text.contains("secret-mount"), "no host path in {text}");
    assert!(!text.contains("bucket"), "not even a bucket name in {text}");
    let rows = body["sources"].as_array().expect("sources");
    assert_eq!(rows[0]["scheme"], "file");
    assert_eq!(rows[1]["scheme"], "s3");
}

/// The milestone's standing invariant, asserted over the serialized
/// bytes: the response carries a credential profile's **name** and there
/// is no field a value could occupy.
#[tokio::test]
async fn nothing_in_the_response_can_carry_a_secret() {
    let mut credentialed = source("s3-imagery", SourceOrigin::Config, "s3://imagery");
    credentialed.credential_profile = Some("imagery-reader".to_owned());
    let app = app(
        vec![
            credentialed,
            source("plain", SourceOrigin::Config, "/srv/incoming"),
        ],
        Vec::new(),
    )
    .await;
    let (_, body) = get_json(&app, "/sources").await;
    let text = body.to_string();
    assert!(
        text.contains("imagery-reader"),
        "the profile name is served"
    );
    for forbidden in [
        "secret",
        "token",
        "password",
        "AKIA",
        "access_key",
        "accessKey",
        "credentials",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} in {text}");
    }
    // A source without a profile omits the field rather than serving an
    // empty string that reads like a cleared secret.
    let rows = body["sources"].as_array().expect("sources");
    let plain = rows
        .iter()
        .find(|row| row["id"] == "plain")
        .expect("the plain source");
    assert!(plain.get("credentialProfile").is_none());
}

/// The taxonomy every read route in this crate shares.
#[tokio::test]
async fn an_unknown_source_is_a_404() {
    let app = app(Vec::new(), Vec::new()).await;
    let (status, _) = get_json(&app, "/sources/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // An empty deployment is an empty list, not an error.
    let (status, body) = get_json(&app, "/sources").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sources"], serde_json::json!([]));
}

// --- The auth interlock (#421, ADR 0031) ---

/// The mutating routes are **absent, not forbidden**: there is no handler
/// to authorise, which cannot be defeated by a middleware
/// misconfiguration the way a 403 can.
///
/// This test is the interlock. A PR that adds `POST /sources` will fail
/// here, and the fix is not to change this test — it is ADR 0031's
/// lifting condition: OIDC, an RBAC role that may manage sources, and an
/// audit trail that records who did it.
#[tokio::test]
async fn the_mutating_routes_are_absent_until_the_auth_interlock_lifts() {
    let app = app(
        vec![source("declared", SourceOrigin::Config, "/srv/incoming")],
        Vec::new(),
    )
    .await;

    for (method, path) in [
        ("POST", "/sources"),
        ("PUT", "/sources/declared"),
        ("PATCH", "/sources/declared"),
        ("DELETE", "/sources/declared"),
        ("POST", "/sources/declared"),
    ] {
        let response = common::request_on(&app, method, path, Some(serde_json::json!({}))).await;
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path} must have no handler at all (ADR 0031); \
             a 4xx from a mounted route would mean one exists"
        );
    }

    // And the read routes are exactly the surface: reachable, unchanged.
    let (status, _) = get_json(&app, "/sources").await;
    assert_eq!(status, StatusCode::OK);
}

// --- Credentials by reference (#423, ADR 0030 §4) ---

/// The credential's *resolution* is served and its value is not — and
/// "unchecked" is a third answer, distinct from resolved and from
/// missing.
#[tokio::test]
async fn the_credential_resolution_is_served_and_the_value_is_not() {
    let mut credentialed = source("s3-imagery", SourceOrigin::Config, "s3://imagery");
    credentialed.credential_profile = Some("imagery-reader".to_owned());
    let mut unchecked = source("unchecked", SourceOrigin::Config, "s3://other");
    unchecked.credential_profile = Some("other-reader".to_owned());

    let app = app(
        vec![
            credentialed,
            unchecked,
            source("plain", SourceOrigin::Config, "/srv/incoming"),
        ],
        vec![event(
            "s3-imagery",
            SourceEventKind::CredentialMissing,
            "2026-09-05T10:00:00Z",
            "credential profile `imagery-reader` did not resolve",
        )],
    )
    .await;
    let (_, body) = get_json(&app, "/sources").await;
    let by_id: BTreeMap<&str, &serde_json::Value> = body["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|row| (row["id"].as_str().expect("id"), row))
        .collect();

    // Checked and missing: the source is failing, and the reason names
    // the profile so an operator is sent to the credential rather than to
    // the network.
    assert_eq!(by_id["s3-imagery"]["credentialResolved"], false);
    assert_eq!(by_id["s3-imagery"]["status"]["state"], "failing");
    assert_eq!(
        by_id["s3-imagery"]["status"]["lastError"],
        "credential profile `imagery-reader` did not resolve"
    );

    // Named but never checked: `null`, not `false`. An unchecked
    // credential is not a broken one.
    assert_eq!(
        by_id["unchecked"]["credentialResolved"],
        serde_json::Value::Null
    );

    // No profile: the field is absent entirely — there is nothing to
    // resolve, so there is nothing to report.
    assert!(by_id["plain"].get("credentialResolved").is_none());

    // And the standing invariant, over the bytes.
    let text = body.to_string();
    for forbidden in ["secret", "token", "password", "AKIA", "access_key"] {
        assert!(!text.contains(forbidden), "{forbidden} in {text}");
    }
}
