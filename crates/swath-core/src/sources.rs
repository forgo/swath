// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The sources domain (ADR 0030, issue #422): **where data came from**.
//!
//! A [`Source`] is an origin, not a transport. It has an id, a [`SourceKind`],
//! a target in that kind's own words, the datasets it feeds, and a state. It
//! owns no bytes, owns no granules, and is never in the read path of a tile —
//! deleting one removes the origin, not the data it produced.
//!
//! # State is derived, never stored
//!
//! [`SourceState`] is a function of the recorded [`SourceEvent`]s, computed by
//! [`state_of`]. Nothing sets a status field, because a field someone forgot
//! to update is exactly how "healthy" becomes a lie. A source with no events
//! is [`SourceState::Unknown`], which the UI renders as an em dash rather than
//! as a reassuring default.
//!
//! # No secrets here
//!
//! A source may name a credential *profile* the operator provisions in the
//! environment or an instance role (ADR 0030 §4, Wave B). This module models
//! the **name** and nothing else: there is no field a secret value could be
//! put in, which is what makes "no secret reaches the catalog" a property of
//! the type rather than a rule someone has to follow.
//!
//! Not to be confused with [`crate::source`], the `RasterSource` port: that is
//! how bytes are read, this is where they came from.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{DatasetId, Datetime};

/// A source identifier, unique within a deployment.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    /// Wraps `value` as an id.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for SourceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What sort of origin a source is. Non-exhaustive: a new kind is an
/// additive change, and a reader that does not know one must say so
/// rather than guess.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SourceKind {
    /// A directory the server watches for dropped granules — the ingest
    /// path that exists today.
    Filedrop,
    /// A STAC API or static catalog Swath reads on an operator's action
    /// (ADR 0030 §5; the fetch itself is Wave C).
    Stac,
}

impl SourceKind {
    /// The kind's stable wire name.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Filedrop => "filedrop",
            Self::Stac => "stac",
            // `non_exhaustive`: a kind added later names itself here.
        }
    }
}

/// An origin, and what it feeds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Source {
    /// Identifier, unique within the deployment.
    pub id: SourceId,
    /// What sort of origin this is.
    pub kind: SourceKind,
    /// Where it points, in the kind's own words: a directory path for
    /// `filedrop`, a catalog URL for `stac`. Opaque to this module.
    pub target: String,
    /// Human title; the id when the operator gave none.
    pub title: String,
    /// The datasets this source feeds, in id order. Empty means it has
    /// produced nothing yet, not that it feeds everything.
    pub bindings: Vec<DatasetId>,
    /// Where the source's definition came from — configuration or the
    /// API — so the UI can say whether editing it here will stick.
    pub origin: SourceOrigin,
    /// The **name** of a credential profile the operator provisions in
    /// the environment or an instance role (ADR 0030 §4). Never a secret
    /// value: there is no field here one could be put in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_profile: Option<String>,
    /// Whether reading this source bills the reader (a requester-pays
    /// bucket, #424). Declared by the operator, because only they know
    /// what their agreement with the provider says — Swath cannot detect
    /// it and will not guess.
    ///
    /// A source marked this way is not read until consent is recorded.
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    pub requester_pays: bool,
}

/// Where a source's definition lives — which is whether editing it
/// through the API can outlive a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    /// Declared in the deployment's configuration file.
    Config,
    /// Created through the API, and persisted.
    Api,
}

/// One thing that happened to a source. The event log is the only
/// authority on a source's state (ADR 0030 §2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceEvent {
    /// The source this happened to.
    pub source: SourceId,
    /// When (RFC 3339 UTC).
    pub at: Datetime,
    /// What happened.
    pub kind: SourceEventKind,
    /// The event's own words — a filename ingested, a refusal's detail.
    /// Never a credential value, and never a URL carrying one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// What kind of thing happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SourceEventKind {
    /// The source began watching or polling.
    Started,
    /// A granule arrived from this source.
    Ingested,
    /// The source is reachable and idle — a heartbeat, so "watching"
    /// is measured rather than assumed.
    Polled,
    /// Something went wrong; `detail` says what, in the origin's words.
    Failed,
    /// The source was deliberately stopped.
    Stopped,
    /// The source's credential profile resolved (ADR 0030 §4, #423).
    /// Recorded as an observation: `detail` names the **profile**, never
    /// a value, because nothing in this crate can hold one.
    CredentialResolved,
    /// The operator consented to being billed for reads of this source
    /// (#424). `detail` says who consented, as well as this deployment
    /// can know — see [`Consent::by`].
    RequesterPaysConsented,
    /// The credential profile did not resolve. `detail` names the profile
    /// and says so; a source in this state cannot reach its target, and
    /// the UI can say why without inventing a reason.
    CredentialMissing,
}

