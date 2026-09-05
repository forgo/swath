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

use std::collections::BTreeMap;

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
        SourceEventKind::Failed => SourceState::Failing {
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
        SourceEventKind::Polled => 1,
        SourceEventKind::Ingested => 2,
        SourceEventKind::Stopped => 3,
        SourceEventKind::Failed => 4,
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
