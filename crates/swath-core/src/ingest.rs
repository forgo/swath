// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The ingest orchestrator's registration step (ARCHITECTURE.md §5/§8,
//! REQUIREMENTS.md R1): a granule event becomes a catalog upsert, stamped
//! with its ingest time.
//!
//! Pure port composition — this module awaits futures the [`Catalog`] port
//! defines but performs no I/O of its own and reads no clock (the ingest
//! timestamp is the event's [`arrived_at`](GranuleEvent::arrived_at), stamped
//! by the event-source adapter). The surrounding *loop* — pull an event, call
//! [`ingest_granule`], log, repeat — lives with the binary, which owns the
//! runtime and the logging spine; the domain decision ("what does ingesting
//! an event mean?") lives here where every adapter pairing shares it.
//!
//! # Decisions (recorded)
//!
//! - **The dataset must pre-exist.** Ingesting a granule of an unknown
//!   dataset fails with [`CatalogError::DatasetNotFound`] (enforced by the
//!   [`Catalog::upsert_granules`] contract) rather than auto-creating a
//!   minimal dataset: datasets are deliberate operator objects carrying
//!   title, license, extent, band vocabulary, and serving layers (R2) — an
//!   auto-created placeholder would be an unservable ghost with made-up
//!   metadata. Serving config declares datasets; `swath serve` registers
//!   them at startup.
//! - **`ingested_at` = the event's arrival time**, not "now": the metric's
//!   zero point is when the granule was observed to arrive (REQUIREMENTS.md
//!   §3), and any queueing delay between observation and this upsert is
//!   ingest latency the metric must include, not hide.

use crate::catalog::{Catalog, CatalogError, Granule};
use crate::events::GranuleEvent;

/// Registers one arrived granule: stamps `ingested_at` from the event's
/// arrival time and upserts it through the catalog port. Returns the granule
/// as persisted.
///
/// # Errors
///
/// Any [`CatalogError`] from the upsert; notably
/// [`CatalogError::DatasetNotFound`] when the event names a dataset the
/// catalog does not contain (see the module docs for why that is not
/// auto-created).
pub async fn ingest_granule<C: Catalog>(
    catalog: &C,
    event: &GranuleEvent,
) -> Result<Granule, CatalogError> {
    let mut granule = event.granule.clone();
    granule.ingested_at = Some(event.arrived_at.clone());
    catalog
        .upsert_granules(std::slice::from_ref(&granule))
        .await?;
    Ok(granule)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::ingest_granule;
    use crate::catalog::{
        Bbox, Catalog, CatalogError, Dataset, DatasetId, Datetime, Granule, GranuleId, GranuleQuery,
    };
    use crate::events::GranuleEvent;

    /// A minimal in-memory catalog enforcing the dataset-must-exist
    /// contract, so the orchestrator's error path is testable without I/O.
    #[derive(Default)]
    struct MemoryCatalog {
        datasets: Mutex<Vec<Dataset>>,
        granules: Mutex<Vec<Granule>>,
    }

    impl Catalog for MemoryCatalog {
        async fn upsert_dataset(&self, dataset: &Dataset) -> Result<(), CatalogError> {
            self.datasets.lock().unwrap().push(dataset.clone());
            Ok(())
        }

        async fn upsert_granules(&self, granules: &[Granule]) -> Result<(), CatalogError> {
            for granule in granules {
                let known = self
                    .datasets
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|d| d.id == granule.dataset);
                if !known {
                    return Err(CatalogError::DatasetNotFound {
                        id: granule.dataset.clone(),
                    });
                }
                let mut stored = self.granules.lock().unwrap();
                stored.retain(|g| g.id != granule.id || g.dataset != granule.dataset);
                stored.push(granule.clone());
            }
            Ok(())
        }

        async fn get_dataset(&self, id: &DatasetId) -> Result<Option<Dataset>, CatalogError> {
            Ok(self
                .datasets
                .lock()
                .unwrap()
                .iter()
                .find(|d| d.id == *id)
                .cloned())
        }

        async fn list_datasets(&self) -> Result<Vec<Dataset>, CatalogError> {
            Ok(self.datasets.lock().unwrap().clone())
        }

        async fn find_granules(
            &self,
            dataset: &DatasetId,
            _query: &GranuleQuery,
        ) -> Result<Vec<Granule>, CatalogError> {
            Ok(self
                .granules
                .lock()
                .unwrap()
                .iter()
                .filter(|g| g.dataset == *dataset)
                .cloned()
                .collect())
        }
    }

    fn event(dataset: &str) -> GranuleEvent {
        GranuleEvent {
            granule: Granule {
                id: GranuleId::new("g1"),
                dataset: DatasetId::new(dataset),
                bbox: Bbox {
                    west: -106.1,
                    south: 39.2,
                    east: -105.9,
                    north: 39.4,
                },
                datetime: Datetime::new("2024-06-06T17:54:00Z").unwrap(),
                assets: BTreeMap::new(),
                ingested_at: None,
            },
            arrived_at: Datetime::new("2026-08-08T12:00:00Z").unwrap(),
        }
    }

    fn dataset(id: &str) -> Dataset {
        Dataset {
            id: DatasetId::new(id),
            title: id.to_owned(),
            description: String::new(),
            license: "other".to_owned(),
            extent: crate::catalog::Extent {
                bbox: Bbox {
                    west: -180.0,
                    south: -90.0,
                    east: 180.0,
                    north: 90.0,
                },
                interval: crate::catalog::TimeRange::default(),
            },
            bands: std::collections::BTreeSet::new(),
            layers: Vec::new(),
        }
    }

    #[test]
    fn ingest_stamps_arrival_time_and_upserts() {
        let catalog = MemoryCatalog::default();
        futures_executor(async {
            catalog.upsert_dataset(&dataset("hls-s30")).await.unwrap();
            let event = event("hls-s30");
            let stored = ingest_granule(&catalog, &event).await.unwrap();
            assert_eq!(stored.ingested_at, Some(event.arrived_at.clone()));
            let found = catalog
                .find_granules(&DatasetId::new("hls-s30"), &GranuleQuery::default())
                .await
                .unwrap();
            assert_eq!(found, vec![stored]);
        });
    }

    #[test]
    fn unknown_dataset_fails_loudly_not_auto_created() {
        let catalog = MemoryCatalog::default();
        futures_executor(async {
            let err = ingest_granule(&catalog, &event("nope")).await.unwrap_err();
            assert!(matches!(err, CatalogError::DatasetNotFound { id } if id.as_str() == "nope"));
            assert!(catalog.list_datasets().await.unwrap().is_empty());
        });
    }

    /// A tiny block-on for futures that never actually pend (the in-memory
    /// catalog resolves immediately) — the core takes no executor dep.
    fn futures_executor<F: core::future::Future<Output = ()>>(future: F) {
        let mut future = core::pin::pin!(future);
        let waker = core::task::Waker::noop();
        let mut cx = core::task::Context::from_waker(waker);
        match future.as_mut().poll(&mut cx) {
            core::task::Poll::Ready(()) => {}
            core::task::Poll::Pending => panic!("in-memory futures never pend"),
        }
    }
}
