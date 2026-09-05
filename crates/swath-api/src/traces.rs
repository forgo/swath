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

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};
use std::time::{Duration, Instant};

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

/// One source event on its way to the stream (#416): what happened to a
/// source, timestamped by the **server**, so freshness is computed from
/// the event's own instant rather than from a client's clock.
#[derive(Debug, Clone)]
pub struct IngestEvent {
    /// Monotonic per-process sequence number — the SSE `id:` field.
    pub id: u64,
    /// The source this happened to.
    pub source: String,
    /// When, RFC 3339 UTC.
    pub at: String,
    /// What happened: the `SourceEventKind` wire name.
    pub kind: &'static str,
    /// The event's own words. Never a credential value (ADR 0030 §4).
    pub detail: String,
    /// Events of this kind folded into this one by the throttle, so a
    /// busy source reports its volume without emitting it (#416).
    pub coalesced: u32,
}

/// The `data:` payload of an `ingest` event.
#[derive(serde::Serialize)]
struct IngestEnvelope<'a> {
    source: &'a str,
    at: &'a str,
    event: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    detail: &'a str,
    #[serde(skip_serializing_if = "is_zero")]
    coalesced: u32,
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if takes a reference"
)]
fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl IngestEvent {
    /// This event on the wire: `event: ingest`, `id:`, envelope `data:`.
    fn to_sse(&self) -> Event {
        let envelope = IngestEnvelope {
            source: &self.source,
            at: &self.at,
            event: self.kind,
            detail: &self.detail,
            coalesced: self.coalesced,
        };
        let data = serde_json::to_string(&envelope).expect("ingest envelope is infallible");
        Event::default()
            .event("ingest")
            .id(self.id.to_string())
            .data(data)
    }
}

/// What the bus carries. Renders and source events ride the same rails —
/// which is why the Sources screen can be live without polling anything.
#[derive(Debug, Clone)]
pub enum BusEvent {
    /// One rendered tile.
    Trace(TraceEvent),
    /// One thing that happened to a source (#416).
    Ingest(IngestEvent),
}

impl BusEvent {
    fn to_sse(&self) -> Event {
        match self {
            Self::Trace(event) => event.to_sse(),
            Self::Ingest(event) => event.to_sse(),
        }
    }
}

/// How often one source may put a *routine* event on the bus. A busy
/// filedrop can register granules far faster than anyone reads them, and
/// the bus is a live view rather than a log — so routine events are
/// coalesced to one per window and the suppressed count rides along.
/// Failures are never throttled: they are rare, and they are the reason
/// anyone is looking.
pub const INGEST_THROTTLE: Duration = Duration::from_secs(1);

/// Per-source throttle state.
#[derive(Debug)]
struct Throttle {
    last: Instant,
    suppressed: u32,
}

/// Publishes source events onto the bus (#416). Cloneable, and holds no
/// borrow of the router — an ingest task spawned before the API exists
/// still publishes onto the same bus the stream serves.
#[derive(Clone)]
pub struct SourcePublisher {
    sender: broadcast::Sender<BusEvent>,
    next_id: Arc<AtomicU64>,
    throttle: Arc<Mutex<BTreeMap<String, Throttle>>>,
}

impl fmt::Debug for SourcePublisher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourcePublisher").finish_non_exhaustive()
    }
}

impl SourcePublisher {
    /// Publishes one source event, subject to the throttle. Never blocks
    /// and never fails, exactly as the render path's publish does — an
    /// ingest task must not stall on telemetry.
    ///
    /// `at` is the server's own timestamp for the event, so the
    /// freshness a client renders is computed from when the thing
    /// happened rather than from the client's clock.
    ///
    /// Returns whether the event went out; a suppressed event is counted
    /// and reported by the next one.
    pub fn publish(&self, source: &str, kind: &'static str, at: &str, detail: &str) -> bool {
        self.publish_at(source, kind, at, detail, Instant::now())
    }

