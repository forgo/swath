// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Trace SSE stream tests (issue #28), over a real socket: `tower`'s
//! `oneshot` has no streaming-body affordance, so these tests run
//! `axum::serve` on an ephemeral loopback port and speak HTTP/1.1 by
//! hand — a minimal chunked-transfer SSE reader, no HTTP client
//! dependency.
//!
//! Every test uses the default `#[tokio::test]` **current-thread**
//! runtime deliberately: publishing to the bus without an intervening
//! `.await` is then atomic with respect to the server's stream task,
//! which makes the slow-consumer (`lagged`) scenario deterministic —
//! relying on TCP backpressure alone would leave lag at the mercy of
//! kernel socket buffer sizes.

#[allow(
    dead_code,
    reason = "shared between the API test targets; not every helper is used in each"
)]
mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use object_store::local::LocalFileSystem;
use swath_api::traces::TraceBus;
use swath_api::{ApiState, LayerRegistry, router};
use swath_core::crs::Crs;
use swath_core::raster::AssetRef;
use swath_core::tile::TileCoord;
use swath_core::trace::{Strategy, Timings, Trace};
use swath_reproject_proj4rs::Proj4rsReproject;
use swath_source_cog::CogSource;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Generous per-read ceiling: only ever reached on failure.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// A keepalive interval no test waits for (the keepalive test overrides).
const QUIET_KEEPALIVE: Duration = Duration::from_mins(10);

type FixtureState = ApiState<CogSource, Proj4rsReproject, LayerRegistry>;

/// The fixture-wired app with an explicit trace bus, plus the state
/// handle tests publish through directly.
fn app_with_bus(bus: TraceBus) -> (Arc<FixtureState>, Router) {
    let store =
        LocalFileSystem::new_with_prefix(common::fixtures_dir()).expect("fixture dir exists");
    let state = Arc::new(
        ApiState::new(
            LayerRegistry::hls_fixtures(),
            CogSource::new(Arc::new(store)),
            Proj4rsReproject,
            common::BASE_URL,
        )
        .with_trace_bus(bus),
    );
    (Arc::clone(&state), router(state))
}

/// Serves `app` on an ephemeral loopback port; the server task dies with
/// the runtime at test end.
async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server runs");
    });
    addr
}

/// One `GET` over its own connection (`Connection: close`); returns the
/// status code. Body is read to EOF and discarded — the tile tests only
/// need the status here (bytes are covered by `tiles.rs`).
async fn http_get_status(addr: SocketAddr, path: &str) -> u16 {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request = format!("GET {path} HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("request written");
    let mut response = Vec::new();
    timeout(READ_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("response within timeout")
        .expect("response read");
    let status_line = response
        .split(|&b| b == b'\n')
        .next()
        .expect("status line present");
    let status_line = String::from_utf8_lossy(status_line);
    status_line
        .split_whitespace()
        .nth(1)
        .expect("status code present")
        .parse()
        .expect("status code numeric")
}

/// One parsed SSE event: field values plus any comment lines.
#[derive(Debug, Default)]
struct Frame {
    event: Option<String>,
    id: Option<String>,
    data: Vec<String>,
    comments: Vec<String>,
}

impl Frame {
    fn data_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.data.join("\n")).expect("data is JSON")
    }
}

/// A minimal SSE subscriber: hand-rolled HTTP/1.1 GET, chunked
/// transfer-encoding decoder, frame splitter. Dropping it closes the
/// connection — the server-side disconnect path every test exercises at
/// teardown.
struct SseClient {
    reader: BufReader<TcpStream>,
    /// Decoded (de-chunked) body bytes not yet split into frames.
    body: Vec<u8>,
}

