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

/// The one HTTP error of this crate (#354): a status, a registry code, a
/// title and a detail. It has two renderings, chosen by the route that
/// answers: the OGC side serialises it as an RFC 7807 problem document
/// (this type's own [`IntoResponse`]), the openEO side as the spec's
/// `{"code","message"}` through [`OpenEo`]. One value, one taxonomy —
/// the same failure never has to be re-shaped on its way out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{status} {title}: {detail}")]
pub struct ApiError {
    /// HTTP status code.
    pub status: StatusCode,
    /// The openEO registry code (`tests/data/openeo/errors.json`) the
    /// openEO rendering answers; the generic name of the status where
    /// no handler named a more specific one.
    pub code: &'static str,
    /// Short, human-readable summary of the problem type (the RFC 7807
    /// `title`).
    pub title: String,
    /// Human-readable explanation of this occurrence (RFC 7807 `detail`,
    /// openEO `message`).
    pub detail: String,
}

impl ApiError {
    /// An error with an explicit registry `code`; the title is the
    /// status's canonical reason phrase.
    #[must_use]
    pub fn coded(status: StatusCode, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            code,
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            detail: detail.into(),
        }
    }

    /// The same error under a more specific registry code.
    #[must_use]
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = code;
        self
    }

    /// 409: the resource already exists.
    #[must_use]
    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::coded(StatusCode::CONFLICT, "Conflict", detail)
    }

    /// 404: the addressed resource does not exist.
    #[must_use]
    pub fn not_found(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "NotFound",
            title: "Not Found".to_owned(),
            detail: detail.into(),
        }
    }

    /// 400: the request itself is malformed.
    #[must_use]
    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "BadRequest",
            title: "Bad Request".to_owned(),
            detail: detail.into(),
        }
    }

    /// 406: no representation satisfying the `Accept` header exists.
    #[must_use]
    pub fn not_acceptable(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_ACCEPTABLE,
            code: "NotAcceptable",
            title: "Not Acceptable".to_owned(),
            detail: detail.into(),
        }
    }

    /// 500: the server failed to produce a response it should have.
    #[must_use]
    pub fn internal(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "Internal",
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

/// The openEO rendering of an [`ApiError`]: the standardized
/// `{"code","message"}` body. Codes come from the spec's `errors.json`
/// registry (pinned under `tests/data/openeo/`); the tests assert every
/// code the openEO surface emits exists there with a matching status.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {}", .0.code, .0.detail)]
pub struct OpenEo(pub ApiError);

impl OpenEo {
    /// An openEO error with an explicit registry code.
    #[must_use]
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self(ApiError::coded(status, code, message))
    }

    /// 500 `Internal` — a backend failure the client cannot fix.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal", message)
    }
}

impl From<ApiError> for OpenEo {
    fn from(err: ApiError) -> Self {
        Self(err)
    }
}

impl IntoResponse for OpenEo {
    fn into_response(self) -> Response {
        (
            self.0.status,
            Json(serde_json::json!({ "code": self.0.code, "message": self.0.detail })),
        )
            .into_response()
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
