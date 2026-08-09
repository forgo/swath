// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `EventSource` adapter over a watched drop directory
//! (ARCHITECTURE.md §7: the Phase-1 ingest trigger; REQUIREMENTS.md R1).
//!
//! # The drop convention (contractual)
//!
//! A granule arrives as a **manifest file** `<granule-id>.json` appearing in
//! the watch directory. The manifest names everything registration needs:
//!
//! ```json
//! {
//!   "dataset": "hls-s30",
//!   "granule": "hlss30-t13sdd-2024158",
//!   "bbox": [-106.1, 39.2, -105.9, 39.4],
//!   "datetime": "2024-06-06T17:54:00Z",
//!   "assets": {
//!     "b04": "hlss30-t13sdd-2024158-b04.tif",
//!     "b03": "hlss30-t13sdd-2024158-b03.tif"
//!   }
//! }
//! ```
//!
//! - **Asset values are opaque `AssetRef` URIs/keys** exactly as serving will
//!   read them (relative keys under the server's store root, or absolute
//!   `s3://…` URIs). The watcher never opens them — whether they resolve is
//!   the serving path's concern, reported per-render in the Trace.
//! - **Atomicity: bands first, manifest last.** Writers must land every band
//!   file before the manifest appears (write the manifest to a temporary
//!   name — a dotfile or a non-`.json` suffix, both ignored — and rename it
//!   into place). The manifest's appearance *is* the arrival signal, so a
//!   granule is never announced half-written.
//! - **A manifest is consumed once per process run** (tracked by file name).
//!   Re-drops under the same name are ignored until restart; a restart
//!   re-announces everything still in the directory, which is harmless — a
//!   catalog upsert is idempotent — but refreshes `ingested_at`. Remove
//!   manifests (or point the watcher at a fresh directory) to avoid that.
//!
//! # Why polling, not inotify/FSEvents
//!
//! A poll loop (`read_dir` every [`FiledropEvents::poll_interval`]) over the
//! `notify` crate's platform watchers, deliberately:
//!
//! 1. **The primary deployment is a container bind mount** (the compose
//!    stack mounts the drop directory from the host), where inotify events
//!    for host-side writes are notoriously unreliable or absent; polling is
//!    the only mechanism that behaves identically on macOS/Linux/NFS/bind
//!    mounts. (`notify` itself ships a polling fallback for exactly this.)
//! 2. **Zero new dependency tree** for the supply-chain gate; the adapter is
//!    ~200 lines over `std::fs`.
//! 3. **The latency cost is honest and bounded**: at most one poll interval,
//!    which is part of the ingest-to-pixel number the system reports — the
//!    metric includes the watcher's own reaction lag rather than hiding it.
//!
//! # The metric's zero point
//!
//! `arrived_at` is stamped from **this process's wall clock when the scan
//! first observes the manifest** — not the file's mtime. The writer's clock
//! (host, CI runner, remote uploader) is not comparable to the clock that
//! later timestamps render completion; using one clock for both ends keeps
//! the subtraction meaningful. The cost — up to one poll interval of
//! detection lag is *excluded* from the metric — is accepted and documented:
//! the metric measures the system from observation onward.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use swath_core::catalog::{Bbox, DatasetId, Datetime, Granule, GranuleId};
use swath_core::events::{EventError, EventSource, GranuleEvent};
use swath_core::raster::AssetRef;

/// The wire shape of a drop manifest (module docs). Unknown fields are
/// rejected: a manifest carrying fields this version does not understand is
/// a version-skew signal, not ignorable noise.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    /// The owning dataset id (must already exist in the catalog).
    dataset: String,
    /// The granule id, unique within the dataset.
    granule: String,
    /// WGS84 footprint, `[west, south, east, north]`.
    bbox: [f64; 4],
    /// Acquisition time, RFC 3339 UTC (`Z`).
    datetime: String,
    /// Band name → asset URI/key, as serving will read them.
    assets: BTreeMap<String, String>,
}

/// A polling file-drop [`EventSource`] over one directory.
///
/// Construct with [`new`](Self::new), then pull events; the watcher scans on
/// demand (inside `next_event`) and sleeps `poll_interval` between empty
/// scans. A missing directory is treated as empty — the drop point may be
/// created (or mounted) after the server starts.
#[derive(Debug)]
pub struct FiledropEvents {
    dir: PathBuf,
    poll_interval: Duration,
    /// Manifest file names already announced (or reported malformed) this
    /// run — the consume-once contract.
    seen: BTreeSet<OsString>,
    /// Events parsed but not yet pulled (one scan can find several).
    pending: VecDeque<GranuleEvent>,
}