impl SseClient {
    /// Connects and reads the response head. Returning implies the
    /// handler ran, so the broadcast subscription exists — renders
    /// published after this point are guaranteed visible to the stream.
    async fn connect(addr: SocketAddr) -> Self {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        let request =
            format!("GET /traces HTTP/1.1\r\nhost: {addr}\r\naccept: text/event-stream\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .await
            .expect("request written");
        let mut reader = BufReader::new(stream);

        let mut head = Vec::new();
        loop {
            let mut line = String::new();
            timeout(READ_TIMEOUT, reader.read_line(&mut line))
                .await
                .expect("header line within timeout")
                .expect("header line read");
            if line == "\r\n" {
                break;
            }
            head.push(line.trim_end().to_ascii_lowercase());
        }
        assert!(
            head[0].starts_with("http/1.1 200"),
            "stream endpoint answers 200: {}",
            head[0],
        );
        assert!(
            head.contains(&"content-type: text/event-stream".to_owned()),
            "content type is text/event-stream: {head:?}",
        );
        Self {
            reader,
            body: Vec::new(),
        }
    }

    /// The next SSE frame, decoding chunked transfer as needed.
    async fn next_frame(&mut self) -> Frame {
        loop {
            if let Some(frame) = self.take_frame() {
                return frame;
            }
            self.read_chunk().await;
        }
    }

    /// Splits one complete frame (terminated by a blank line) off the
    /// decoded body, if present.
    fn take_frame(&mut self) -> Option<Frame> {
        let end = self.body.windows(2).position(|w| w == b"\n\n")?;
        let raw: Vec<u8> = self.body.drain(..end + 2).collect();
        let text = String::from_utf8(raw).expect("SSE frame is UTF-8");

        let mut frame = Frame::default();
        for line in text.lines().filter(|line| !line.is_empty()) {
            if let Some(comment) = line.strip_prefix(':') {
                frame.comments.push(comment.trim_start().to_owned());
                continue;
            }
            let (field, value) = line.split_once(':').expect("field line");
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => frame.event = Some(value.to_owned()),
                "id" => frame.id = Some(value.to_owned()),
                "data" => frame.data.push(value.to_owned()),
                other => panic!("unexpected SSE field `{other}`"),
            }
        }
        Some(frame)
    }

    /// Reads one HTTP/1.1 chunk into the decoded body buffer.
    async fn read_chunk(&mut self) {
        let mut size_line = String::new();
        timeout(READ_TIMEOUT, self.reader.read_line(&mut size_line))
            .await
            .expect("chunk size within timeout")
            .expect("chunk size read");
        let size = usize::from_str_radix(size_line.trim(), 16).expect("chunk size is hex");
        assert!(size > 0, "stream ended (zero-length chunk)");
        let mut chunk = vec![0u8; size + 2]; // chunk data + trailing CRLF
        timeout(READ_TIMEOUT, self.reader.read_exact(&mut chunk))
            .await
            .expect("chunk within timeout")
            .expect("chunk read");
        chunk.truncate(size);
        self.body.extend_from_slice(&chunk);
    }
}

/// A synthetic trace for direct-publish scenarios (shape mirrors the
/// pinned swath-core sample).
fn sample_trace() -> Trace {
    Trace {
        decision: Strategy::Live,
        source: AssetRef::new("s3://hls/granule/B04.tif"),
        sources: vec![AssetRef::new("s3://hls/granule/B04.tif")],
        crs_from: Crs::from_epsg(32613),
        crs_to: Crs::WEB_MERCATOR,
        bytes_read: 1024,
        provenance: vec![],
        timings: Timings::default(),
        ingest_to_pixel_ms: None,
        plan: None,
        temporal: None,
    }
}

// --- The contract: renders arrive as enveloped, deserializable traces ---

/// Render N tiles via the API → the subscriber receives N `trace` events
/// with monotonic ids, correct XYZ tile/layer envelopes, and payloads
/// that deserialize into the core `Trace` (the #21/#26 contract).
#[tokio::test]
async fn rendered_tiles_stream_as_enveloped_traces() {
    let (_state, app) = app_with_bus(TraceBus::new(256, QUIET_KEEPALIVE));
    let addr = serve(app).await;
    let mut sse = SseClient::connect(addr).await;

    let renders = [
        ("/tilesets/truecolor/tiles/12/1561/848", "truecolor"),
        ("/tilesets/ndvi/tiles/12/1561/848", "ndvi"),
    ];
    for (path, _) in renders {
        assert_eq!(http_get_status(addr, path).await, 200, "GET {path}");
    }

    for (i, (_, layer)) in renders.iter().enumerate() {
        let frame = sse.next_frame().await;
        assert_eq!(frame.event.as_deref(), Some("trace"));
        assert_eq!(frame.id.as_deref(), Some(i.to_string().as_str()));

        let envelope = frame.data_json();
        assert_eq!(envelope["layer"], *layer);
        // Envelope tile is XYZ z/x/y; the request path was OGC z/row/col.
        assert_eq!(envelope["tile"], "12/848/1561");

        let trace: Trace =
            serde_json::from_value(envelope["trace"].clone()).expect("payload is a core Trace");
        assert!(trace.bytes_read > 0, "a live render reads bytes");
        assert!(!trace.provenance.is_empty());
    }
}

