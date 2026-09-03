// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! One in-memory [`Catalog`] for every test that needs one (#348). It
//! keeps the strictest contract any of the five doubles it replaces
//! enforced, so a test that passes here passes against pgstac:
//!
//! - a granule's dataset must already exist (`DatasetNotFound`), and
//!   re-upserting a granule replaces it (one row per `(dataset, id)`);
//! - querying a dataset that was never registered is a hard error, not an
//!   empty set — `swath serve`'s startup registration order depends on it;
//! - `find_granules` honours the query exactly as the port documents it:
//!   bbox intersection with inclusive edges (no antimeridian handling —
//!   the fixture footprints never cross it) and inclusive datetime bounds,
//!   each side optional.
//!
//! Clones share one store (the provider and the services handlers must see
//! the same catalog, like pgstac in production).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use swath_core::catalog::{Catalog, CatalogError, Dataset, DatasetId, Granule, GranuleQuery};

/// The shared in-memory catalog double.
#[derive(Debug, Clone, Default)]
pub struct MemoryCatalog {
    datasets: Arc<Mutex<BTreeMap<String, Dataset>>>,
    granules: Arc<Mutex<Vec<Granule>>>,
}

impl MemoryCatalog {
    /// Seeds the store synchronously (test setup); the dataset is
    /// registered first, so the granules' contract holds.
    pub fn seed(&self, dataset: Dataset, granules: Vec<Granule>) {
        self.datasets
            .lock()
            .unwrap()
            .insert(dataset.id.as_str().to_owned(), dataset);
        let mut stored = self.granules.lock().unwrap();
        for granule in granules {
            stored.retain(|g| g.id != granule.id || g.dataset != granule.dataset);
            stored.push(granule);
        }
    }

    /// The stored dataset, for post-mutation assertions.
    #[must_use]
    pub fn stored_dataset(&self, id: &str) -> Option<Dataset> {
        self.datasets.lock().unwrap().get(id).cloned()
    }

    /// Every stored granule, in insertion order.
    #[must_use]
    pub fn stored_granules(&self) -> Vec<Granule> {
        self.granules.lock().unwrap().clone()
    }

    fn has_dataset(&self, id: &DatasetId) -> bool {
        self.datasets.lock().unwrap().contains_key(id.as_str())
    }
}

impl Catalog for MemoryCatalog {
    async fn upsert_dataset(&self, dataset: &Dataset) -> Result<(), CatalogError> {
        self.datasets
            .lock()
            .unwrap()
            .insert(dataset.id.as_str().to_owned(), dataset.clone());
        Ok(())
    }

    async fn upsert_granules(&self, granules: &[Granule]) -> Result<(), CatalogError> {
        for granule in granules {
            if !self.has_dataset(&granule.dataset) {
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
        Ok(self.datasets.lock().unwrap().get(id.as_str()).cloned())
    }

    async fn list_datasets(&self) -> Result<Vec<Dataset>, CatalogError> {
        Ok(self.datasets.lock().unwrap().values().cloned().collect())
    }

    async fn find_granules(
        &self,
        dataset: &DatasetId,
        query: &GranuleQuery,
    ) -> Result<Vec<Granule>, CatalogError> {
        if !self.has_dataset(dataset) {
            return Err(CatalogError::DatasetNotFound {
                id: dataset.clone(),
            });
        }
        Ok(self
            .granules
            .lock()
            .unwrap()
            .iter()
            .filter(|granule| granule.dataset == *dataset && matches_query(query, granule))
            .cloned()
            .collect())
    }
}

/// Whether `granule` satisfies `query` — closed on both ends, matching the
/// domain's inclusive [`TimeRange`] and the pgstac adapter, whose search
/// compares inclusively against its window end (#431 corrected the one case
/// where it could not: a zero-width window, which pgstac parses to an empty
/// range). The boundary the two must agree on is asserted here and in the
/// live suite (`a_granule_exactly_on_the_window_end_is_inside_it`).
fn matches_query(query: &GranuleQuery, granule: &Granule) -> bool {
    if let Some(bbox) = query.bbox {
        let g = granule.bbox;
        if bbox.west > g.east || g.west > bbox.east || bbox.south > g.north || g.south > bbox.north
        {
            return false;
        }
    }
    if let Some(range) = &query.datetime {
        let t = granule.datetime.to_unix_millis();
        if range.start.as_ref().is_some_and(|s| t < s.to_unix_millis())
            || range.end.as_ref().is_some_and(|e| t > e.to_unix_millis())
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use swath_core::catalog::{
        Bbox, DatasetId, Datetime, Granule, GranuleId, GranuleQuery, TimeRange,
    };

    use super::matches_query;

    fn granule(datetime: &str) -> Granule {
        Granule {
            id: GranuleId::new("g"),
            dataset: DatasetId::new("d"),
            bbox: Bbox {
                west: -1.0,
                south: -1.0,
                east: 1.0,
                north: 1.0,
            },
            datetime: Datetime::new(datetime).unwrap(),
            assets: std::collections::BTreeMap::new(),
            ingested_at: None,
        }
    }

    #[test]
    fn the_window_is_closed_on_both_ends() {
        let at = "2024-06-06T17:54:00Z";
        let g = granule(at);
        let window = |start: Option<&str>, end: Option<&str>| GranuleQuery {
            bbox: None,
            datetime: Some(TimeRange {
                start: start.map(|s| Datetime::new(s).unwrap()),
                end: end.map(|s| Datetime::new(s).unwrap()),
            }),
        };
        // Exactly on either end is inside (#431) — the property the pgstac
        // adapter had to be corrected to share.
        assert!(matches_query(&window(None, Some(at)), &g));
        assert!(matches_query(&window(Some(at), None), &g));
        assert!(matches_query(&window(Some(at), Some(at)), &g));
        // One millisecond outside either end is outside.
        assert!(!matches_query(
            &window(None, Some("2024-06-06T17:53:59.999Z")),
            &g
        ));
        assert!(!matches_query(
            &window(Some("2024-06-06T17:54:00.001Z"), None),
            &g
        ));
    }
}
