// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The sources resource (issue #417, ADR 0030): `GET /sources` and
//! `GET /sources/{sourceId}` — what each origin is and how it is doing.
//!
//! **Read-only, and the mutating routes are absent rather than
//! forbidden** (ADR 0031). A route that can add an origin is a route that
//! can point the server somewhere and spend the operator's credentials,
//! and this server does not yet know who is asking. There is therefore no
//! handler to authorise — a stronger guarantee than a 403, because a
//! middleware mistake cannot undo it, and one the test suite asserts.
//!
//! ADR 0031 records what lifts the interlock: OIDC, an RBAC role that may
//! manage sources as distinct from reading them, and an audit trail.
//!
//! # Everything here is measured
//!
//! The status is derived from the recorded event log
//! ([`swath_core::sources::state_of`]) — there is no stored health field
//! to go stale. Reachability is the same derivation read one way: a
//! source whose most recent event is a failure is not reachable, one that
//! has reported since is, and one that has never reported says `null`
//! rather than claiming either. The probe that produces those events, and
//! its timeout, live in the serving binary; this module reports what it
//! recorded.
//!
//! # What is deliberately absent
//!
//! - **The target path.** A filedrop source watches a directory on the
//!   serving host; the response carries its **scheme** (`file`, `s3`,
//!   `https`), never the path — the same rule the granules route follows
//!   for asset hrefs, for the same reason.
//! - **Any secret.** A source names a credential profile the operator
//!   provisions (ADR 0030 §4); the response carries that **name** and
//!   there is no field a value could occupy. A test reads the serialized
//!   shape and says so.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::routing::get;
use swath_core::sources::{
    Consent, EgressPolicy, RegisterEntry, Source, SourceId, SourceState, SourceStatus, SourceStore,
    SourceStoreError, consent_of, credential_resolution, state_of,
};

use crate::error::ApiError;
use crate::model::Link;

/// Everything the sources handlers need.
#[derive(Debug)]
pub struct SourcesState<S> {
    store: S,
    base_url: String,
    register: Vec<RegisterEntry>,
    egress: EgressPolicy,
}

impl<S> SourcesState<S> {
    /// Wires the surface over `store`.
    pub fn new(store: S, base_url: impl Into<String>) -> Self {
        let mut base_url: String = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            store,
            base_url,
            register: Vec::new(),
            egress: EgressPolicy::default(),
        }
    }

    /// The register this deployment offers, and the policy that says
    /// which of its entries are reachable (#420). Absent, the register is
    /// empty and federation reads as off — the default.
    #[must_use]
    pub fn with_register(mut self, register: Vec<RegisterEntry>, egress: EgressPolicy) -> Self {
        self.register = register;
        self.egress = egress;
        self
    }
}

/// The read-only sources router over `state`.
pub fn sources_router<S>(state: Arc<SourcesState<S>>) -> axum::Router
where
    S: SourceStore + 'static,
{
    axum::Router::new()
        .route("/sources", get(list_sources))
        .route("/sources/{sourceId}", get(one_source))
        .route("/sources/register", get(register))
        .with_state(state)
}

// --- The response shape (contractual) ---