/// What a source is doing, derived from its events. Never stored.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SourceState {
    /// No events recorded. The UI says so with an em dash; it does not
    /// say "healthy".
    Unknown,
    /// Running, with the last event's instant.
    Watching {
        /// When the current run started.
        since: Datetime,
        /// The most recent event of any kind.
        last_event: Datetime,
    },
    /// The most recent event was a failure, and this is what it said.
    Failing {
        /// When the failure was recorded.
        since: Datetime,
        /// The origin's own words.
        detail: String,
    },
    /// Deliberately stopped.
    Stopped {
        /// When it was stopped.
        since: Datetime,
    },
}

/// A source's state, and the counts behind it. Everything here is read
/// off the event log; nothing is remembered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceStatus {
    /// What the source is doing.
    pub state: SourceState,
    /// Granules ingested through this source, all time.
    pub ingested: usize,
    /// Failures recorded, all time.
    pub failures: usize,
    /// The last event of any kind, when there has been one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event: Option<Datetime>,
}

/// The state `events` imply, newest event wins (ADR 0030 §2). The slice
/// need not be sorted: the derivation reads instants, not order.
///
/// A `Failed` that is followed by an `Ingested` or a `Polled` is over —
/// the source recovered, and saying otherwise would be as wrong as a
/// stale healthy field.
#[must_use]
pub fn state_of(events: &[SourceEvent]) -> SourceStatus {
    let mut ingested = 0;
    let mut failures = 0;
    let mut latest: Option<&SourceEvent> = None;
    let mut started: Option<&SourceEvent> = None;
    for event in events {
        match event.kind {
            SourceEventKind::Ingested => ingested += 1,
            SourceEventKind::Failed => failures += 1,
            SourceEventKind::Started if started.is_none_or(|current| newer(event, current)) => {
                started = Some(event);
            }
            _ => {}
        }
        if latest.is_none_or(|current| newer(event, current)) {
            latest = Some(event);
        }
    }

    let Some(last) = latest else {
        return SourceStatus {
            state: SourceState::Unknown,
            ingested: 0,
            failures: 0,
            last_event: None,
        };
    };
    // "Since" is the last start when there is one; a source that was
    // never started but has ingested something says so from its own
    // first event rather than claiming a start that is not recorded.
    let since = started.map_or_else(|| last.at.clone(), |event| event.at.clone());
    let state = match last.kind {
        SourceEventKind::Failed | SourceEventKind::CredentialMissing => SourceState::Failing {
            since: last.at.clone(),
            detail: last.detail.clone(),
        },
        SourceEventKind::Stopped => SourceState::Stopped {
            since: last.at.clone(),
        },
        _ => SourceState::Watching {
            since,
            last_event: last.at.clone(),
        },
    };
    SourceStatus {
        state,
        ingested,
        failures,
        last_event: Some(last.at.clone()),
    }
}

/// Whether the source's credential profile resolved the last time
/// anything checked (#423): `Some(true)`/`Some(false)`, or `None` when
/// nothing has checked — which the UI renders as an em dash rather than
/// as a reassuring default.
///
/// Like every other state here it is read off the events, so it cannot
/// drift from what was observed.
#[must_use]
pub fn credential_resolution(events: &[SourceEvent]) -> Option<bool> {
    let mut latest: Option<&SourceEvent> = None;
    for event in events {
        if !matches!(
            event.kind,
            SourceEventKind::CredentialResolved | SourceEventKind::CredentialMissing
        ) {
            continue;
        }
        if latest.is_none_or(|current| newer(event, current)) {
            latest = Some(event);
        }
    }
    latest.map(|event| event.kind == SourceEventKind::CredentialResolved)
}

