// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `datetime` query-parameter grammar (OGC API - Features/Common
//! Part 2, reused verbatim by EDR — the "designed surface, paves EDR"
//! requirement of ADR 0015): an RFC 3339 UTC instant, or an interval
//! `start/end` with either side openable as `..` (never both). One
//! parser, two consumers with different *instant* semantics:
//!
//! - the granule-browsing filter ([`crate::granules`]) treats an instant
//!   as an inclusive point-range (`[t, t]`) — "granules acquired at `t`";
//! - the tiles route ([`crate::routes`]) resolves an instant as
//!   **latest-at-or-before** — the granule that was *current* at `t` —
//!   so its resolution window is `(.., t]` ([`DatetimeParam::window`]).
//!
//! Both consumers reject a malformed value as a 400 naming the grammar
//! (RFC 7807 via [`ApiError`]).

use swath_core::catalog::{Datetime, TimeRange};

use crate::error::ApiError;

/// A parsed `datetime` parameter, form preserved: the two forms carry
/// different resolution semantics on the tiles route (module docs), so
/// flattening to a [`TimeRange`] at parse time would lose the decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DatetimeParam {
    /// A single RFC 3339 UTC instant.
    Instant(Datetime),
    /// `start/end`, either side possibly open (`..`) — never both.
    Interval(TimeRange),
}

impl DatetimeParam {
    /// The frame-resolution window of ADR 0015: an instant `t` becomes
    /// `(.., t]` (latest-at-or-before selects from everything current at
    /// `t`); an interval is itself (latest-*within*, both bounds
    /// inclusive, open sides open).
    pub(crate) fn window(&self) -> TimeRange {
        match self {
            Self::Instant(t) => TimeRange {
                start: None,
                end: Some(t.clone()),
            },
            Self::Interval(range) => range.clone(),
        }
    }
}

/// Parses the shared grammar. Every failure is a 400 naming the value and
/// the grammar rule it broke.
pub(crate) fn parse_datetime_param(raw: &str) -> Result<DatetimeParam, ApiError> {
    let instant = |part: &str| -> Result<Option<Datetime>, ApiError> {
        if part == ".." {
            return Ok(None);
        }
        Datetime::new(part).map(Some).map_err(|_| {
            ApiError::bad_request(format!(
                "datetime `{raw}`: `{part}` is not an RFC 3339 UTC (`Z`) timestamp"
            ))
        })
    };
    match raw.split_once('/') {
        None => {
            let point = instant(raw)?.ok_or_else(|| {
                ApiError::bad_request(format!("datetime `{raw}` is open on both sides"))
            })?;
            Ok(DatetimeParam::Instant(point))
        }
        Some((start, end)) => {
            let range = TimeRange {
                start: instant(start)?,
                end: instant(end)?,
            };
            if range.start.is_none() && range.end.is_none() {
                return Err(ApiError::bad_request(format!(
                    "datetime `{raw}` is open on both sides"
                )));
            }
            Ok(DatetimeParam::Interval(range))
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use swath_core::catalog::Datetime;

    use super::{DatetimeParam, parse_datetime_param};

    /// The OGC grammar, form preserved: instant vs interval matters to
    /// the tiles route's resolution rule, so the parser must not conflate
    /// `t` with `t/t`.
    #[test]
    fn grammar_forms_and_rejections() {
        let dt = |s: &str| Datetime::new(s).unwrap();
        assert_eq!(
            parse_datetime_param("2024-06-06T17:54:00Z").unwrap(),
            DatetimeParam::Instant(dt("2024-06-06T17:54:00Z")),
        );
        let interval = parse_datetime_param("2024-06-01T00:00:00Z/2024-06-30T23:59:59Z").unwrap();
        let DatetimeParam::Interval(range) = &interval else {
            panic!("start/end must parse as an interval, got {interval:?}");
        };
        assert_eq!(range.start, Some(dt("2024-06-01T00:00:00Z")));
        assert_eq!(range.end, Some(dt("2024-06-30T23:59:59Z")));
        for half_open in ["../2024-06-30T23:59:59Z", "2024-06-01T00:00:00Z/.."] {
            assert!(matches!(
                parse_datetime_param(half_open).unwrap(),
                DatetimeParam::Interval(_)
            ));
        }
        // The standards' double-open rejection, plus malformed instants.
        for bad in [
            "../..",
            "yesterday",
            "2024-06-06",
            "2024-06-06T17:54:00+00:00",
        ] {
            assert_eq!(
                parse_datetime_param(bad).unwrap_err().status,
                StatusCode::BAD_REQUEST,
                "datetime `{bad}`"
            );
        }
    }

    /// The ADR 0015 resolution-window mapping: instant → `(.., t]`
    /// (latest-at-or-before), interval → itself (latest-within).
    #[test]
    fn windows_encode_the_resolution_rule() {
        let dt = |s: &str| Datetime::new(s).unwrap();
        let window = parse_datetime_param("2024-08-01T00:00:00Z")
            .unwrap()
            .window();
        assert_eq!(window.start, None, "an instant's window is open-start");
        assert_eq!(window.end, Some(dt("2024-08-01T00:00:00Z")));

        let window = parse_datetime_param("2024-06-01T00:00:00Z/2024-06-30T23:59:59Z")
            .unwrap()
            .window();
        assert_eq!(window.start, Some(dt("2024-06-01T00:00:00Z")));
        assert_eq!(window.end, Some(dt("2024-06-30T23:59:59Z")));
    }
}
