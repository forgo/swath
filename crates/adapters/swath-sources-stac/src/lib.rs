// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Server-side STAC reads behind an egress allowlist (issue #419,
//! ADR 0030 §5) — **the only place Swath makes an outbound request**.
//!
//! # What this changes, and what it does not
//!
//! Before this crate, `swath-api` recorded that the server never fetches
//! remote metadata: the client supplied the document, so registration had
//! no SSRF surface. That property was preserved *by absence*. It is now
//! preserved *by policy*, and the policy is this:
//!
//! 1. **An allowlist, empty by default.** No host is reachable until an
//!    operator names it. Federation off is the default and a working
//!    configuration — the behaviour Swath had before.
//! 2. **The host is checked before a connection is attempted.** A
//!    refused host never becomes a socket, so a non-allowlisted name is
//!    not even a DNS lookup this process performs on a caller's behalf.
//! 3. **No redirect off-host.** Redirects are not followed automatically;
//!    one to a host the allowlist does not name is refused by name, so an
//!    allowlisted host cannot be used as a hop to somewhere else.
//! 4. **Caps enforced as the body arrives**, not after it is buffered —
//!    a response that would exceed the limit is abandoned mid-stream,
//!    which is the only version of a size cap that protects anything.
//! 5. **Only on an operator's action.** There is no HTTP route that
//!    reaches this crate; there cannot be one until ADR 0031's interlock
//!    lifts. Today the caller is the serving binary acting on a
//!    config-declared source.
//!
//! # What it deliberately is not
//!
//! Not a STAC library. It fetches a document, enforces the policy, and
//! parses JSON. Interpreting that JSON is the catalog's job, and the
//! reader that already exists (`swath_core::catalog::stac`) is the one
//! that does it.

use std::time::Duration;

use swath_core::sources::EgressPolicy;
use url::Url;

/// A refusal, or a transport failure, in the words an operator needs.
///
/// Every variant names what was refused and why. None of them carries a
/// response body: a refused fetch's content is not something this crate
/// passes along.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The URL did not parse, or named no host.
    #[error("`{url}` is not an absolute http(s) URL with a host")]
    NotAUrl {
        /// What was given.
        url: String,
    },
    /// The scheme was not `http` or `https`. `file:`, `gopher:` and the
    /// rest are not things a server fetches on anyone's behalf.
    #[error("scheme `{scheme}` is not http or https")]
    Scheme {
        /// The scheme that was refused.
        scheme: String,
    },
    /// The host is not on the allowlist. **No connection was attempted.**
    #[error(
        "host `{host}` is not on this deployment's egress allowlist; \
         an operator adds it in the config, host by host"
    )]
    HostNotAllowed {
        /// The host that was refused.
        host: String,
    },
    /// A redirect pointed off the allowlist. Following it would let an
    /// allowlisted host act as a hop to somewhere else.
    #[error("`{from}` redirected to `{to}`, which is not on the allowlist")]
    RedirectOffHost {
        /// Where the redirect came from.
        from: String,
        /// Where it pointed.
        to: String,
    },
    /// More redirects than any real document needs.
    #[error("`{url}` redirected more than {limit} times")]
    TooManyRedirects {
        /// The starting URL.
        url: String,
        /// The hop limit.
        limit: usize,
    },
    /// The body passed the size cap. Refused mid-stream, so the excess
    /// was never held.
    #[error("the response passed the {limit}-byte cap and was abandoned")]
    TooLarge {
        /// The cap, in bytes.
        limit: u64,
    },
    /// The whole fetch passed its deadline.
    #[error("no complete response within {seconds}s")]
    Timeout {
        /// The deadline, in seconds.
        seconds: u64,
    },
    /// The server answered, unsuccessfully.
    #[error("`{url}` answered {status}")]
    Status {
        /// The URL fetched.
        url: String,
        /// The HTTP status.
        status: u16,
    },
    /// The connection failed, or the body was not JSON.
    #[error("{detail}")]
    Transport {
        /// What went wrong, in the transport's own words.
        detail: String,
    },
}

/// How many redirects a fetch may follow before giving up. Each hop is
/// re-checked against the allowlist; the limit is here so a redirect loop
/// between two allowlisted hosts still terminates.
pub const MAX_REDIRECTS: usize = 4;

/// What a read actually cost, measured (#424).
///
/// **Bytes and requests, and nothing else.** No currency appears here or
/// anywhere downstream: Swath does not know the operator's rate card,
/// their egress agreement or their region, and a wrong money figure is
/// worse than no money figure. What it can measure it reports; what it
/// cannot it leaves to the operator and their provider's bill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FetchCost {
    /// Bytes received in the response body.
    pub bytes: u64,
    /// HTTP requests made, redirects included — a requester-pays bucket
    /// bills per request as well as per byte.
    pub requests: u32,
}

/// Reads STAC documents from allowlisted hosts.
#[derive(Debug, Clone)]
pub struct StacClient {
    policy: EgressPolicy,
    http: reqwest::Client,
}

impl StacClient {
    /// A client bound to `policy`.
    ///
    /// # Errors
    ///
    /// [`FetchError::Transport`] when the HTTP client cannot be built —
    /// a missing TLS backend, normally, which is a deployment fault
    /// rather than a request one.
    pub fn new(policy: EgressPolicy) -> Result<Self, FetchError> {
        let http = reqwest::Client::builder()
            // Redirects are handled here, not by the client: the point is
            // to re-check the allowlist at every hop, which an automatic
            // policy cannot do.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(policy.timeout_secs))
            .build()
            .map_err(|err| FetchError::Transport {
                detail: err.to_string(),
            })?;
        Ok(Self { policy, http })
    }

