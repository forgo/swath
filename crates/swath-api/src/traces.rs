// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The Trace SSE stream (issue #28): every rendered tile's
//! [`Trace`](swath_core::trace::Trace) fanned out live to x-ray overlay
//! subscribers as Server-Sent Events on `GET /traces`.
//!
//! # Best-effort telemetry, never a durable log
//!
//! The bus is a [`tokio::sync::broadcast`] channel. Publishing never
//! blocks and never fails — the render path cannot stall on a slow (or
//! absent) x-ray client, full stop. The cost of that guarantee is the
//! broadcast eviction policy: each subscriber sees a bounded window
//! ([`DEFAULT_CAPACITY`] events), and a subscriber that falls behind has
//! its oldest undelivered events dropped. The drop is *reported*, not
//! silent: the subscriber receives an `event: lagged` with the count (the
//! overlay can render "missed N") and the stream continues live. Anyone
//! needing every trace ever rendered wants a log, not this stream.
//!
//! # The wire contract (API-layer envelope)
//!
//! Each render is one SSE event:
//!
//! ```text
//! event: trace
//! id: 42
//! data: {"tile":"12/848/1561","layer":"truecolor","trace":{...}}
//! ```
//!
//! - `id:` is a monotonic per-process sequence number, so a client can
//!   detect its own gaps independently of `lagged` events.
//! - `data:` is an **envelope** around the pinned core `Trace` JSON
//!   (swath-core `trace` module — the #21/#26 serde contract, untouched
//!   here): the `Trace` itself deliberately carries no tile coordinate or
//!   layer identity, so the stream supplies both. `tile` is XYZ-ordered
//!   `"z/x/y"` (the map-client habit — the overlay indexes tiles by it),
//!   *not* the OGC path order z/row/col. The envelope shape is pinned by
//!   a test below; changing it is a deliberate, reviewed act.
//! - A lagging subscriber gets `event: lagged` with
//!   `data: {"missed":N}` and no `id:` — the client's last-event-id
//!   still names the last trace actually delivered.
//! - Idle streams carry a `: keepalive` comment every
//!   [`DEFAULT_KEEPALIVE`] so proxies don't reap the connection.
//!
//! # Lifecycle
//!
//! Subscribing is handler-local: the receiver lives inside the response
//! body stream, so a client disconnect drops the stream, the receiver
//! unsubscribes, and nothing leaks — there is no per-connection task to
//! reap.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, KeepAliveStream, Sse};
use futures_core::Stream;
use swath_core::tile::TileCoord;
use swath_core::trace::Trace;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

/// Default per-subscriber buffer: how many published traces a subscriber
/// may fall behind before its oldest undelivered events are dropped (and
/// reported via `event: lagged`). 256 traces is roughly a full viewport
/// refresh at every zoom level a map client realistically holds — deep
/// enough to absorb bursts, small enough that memory stays bounded per
/// subscriber-independent ring buffer.
const DEFAULT_CAPACITY: usize = 256;

/// Default interval between `: keepalive` comments on an idle stream —
/// comfortably inside common proxy idle timeouts (30–60 s).
const DEFAULT_KEEPALIVE: Duration = Duration::from_secs(15);

/// One published render: the SSE sequence number, the tile/layer identity
/// the envelope carries, and the shared [`Trace`].
#[derive(Debug, Clone)]
pub struct TraceEvent {
    /// Monotonic per-process sequence number — the SSE `id:` field.
    pub id: u64,
    /// The rendered tile, XYZ-ordered: `"z/x/y"`.
    pub tile: String,
    /// The layer id the tile belongs to.
    pub layer: String,
    /// The render trace, shared read-only (the same `Arc` the tile
    /// response's `TraceExtension` carries).
    pub trace: Arc<Trace>,
}

/// The `data:` payload of a `trace` event — the API-layer envelope around
/// the pinned core `Trace` JSON. Serialize-only: the crate publishes this
/// shape, it never parses it.
#[derive(serde::Serialize)]
struct Envelope<'a> {
    tile: &'a str,
    layer: &'a str,
    trace: &'a Trace,
}

