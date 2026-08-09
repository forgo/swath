// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `Catalog` adapter over Postgres + pgstac.
//!
//! Implements the [`Catalog`](swath_core::catalog::Catalog) port by calling
//! pgstac's SQL functions (`pgstac.upsert_collection`, `pgstac.upsert_items`,
//! `pgstac.get_collection`, `pgstac.all_collections`, `pgstac.search` —
//! surface verified against pgstac v0.9.10, the compose-stack image) with the
//! STAC documents produced by the pure converters in
//! [`swath_core::catalog::stac`]. This crate contains **no mapping logic** —
//! only SQL plumbing and error translation; the lossless domain ⇄ STAC
//! contract lives in the core and is what the integration suite exercises
//! end to end (`just test-catalog`).
//!
//! Connection config is a plain postgres URL
//! (`postgres://user:pass@host:5432/db`); pgstac's `search_path` puts the
//! `pgstac` schema in scope, and every call schema-qualifies anyway.
//!
//! # Error translation
//!
//! - A granule upsert against an absent dataset surfaces as
//!   [`CatalogError::DatasetNotFound`]: pgstac partitions `items` by
//!   `collection`, so the failure arrives as a "no partition for row" /
//!   foreign-key database error whose detail names the collection.
//! - A stored document that fails to map back surfaces the core's
//!   [`StacError`](swath_core::catalog::stac::StacError) via
//!   [`CatalogError::Stac`] — the "someone else wrote to this catalog" signal.
//! - Everything else (connection, SQL, transport) is
//!   [`CatalogError::Backend`] with the sqlx error as source.

use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use swath_core::catalog::stac::{
    dataset_from_stac_collection, dataset_to_stac_collection, granule_from_stac_item,
    granule_to_stac_item,
};
use swath_core::catalog::{
    Catalog, CatalogError, Dataset, DatasetId, Granule, GranuleQuery, TimeRange,
};

/// Search page size: internal only — `find_granules` pages exhaustively and
/// callers see the full result set (design doc §4). pgstac caps a single
/// page at 10 000; 1 000 keeps individual result documents modest.
const PAGE_LIMIT: u32 = 1000;

/// The pgstac-backed [`Catalog`].
///
/// Cheap to clone (the pool is internally shared).
#[derive(Debug, Clone)]
pub struct PgstacCatalog {
    pool: PgPool,
}

impl PgstacCatalog {
    /// Connects to `url` (a plain postgres URL,
    /// `postgres://user:pass@host:port/db`) and verifies pgstac is present by
    /// reading its version.
    ///
    /// # Errors
    ///
    /// [`CatalogError::Backend`] when the database is unreachable or has no
    /// pgstac schema installed.
    pub async fn connect(url: &str) -> Result<Self, CatalogError> {
        let pool = PgPoolOptions::new()
            .connect(url)
            .await
            .map_err(|e| backend("connecting to postgres", e))?;
        let catalog = Self { pool };
        // Fail fast and clearly on a database that is Postgres but not pgstac.
        catalog.pgstac_version().await?;
        Ok(catalog)
    }

    /// Wraps an existing pool (the caller owns pool sizing/lifecycle).
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The underlying pool, for callers that need raw SQL alongside the port
    /// (the integration suite's plain-STAC visibility checks, health probes).
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The installed pgstac version (e.g. `0.9.10`).
    ///
    /// # Errors
    ///
    /// [`CatalogError::Backend`] when the query fails (no pgstac schema, or
    /// connection loss).
    pub async fn pgstac_version(&self) -> Result<String, CatalogError> {
        sqlx::query_scalar("select pgstac.get_version()")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| backend("reading pgstac version", e))
    }

    /// Whether the dataset exists, without converting its document.
    async fn dataset_exists(&self, id: &DatasetId) -> Result<bool, CatalogError> {
        sqlx::query_scalar("select pgstac.get_collection($1) is not null")
            .bind(id.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(|e| backend("checking dataset existence", e))
    }

    /// One `pgstac.search` page for `body`.
    async fn search_page(&self, body: &Value) -> Result<Value, CatalogError> {
        let Json(page): Json<Value> = sqlx::query_scalar("select pgstac.search($1)")
            .bind(Json(body))
            .fetch_one(&self.pool)
            .await
            .map_err(|e| backend("searching granules", e))?;
        Ok(page)
    }
}

impl Catalog for PgstacCatalog {
    async fn upsert_dataset(&self, dataset: &Dataset) -> Result<(), CatalogError> {
        let doc = dataset_to_stac_collection(dataset);
        sqlx::query("select pgstac.upsert_collection($1)")
            .bind(Json(doc))
            .execute(&self.pool)
            .await
            .map_err(|e| backend("upserting dataset", e))?;
        Ok(())
    }

    async fn upsert_granules(&self, granules: &[Granule]) -> Result<(), CatalogError> {
        if granules.is_empty() {
            return Ok(());
        }
        let docs: Vec<Value> = granules.iter().map(granule_to_stac_item).collect();
        sqlx::query("select pgstac.upsert_items($1)")
            .bind(Json(Value::Array(docs)))
            .execute(&self.pool)
            .await
            .map_err(|e| match missing_collection(&e) {
                Some(id) => CatalogError::DatasetNotFound {
                    id: DatasetId::new(id),
                },
                None => backend("upserting granules", e),
            })?;
        Ok(())
    }