    /// [`Self::publish`] with an explicit "now", so the throttle is
    /// testable without sleeping.
    pub fn publish_at(
        &self,
        source: &str,
        kind: &'static str,
        at: &str,
        detail: &str,
        now: Instant,
    ) -> bool {
        // A failure is never suppressed: it is rare, and it is the whole
        // reason anyone is watching.
        let routine = kind != "failed";
        let coalesced = {
            let mut held = self.throttle.lock().expect("ingest throttle");
            match held.get_mut(source) {
                Some(state) if routine && now.duration_since(state.last) < INGEST_THROTTLE => {
                    state.suppressed = state.suppressed.saturating_add(1);
                    return false;
                }
                Some(state) => {
                    let suppressed = state.suppressed;
                    state.last = now;
                    state.suppressed = 0;
                    suppressed
                }
                None => {
                    held.insert(
                        source.to_owned(),
                        Throttle {
                            last: now,
                            suppressed: 0,
                        },
                    );
                    0
                }
            }
        };
        let event = IngestEvent {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            source: source.to_owned(),
            at: at.to_owned(),
            kind,
            detail: detail.to_owned(),
            coalesced,
        };
        // Err just means nobody is watching right now.
        let _ = self.sender.send(BusEvent::Ingest(event));
        true
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
    sender: broadcast::Sender<BusEvent>,
    next_id: Arc<AtomicU64>,
    keepalive: Duration,
    /// Per-source publish throttle state (#416): the last instant an
    /// ingest event went out, and how many were folded into the next one.
    ingest: Arc<Mutex<BTreeMap<String, Throttle>>>,
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
            next_id: Arc::new(AtomicU64::new(0)),
            keepalive,
            ingest: Arc::new(Mutex::new(BTreeMap::new())),
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
        let _ = self.sender.send(BusEvent::Trace(event));
    }

    /// A raw subscriber, for tests that assert what reaches the bus
    /// without going through the SSE encoding.
    #[must_use]
    pub fn subscribe_for_test(&self) -> broadcast::Receiver<BusEvent> {
        self.sender.subscribe()
    }