impl TraceEvent {
    /// This event on the wire: `event: trace`, `id:`, envelope `data:`.
    fn to_sse(&self) -> Event {
        let envelope = Envelope {
            tile: &self.tile,
            layer: &self.layer,
            trace: &self.trace,
        };
        let data =
            serde_json::to_string(&envelope).expect("Trace serialization is infallible (pinned)");
        Event::default()
            .event("trace")
            .id(self.id.to_string())
            .data(data)
    }
}

/// The `event: lagged` a subscriber receives after falling behind:
/// `missed` events were dropped for it; the stream continues live.
fn lagged_event(missed: u64) -> Event {
    Event::default()
        .event("lagged")
        .data(serde_json::json!({ "missed": missed }).to_string())
}

/// The trace bus: the tile handler publishes every render, `GET /traces`
/// subscribers receive them as SSE (module docs have the full contract).
#[derive(Debug)]
pub struct TraceBus {
    sender: broadcast::Sender<TraceEvent>,
    next_id: AtomicU64,
    keepalive: Duration,
}

impl Default for TraceBus {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY, DEFAULT_KEEPALIVE)
    }
}

impl TraceBus {
    /// A bus with an explicit per-subscriber buffer capacity and
    /// keepalive interval. Production wiring wants [`TraceBus::default`];
    /// this constructor is the seam tests use to force lag (tiny
    /// `capacity`) and observable pings (short `keepalive`).
    #[must_use]
    pub fn new(capacity: usize, keepalive: Duration) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            next_id: AtomicU64::new(0),
            keepalive,
        }
    }

    /// Publishes one render to every current subscriber. Never blocks and
    /// never fails: with no subscribers the event vanishes (best-effort
    /// telemetry), and a full subscriber buffer evicts that subscriber's
    /// oldest events (surfaced to it as `event: lagged`) — the render
    /// path is never the one that waits.
    pub fn publish(&self, layer: &str, coord: TileCoord, trace: Arc<Trace>) {
        let event = TraceEvent {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            tile: format!("{}/{}/{}", coord.z, coord.x, coord.y),
            layer: layer.to_owned(),
            trace,
        };
        // Err just means nobody is watching right now.
        let _ = self.sender.send(event);
    }

    /// The SSE response for one `GET /traces` subscriber: every trace
    /// published from this moment on, with keepalive comments while idle.
    pub(crate) fn sse(&self) -> Sse<KeepAliveStream<TraceEvents>> {
        Sse::new(TraceEvents::new(self.sender.subscribe()))
            .keep_alive(KeepAlive::new().interval(self.keepalive).text("keepalive"))
    }
}

/// The future one `recv` occupies: takes the receiver, returns it with
/// the result so the stream can re-arm for the next event.
type RecvFut = Pin<Box<dyn Future<Output = (broadcast::Receiver<TraceEvent>, RecvResult)> + Send>>;

type RecvResult = Result<TraceEvent, RecvError>;

fn recv_next(mut receiver: broadcast::Receiver<TraceEvent>) -> RecvFut {
    Box::pin(async move {
        let result = receiver.recv().await;
        (receiver, result)
    })
}

/// One subscriber's view of the bus as the stream axum's `Sse` body
/// polls: `Ok(event)` → `event: trace`; `Lagged(n)` → `event: lagged`
/// and the stream *continues*; `Closed` (bus dropped) → stream end.
/// Dropping the stream (client disconnect) drops the receiver — that is
/// the entire unsubscribe path.
pub(crate) struct TraceEvents {
    next: RecvFut,
}

impl TraceEvents {
    fn new(receiver: broadcast::Receiver<TraceEvent>) -> Self {
        Self {
            next: recv_next(receiver),
        }
    }
}

impl fmt::Debug for TraceEvents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraceEvents").finish_non_exhaustive()
    }
}