    async fn get_dataset(&self, id: &DatasetId) -> Result<Option<Dataset>, CatalogError> {
        let doc: Option<Json<Value>> = sqlx::query_scalar("select pgstac.get_collection($1)")
            .bind(id.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(|e| backend("reading dataset", e))?;
        doc.map(|Json(doc)| dataset_from_stac_collection(&doc).map_err(CatalogError::from))
            .transpose()
    }

    async fn list_datasets(&self) -> Result<Vec<Dataset>, CatalogError> {
        let Json(all): Json<Value> = sqlx::query_scalar("select pgstac.all_collections()")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| backend("listing datasets", e))?;
        let docs = all.as_array().cloned().unwrap_or_default();
        let mut datasets = docs
            .iter()
            .map(dataset_from_stac_collection)
            .collect::<Result<Vec<_>, _>>()?;
        datasets.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(datasets)
    }

    async fn find_granules(
        &self,
        dataset: &DatasetId,
        query: &GranuleQuery,
    ) -> Result<Vec<Granule>, CatalogError> {
        // pgstac.search over an unknown collection returns empty rather than
        // erroring; resolve the ambiguity for callers (design doc §4 error
        // contract) with an existence check first.
        if !self.dataset_exists(dataset).await? {
            return Err(CatalogError::DatasetNotFound {
                id: dataset.clone(),
            });
        }

        let mut body = json!({
            "collections": [dataset.as_str()],
            "limit": PAGE_LIMIT,
        });
        if let Some(bbox) = &query.bbox {
            body["bbox"] = json!(bbox.to_array());
        }
        if let Some(range) = &query.datetime {
            body["datetime"] = json!(datetime_filter(range));
        }

        let mut granules = Vec::new();
        loop {
            let page = self.search_page(&body).await?;
            for feature in page["features"].as_array().into_iter().flatten() {
                granules.push(granule_from_stac_item(feature)?);
            }
            match next_token(&page) {
                Some(token) => body["token"] = json!(token),
                None => break,
            }
        }
        Ok(granules)
    }
}

/// The STAC API interval string for a range: `start/end`, `..` for open ends.
fn datetime_filter(range: &TimeRange) -> String {
    let start = range.start.as_ref().map_or("..", |d| d.as_str());
    let end = range.end.as_ref().map_or("..", |d| d.as_str());
    format!("{start}/{end}")
}

/// The pagination token from a search page's `next` link, if any.
///
/// pgstac returns pagination as STAC API links
/// (`{"rel": "next", "href": "./search?token=next:…"}`); the token is the
/// `token` query parameter, passed back as the next request's `"token"`.
fn next_token(page: &Value) -> Option<&str> {
    page["links"]
        .as_array()?
        .iter()
        .find(|link| link["rel"] == "next")?["href"]
        .as_str()?
        .split_once("token=")
        .map(|(_, token)| token)
}

/// The collection id named by a missing-dataset database error, if this is
/// one.
///
/// pgstac partitions `items` by `collection`, so upserting granules of an
/// absent dataset fails with `no partition of relation "items" found for
/// row` whose DETAIL is `Partition key of the failing row contains
/// (collection) = (<id>).` (or, via other paths, a foreign-key violation
/// with `Key (collection)=(<id>) …`). Both shapes name the collection in a
/// `(collection)…(<id>)` detail; anything else is not a missing dataset.
fn missing_collection(e: &sqlx::Error) -> Option<String> {
    let detail = e
        .as_database_error()?
        .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()?
        .detail()?;
    let after = detail.split_once("(collection)")?.1;
    let start = after.find('(')?;
    let end = after[start + 1..].find(')')?;
    Some(after[start + 1..start + 1 + end].to_owned())
}

/// Wraps a sqlx error as the port's backend variant.
fn backend(detail: &str, e: sqlx::Error) -> CatalogError {
    CatalogError::Backend {
        detail: detail.to_owned(),
        source: Box::new(e),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use swath_core::catalog::{Datetime, TimeRange};

    use super::{datetime_filter, next_token};

    #[test]
    fn datetime_filter_renders_open_and_closed_ends() {
        let dt = |s: &str| Datetime::new(s).unwrap();
        assert_eq!(
            datetime_filter(&TimeRange {
                start: Some(dt("2024-06-01T00:00:00Z")),
                end: Some(dt("2024-06-30T00:00:00Z")),
            }),
            "2024-06-01T00:00:00Z/2024-06-30T00:00:00Z"
        );
        assert_eq!(
            datetime_filter(&TimeRange {
                start: Some(dt("2024-06-01T00:00:00Z")),
                end: None,
            }),
            "2024-06-01T00:00:00Z/.."
        );
        assert_eq!(datetime_filter(&TimeRange::default()), "../..");
    }

    #[test]
    fn next_token_reads_the_next_link_only() {
        let page = json!({
            "links": [
                {"rel": "root", "href": "."},
                {"rel": "next", "href": "./search?token=next:probe:g2"},
            ],
        });
        assert_eq!(next_token(&page), Some("next:probe:g2"));
        assert_eq!(
            next_token(&json!({"links": [{"rel": "root", "href": "."}]})),
            None
        );
        assert_eq!(next_token(&json!({})), None);
    }
}