    /// The policy this client obeys.
    #[must_use]
    pub fn policy(&self) -> &EgressPolicy {
        &self.policy
    }

    /// Checks `url` against the policy without fetching it — the same
    /// check `fetch` makes first, exposed so a caller can refuse early
    /// and say why.
    ///
    /// # Errors
    ///
    /// [`FetchError::NotAUrl`], [`FetchError::Scheme`] or
    /// [`FetchError::HostNotAllowed`].
    pub fn check(&self, url: &str) -> Result<Url, FetchError> {
        let parsed = Url::parse(url).map_err(|_| FetchError::NotAUrl {
            url: url.to_owned(),
        })?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => {
                return Err(FetchError::Scheme {
                    scheme: other.to_owned(),
                });
            }
        }
        let host = parsed.host_str().ok_or_else(|| FetchError::NotAUrl {
            url: url.to_owned(),
        })?;
        if !self.policy.allows(host) {
            return Err(FetchError::HostNotAllowed {
                host: host.to_owned(),
            });
        }
        Ok(parsed)
    }

    /// Fetches `url` and parses it as JSON, under the whole policy.
    ///
    /// # Errors
    ///
    /// Any [`FetchError`]: a refused host or scheme (before any
    /// connection), a redirect off the allowlist, the size cap, the
    /// deadline, an unsuccessful status, or a transport failure.
    pub async fn fetch_json(&self, url: &str) -> Result<serde_json::Value, FetchError> {
        let bytes = self.fetch_bytes(url).await?;
        serde_json::from_slice(&bytes).map_err(|err| FetchError::Transport {
            detail: format!("`{url}` did not parse as JSON: {err}"),
        })
    }

    /// Fetches `url` under the policy and returns its bytes.
    ///
    /// # Errors
    ///
    /// See [`Self::fetch_json`].
    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        Ok(self.fetch_measured(url).await?.0)
    }

    /// Fetches `url` under the policy and reports what the read cost
    /// (#424): the bytes received and the requests made, both counted as
    /// they happened rather than estimated.
    ///
    /// # Errors
    ///
    /// See [`Self::fetch_json`].
    pub async fn fetch_measured(&self, url: &str) -> Result<(Vec<u8>, FetchCost), FetchError> {
        // The deadline covers redirects too: four hops that each take
        // nine seconds is not "within ten seconds".
        let deadline = Duration::from_secs(self.policy.timeout_secs);
        tokio::time::timeout(deadline, self.follow(url))
            .await
            .map_err(|_| FetchError::Timeout {
                seconds: self.policy.timeout_secs,
            })?
    }

    /// The redirect loop: every hop is checked against the allowlist
    /// before it is followed, which is the whole reason redirects are not
    /// left to the HTTP client.
    async fn follow(&self, url: &str) -> Result<(Vec<u8>, FetchCost), FetchError> {
        let mut current = self.check(url)?;
        let mut cost = FetchCost::default();
        for _ in 0..=MAX_REDIRECTS {
            // Counted before the send: a request that fails still
            // happened, and a bucket still bills for it.
            cost.requests = cost.requests.saturating_add(1);
            let response = self.http.get(current.clone()).send().await.map_err(|err| {
                FetchError::Transport {
                    detail: err.to_string(),
                }
            })?;
            let status = response.status();
            if status.is_redirection() {
                let target = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| FetchError::Transport {
                        detail: format!("`{current}` answered {status} with no usable Location"),
                    })?;
                // Relative redirects resolve against the current URL, as
                // any client would — and are then checked like any other.
                let next = current
                    .join(target)
                    .map_err(|_| FetchError::RedirectOffHost {
                        from: current.to_string(),
                        to: target.to_owned(),
                    })?;
                self.check(next.as_str())
                    .map_err(|_| FetchError::RedirectOffHost {
                        from: current.to_string(),
                        to: next.to_string(),
                    })?;
                current = next;
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Status {
                    url: current.to_string(),
                    status: status.as_u16(),
                });
            }
            let body = self.read_capped(response).await?;
            cost.bytes = body.len() as u64;
            return Ok((body, cost));
        }
        Err(FetchError::TooManyRedirects {
            url: url.to_owned(),
            limit: MAX_REDIRECTS,
        })
    }

    /// Reads the body chunk by chunk, refusing the moment it passes the
    /// cap. A `Content-Length` that claims too much is refused before a
    /// byte is read; a response that lies about its length is refused as
    /// it arrives. Buffering first and measuring after would protect
    /// nothing.
    async fn read_capped(&self, response: reqwest::Response) -> Result<Vec<u8>, FetchError> {
        let limit = self.policy.max_bytes;
        if response.content_length().is_some_and(|len| len > limit) {
            return Err(FetchError::TooLarge { limit });
        }
        let mut body = Vec::new();
        let mut stream = response;
        while let Some(chunk) = stream.chunk().await.map_err(|err| FetchError::Transport {
            detail: err.to_string(),
        })? {
            if body.len() as u64 + chunk.len() as u64 > limit {
                // Dropped here: the rest of the body is never read, and
                // what was read is discarded with it.
                return Err(FetchError::TooLarge { limit });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}