/// A recorded consent to be billed for reading a source (#424).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Consent {
    /// Who consented, as well as this deployment can know.
    ///
    /// Today that is the operator identity available to the process that
    /// recorded it — the OS user running the command. It is **not** an
    /// authenticated identity, because there is no authentication yet
    /// (ADR 0031); when there is, this becomes the authenticated subject
    /// and the audit trail becomes a real one. Recording the weaker fact
    /// honestly is better than recording nothing and better than
    /// implying more than we know.
    pub by: String,
    /// When (RFC 3339 UTC).
    pub at: Datetime,
}

/// The consent recorded for a source, or `None` — read off the events
/// like every other state, so it cannot be set by someone forgetting to
/// unset it.
#[must_use]
pub fn consent_of(events: &[SourceEvent]) -> Option<Consent> {
    let mut latest: Option<&SourceEvent> = None;
    for event in events {
        if event.kind != SourceEventKind::RequesterPaysConsented {
            continue;
        }
        if latest.is_none_or(|current| newer(event, current)) {
            latest = Some(event);
        }
    }
    latest.map(|event| Consent {
        by: event.detail.clone(),
        at: event.at.clone(),
    })
}

/// The event a consent produces.
#[must_use]
pub fn consent_event(source: &SourceId, by: &str, at: Datetime) -> SourceEvent {
    SourceEvent {
        source: source.clone(),
        at,
        kind: SourceEventKind::RequesterPaysConsented,
        detail: by.to_owned(),
    }
}

/// Why a read was refused before it was attempted (#424).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConsentRefusal {
    /// The source bills the reader and nobody has agreed to that.
    #[error(
        "`{id}` is a requester-pays source and no consent is recorded:          reading it bills this deployment, so an operator agrees to that once,          explicitly, before the first read"
    )]
    NoConsent {
        /// The source that was not read.
        id: SourceId,
    },
}

/// Whether `source` may be read, given its recorded `events`.
///
/// A source that does not bill the reader is always readable. One that
/// does is readable only once consent is recorded — and this is a pure
/// check the caller makes **before** opening a connection, so a refused
/// read is not a read that failed, it is a read that never happened.
///
/// # Errors
///
/// [`ConsentRefusal::NoConsent`] when the source bills the reader and no
/// consent has been recorded.
pub fn may_read(source: &Source, events: &[SourceEvent]) -> Result<(), ConsentRefusal> {
    if !source.requester_pays || consent_of(events).is_some() {
        return Ok(());
    }
    Err(ConsentRefusal::NoConsent {
        id: source.id.clone(),
    })
}

/// What a credential profile lookup found. **There is no variant that
/// carries a value** — that absence is the whole design (ADR 0030 §4):
/// Swath stores and reports the profile's *name* and whether it
/// resolved, and the secret stays where the operator put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialResolution {
    /// The profile resolved: something answered to that name.
    Resolved,
    /// It did not. The caller reports the **profile**, never a value.
    Missing,
}

/// Resolves credential profiles by name (#423).
///
/// # Contract for implementors
///
/// An implementation may look anywhere the operator's platform puts
/// credentials — the environment, an instance role, a mounted file — but
/// it **must not return, log, store or trace the value it found**. The
/// return type gives it nowhere to put one; the rest is the implementor's
/// obligation, and the deployment's own audit is what checks it.
pub trait CredentialResolver: Send + Sync {
    /// Whether `profile` resolves right now.
    fn resolve(
        &self,
        profile: &str,
    ) -> impl core::future::Future<Output = CredentialResolution> + Send;
}

/// The event a resolution produces, ready to record. The detail names the
/// profile and says what happened — the one sentence a UI can show, and
/// the only place a profile name appears in the log.
#[must_use]
pub fn credential_event(
    source: &SourceId,
    profile: &str,
    at: Datetime,
    resolution: CredentialResolution,
) -> SourceEvent {
    let (kind, detail) = match resolution {
        CredentialResolution::Resolved => (
            SourceEventKind::CredentialResolved,
            format!("credential profile `{profile}` resolved"),
        ),
        CredentialResolution::Missing => (
            SourceEventKind::CredentialMissing,
            format!("credential profile `{profile}` did not resolve"),
        ),
    };
    SourceEvent {
        source: source.clone(),
        at,
        kind,
        detail,
    }
}