/// One source, as the API serves it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SourceItem {
    /// Identifier, unique within the deployment.
    pub id: String,
    /// Human title.
    pub title: String,
    /// `"filedrop"` or `"stac"`.
    pub kind: &'static str,
    /// The target's scheme — `file`, `s3`, `https`. **Never the path**:
    /// a filedrop source watches a directory on the serving host, and
    /// host paths do not leave this process.
    pub scheme: String,
    /// `"config"` (declared in the deployment's configuration, and so not
    /// editable here) or `"api"`. Explicit, so an operator can see which
    /// sources their config owns.
    pub origin: &'static str,
    /// The datasets this source feeds, in id order.
    pub datasets: Vec<String>,
    /// The **name** of the credential profile the operator provisions.
    /// Omitted when there is none, and never a value.
    #[serde(rename = "credentialProfile", skip_serializing_if = "Option::is_none")]
    pub credential_profile: Option<String>,
    /// Whether that profile resolved the last time anything checked
    /// (#423). `null` until something has — an unchecked credential is
    /// not a working one, and it is not a broken one either. Omitted
    /// entirely for a source that names no profile: there is nothing to
    /// resolve, so there is nothing to report.
    #[serde(rename = "credentialResolved", skip_serializing_if = "Option::is_none")]
    pub credential_resolved: Option<Option<bool>>,
    /// Whether reading this source bills the reader (#424). Omitted when
    /// it does not, so an ordinary source's row is unchanged.
    #[serde(rename = "requesterPays", skip_serializing_if = "core::ops::Not::not")]
    pub requester_pays: bool,
    /// Who consented to being billed, and when — absent when nobody has.
    /// A requester-pays source with no consent here is not read.
    #[serde(rename = "consentedBy", skip_serializing_if = "Option::is_none")]
    pub consented_by: Option<String>,
    /// What the source is doing, derived from its events.
    pub status: SourceStatusItem,
}

/// A source's measured status.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SourceStatusItem {
    /// `"unknown"`, `"watching"`, `"failing"` or `"stopped"`.
    pub state: &'static str,
    /// Whether the source answered the last time anything looked.
    /// `null` when nothing has looked yet — the UI renders an em dash,
    /// not a reassuring default.
    pub reachable: Option<bool>,
    /// The most recent event of any kind (RFC 3339 UTC), or absent.
    #[serde(rename = "lastEvent", skip_serializing_if = "Option::is_none")]
    pub last_event: Option<String>,
    /// The most recent failure's own words, while it is still the last
    /// word. Absent once the source has reported since.
    #[serde(rename = "lastError", skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Granules ingested through this source, all time in this process.
    pub ingested: usize,
    /// Failures recorded, all time in this process.
    pub failures: usize,
}

/// The sources listing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SourceList {
    /// Every source, in id order.
    pub sources: Vec<SourceItem>,
    /// `self`.
    pub links: Vec<Link>,
}

/// One offered endpoint, with whether this deployment may actually reach
/// it (#420).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RegisterItem {
    /// Stable identifier.
    pub id: String,
    /// What to call it on screen.
    pub title: String,
    /// The catalog's URL.
    pub url: String,
    /// Its host, when the URL has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Whether reading it bills the reader.
    #[serde(rename = "requesterPays", skip_serializing_if = "core::ops::Not::not")]
    pub requester_pays: bool,
    /// Whether this deployment's egress allowlist permits its host. An
    /// entry that is offered but not allowed is still listed, so the UI
    /// can say **why** it cannot be used rather than hiding it and
    /// leaving an operator to guess.
    pub allowed: bool,
}

/// The register: what an operator can import from in one action.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RegisterList {
    /// The offered endpoints, in the order the config declares them.
    pub register: Vec<RegisterItem>,
    /// True when the deployment's egress allowlist is empty, so nothing
    /// in the register is reachable yet — the UI says that once rather
    /// than repeating it per row.
    #[serde(rename = "federationOff")]
    pub federation_off: bool,
    /// `self`.
    pub links: Vec<Link>,
}

/// `GET /sources/register` — the public endpoints this deployment offers,
/// each marked with whether its host is on the egress allowlist (#420).
///
/// The register is data: it comes from the deployment's configuration, so
/// adding an endpoint is an edit and a restart rather than a release.
/// Nothing is fetched here — an entry is an offer, and the fetch is a
/// separate operator action.
async fn register<S>(State(app): State<Arc<SourcesState<S>>>) -> Json<RegisterList>
where
    S: SourceStore + 'static,
{
    let items = app
        .register
        .iter()
        .map(|entry| {
            let host = entry.host().map(str::to_owned);
            RegisterItem {
                id: entry.id.clone(),
                title: entry.title.clone(),
                url: entry.url.clone(),
                allowed: host.as_deref().is_some_and(|host| app.egress.allows(host)),
                host,
                requester_pays: entry.requester_pays,
            }
        })
        .collect();
    Json(RegisterList {
        register: items,
        federation_off: app.egress.is_empty(),
        links: vec![
            Link::new(format!("{}/sources/register", app.base_url), "self")
                .media_type("application/json")
                .title("Source register"),
        ],
    })
}

