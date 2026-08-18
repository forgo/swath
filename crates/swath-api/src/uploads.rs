// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The local-mode upload surface (#197): `PUT /uploads/{filename}` writes
//! the request body into the serving object store under `uploads/`, and
//! answers with the store key granule registration (#196) then names as
//! an asset `href` — upload-then-register is the browser file-drop flow.
//!
//! Deliberately narrow, matching the dataset surface's scope fence
//! ("register, don't manage"):
//!
//! - **One flat namespace.** `{filename}` is a single URL-safe segment
//!   (ascii alphanumerics, `.`, `-`, `_`; no leading dot), stored at
//!   `uploads/{filename}`. No directories, no listing, no deletes — a
//!   mis-uploaded file is superseded by re-uploading it (the same upsert
//!   semantics granule re-registration has).
//! - **Bytes only.** The body is stored verbatim; whether it is a
//!   servable COG is decided where it always is — the registration
//!   route's header validation, through the serving source stack. An
//!   upload that never registers is inert bytes under `uploads/`.
//! - **Mounted only where it is true.** `swath serve` merges this router
//!   solely in writable catalog mode over a *local* store root (the same
//!   local-vs-remote distinction the legacy referencer draws): a remote
//!   store has real upload tooling, and `--read-only` (#198) mounts no
//!   write surface at all. The capabilities document advertises the
//!   route exactly when mounted ([`crate::openeo::extend_capabilities`]),
//!   so the web panel's file drop is capabilities-driven, not guessed.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::put;
use object_store::{ObjectStore, ObjectStoreExt as _};
use serde_json::json;

use crate::error::ApiError;

/// Store prefix uploaded files land under — kept out of the way of
/// ingested granules and `pyramids/` in the same root.
pub const UPLOAD_PREFIX: &str = "uploads";

/// Largest accepted upload body. A full-resolution HLS band COG is tens
/// of megabytes; half a gigabyte leaves room for generous rasters while
/// still refusing an accidental bulk dump at the door.
pub const MAX_UPLOAD_BYTES: usize = 512 * 1024 * 1024;

/// Shared state: the serving object store (uploads land where the source
/// stack reads).
pub struct UploadsState {
    store: Arc<dyn ObjectStore>,
}

impl UploadsState {
    /// Wires the surface over the serving store.
    #[must_use]
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }
}

/// The upload router (`PUT /uploads/{filename}`) — merged by `swath
/// serve` only for writable catalog serving over a local store root
/// (module docs).
pub fn uploads_router(state: Arc<UploadsState>) -> axum::Router {
    axum::Router::new()
        .route("/uploads/{filename}", put(upload))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
        .with_state(state)
}

/// True when `filename` is one safe path segment: ascii alphanumerics,
/// `.`, `-`, `_`, not empty, no leading dot (which would hide the file
/// from a directory listing and admit `..`).
fn valid_filename(filename: &str) -> bool {
    !filename.is_empty()
        && !filename.starts_with('.')
        && filename
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// `PUT /uploads/{filename}` — stores the body at `uploads/{filename}`
/// (upsert) and answers `201` with the `href` to register.
async fn upload(
    State(app): State<Arc<UploadsState>>,
    Path(filename): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    if !valid_filename(&filename) {
        return Err(ApiError::bad_request(format!(
            "filename `{filename}` is not a safe name (ascii alphanumerics, `.`, `-`, `_`; \
             no leading dot)"
        )));
    }
    if body.is_empty() {
        return Err(ApiError::bad_request("upload body is empty"));
    }
    let key = format!("{UPLOAD_PREFIX}/{filename}");
    let path = object_store::path::Path::parse(&key)
        .map_err(|e| ApiError::bad_request(format!("filename `{filename}` is not a key: {e}")))?;
    app.store
        .put(&path, body.into())
        .await
        .map_err(|e| ApiError::internal(format!("storing `{key}` failed: {e}")))?;
    Ok((StatusCode::CREATED, Json(json!({ "href": key }))))
}

#[cfg(test)]
mod tests {
    use super::valid_filename;

    #[test]
    fn filename_taxonomy() {
        for good in ["scene-b04.tif", "a", "x_1.2.tiff", "UPPER-ok_9"] {
            assert!(valid_filename(good), "`{good}` must be accepted");
        }
        for bad in [
            "", ".hidden", "..", "a/b", "a\\b", "sp ace", "q?uery", "ü.tif",
        ] {
            assert!(!valid_filename(bad), "`{bad}` must be refused");
        }
    }
}
