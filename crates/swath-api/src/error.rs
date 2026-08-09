// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The API error shape: RFC 7807 problem details, the exception encoding
//! OGC API - Common's `exception.json` schema describes.
//!
//! Every non-2xx response this crate produces goes through [`ApiError`],
//! so error bodies are uniform and schema-valid. Handlers construct
//! errors semantically (`not_found` / `bad_request` / …); internal render
//! failures are mapped once, here, and never leak adapter internals
//! beyond the error chain's display text.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// An HTTP error response: status plus RFC 7807 problem-details body.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{status} {title}: {detail}")]
pub struct ApiError {
    /// HTTP status code.
    pub status: StatusCode,
    /// Short, human-readable summary of the problem type.
    pub title: String,
    /// Human-readable explanation of this occurrence.
    pub detail: String,
}

impl ApiError {
    /// 404: the addressed resource does not exist.
    #[must_use]
    pub fn not_found(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            title: "Not Found".to_owned(),
            detail: detail.into(),
        }
    }

    /// 400: the request itself is malformed.
    #[must_use]
    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            title: "Bad Request".to_owned(),
            detail: detail.into(),
        }
    }

    /// 406: no representation satisfying the `Accept` header exists.
    #[must_use]
    pub fn not_acceptable(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_ACCEPTABLE,
            title: "Not Acceptable".to_owned(),
            detail: detail.into(),
        }
    }

    /// 500: the server failed to produce a response it should have.
    #[must_use]
    pub fn internal(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            title: "Internal Server Error".to_owned(),
            detail: detail.into(),
        }
    }
}

impl IntoResponse for ApiError {
    /// Serializes as the OGC exception shape: `type` (RFC 7807 requires
    /// it; `about:blank` = "the status code says it all"), `title`,
    /// `status`, `detail`.
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "type": "about:blank",
            "title": self.title,
            "status": self.status.as_u16(),
            "detail": self.detail,
        });
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::ApiError;
    use axum::http::StatusCode;

    #[test]
    fn constructors_carry_their_status() {
        assert_eq!(ApiError::not_found("x").status, StatusCode::NOT_FOUND);
        assert_eq!(ApiError::bad_request("x").status, StatusCode::BAD_REQUEST);
        assert_eq!(
            ApiError::not_acceptable("x").status,
            StatusCode::NOT_ACCEPTABLE
        );
        assert_eq!(
            ApiError::internal("x").status,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
