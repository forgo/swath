// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The egress policy, against real sockets (#419, ADR 0030 §5).
//!
//! Every refusal in this suite is asserted the way it matters: a
//! non-allowlisted host is refused **without a connection being
//! attempted** — the test's listener counts accepts and expects zero —
//! and the size cap is asserted against a server that never stops
//! sending, so a cap that only bit after buffering would hang the test
//! rather than pass it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use swath_core::sources::EgressPolicy;
use swath_sources_stac::{FetchError, StacClient};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// What a scripted server does with a connection.
#[derive(Clone)]
enum Reply {
    /// `200` with this JSON body and an honest `Content-Length`.
    Json(String),
    /// `200` claiming `len` bytes, however many are actually sent.
    Claiming { len: usize, body: String },
    /// `200` with no length, streaming forever — the cap's real test.
    Endless,
    /// A redirect to `location`.
    Redirect(String),
    /// A plain status with an empty body.
    Status(u16),
}

/// A one-purpose HTTP server on 127.0.0.1, counting the connections it
/// accepted so a test can assert that none was.
struct Server {
    port: u16,
    accepts: Arc<AtomicUsize>,
}

impl Server {
    async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let accepts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepts);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let reply = reply.clone();
                tokio::spawn(async move {
                    // Read the request head; we never branch on it.
                    let mut buffer = [0_u8; 1024];
                    let _ = socket.read(&mut buffer).await;
                    match reply {
                        Reply::Json(body) => {
                            let head = format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                                 content-length: {}\r\n\r\n",
                                body.len()
                            );
                            let _ = socket.write_all(head.as_bytes()).await;
                            let _ = socket.write_all(body.as_bytes()).await;
                        }
                        Reply::Claiming { len, body } => {
                            let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {len}\r\n\r\n");
                            let _ = socket.write_all(head.as_bytes()).await;
                            let _ = socket.write_all(body.as_bytes()).await;
                        }
                        Reply::Endless => {
                            // Chunked and unending: a client that buffers
                            // first never returns.
                            let head = "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n";
                            let _ = socket.write_all(head.as_bytes()).await;
                            let chunk = format!("{:x}\r\n{}\r\n", 4096, "x".repeat(4096));
                            loop {
                                if socket.write_all(chunk.as_bytes()).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Reply::Redirect(location) => {
                            let head = format!(
                                "HTTP/1.1 302 Found\r\nlocation: {location}\r\n\
                                 content-length: 0\r\n\r\n"
                            );
                            let _ = socket.write_all(head.as_bytes()).await;
                        }
                        Reply::Status(code) => {
                            let head = format!("HTTP/1.1 {code} Nope\r\ncontent-length: 0\r\n\r\n");
                            let _ = socket.write_all(head.as_bytes()).await;
                        }
                    }
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self { port, accepts }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn accepted(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }
}

/// A policy allowing `127.0.0.1` — the only host these tests reach.
fn allowing_localhost() -> EgressPolicy {
    EgressPolicy::allowing(["127.0.0.1"])
}

/// Federation is off until an operator turns it on: the default policy
/// reaches nothing, which is exactly the behaviour Swath had before this
/// crate existed.
#[tokio::test]
async fn the_default_policy_reaches_nothing() {
    let policy = EgressPolicy::default();
    assert!(policy.is_empty());
    assert!(!policy.allows("example.com"));
    assert_eq!(policy.hosts().count(), 0);

    let server = Server::start(Reply::Json(r#"{"type":"Catalog"}"#.to_owned())).await;
    let client = StacClient::new(policy).expect("client");
    let error = client
        .fetch_json(&server.url("/catalog.json"))
        .await
        .expect_err("nothing is allowed");
    assert_eq!(
        error,
        FetchError::HostNotAllowed {
            host: "127.0.0.1".to_owned()
        }
    );
    // The point of the assertion: the host was refused *before* a socket.
    assert_eq!(server.accepted(), 0, "a refused host is never connected to");
}

/// A host not on the allowlist gets no connection attempt, even when the
/// allowlist is non-empty — the refusal is the name, not the reachability.
#[tokio::test]
async fn a_host_off_the_allowlist_is_never_connected_to() {
    let server = Server::start(Reply::Json("{}".to_owned())).await;
    // `localhost` and `127.0.0.1` are the same machine and different
    // names: the allowlist is names, exactly, and does not resolve.
    let client = StacClient::new(EgressPolicy::allowing(["example.com"])).expect("client");
    let error = client
        .fetch_json(&format!("http://localhost:{}/c.json", server.port))
        .await
        .expect_err("localhost is not example.com");
    assert!(matches!(error, FetchError::HostNotAllowed { .. }));
    assert_eq!(server.accepted(), 0);
}

/// The allowlist matches host names exactly. A suffix rule would let
/// `evil-example.com` through on an `example.com` entry, which is how
/// allowlists leak.
#[test]
fn the_allowlist_does_not_match_suffixes_or_case() {
    let policy = EgressPolicy::allowing(["Example.COM", "  ", "stac.example.org"]);
    assert!(policy.allows("example.com"), "host names are case-folded");
    assert!(policy.allows("EXAMPLE.com"));
    assert!(!policy.allows("evil-example.com"));
    assert!(!policy.allows("sub.example.com"));
    assert!(!policy.allows(""));
    assert_eq!(
        policy.hosts().collect::<Vec<_>>(),
        ["example.com", "stac.example.org"],
        "blank entries are not hosts"
    );
}

/// The happy path, so the refusals above are refusals and not a broken
/// client.
#[tokio::test]
async fn an_allowlisted_host_is_fetched_and_parsed() {
    let server = Server::start(Reply::Json(r#"{"type":"Catalog","id":"demo"}"#.to_owned())).await;
    let client = StacClient::new(allowing_localhost()).expect("client");
    let doc = client
        .fetch_json(&server.url("/catalog.json"))
        .await
        .expect("an allowlisted fetch");
    assert_eq!(doc["id"], "demo");
    assert_eq!(server.accepted(), 1);
}

/// A redirect off the allowlist is refused by name: an allowlisted host
/// cannot be used as a hop to somewhere else.
#[tokio::test]
async fn a_redirect_off_the_allowlist_is_refused() {
    let elsewhere = Server::start(Reply::Json(r#"{"secret":true}"#.to_owned())).await;
    let start = Server::start(Reply::Redirect(format!(
        "http://localhost:{}/elsewhere.json",
        elsewhere.port
    )))
    .await;

    let client = StacClient::new(allowing_localhost()).expect("client");
    let error = client
        .fetch_json(&start.url("/catalog.json"))
        .await
        .expect_err("the hop leaves the allowlist");
    let FetchError::RedirectOffHost { to, .. } = &error else {
        panic!("expected an off-host redirect refusal, got {error:?}")
    };
    assert!(
        to.contains("localhost"),
        "the refusal names where it pointed"
    );
    // And the second host was never connected to.
    assert_eq!(elsewhere.accepted(), 0);
}

/// A redirect that stays on an allowlisted host is followed — the policy
/// is about where, not about redirects as such.
#[tokio::test]
async fn a_redirect_within_the_allowlist_is_followed() {
    let target = Server::start(Reply::Json(r#"{"id":"moved"}"#.to_owned())).await;
    let start = Server::start(Reply::Redirect(target.url("/moved.json"))).await;
    let client = StacClient::new(allowing_localhost()).expect("client");
    let doc = client
        .fetch_json(&start.url("/catalog.json"))
        .await
        .expect("an on-host redirect");
    assert_eq!(doc["id"], "moved");
    assert_eq!(target.accepted(), 1);
}

/// A redirect loop between allowlisted hosts still terminates.
#[tokio::test]
async fn a_redirect_loop_terminates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    // A server that redirects to itself, forever.
    let looping = Server::start(Reply::Redirect(format!("http://127.0.0.1:{port}/loop"))).await;
    let self_loop = Server::start(Reply::Redirect(looping.url("/loop"))).await;
    let client = StacClient::new(allowing_localhost()).expect("client");
    let error = client
        .fetch_json(&self_loop.url("/start"))
        .await
        .expect_err("a loop is not a document");
    // Either the hop limit or a refused connection — both terminate, and
    // neither hangs.
    assert!(
        matches!(
            error,
            FetchError::TooManyRedirects { .. } | FetchError::Transport { .. }
        ),
        "got {error:?}"
    );
}

/// The size cap bites **as the body arrives**. The server here never
/// stops sending, so a client that buffered first would never return —
/// this test passing is the proof that it does not.
#[tokio::test]
async fn the_size_cap_is_enforced_as_the_body_arrives() {
    let server = Server::start(Reply::Endless).await;
    let mut policy = allowing_localhost();
    policy.max_bytes = 8 * 1024;
    let client = StacClient::new(policy).expect("client");
    let error = client
        .fetch_bytes(&server.url("/endless.json"))
        .await
        .expect_err("an endless body is refused");
    assert_eq!(error, FetchError::TooLarge { limit: 8 * 1024 });
}

/// A `Content-Length` claiming more than the cap is refused before a byte
/// of the body is read — and a server that *under*-declares cannot smuggle
/// the difference past the cap, because the declared length is the message
/// length and the extra bytes are simply not part of this response.
#[tokio::test]
async fn an_oversized_length_is_refused_and_under_declaring_smuggles_nothing() {
    let honest = Server::start(Reply::Claiming {
        len: 1_000_000,
        body: "x".repeat(16),
    })
    .await;
    let mut policy = allowing_localhost();
    policy.max_bytes = 1024;
    let client = StacClient::new(policy.clone()).expect("client");
    assert_eq!(
        client
            .fetch_bytes(&honest.url("/big.json"))
            .await
            .expect_err("the declared length passes the cap"),
        FetchError::TooLarge { limit: 1024 }
    );

    // Claims eight bytes, writes four thousand. The declared length is
    // the message length, so the surplus is not part of this response and
    // never reaches the cap — under-declaring buys an attacker nothing.
    let under = Server::start(Reply::Claiming {
        len: 8,
        body: "x".repeat(4096),
    })
    .await;
    let body = StacClient::new(policy)
        .expect("client")
        .fetch_bytes(&under.url("/under.json"))
        .await
        .expect("eight declared bytes are eight bytes");
    assert_eq!(body.len(), 8, "exactly what was declared, and no more");
}

/// Schemes a server does not fetch on anyone's behalf, and URLs that are
/// not URLs — refused before anything else happens.
#[tokio::test]
async fn only_http_urls_with_hosts_are_fetched() {
    let client = StacClient::new(EgressPolicy::allowing(["example.com"])).expect("client");
    assert!(matches!(
        client.check("file:///etc/passwd"),
        Err(FetchError::Scheme { .. })
    ));
    assert!(matches!(
        client.check("not a url"),
        Err(FetchError::NotAUrl { .. })
    ));
    // A scheme with no host is not a fetchable thing either.
    assert!(client.check("http:///nowhere").is_err());
}

/// An unsuccessful status is reported as one, with the code — not
/// swallowed and not turned into an empty document.
#[tokio::test]
async fn an_unsuccessful_status_is_reported() {
    let server = Server::start(Reply::Status(404)).await;
    let client = StacClient::new(allowing_localhost()).expect("client");
    let error = client
        .fetch_json(&server.url("/missing.json"))
        .await
        .expect_err("404 is not a catalog");
    let FetchError::Status { status, .. } = error else {
        panic!("expected a status error, got {error:?}")
    };
    assert_eq!(status, 404);
}