/// Later in time, ties broken so the comparison is total and the derived
/// state is a function of the set rather than of the iteration order.
fn newer(candidate: &SourceEvent, current: &SourceEvent) -> bool {
    (
        candidate.at.to_unix_millis(),
        rank(candidate.kind),
        &candidate.detail,
    )
        .gt(&(
            current.at.to_unix_millis(),
            rank(current.kind),
            &current.detail,
        ))
}

/// Tie-break order for events at the same instant: the more decisive
/// outcome wins, so a failure recorded in the same millisecond as a poll
/// is not hidden by it.
fn rank(kind: SourceEventKind) -> u8 {
    match kind {
        SourceEventKind::Started => 0,
        SourceEventKind::Ingested => 2,
        SourceEventKind::Stopped => 3,
        // As decisive as a failure: a source whose credential did not
        // resolve is not working, and a poll recorded in the same
        // millisecond must not hide that.
        SourceEventKind::Failed | SourceEventKind::CredentialMissing => 4,
        // Ordinary observations, and the fallback a kind added later gets
        // until it says otherwise here (`non_exhaustive`).
        _ => 1,
    }
}

/// One entry of the public register (#420): a STAC endpoint an operator
/// can import from in a single action.
///
/// The register is **data, not code**: it lives in the deployment's
/// configuration, so adding an endpoint is an edit and a restart, never a
/// release. Nothing here is fetched until an operator asks — an entry is
/// an offer, not a subscription.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegisterEntry {
    /// Stable identifier, for linking to a half-finished import.
    pub id: String,
    /// What to call it on screen.
    pub title: String,
    /// The catalog's URL.
    pub url: String,
    /// Whether reading it bills the reader — declared by whoever wrote
    /// the entry, because only they know the agreement.
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    pub requester_pays: bool,
}

impl RegisterEntry {
    /// The entry's host, when its URL has one.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        // Deliberately not a URL parser: the register is text an operator
        // wrote, and the allowlist check that matters happens in the
        // adapter against a parsed URL. This is for display and for
        // saying, before anything is attempted, that a host is not
        // permitted.
        let rest = self.url.split_once("://")?.1;
        let authority = rest.split(['/', '?', '#']).next()?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        // Strip a port; an IPv6 literal keeps its brackets.
        let host = if host.starts_with('[') {
            host.split_once(']')
                .map_or(host, |(h, _)| &host[..=h.len()])
        } else {
            host.split_once(':').map_or(host, |(h, _)| h)
        };
        (!host.is_empty()).then_some(host)
    }
}

// --- Egress policy (ADR 0030 §5, #419) ---

/// Bytes a fetched document may reach before it is refused. A STAC
/// catalog page is kilobytes; a megabyte is already a document nobody
/// meant to serve, and buffering more than that on a stranger's say-so
/// is how a fetch becomes a denial of service.
pub const DEFAULT_MAX_FETCH_BYTES: u64 = 1_048_576;

/// Seconds a fetch may take in total.
pub const DEFAULT_FETCH_TIMEOUT_SECS: u64 = 10;

/// What a deployment permits its server to reach (ADR 0030 §5).
///
/// **The default is an empty allowlist**, which means federation is off:
/// no host is reachable, and that is exactly the behaviour Swath had
/// before this existed. Turning it on is an operator's deliberate act,
/// host by host — there is no wildcard, because a wildcard allowlist is
/// not an allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressPolicy {
    allowed: BTreeSet<String>,
    /// Bytes a response may reach before it is refused, enforced as the
    /// body arrives rather than after it is buffered.
    pub max_bytes: u64,
    /// Whole-fetch timeout, in seconds.
    pub timeout_secs: u64,
}

impl Default for EgressPolicy {
    fn default() -> Self {
        Self {
            allowed: BTreeSet::new(),
            max_bytes: DEFAULT_MAX_FETCH_BYTES,
            timeout_secs: DEFAULT_FETCH_TIMEOUT_SECS,
        }
    }
}

impl EgressPolicy {
    /// A policy permitting exactly `hosts` (compared case-insensitively,
    /// as host names are). An empty iterator is the default: nothing.
    #[must_use]
    pub fn allowing(hosts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allowed: hosts
                .into_iter()
                .map(|host| host.into().trim().to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect(),
            ..Self::default()
        }
    }

    /// The permitted hosts, in order — what an operator sees when asking
    /// what this deployment may reach.
    pub fn hosts(&self) -> impl Iterator<Item = &str> {
        self.allowed.iter().map(String::as_str)
    }