impl Stream for TraceEvents {
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let (receiver, result) = ready!(self.next.as_mut().poll(cx));
        self.next = recv_next(receiver);
        match result {
            Ok(event) => Poll::Ready(Some(Ok(event.to_sse()))),
            Err(RecvError::Lagged(missed)) => Poll::Ready(Some(Ok(lagged_event(missed)))),
            Err(RecvError::Closed) => Poll::Ready(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use swath_core::crs::Crs;
    use swath_core::raster::AssetRef;
    use swath_core::tile::TileCoord;
    use swath_core::trace::{Provenance, Strategy, Timings, Trace};

    use super::{Envelope, TraceBus, TraceEvent};

    fn sample_trace() -> Trace {
        Trace {
            decision: Strategy::Live,
            source: AssetRef::new("s3://hls/granule/B04.tif"),
            sources: vec![AssetRef::new("s3://hls/granule/B04.tif")],
            crs_from: Crs::from_epsg(32613),
            crs_to: Crs::WEB_MERCATOR,
            bytes_read: 131_072,
            provenance: vec![Provenance {
                path: "granule/B04.tif".to_owned(),
                offset: 4096,
                length: 131_072,
            }],
            timings: Timings {
                read_ms: 12,
                warp_ms: 3,
                pixel_ops_ms: 1,
                encode_ms: 2,
                total_ms: 18,
            },
            ingest_to_pixel_ms: None,
        }
    }

    /// The envelope JSON is a wire contract the overlay parses (like the
    /// core `Trace` shape it wraps, pinned in swath-core): this pins the
    /// exact serialized form so drift is a visible diff. The embedded
    /// `trace` value is the *core-pinned* JSON verbatim — the envelope
    /// adds `tile` + `layer` and touches nothing inside.
    #[test]
    fn envelope_json_is_pinned() {
        let trace = sample_trace();
        let envelope = Envelope {
            tile: "12/848/1561",
            layer: "truecolor",
            trace: &trace,
        };
        let expected = serde_json::json!({
            "tile": "12/848/1561",
            "layer": "truecolor",
            "trace": {
                "decision": "live",
                "source": "s3://hls/granule/B04.tif",
                "sources": ["s3://hls/granule/B04.tif"],
                "crs_from": 32613,
                "crs_to": 3857,
                "bytes_read": 131_072,
                "provenance": [
                    {"path": "granule/B04.tif", "offset": 4096, "length": 131_072},
                ],
                "timings": {
                    "read_ms": 12,
                    "warp_ms": 3,
                    "pixel_ops_ms": 1,
                    "encode_ms": 2,
                    "total_ms": 18,
                },
                "ingest_to_pixel_ms": null,
            },
        });
        assert_eq!(serde_json::to_value(&envelope).unwrap(), expected);
    }

    /// Publishing to a bus nobody subscribed to is a no-op, not an error
    /// — the render path must be indifferent to watchers.
    #[test]
    fn publish_without_subscribers_is_a_no_op() {
        let bus = TraceBus::new(4, Duration::from_mins(1));
        let coord = TileCoord::new(12, 848, 1561).unwrap();
        bus.publish("truecolor", coord, Arc::new(sample_trace()));
        bus.publish("truecolor", coord, Arc::new(sample_trace()));
    }

    /// Ids are monotonic across publishes and the tile field is
    /// XYZ-ordered `z/x/y`.
    #[tokio::test]
    async fn published_events_carry_monotonic_ids_and_xyz_tiles() {
        let bus = TraceBus::new(4, Duration::from_mins(1));
        let mut receiver = bus.sender.subscribe();
        let coord = TileCoord::new(12, 848, 1561).unwrap();
        bus.publish("truecolor", coord, Arc::new(sample_trace()));
        bus.publish("ndvi", coord, Arc::new(sample_trace()));

        let first: TraceEvent = receiver.recv().await.unwrap();
        let second: TraceEvent = receiver.recv().await.unwrap();
        assert_eq!((first.id, second.id), (0, 1));
        assert_eq!(first.tile, "12/848/1561", "tile is z/x/y, not z/row/col");
        assert_eq!(
            (first.layer.as_str(), second.layer.as_str()),
            ("truecolor", "ndvi")
        );
    }
}