/// The scheme of `target`: the part before `://`, or `file` for a bare
/// path. Never more than that — the rest is a host path.
fn scheme_of(target: &str) -> String {
    target
        .split_once("://")
        .map_or_else(|| "file".to_owned(), |(scheme, _)| scheme.to_owned())
}

fn item(
    source: Source,
    status: &SourceStatus,
    credential: Option<bool>,
    consent: Option<Consent>,
) -> SourceItem {
    let credential_resolved = source.credential_profile.as_ref().map(|_| credential);
    SourceItem {
        id: source.id.to_string(),
        title: source.title,
        kind: source.kind.as_str(),
        scheme: scheme_of(&source.target),
        origin: match source.origin {
            swath_core::sources::SourceOrigin::Config => "config",
            swath_core::sources::SourceOrigin::Api => "api",
        },
        datasets: source.bindings.iter().map(ToString::to_string).collect(),
        credential_profile: source.credential_profile,
        credential_resolved,
        requester_pays: source.requester_pays,
        consented_by: consent.map(|consent| consent.by),
        status: status_item(status),
    }
}

fn status_item(status: &SourceStatus) -> SourceStatusItem {
    // Reachability is the derived state read one way. A source that has
    // never reported claims neither answer.
    let (state, reachable, last_error) = match &status.state {
        SourceState::Unknown => ("unknown", None, None),
        SourceState::Watching { .. } => ("watching", Some(true), None),
        SourceState::Failing { detail, .. } => ("failing", Some(false), Some(detail.clone())),
        SourceState::Stopped { .. } => ("stopped", Some(false), None),
    };
    SourceStatusItem {
        state,
        reachable,
        last_event: status.last_event.as_ref().map(ToString::to_string),
        last_error,
        ingested: status.ingested,
        failures: status.failures,
    }
}

/// `GET /sources` — every source with its measured status.
async fn list_sources<S>(
    State(app): State<Arc<SourcesState<S>>>,
) -> Result<Json<SourceList>, ApiError>
where
    S: SourceStore + 'static,
{
    let mut sources = app.store.list_sources().await.map_err(store_error)?;
    sources.sort_by(|a, b| a.id.cmp(&b.id));
    let mut items = Vec::with_capacity(sources.len());
    for source in sources {
        let events = app.store.events(&source.id).await.map_err(store_error)?;
        let status = state_of(&events);
        let credential = credential_resolution(&events);
        let consent = consent_of(&events);
        items.push(item(source, &status, credential, consent));
    }
    Ok(Json(SourceList {
        sources: items,
        links: vec![
            Link::new(format!("{}/sources", app.base_url), "self")
                .media_type("application/json")
                .title("Sources"),
        ],
    }))
}

/// `GET /sources/{sourceId}` — one source. Unknown id → 404, the same
/// taxonomy every read route in this crate uses.
async fn one_source<S>(
    State(app): State<Arc<SourcesState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<SourceItem>, ApiError>
where
    S: SourceStore + 'static,
{
    let source_id = SourceId::new(&id);
    let source = app
        .store
        .get_source(&source_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ApiError::not_found(format!("no source `{id}`")))?;
    let events = app.store.events(&source_id).await.map_err(store_error)?;
    let status = state_of(&events);
    let credential = credential_resolution(&events);
    let consent = consent_of(&events);
    Ok(Json(item(source, &status, credential, consent)))
}

/// Store failures, translated as every other backend failure is.
fn store_error(err: SourceStoreError) -> ApiError {
    match err {
        SourceStoreError::NotFound { id } => ApiError::not_found(format!("no source `{id}`")),
        other => ApiError::internal(format!("source store failed: {other}")),
    }
}
