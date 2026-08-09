// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The storage boundary: an [`AsyncFileReader`] over [`ObjectStore`] that can
//! record every byte range it fetches.
//!
//! async-tiff never touches the store directly — all of its reads (metadata
//! walks and tile fetches alike) come through [`StoreReader::get_bytes`].
//! In recording mode each fetch is appended to a provenance log, so the
//! ranges reported for a window are the ranges *observed on the wire*, not
//! reconstructed from IFD offsets after the fact.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use async_tiff::error::{AsyncTiffError, AsyncTiffResult};
use async_tiff::reader::AsyncFileReader;
use async_trait::async_trait;
use bytes::Bytes;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _};
use swath_core::trace::Provenance;

/// [`AsyncFileReader`] for one object in an [`ObjectStore`], optionally
/// logging every range fetched.
#[derive(Debug)]
pub(crate) struct StoreReader {
    store: Arc<dyn ObjectStore>,
    path: Path,
    log: Option<Mutex<Vec<Provenance>>>,
}

impl StoreReader {
    /// A plain reader: fetches are not logged (metadata I/O).
    pub(crate) fn new(store: Arc<dyn ObjectStore>, path: Path) -> Self {
        Self {
            store,
            path,
            log: None,
        }
    }

    /// A recording reader: every fetch is appended to the provenance log.
    pub(crate) fn recording(store: Arc<dyn ObjectStore>, path: Path) -> Self {
        Self {
            store,
            path,
            log: Some(Mutex::new(Vec::new())),
        }
    }

    /// Drains the provenance log (empty for a non-recording reader).
    pub(crate) fn take_provenance(&self) -> Vec<Provenance> {
        self.log
            .as_ref()
            .map(|log| std::mem::take(&mut *log.lock().expect("provenance log poisoned")))
            .unwrap_or_default()
    }
}

#[async_trait]
impl AsyncFileReader for StoreReader {
    async fn get_bytes(&self, range: Range<u64>) -> AsyncTiffResult<Bytes> {
        let bytes = self
            .store
            .get_range(&self.path, range.clone())
            .await
            .map_err(|e| AsyncTiffError::External(Box::new(e)))?;
        if let Some(log) = &self.log {
            log.lock()
                .expect("provenance log poisoned")
                .push(Provenance {
                    path: self.path.to_string(),
                    offset: range.start,
                    length: range.end - range.start,
                });
        }
        Ok(bytes)
    }
}