impl FiledropEvents {
    /// A watcher over `dir`, scanning every `poll_interval`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>, poll_interval: Duration) -> Self {
        Self {
            dir: dir.into(),
            poll_interval,
            seen: BTreeSet::new(),
            pending: VecDeque::new(),
        }
    }

    /// The directory being watched.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The scan cadence.
    #[must_use]
    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    /// One directory scan: queue every new well-formed manifest (in name
    /// order — deterministic when several land between scans), or report the
    /// first malformed one. New manifests are marked seen either way.
    fn scan(&mut self) -> Result<(), EventError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            // Not-yet-existing drop points are simply empty; real I/O
            // failures (permissions, not-a-directory) surface.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(EventError::Backend {
                    detail: format!("scanning drop directory `{}`", self.dir.display()),
                    source: Box::new(e),
                });
            }
        };

        let mut fresh: Vec<OsString> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| is_manifest_name(name) && !self.seen.contains(name))
            .collect();
        fresh.sort_unstable();

        for name in fresh {
            // Consume-once: marked seen before parsing, so a malformed
            // manifest is reported exactly once, not on every scan.
            self.seen.insert(name.clone());
            let path = self.dir.join(&name);
            let arrived_at = now_utc();
            let granule = read_manifest(&path)?;
            self.pending.push_back(GranuleEvent {
                granule,
                arrived_at,
            });
        }
        Ok(())
    }
}

impl EventSource for FiledropEvents {
    /// Pends until a manifest arrives; never returns `Ok(None)` (a drop
    /// directory has no natural end).
    async fn next_event(&mut self) -> Result<Option<GranuleEvent>, EventError> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
            self.scan()?;
            if self.pending.is_empty() {
                tokio::time::sleep(self.poll_interval).await;
            }
        }
    }
}

/// Whether a directory entry name is a droppable manifest: `*.json`, not
/// hidden (dot-prefixed names are the sanctioned temporary/rename staging
/// form — module docs).
fn is_manifest_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    // Case-sensitive on purpose: the convention says `.json`, exactly.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    let is_json = name.ends_with(".json");
    !name.starts_with('.') && is_json && name.len() > ".json".len()
}

/// This process's wall clock as a catalog [`Datetime`].
fn now_utc() -> Datetime {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Datetime::from_unix_millis(millis).expect("the present is within year 0..=9999")
}

/// Reads and validates one manifest into a domain [`Granule`]
/// (`ingested_at` unset — the orchestrator stamps it).
fn read_manifest(path: &Path) -> Result<Granule, EventError> {
    let malformed = |detail: String| EventError::Malformed {
        detail: format!("manifest `{}`: {detail}", path.display()),
    };
    let raw = std::fs::read_to_string(path).map_err(|e| EventError::Backend {
        detail: format!("reading manifest `{}`", path.display()),
        source: Box::new(e),
    })?;
    let manifest: Manifest = serde_json::from_str(&raw).map_err(|e| malformed(e.to_string()))?;
    let datetime = Datetime::new(manifest.datetime.clone()).map_err(|_| {
        malformed(format!(
            "`datetime` `{}` is not RFC 3339 UTC (Z)",
            manifest.datetime
        ))
    })?;
    if manifest.assets.is_empty() {
        return Err(malformed("`assets` is empty — nothing to serve".to_owned()));
    }
    // Convention, checked: the manifest is named after its granule id, so
    // the directory listing alone identifies what has arrived.
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem != manifest.granule {
        return Err(malformed(format!(
            "file name stem `{stem}` does not match `granule` `{}`",
            manifest.granule
        )));
    }
    Ok(Granule {
        id: GranuleId::new(manifest.granule),
        dataset: DatasetId::new(manifest.dataset),
        bbox: Bbox::from_array(manifest.bbox),
        datetime,
        assets: manifest
            .assets
            .into_iter()
            .map(|(band, uri)| (band, AssetRef::new(uri)))
            .collect(),
        ingested_at: None,
    })
}