    /// A handle the ingest tasks publish source events through (#416).
    /// Cheap to clone and independent of the router's lifetime, so a
    /// watch spawned before the API is built can still reach the bus.
    #[must_use]
    pub fn publisher(&self) -> SourcePublisher {
        SourcePublisher {
            sender: self.sender.clone(),
            next_id: Arc::clone(&self.next_id),
            throttle: Arc::clone(&self.ingest),
        }
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
type RecvFut = Pin<Box<dyn Future<Output = (broadcast::Receiver<BusEvent>, RecvResult)> + Send>>;

type RecvResult = Result<BusEvent, RecvError>;

fn recv_next(mut receiver: broadcast::Receiver<BusEvent>) -> RecvFut {
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
    fn new(receiver: broadcast::Receiver<BusEvent>) -> Self {
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
    use std::time::{Duration, Instant};

    use swath_core::crs::Crs;
    use swath_core::raster::AssetRef;
    use swath_core::tile::TileCoord;
    use swath_core::trace::{Provenance, Strategy, Timings, Trace};

    use super::{BusEvent, Envelope, INGEST_THROTTLE, TraceBus};

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
                udf_ms: 0,
            },
            ingest_to_pixel_ms: None,
            // Synthetic direct-publish trace: no planner ran. Planned
            // renders carry `Some(PlanTrace)`; the wire shape of the
            // plan payload is pinned in swath-core (trace.rs).
            plan: None,
            temporal: None,
            udf_fuel_used: None,
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
                "plan": null,
            },
        });
        assert_eq!(serde_json::to_value(&envelope).unwrap(), expected);
    }

    /// The cache-hit shape on the wire (#36): `decision` becomes the
    /// externally tagged `{"cache_hit":{"key":…}}` object and the
    /// source/provenance fields carry the documented hit semantics (cache
    /// entry as source, zero source bytes, empty provenance). Pinned so
    /// the overlay's parser and this stream can never drift apart.
    #[test]
    fn cache_hit_envelope_json_is_pinned() {
        let key = "1d31e53806985ca6ed44e8fe79cc8fc9b9c5b4676bafbf8a4090e5f33fb07b2a";
        let trace = Trace {
            decision: Strategy::CacheHit {
                key: key.to_owned(),
            },
            source: AssetRef::new(format!("cache://{key}")),
            sources: vec![AssetRef::new(format!("cache://{key}"))],
            crs_from: Crs::WEB_MERCATOR,
            crs_to: Crs::WEB_MERCATOR,
            bytes_read: 0,
            provenance: Vec::new(),
            timings: Timings {
                total_ms: 1,
                ..Timings::default()
            },
            ingest_to_pixel_ms: None,
            plan: None,
            temporal: None,
            udf_fuel_used: None,
        };
        let envelope = Envelope {
            tile: "12/848/1561",
            layer: "truecolor",
            trace: &trace,
        };
        let expected = serde_json::json!({
            "tile": "12/848/1561",
            "layer": "truecolor",
            "trace": {
                "decision": {"cache_hit": {"key": key}},
                "source": format!("cache://{key}"),
                "sources": [format!("cache://{key}")],
                "crs_from": 3857,
                "crs_to": 3857,
                "bytes_read": 0,
                "provenance": [],
                "timings": {
                    "read_ms": 0,
                    "warp_ms": 0,
                    "pixel_ops_ms": 0,
                    "encode_ms": 0,
                    "total_ms": 1,
                },
                "ingest_to_pixel_ms": null,
                "plan": null,
            },
        });
        assert_eq!(serde_json::to_value(&envelope).unwrap(), expected);
    }

    /// The overview shape on the wire (#38): `decision` is the externally
    /// tagged `{"overview":{"level":…}}` object carrying the decimation
    /// factor of the overview grid served; everything else is a normal
    /// live-style render (real source, real provenance — just fewer bytes,
    /// which is the point). Pinned like the other two decisions so the
    /// overlay's parser and this stream can never drift apart.
    #[test]
    fn overview_envelope_json_is_pinned() {
        let mut trace = sample_trace();
        trace.decision = Strategy::Overview { level: 2 };
        let envelope = Envelope {
            tile: "11/424/780",
            layer: "b04",
            trace: &trace,
        };
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(
            json["trace"]["decision"],
            serde_json::json!({"overview": {"level": 2}})
        );
        assert_eq!(json["tile"], "11/424/780");
        // Provenance/bytes stay real-read fields, exactly as in a live
        // render — nothing else about the envelope changes shape.
        assert_eq!(json["trace"]["bytes_read"], 131_072);
    }

    /// The planner payload on the wire (#37): a planned render's
    /// envelope carries `trace.plan` — chosen strategy + every candidate
    /// with estimate/admissibility/reason — verbatim (exact shape pinned
    /// in swath-core; this pins that the envelope forwards it untouched
    /// and that candidates reuse the decision tag vocabulary).
    #[test]
    fn plan_payload_rides_the_envelope() {
        use std::borrow::Cow;
        use swath_core::planner::{CandidateTrace, PlannedStrategy};
        use swath_core::trace::PlanTrace;

        let mut trace = sample_trace();
        trace.plan = Some(PlanTrace {
            chosen: PlannedStrategy::Live,
            considered: vec![CandidateTrace {
                strategy: PlannedStrategy::Live,
                estimated_cost_bytes: 510_050,
                admissible: true,
                reason: Cow::Borrowed("full-resolution read"),
            }],
        });
        let envelope = Envelope {
            tile: "12/848/1561",
            layer: "truecolor",
            trace: &trace,
        };
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["trace"]["plan"]["chosen"], serde_json::json!("live"));
        assert_eq!(
            json["trace"]["plan"]["considered"][0],
            serde_json::json!({
                "strategy": "live",
                "estimated_cost_bytes": 510_050,
                "admissible": true,
                "reason": "full-resolution read",
            })
        );
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

        let BusEvent::Trace(first) = receiver.recv().await.unwrap() else {
            panic!("a render publishes a trace event")
        };
        let BusEvent::Trace(second) = receiver.recv().await.unwrap() else {
            panic!("a render publishes a trace event")
        };
        assert_eq!((first.id, second.id), (0, 1));
        assert_eq!(first.tile, "12/848/1561", "tile is z/x/y, not z/row/col");
        assert_eq!(
            (first.layer.as_str(), second.layer.as_str()),
            ("truecolor", "ndvi")
        );
    }

    /// Source events ride the same rails as renders (#416): one bus, two
    /// kinds, and the ingest envelope carries the **server's** instant so
    /// freshness is never computed from a client clock alone.
    #[tokio::test]
    async fn ingest_events_ride_the_bus_with_their_own_timestamp() {
        let bus = TraceBus::new(8, Duration::from_mins(1));
        let mut receiver = bus.sender.subscribe();
        let publisher = bus.publisher();

        assert!(publisher.publish("fire", "started", "2026-09-04T10:00:00Z", "/data/fire"));
        let BusEvent::Ingest(event) = receiver.recv().await.unwrap() else {
            panic!("an ingest publish is an ingest event")
        };
        assert_eq!(event.source, "fire");
        assert_eq!(event.kind, "started");
        assert_eq!(event.at, "2026-09-04T10:00:00Z");
        assert_eq!(event.coalesced, 0);

        let sse = format!("{:?}", BusEvent::Ingest(event.clone()).to_sse());
        assert!(sse.contains("ingest"), "the wire event names itself: {sse}");

        // Renders and source events share the sequence, so a client's
        // gap detection works across both.
        let coord = TileCoord::new(1, 0, 0).unwrap();
        bus.publish("truecolor", coord, Arc::new(sample_trace()));
        let BusEvent::Trace(render) = receiver.recv().await.unwrap() else {
            panic!("a render publishes a trace event")
        };
        assert_eq!(render.id, event.id + 1);
    }

    /// A busy source cannot flood the bus: routine events are coalesced
    /// to one per window and the suppressed count rides on the next one.
    /// A failure is never suppressed — it is why anyone is watching.
    #[tokio::test]
    async fn a_busy_source_is_coalesced_but_a_failure_is_never_suppressed() {
        let bus = TraceBus::new(16, Duration::from_mins(1));
        let mut receiver = bus.sender.subscribe();
        let publisher = bus.publisher();
        let start = Instant::now();

        assert!(publisher.publish_at("fire", "ingested", "t0", "a", start));
        // Nine more in the same window: counted, not sent.
        for i in 0..9 {
            assert!(
                !publisher.publish_at(
                    "fire",
                    "ingested",
                    "t0",
                    "b",
                    start + Duration::from_millis(i)
                ),
                "a routine event inside the window is suppressed"
            );
        }
        // A failure in the same window still goes out, immediately.
        assert!(publisher.publish_at("fire", "failed", "t0", "denied", start));
        // And the next routine event past the window reports the volume.
        assert!(publisher.publish_at(
            "fire",
            "ingested",
            "t1",
            "c",
            start + INGEST_THROTTLE + Duration::from_millis(1)
        ));

        let mut seen = Vec::new();
        while let Ok(BusEvent::Ingest(event)) = receiver.try_recv() {
            seen.push((event.kind, event.coalesced));
        }
        assert_eq!(seen, [("ingested", 0), ("failed", 9), ("ingested", 0)]);

        // A different source has its own window: one busy origin does not
        // silence another.
        assert!(publisher.publish_at("archive", "ingested", "t0", "z", start));
    }
}