    /// Whether anything is permitted at all. False means federation is
    /// off, which is a working configuration and the default one.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }

    /// Whether `host` is permitted. Exact match, case-insensitive: no
    /// suffix matching, because `evil-example.com` ends with
    /// `example.com` and a subdomain rule is how allowlists leak.
    #[must_use]
    pub fn allows(&self, host: &str) -> bool {
        self.allowed.contains(&host.trim().to_ascii_lowercase())
    }
}

/// What can go wrong talking to the source store.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SourceStoreError {
    /// No source with this id.
    #[error("no source `{id}`")]
    NotFound {
        /// The id asked for.
        id: SourceId,
    },
    /// The backing store failed.
    #[error("source store: {message}")]
    Backend {
        /// What the backend said.
        message: String,
    },
}

/// Persistence for the sources domain. Deliberately narrow: sources and
/// their events, nothing about bytes.
///
/// **Deleting a source removes the origin and its events, and leaves the
/// granules it produced** (ADR 0030 §3). Implementors must not cascade.
pub trait SourceStore: Send + Sync {
    /// Creates or replaces a source.
    fn upsert_source(
        &self,
        source: &Source,
    ) -> impl core::future::Future<Output = Result<(), SourceStoreError>> + Send;

    /// The source with this id, or `None`.
    fn get_source(
        &self,
        id: &SourceId,
    ) -> impl core::future::Future<Output = Result<Option<Source>, SourceStoreError>> + Send;

    /// Every source, in id order.
    fn list_sources(
        &self,
    ) -> impl core::future::Future<Output = Result<Vec<Source>, SourceStoreError>> + Send;

    /// Removes the source and its events. The granules it ingested are
    /// untouched.
    fn delete_source(
        &self,
        id: &SourceId,
    ) -> impl core::future::Future<Output = Result<(), SourceStoreError>> + Send;

    /// Records one event.
    fn record_event(
        &self,
        event: &SourceEvent,
    ) -> impl core::future::Future<Output = Result<(), SourceStoreError>> + Send;

    /// The events of `id`, oldest first.
    fn events(
        &self,
        id: &SourceId,
    ) -> impl core::future::Future<Output = Result<Vec<SourceEvent>, SourceStoreError>> + Send;
}

/// A shared store is a store: the serving binary keeps one registry
/// behind an `Arc` and hands the same handle to the ingest tasks and to
/// the API, rather than wrapping it twice.
impl<S: SourceStore + ?Sized> SourceStore for std::sync::Arc<S> {
    fn upsert_source(
        &self,
        source: &Source,
    ) -> impl core::future::Future<Output = Result<(), SourceStoreError>> + Send {
        (**self).upsert_source(source)
    }

    fn get_source(
        &self,
        id: &SourceId,
    ) -> impl core::future::Future<Output = Result<Option<Source>, SourceStoreError>> + Send {
        (**self).get_source(id)
    }

    fn list_sources(
        &self,
    ) -> impl core::future::Future<Output = Result<Vec<Source>, SourceStoreError>> + Send {
        (**self).list_sources()
    }

    fn delete_source(
        &self,
        id: &SourceId,
    ) -> impl core::future::Future<Output = Result<(), SourceStoreError>> + Send {
        (**self).delete_source(id)
    }

    fn record_event(
        &self,
        event: &SourceEvent,
    ) -> impl core::future::Future<Output = Result<(), SourceStoreError>> + Send {
        (**self).record_event(event)
    }

    fn events(
        &self,
        id: &SourceId,
    ) -> impl core::future::Future<Output = Result<Vec<SourceEvent>, SourceStoreError>> + Send {
        (**self).events(id)
    }
}

/// The statuses of `sources`, keyed by id — one derivation per source, so
/// a caller cannot accidentally read one source's events against
/// another's identity.
#[must_use]
pub fn statuses(events: &[SourceEvent]) -> BTreeMap<SourceId, SourceStatus> {
    let mut by_source: BTreeMap<SourceId, Vec<SourceEvent>> = BTreeMap::new();
    for event in events {
        by_source
            .entry(event.source.clone())
            .or_default()
            .push(event.clone());
    }
    by_source
        .into_iter()
        .map(|(id, own)| {
            let status = state_of(&own);
            (id, status)
        })
        .collect()
}