// --- Slow consumer: lag is reported, rendering never stalls ---

/// A subscriber that reads nothing while more events than its buffer
/// holds are published sees one `lagged` report and then live traces —
/// and the render/publish path never blocks on it.
///
/// Determinism: renders more tiles than the capacity-2 buffer over HTTP
/// (their completion with an unread subscriber is itself the no-stall
/// proof — a blocking publish would deadlock this single-threaded test),
/// then publishes a burst directly with no intervening await, which the
/// server's stream task cannot interleave with on a current-thread
/// runtime — guaranteeing an overflow regardless of scheduling.
#[tokio::test]
async fn slow_consumer_gets_lagged_and_recovers_without_stalling_renders() {
    let (state, app) = app_with_bus(TraceBus::new(2, QUIET_KEEPALIVE));
    let addr = serve(app).await;
    let mut sse = SseClient::connect(addr).await;

    // 3 renders (> capacity 2) while the subscriber reads nothing. All
    // must return 200 — publish never blocks the render path.
    let path = "/tilesets/truecolor/tiles/12/1561/840"; // off-data: cheap
    for _ in 0..3 {
        assert_eq!(http_get_status(addr, path).await, 200);
    }

    // A burst of 6 direct publishes, atomic w.r.t. the stream task:
    // overflow (at least 6 - 2 = 4 drops) is now certain.
    let coord = TileCoord::new(0, 0, 0).unwrap();
    for _ in 0..6 {
        state
            .trace_bus()
            .publish("direct", coord, Arc::new(sample_trace()));
    }
    let published = 3 + 6;
    let last_id = (published - 1).to_string();

    // Read until the final published event: exactly one lagged report,
    // and delivered + missed accounts for every publish.
    let mut delivered = 0u64;
    let mut missed = 0u64;
    let mut lagged_frames = 0u32;
    loop {
        let frame = sse.next_frame().await;
        match frame.event.as_deref() {
            Some("trace") => {
                delivered += 1;
                if frame.id.as_deref() == Some(&last_id) {
                    break;
                }
            }
            Some("lagged") => {
                lagged_frames += 1;
                assert!(frame.id.is_none(), "lagged events carry no id");
                missed += frame.data_json()["missed"].as_u64().expect("missed count");
            }
            other => panic!("unexpected event {other:?}"),
        }
    }
    assert_eq!(lagged_frames, 1, "one lagged report for one overflow");
    assert!(missed >= 4, "burst of 6 into capacity 2 drops at least 4");
    assert_eq!(delivered + missed, published, "every publish accounted for");

    // The stream is live again after the lag: a fresh render arrives.
    assert_eq!(http_get_status(addr, path).await, 200);
    let frame = sse.next_frame().await;
    assert_eq!(frame.event.as_deref(), Some("trace"));
    assert_eq!(frame.id.as_deref(), Some(published.to_string().as_str()));
    assert_eq!(frame.data_json()["layer"], "truecolor");
}

// --- Fan-out: broadcast means every subscriber sees every event ---

#[tokio::test]
async fn two_concurrent_subscribers_both_receive_the_event() {
    let (_state, app) = app_with_bus(TraceBus::new(256, QUIET_KEEPALIVE));
    let addr = serve(app).await;
    let mut first = SseClient::connect(addr).await;
    let mut second = SseClient::connect(addr).await;

    let path = "/tilesets/truecolor/tiles/12/1561/848";
    assert_eq!(http_get_status(addr, path).await, 200);

    for sse in [&mut first, &mut second] {
        let frame = sse.next_frame().await;
        assert_eq!(frame.event.as_deref(), Some("trace"));
        assert_eq!(frame.id.as_deref(), Some("0"));
        assert_eq!(frame.data_json()["layer"], "truecolor");
    }
}

// --- Keepalive: idle streams stay warm ---

#[tokio::test]
async fn idle_stream_carries_keepalive_comments() {
    let (_state, app) = app_with_bus(TraceBus::new(256, Duration::from_millis(50)));
    let addr = serve(app).await;
    let mut sse = SseClient::connect(addr).await;

    // No renders, no publishes: the next frame on the wire can only be
    // the keepalive comment.
    let frame = sse.next_frame().await;
    assert_eq!(frame.comments, ["keepalive"]);
    assert!(frame.event.is_none() && frame.data.is_empty() && frame.id.is_none());
}
