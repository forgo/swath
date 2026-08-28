// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The process compiler: openEO process-graph JSON lowered to the Render IR
//! (ARCHITECTURE.md §5, REQUIREMENTS.md R3/R5). The graph is *interchange*;
//! the [`RenderPlan`] is *ours* — a derived product defined by a standard
//! openEO graph serves through exactly the same executable IR as a built-in
//! layer.
//!
//! # Conformance statement (v0)
//!
//! The compiler accepts the openEO process-graph JSON of the [openEO API]
//! (an object of nodes `{"process_id", "arguments", "result"}` with
//! `{"from_node": ...}` / `{"from_parameter": ...}` references, optionally
//! wrapped in `{"process_graph": ...}`) and lowers this subset of
//! [openeo-processes 1.2.0] (the definitions are committed under
//! `tests/data/openeo/` as pinned truth):
//!
//! * **`load_collection`** — the leaf. `id` must name the context's
//!   collection; `bands` is required in v0 and every entry must resolve
//!   against the context's band bindings. `temporal_extent` compiles
//!   into the product's **resolution window** ([`CompiledProduct::window`],
//!   ADR 0015 frame selection): it constrains which granule backs a
//!   frame, never how pixels combine. Bounds are RFC 3339 UTC (`Z`)
//!   date-times, dates, or years, compared at millisecond precision;
//!   the interval is left-closed per the spec. `spatial_extent` and
//!   `properties` remain accepted and ignored: tile serving decides the
//!   spatial window (`docs/ROADMAP.md`).
//! * **`filter_temporal`** — narrows the resolution window further (the
//!   intersection with the cube's window so far — same frame-selection
//!   semantics as `temporal_extent`, per ADR 0015). `dimension` must be
//!   omitted, `null`, or the temporal dimension `t`; anything else is
//!   the spec's `DimensionNotAvailable`. A window that provably selects
//!   nothing (an empty interval, or one disjoint from the window already
//!   applied) is rejected at compile time.
//! * **`reduce_dimension`** — over `dimension: "bands"` only, with an
//!   embedded reducer sub-graph (the standard NDVI idiom). Inside the
//!   reducer, `from_parameter: "data"` is the band array.
//! * **`array_element`** — inside a reducer, by `index` (into the loaded
//!   band order) or `label` (a loaded band's openEO name, or any alias the
//!   context binds); exactly one of the two, per the spec.
//! * **`add` / `subtract` / `multiply` / `divide`** — scalar arithmetic
//!   inside a reducer, over band elements, numbers, and each other's
//!   results. Division by zero follows the IR's semantics: the pixel
//!   becomes *no data* (the spec's `DivisionByZero` exception, resolved
//!   per pixel instead of per request).
//! * **`linear_scale_range`** — on a whole cube (gray or multi-band),
//!   lowered to [`PixelOp::Rescale`]. `outputMin`/`outputMax` must be
//!   exactly `0`/`255`: the IR quantizes to 8-bit RGBA, so any other
//!   output range cannot be honored and is rejected rather than silently
//!   rescaled. The spec's clip-to-input-range contract is exactly
//!   [`PixelOp::Rescale`]'s clamp.
//! * **`ndvi`** — the convenience process; `nir`/`red` default to the
//!   common names `"nir"`/`"red"` and resolve like `array_element` labels.
//!   `target_band` must be omitted or `null` (the bands dimension is
//!   dropped; the result is gray). Lowers to the same
//!   `(nir - red) / (nir + red)` expression as the reduce idiom.
//! * **`save_result`** — the required result node; `format` must be PNG
//!   (case-insensitive, per the spec). `options` accepts exactly one
//!   optional key, `colormap` (`"grayscale"` | `"viridis"` | `"magma"` |
//!   `"rdylgn"`) — Swath's format option naming the palette applied to a
//!   gray result (openEO has no colormap process; the palette is
//!   post-eval presentation, so it rides the save node). It is rejected
//!   on a multi-band (composite) result: a LUT maps one gray value per
//!   pixel. Absent, gray results default to `"grayscale"`.
//! * **`merge_cubes`** (ADR 0022, the two-cube join) — over two **gray**,
//!   unscaled cubes (`ndvi` / `reduce_dimension` results) that load the
//!   context's collection through **two different `load_collection`
//!   nodes** — one per frame, each with its own resolution window.
//!   `overlap_resolver` is **required**: a child graph over `x` (from
//!   `cube1`) and `y` (from `cube2`) producing a scalar per pixel pair —
//!   the same reducer mechanism, so `subtract` and its siblings apply
//!   verbatim. `context` is not admitted. Multi-band, scaled, and UDF
//!   inputs are rejected with the fix named (reduce first, scale after,
//!   no UDF join in v1). The result is gray, over both sources.
//! * **`run_udf`** (ADR 0018, the bounded profile's sandboxed-WASM
//!   extension) — over a loaded, unscaled cube: every loaded band
//!   (deduplicated, load order) becomes one request plane, in that order,
//!   of a [`PixelOp::Udf`](crate::ir::PixelOp::Udf) stage. `runtime`
//!   must be `"wasm"` and `version` omitted, `null`, or `"1"` (the spec's
//!   `InvalidRuntime` / `InvalidVersion`); `context` passes through
//!   verbatim as the stage's params (`null` when absent). `udf` is
//!   [`UdfSource`]: inline `data:application/wasm;base64,…` (≤ 8 MiB) or
//!   an absolute `http(s)` URL — remote modules are fetched **once** by
//!   the caller at this compile motion and handed in through
//!   [`CompileContext::with_udf_module`]; the compiler never fetches.
//!   The module is validated at compile time through the
//!   [`UdfRegistrar`] port (zero imports, the four v1 exports, the ABI
//!   version, the output-arity probe): a rejected module is a typed error
//!   naming the node and the `udf` argument, and the result cube is
//!   typed by the pinned arity — 1 plane renders gray, 3 RGB, anything
//!   else is rejected. **One `run_udf` per graph** in v1: its result is
//!   a UDF cube no further process except `linear_scale_range` and
//!   `save_result` (without a colormap) accepts.
//!
//! Anything else is [`CompileError::UnsupportedProcess`], whose message
//! lists this set. Structural validation — exactly one `result: true` node
//! per (sub-)graph, no dangling `from_node` references, no cycles (openEO
//! graphs are DAGs), cube-vs-scalar type mismatches — produces typed
//! errors naming the offending node.
//!
//! # Sources (ADR 0022)
//!
//! A graph with one `load_collection` compiles exactly as before. With
//! more than one, every loaded band is qualified `band@node` (the node
//! id of its `load_collection`), so the plan's inputs stay a flat,
//! unique namespace while [`BandInput::source`](crate::ir::BandInput)
//! names the branch each is read through; [`CompiledProduct::sources`]
//! lists every source with its own resolution window, and
//! [`CompiledProduct::window`] is their hull. Serving resolves one
//! granule per source (issue #296).
//!
//! A gray result (from `reduce_dimension`, `ndvi`, or `merge_cubes`) compiles to
//! `BandMath → [Rescale] → Colormap(...)` (the `save_result` colormap
//! option, `Grayscale` when absent); a multi-band result must
//! have exactly three bands and compiles to `Composite → [Rescale]` in
//! loaded-band order; a UDF result compiles to `Udf → [Rescale]` and
//! carries its module bytes out in [`CompiledProduct::udf_module`] for
//! the caller to persist by hash. The compiled plan's inputs are only the
//! bands the ops actually reference (first-reference order), so serving
//! fetches nothing the product does not read.
//!
//! [openEO API]: https://api.openeo.org/
//! [openeo-processes 1.2.0]: https://github.com/Open-EO/openeo-processes/tree/1.2.0

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value as Json;
use swath_core::catalog::{Datetime, TimeRange};
use swath_core::udf::MODULE_MAX_BYTES;

use crate::ir::{BinaryOp, Colormap, Expr, RenderPlan};
use crate::plan::{PlanSpec, ndvi_expr, plan_for};
use crate::udf::{UdfError, UdfRegistrar, UdfSource, UdfStage};

/// What a graph's `load_collection` bands may call one dataset band: the
/// dataset band name itself plus any openEO names / common names bound to
/// it (from a Dataset's `swath:bands` vocabulary at wire-up).
#[derive(Debug, Clone)]
struct BandBinding {
    /// The dataset band name the compiled plan reads (e.g. `b8a`).
    name: String,
    /// openEO band names and common names that resolve to it (e.g.
    /// `nir`, `B8A`).
    aliases: Vec<String>,
}

/// The compilation context: which collection the graph may load, how
/// openEO band names map onto dataset bands, and — for `run_udf` — the
/// [`UdfRegistrar`] that validates modules plus any remote modules the
/// caller already fetched for this graph.
#[derive(Clone)]
pub struct CompileContext {
    collection: String,
    bands: Vec<BandBinding>,
    /// The registration seam; `None` = `run_udf` is unavailable here.
    udf: Option<Arc<dyn UdfRegistrar>>,
    /// Remote modules resolved for this compile motion, keyed by the
    /// exact `udf` argument (URL) that names them.
    modules: Vec<(String, Arc<[u8]>)>,
}

impl std::fmt::Debug for CompileContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let modules: Vec<(&str, usize)> = self
            .modules
            .iter()
            .map(|(url, bytes)| (url.as_str(), bytes.len()))
            .collect();
        f.debug_struct("CompileContext")
            .field("collection", &self.collection)
            .field("bands", &self.bands)
            .field("udf", &self.udf.is_some())
            .field("modules", &modules)
            .finish()
    }
}

impl CompileContext {
    /// A context for `collection` with no bands bound yet and no
    /// `run_udf` support.
    #[must_use]
    pub fn new(collection: impl Into<String>) -> Self {
        Self {
            collection: collection.into(),
            bands: Vec::new(),
            udf: None,
            modules: Vec::new(),
        }
    }

    /// Enables `run_udf`: modules register through `registrar` at compile
    /// time (ADR 0018, #204). Without one, a graph using `run_udf` is
    /// [`CompileError::UdfUnavailable`].
    #[must_use]
    pub fn with_udf_registrar(mut self, registrar: Arc<dyn UdfRegistrar>) -> Self {
        self.udf = Some(registrar);
        self
    }

    /// Hands the compiler the bytes of a remote module the caller fetched
    /// for this graph's `run_udf` node whose `udf` argument is exactly
    /// `url` — fetched once at the compile motion, or resolved from the
    /// module store by hash on rehydration; the compiler itself never
    /// fetches.
    #[must_use]
    pub fn with_udf_module(mut self, url: impl Into<String>, bytes: Vec<u8>) -> Self {
        self.modules.push((url.into(), Arc::from(bytes)));
        self
    }

    /// Binds dataset band `name`, resolvable in graphs by `name` itself or
    /// any of `aliases` (openEO band names, common names).
    #[must_use]
    pub fn with_band<I, S>(mut self, name: impl Into<String>, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.bands.push(BandBinding {
            name: name.into(),
            aliases: aliases.into_iter().map(Into::into).collect(),
        });
        self
    }

    /// Resolves an openEO band name to the dataset band it binds.
    fn resolve(&self, openeo_name: &str) -> Option<&str> {
        self.bands
            .iter()
            .find(|b| b.name == openeo_name || b.aliases.iter().any(|a| a == openeo_name))
            .map(|b| b.name.as_str())
    }

    /// Every name a graph could use, for "unknown band" diagnostics.
    fn known_names(&self) -> Vec<String> {
        self.bands
            .iter()
            .flat_map(|b| std::iter::once(b.name.clone()).chain(b.aliases.iter().cloned()))
            .collect()
    }
}

/// A compiled product: the executable plan plus the serving metadata the
/// graph implies.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CompiledProduct {
    /// The executable Render IR.
    pub plan: RenderPlan,
    /// The collection the graph loads (validated against the context).
    pub collection: String,
    /// The dataset bands the plan actually reads, in plan-input order —
    /// serving fetches exactly these.
    pub bands: Vec<String>,
    /// The plan spec the graph lowered to — the same vocabulary every
    /// other construction site speaks, so callers can derive the persisted
    /// metadata ([`crate::plan::plan_for`]) without re-reading the ops.
    pub spec: PlanSpec,
    /// The temporal resolution window the graph implies —
    /// `load_collection`'s `temporal_extent` intersected with every
    /// `filter_temporal` on the result path. Granule resolution is
    /// constrained to it (ADR 0015 frame selection: the window selects
    /// *which frames the layer can show*, never how pixels combine).
    /// Open on both sides when the graph says nothing about time. For a
    /// product over more than one source this is the **hull** of the
    /// sources' windows (a frame either branch can show lies inside it);
    /// the per-branch windows are in [`sources`](Self::sources).
    pub window: TimeRange,
    /// Every `load_collection` the plan reads through, in first-use
    /// order, each with its own resolution window (ADR 0022). One entry
    /// for the single-source chain; two for a `merge_cubes` join —
    /// serving resolves one granule per entry and reads the plan inputs
    /// whose [`BandInput::source`](crate::ir::BandInput) names it.
    pub sources: Vec<SourceWindow>,
    /// The `run_udf` module bytes the plan's UDF stage was registered
    /// from — what the publish motion persists in the module store under
    /// the stage's `code_hash` (ADR 0018, #204). `None` for graphs
    /// without `run_udf`.
    pub udf_module: Option<Vec<u8>>,
}

/// One source of a compiled product: a `load_collection` node and the
/// resolution window its branch implies (ADR 0022).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceWindow {
    /// The `load_collection` node id (what `band@node` inputs name).
    pub node: String,
    /// `temporal_extent` intersected with every `filter_temporal` on the
    /// branch; open on both sides when the branch says nothing.
    pub window: TimeRange,
}

/// The supported process ids, for diagnostics.
const SUPPORTED: &str = "load_collection, filter_temporal, reduce_dimension, array_element, \
     add, subtract, multiply, divide, linear_scale_range, ndvi, merge_cubes, run_udf, \
     save_result";

/// Why a process graph could not be compiled. Every variant names the
/// offending node; the Display strings are user-facing diagnostics and are
/// pinned by snapshot tests.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum CompileError {
    /// The JSON is not a process graph at all (not an object of nodes,
    /// a node without `process_id`, ...).
    #[error("malformed process graph: {detail}")]
    Malformed {
        /// What was structurally wrong.
        detail: String,
    },
    /// No node carries `"result": true`.
    #[error("no result node: exactly one node must set \"result\": true")]
    NoResult,
    /// More than one node carries `"result": true`.
    #[error("multiple result nodes ({nodes:?}): exactly one node may set \"result\": true")]
    MultipleResults {
        /// The offending node ids, sorted.
        nodes: Vec<String>,
    },
    /// A `from_node` reference names a node that does not exist in its
    /// graph.
    #[error("node `{node}` references `{target}` via from_node, but no such node exists")]
    DanglingReference {
        /// The referencing node.
        node: String,
        /// The missing target.
        target: String,
    },
    /// The graph is cyclic (openEO graphs are DAGs).
    #[error("cycle detected through node `{node}`: process graphs must be acyclic")]
    Cycle {
        /// A node on the cycle.
        node: String,
    },
    /// A process outside the supported v0 subset.
    #[error("node `{node}`: unsupported process `{id}` — the supported subset is: {SUPPORTED}")]
    UnsupportedProcess {
        /// The node using the process.
        node: String,
        /// The unsupported process id.
        id: String,
    },
    /// `load_collection` names a collection the context does not serve.
    #[error(
        "node `{node}`: unknown collection `{id}` (this product compiles against `{expected}`)"
    )]
    UnknownCollection {
        /// The `load_collection` node.
        node: String,
        /// The requested collection id.
        id: String,
        /// The collection the context binds.
        expected: String,
    },
    /// A band name that resolves to no bound dataset band.
    #[error("node `{node}`: unknown band `{band}` — known bands and aliases: {available:?}")]
    UnknownBand {
        /// The referencing node.
        node: String,
        /// The unresolvable name.
        band: String,
        /// Every name the context binds.
        available: Vec<String>,
    },
    /// A required argument is missing (including `bands` on
    /// `load_collection`, which v0 requires explicitly).
    #[error("node `{node}` ({process}): missing required argument `{argument}`")]
    MissingArgument {
        /// The node.
        node: String,
        /// Its process id.
        process: String,
        /// The missing argument.
        argument: String,
    },
    /// An argument is present but unusable.
    #[error("node `{node}` ({process}): invalid argument `{argument}`: {detail}")]
    InvalidArgument {
        /// The node.
        node: String,
        /// Its process id.
        process: String,
        /// The offending argument.
        argument: String,
        /// Why it was rejected.
        detail: String,
    },
    /// A value of the wrong kind flowed into a process (cube where a
    /// scalar was expected, a gray cube into a reducer, ...).
    #[error("node `{node}` ({process}): type mismatch — expected {expected}, got {got}")]
    TypeMismatch {
        /// The node.
        node: String,
        /// Its process id.
        process: String,
        /// What the process needed.
        expected: String,
        /// What actually arrived.
        got: String,
    },
    /// A temporal window that can select nothing: an interval whose end
    /// is not after its start, or one disjoint from the window already
    /// applied upstream. Frame selection (ADR 0015) resolves exactly one
    /// granule inside the window, so a provably empty window is rejected
    /// at compile time instead of 404ing every tile forever.
    #[error("node `{node}` ({process}): empty temporal window: {detail}")]
    EmptyTemporalWindow {
        /// The node carrying the interval.
        node: String,
        /// Its process id.
        process: String,
        /// Why the window is empty.
        detail: String,
    },
    /// `merge_cubes` without an `overlap_resolver` (ADR 0022): the spec's
    /// default — fail on overlap — would reject every pixel of a join
    /// whose whole point is the overlap, so the resolver is required.
    #[error(
        "node `{node}` (merge_cubes): overlap_resolver is required — a child graph over x \
         (from cube1) and y (from cube2) producing one value per pixel pair, e.g. subtract"
    )]
    MissingResolver {
        /// The `merge_cubes` node.
        node: String,
    },
    /// `filter_temporal`'s `dimension` names a dimension that is not the
    /// temporal dimension (the spec's `DimensionNotAvailable` exception).
    #[error(
        "node `{node}` (filter_temporal): dimension `{dimension}` does not exist — \
         the temporal dimension is `t` (DimensionNotAvailable)"
    )]
    DimensionNotAvailable {
        /// The `filter_temporal` node.
        node: String,
        /// The unknown dimension name.
        dimension: String,
    },
    /// `from_parameter` names a parameter the surrounding scope does not
    /// define.
    #[error("node `{node}`: from_parameter `{name}` is not defined in this scope")]
    UnknownParameter {
        /// The referencing node.
        node: String,
        /// The unknown parameter name.
        name: String,
    },
    /// The result node is not `save_result`.
    #[error(
        "result node `{node}` is `{process}`: the graph must end in save_result (format \"png\")"
    )]
    UnsavedResult {
        /// The result node.
        node: String,
        /// Its process id.
        process: String,
    },
    /// `run_udf` in a context with no [`UdfRegistrar`]: this deployment
    /// wires no UDF executor / module store (ADR 0018), so the process is
    /// not offered — the served process list omits it too.
    #[error(
        "node `{node}`: run_udf is not available — this deployment wires no UDF executor or \
         module store (ADR 0018)"
    )]
    UdfUnavailable {
        /// The `run_udf` node.
        node: String,
    },
    /// The registrar rejected the `udf` module (`docs/udf-abi/v1.md`:
    /// imports, missing exports, ABI version, memory cap, arity refusal,
    /// or a misbehaving probe). The source is the port's typed reason.
    #[error(
        "node `{node}` (run_udf): invalid argument `udf`: module rejected at registration: \
         {source}"
    )]
    UdfModule {
        /// The `run_udf` node.
        node: String,
        /// The registrar's reason.
        #[source]
        source: UdfError,
    },
    /// The module's pinned output arity is one the IR cannot render:
    /// 1 plane is gray, 3 is RGB, anything else has no image meaning
    /// (ADR 0018; other arities are a v2 reopen condition).
    #[error(
        "node `{node}` (run_udf): invalid argument `udf`: module declares {output_planes} \
         output planes for {input_planes} input bands; v1 renders 1 (gray) or 3 (RGB)"
    )]
    UdfOutputPlanes {
        /// The `run_udf` node.
        node: String,
        /// The input arity the module was registered against.
        input_planes: u32,
        /// The arity it pinned.
        output_planes: u32,
    },
}

/// One loaded band: the openEO label the graph uses and the dataset band
/// it resolved to.
#[derive(Debug, Clone, PartialEq)]
struct LoadedBand {
    label: String,
    dataset: String,
}

/// A data cube flowing through the top-level graph.
#[derive(Debug, Clone, PartialEq)]
struct Cube {
    /// `None` = the bands dimension was reduced away (gray).
    kind: CubeKind,
    /// A pending `linear_scale_range`, at most one in v0.
    rescale: Option<(f64, f64)>,
    /// The palette requested by `save_result`'s `colormap` option
    /// (gray results only; `None` = grayscale).
    colormap: Option<Colormap>,
    /// The sources this cube reads through, each with its resolution
    /// window so far: `temporal_extent` intersected with every
    /// `filter_temporal` applied to the cube (ADR 0015 frame selection;
    /// open on both sides = unconstrained). One entry until a
    /// `merge_cubes` joins two (ADR 0022).
    sources: Vec<SourceWindow>,
}

#[derive(Debug, Clone, PartialEq)]
enum CubeKind {
    Multi(Vec<LoadedBand>),
    Gray(Expr),
    /// A `run_udf` result: the registered stage over the dataset bands it
    /// receives, in request-plane order. The module bytes ride along
    /// (shared, never copied per memo hit) so [`compile`] can hand them
    /// out for persistence.
    Udf {
        bands: Vec<String>,
        stage: UdfStage,
        module: Arc<[u8]>,
    },
}

/// What a node evaluates to.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Cube(Cube),
    Scalar(Expr),
    /// The reducer's `data` parameter: the band array.
    Bands(Vec<LoadedBand>),
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Self::Cube(cube) => match cube.kind {
                CubeKind::Multi(_) => "a multi-band data cube",
                CubeKind::Gray(_) => "a gray (reduced) data cube",
                CubeKind::Udf { .. } => "a UDF result data cube",
            },
            Self::Scalar(_) => "a scalar value",
            Self::Bands(_) => "a band array",
        }
    }
}

/// One parsed node of a (sub-)graph.
struct Node<'a> {
    process_id: &'a str,
    arguments: Option<&'a serde_json::Map<String, Json>>,
}

/// A parsed (sub-)graph: nodes by id, plus the single result node.
struct Graph<'a> {
    nodes: BTreeMap<&'a str, Node<'a>>,
    result: &'a str,
}

impl<'a> Graph<'a> {
    /// Parses `{"process_graph": {...}}` or a bare node map, validating
    /// node shape and the exactly-one-result rule.
    fn parse(json: &'a Json) -> Result<Self, CompileError> {
        let obj = json.as_object().ok_or_else(|| CompileError::Malformed {
            detail: "a process graph must be a JSON object".into(),
        })?;
        let nodes_obj = match obj.get("process_graph") {
            Some(inner) => inner.as_object().ok_or_else(|| CompileError::Malformed {
                detail: "\"process_graph\" must be an object of nodes".into(),
            })?,
            None => obj,
        };
        let mut nodes = BTreeMap::new();
        let mut results = Vec::new();
        for (id, node) in nodes_obj {
            let node = node.as_object().ok_or_else(|| CompileError::Malformed {
                detail: format!("node `{id}` must be an object"),
            })?;
            let process_id = node
                .get("process_id")
                .and_then(Json::as_str)
                .ok_or_else(|| CompileError::Malformed {
                    detail: format!("node `{id}` has no string \"process_id\""),
                })?;
            let arguments = match node.get("arguments") {
                None => None,
                Some(args) => Some(args.as_object().ok_or_else(|| CompileError::Malformed {
                    detail: format!("node `{id}`: \"arguments\" must be an object"),
                })?),
            };
            if node.get("result").and_then(Json::as_bool) == Some(true) {
                results.push(id.as_str());
            }
            nodes.insert(
                id.as_str(),
                Node {
                    process_id,
                    arguments,
                },
            );
        }
        match results.as_slice() {
            [] => Err(CompileError::NoResult),
            [result] => Ok(Self { nodes, result }),
            many => Err(CompileError::MultipleResults {
                nodes: many.iter().map(|&s| s.to_owned()).collect(),
            }),
        }
    }
}

/// Per-graph evaluation state: memoized node values with in-progress
/// marking for cycle detection.
enum NodeState {
    InProgress,
    /// Boxed: a memoized value can carry a UDF stage plus its module
    /// handle, and the map holds one per node.
    Done(Box<Value>),
}

/// One evaluation scope: a graph plus the parameters its nodes may
/// reference via `from_parameter`.
struct Scope<'a> {
    graph: Graph<'a>,
    params: BTreeMap<&'a str, Value>,
    states: BTreeMap<&'a str, NodeState>,
}

struct Compiler<'a> {
    ctx: &'a CompileContext,
    /// The graph loads the collection through more than one node
    /// (ADR 0022): loaded bands are qualified `band@node`.
    multi_source: bool,
}

impl<'a> Compiler<'a> {
    /// Evaluates node `id` in `scope`, memoized, detecting cycles.
    fn eval_node(&self, scope: &mut Scope<'a>, id: &'a str) -> Result<Value, CompileError> {
        match scope.states.get(id) {
            Some(NodeState::InProgress) => {
                return Err(CompileError::Cycle { node: id.into() });
            }
            Some(NodeState::Done(value)) => return Ok((**value).clone()),
            None => {}
        }
        scope.states.insert(id, NodeState::InProgress);
        let value = self.eval_process(scope, id)?;
        scope
            .states
            .insert(id, NodeState::Done(Box::new(value.clone())));
        Ok(value)
    }

    fn eval_process(&self, scope: &mut Scope<'a>, id: &'a str) -> Result<Value, CompileError> {
        let process = scope.graph.nodes[id].process_id;
        match process {
            "load_collection" => self.load_collection(scope, id),
            "filter_temporal" => self.filter_temporal(scope, id),
            "reduce_dimension" => self.reduce_dimension(scope, id),
            "ndvi" => self.ndvi(scope, id),
            "merge_cubes" => self.merge_cubes(scope, id),
            "linear_scale_range" => self.linear_scale_range(scope, id),
            "run_udf" => self.run_udf(scope, id),
            "save_result" => self.save_result(scope, id),
            "array_element" => self.array_element(scope, id),
            "add" | "subtract" | "multiply" | "divide" => self.arithmetic(scope, id, process),
            other => Err(CompileError::UnsupportedProcess {
                node: id.into(),
                id: other.into(),
            }),
        }
    }

    // --- argument plumbing -------------------------------------------------

    /// Resolves one argument value: references evaluate, JSON literals
    /// pass through as `Err(json)` for the caller to interpret.
    fn resolve_ref(
        &self,
        scope: &mut Scope<'a>,
        node: &'a str,
        json: &'a Json,
    ) -> Result<Option<Value>, CompileError> {
        if let Some(obj) = json.as_object() {
            if let Some(target) = obj.get("from_node").and_then(Json::as_str) {
                let Some((&target, _)) = scope.graph.nodes.get_key_value(target) else {
                    return Err(CompileError::DanglingReference {
                        node: node.into(),
                        target: target.into(),
                    });
                };
                return self.eval_node(scope, target).map(Some);
            }
            if let Some(name) = obj.get("from_parameter").and_then(Json::as_str) {
                return match scope.params.get(name) {
                    Some(value) => Ok(Some(value.clone())),
                    None => Err(CompileError::UnknownParameter {
                        node: node.into(),
                        name: name.into(),
                    }),
                };
            }
        }
        Ok(None)
    }

    fn arg(node: &Node<'a>, name: &str) -> Option<&'a Json> {
        node.arguments.and_then(|args| args.get(name))
    }

    fn require(scope: &Scope<'a>, node: &'a str, name: &str) -> Result<&'a Json, CompileError> {
        let n = &scope.graph.nodes[node];
        Self::arg(n, name).ok_or_else(|| CompileError::MissingArgument {
            node: node.into(),
            process: n.process_id.into(),
            argument: name.into(),
        })
    }

    /// A required argument that must evaluate to a cube.
    fn cube_arg(
        &self,
        scope: &mut Scope<'a>,
        node: &'a str,
        name: &str,
    ) -> Result<Cube, CompileError> {
        let json = Self::require(scope, node, name)?;
        let process = scope.graph.nodes[node].process_id;
        match self.resolve_ref(scope, node, json)? {
            Some(Value::Cube(cube)) => Ok(cube),
            Some(other) => Err(CompileError::TypeMismatch {
                node: node.into(),
                process: process.into(),
                expected: "a data cube".into(),
                got: other.kind().into(),
            }),
            None => Err(CompileError::TypeMismatch {
                node: node.into(),
                process: process.into(),
                expected: "a data cube (a from_node reference)".into(),
                got: format!("a JSON literal ({json})"),
            }),
        }
    }

    /// A required argument that must evaluate to a scalar expression:
    /// a number literal, or a reference to a scalar-producing node.
    fn scalar_arg(
        &self,
        scope: &mut Scope<'a>,
        node: &'a str,
        name: &str,
    ) -> Result<Expr, CompileError> {
        let json = Self::require(scope, node, name)?;
        let process = scope.graph.nodes[node].process_id;
        if let Some(value) = self.resolve_ref(scope, node, json)? {
            return match value {
                Value::Scalar(expr) => Ok(expr),
                other => Err(CompileError::TypeMismatch {
                    node: node.into(),
                    process: process.into(),
                    expected: "a scalar (number, band element, or arithmetic result)".into(),
                    got: other.kind().into(),
                }),
            };
        }
        json.as_f64()
            .map(Expr::Const)
            .ok_or_else(|| CompileError::TypeMismatch {
                node: node.into(),
                process: process.into(),
                expected: "a scalar (number, band element, or arithmetic result)".into(),
                got: format!("a JSON literal ({json})"),
            })
    }

    fn number_arg(scope: &Scope<'a>, node: &'a str, name: &str) -> Result<f64, CompileError> {
        let json = Self::require(scope, node, name)?;
        json.as_f64().ok_or_else(|| CompileError::InvalidArgument {
            node: node.into(),
            process: scope.graph.nodes[node].process_id.into(),
            argument: name.into(),
            detail: format!("expected a number, got {json}"),
        })
    }

    // --- processes ---------------------------------------------------------

    fn load_collection(&self, scope: &mut Scope<'a>, node: &'a str) -> Result<Value, CompileError> {
        let id = Self::require(scope, node, "id")?;
        let id = id.as_str().ok_or_else(|| CompileError::InvalidArgument {
            node: node.into(),
            process: "load_collection".into(),
            argument: "id".into(),
            detail: format!("expected a collection id string, got {id}"),
        })?;
        if id != self.ctx.collection {
            return Err(CompileError::UnknownCollection {
                node: node.into(),
                id: id.into(),
                expected: self.ctx.collection.clone(),
            });
        }
        let n = &scope.graph.nodes[node];
        let bands = match Self::arg(n, "bands") {
            None | Some(Json::Null) => {
                return Err(CompileError::MissingArgument {
                    node: node.into(),
                    process: "load_collection".into(),
                    argument: "bands".into(),
                });
            }
            Some(bands) => bands
                .as_array()
                .ok_or_else(|| CompileError::InvalidArgument {
                    node: node.into(),
                    process: "load_collection".into(),
                    argument: "bands".into(),
                    detail: format!("expected an array of band names, got {bands}"),
                })?,
        };
        let mut loaded = Vec::with_capacity(bands.len());
        for band in bands {
            let label = band.as_str().ok_or_else(|| CompileError::InvalidArgument {
                node: node.into(),
                process: "load_collection".into(),
                argument: "bands".into(),
                detail: format!("band names must be strings, got {band}"),
            })?;
            // `@` is the source qualifier of a multi-source plan (ADR
            // 0022): a label carrying it could never be told apart.
            if label.contains('@') {
                return Err(CompileError::InvalidArgument {
                    node: node.into(),
                    process: "load_collection".into(),
                    argument: "bands".into(),
                    detail: format!("band name `{label}` contains `@`, the source qualifier"),
                });
            }
            let dataset = self
                .ctx
                .resolve(label)
                .ok_or_else(|| CompileError::UnknownBand {
                    node: node.into(),
                    band: label.into(),
                    available: self.ctx.known_names(),
                })?;
            // More than one source in the graph: qualify the band by its
            // load node so the plan's inputs stay unique (ADR 0022).
            let dataset = if self.multi_source {
                format!("{dataset}@{node}")
            } else {
                dataset.to_owned()
            };
            loaded.push(LoadedBand {
                label: label.into(),
                dataset,
            });
        }
        let window = match Self::arg(n, "temporal_extent") {
            None | Some(Json::Null) => TimeRange::default(),
            Some(extent) => {
                Self::temporal_interval(extent, node, "load_collection", "temporal_extent")?
            }
        };
        Ok(Value::Cube(Cube {
            kind: CubeKind::Multi(loaded),
            rescale: None,
            colormap: None,
            sources: vec![SourceWindow {
                node: node.into(),
                window,
            }],
        }))
    }

    /// `filter_temporal`: narrows the cube's resolution window to its
    /// intersection with `extent` (frame-selection semantics, ADR 0015 —
    /// pixels are untouched, so it composes anywhere a cube flows).
    fn filter_temporal(&self, scope: &mut Scope<'a>, node: &'a str) -> Result<Value, CompileError> {
        let mut cube = self.cube_arg(scope, node, "data")?;
        let n = &scope.graph.nodes[node];
        if let Some(dimension) = Self::arg(n, "dimension")
            && !dimension.is_null()
        {
            let name = dimension
                .as_str()
                .ok_or_else(|| CompileError::InvalidArgument {
                    node: node.into(),
                    process: "filter_temporal".into(),
                    argument: "dimension".into(),
                    detail: format!("expected a dimension name string or null, got {dimension}"),
                })?;
            if name != "t" {
                return Err(CompileError::DimensionNotAvailable {
                    node: node.into(),
                    dimension: name.into(),
                });
            }
        }
        let extent = Self::require(scope, node, "extent")?;
        let filter = Self::temporal_interval(extent, node, "filter_temporal", "extent")?;
        for source in &mut cube.sources {
            source.window = Self::intersect_windows(&source.window, &filter, node)?;
        }
        Ok(Value::Cube(cube))
    }

    // --- temporal windows --------------------------------------------------

    /// Parses one bound of a temporal interval: an RFC 3339 UTC (`Z`)
    /// date-time, a date (`YYYY-MM-DD`), or a year (`YYYY`) — the three
    /// string forms of the spec's `temporal-interval` subtype, narrowed
    /// to UTC. Dates and years denote their first instant.
    fn temporal_instant(value: &str) -> Option<Datetime> {
        let expanded = if value.len() == 4 {
            format!("{value}-01-01T00:00:00Z")
        } else if value.len() == 10 {
            format!("{value}T00:00:00Z")
        } else {
            value.to_owned()
        };
        Datetime::new(expanded).ok()
    }

    /// Parses a `temporal-interval` argument into the inclusive
    /// [`TimeRange`] the catalog speaks: the interval is left-closed per
    /// the spec, so the (exclusive) end becomes inclusive by stepping
    /// back one millisecond — the domain's comparison resolution.
    fn temporal_interval(
        json: &Json,
        node: &'a str,
        process: &str,
        argument: &str,
    ) -> Result<TimeRange, CompileError> {
        let invalid = |detail: String| CompileError::InvalidArgument {
            node: node.into(),
            process: process.into(),
            argument: argument.into(),
            detail,
        };
        let pair = json
            .as_array()
            .filter(|items| items.len() == 2)
            .ok_or_else(|| {
                invalid(format!(
                    "expected a temporal interval [start, end], got {json}"
                ))
            })?;
        let bound = |value: &Json| -> Result<Option<Datetime>, CompileError> {
            match value {
                Json::Null => Ok(None),
                Json::String(s) => Self::temporal_instant(s).map(Some).ok_or_else(|| {
                    invalid(format!(
                        "`{s}` is not an RFC 3339 UTC (Z) date-time, date, or year"
                    ))
                }),
                other => Err(invalid(format!(
                    "interval bounds must be temporal strings or null, got {other}"
                ))),
            }
        };
        let (start, end) = (bound(&pair[0])?, bound(&pair[1])?);
        if start.is_none() && end.is_none() {
            return Err(invalid(
                "an interval open on both sides selects everything — \
                 use null for the whole argument instead of [null, null]"
                    .into(),
            ));
        }
        let end = match end {
            None => None,
            Some(end) => {
                let end_ms = end.to_unix_millis();
                if start
                    .as_ref()
                    .is_some_and(|start| start.to_unix_millis() >= end_ms)
                {
                    return Err(CompileError::EmptyTemporalWindow {
                        node: node.into(),
                        process: process.into(),
                        detail: format!(
                            "the left-closed interval [{}, {}) contains no instant — \
                             the end must be after the start",
                            start.as_ref().map_or_else(String::new, ToString::to_string),
                            end
                        ),
                    });
                }
                Some(Datetime::from_unix_millis(end_ms - 1).map_err(|_| {
                    CompileError::EmptyTemporalWindow {
                        node: node.into(),
                        process: process.into(),
                        detail: format!("no representable instant precedes the end {end}"),
                    }
                })?)
            }
        };
        Ok(TimeRange { start, end })
    }

    /// The intersection of two resolution windows: the later start, the
    /// earlier end. A provably empty result (disjoint windows) is a
    /// compile-time diagnostic — no granule could ever resolve.
    fn intersect_windows(
        current: &TimeRange,
        filter: &TimeRange,
        node: &'a str,
    ) -> Result<TimeRange, CompileError> {
        let later = |a: Option<&Datetime>, b: Option<&Datetime>| match (a, b) {
            (Some(a), Some(b)) => {
                if a.to_unix_millis() >= b.to_unix_millis() {
                    Some(a.clone())
                } else {
                    Some(b.clone())
                }
            }
            (bound, None) | (None, bound) => bound.cloned(),
        };
        let earlier = |a: Option<&Datetime>, b: Option<&Datetime>| match (a, b) {
            (Some(a), Some(b)) => {
                if a.to_unix_millis() <= b.to_unix_millis() {
                    Some(a.clone())
                } else {
                    Some(b.clone())
                }
            }
            (bound, None) | (None, bound) => bound.cloned(),
        };
        let start = later(current.start.as_ref(), filter.start.as_ref());
        let end = earlier(current.end.as_ref(), filter.end.as_ref());
        if let (Some(s), Some(e)) = (&start, &end)
            && s.to_unix_millis() > e.to_unix_millis()
        {
            return Err(CompileError::EmptyTemporalWindow {
                node: node.into(),
                process: "filter_temporal".into(),
                detail: format!(
                    "this interval does not overlap the window already applied — \
                     the combined window ({s} .. {e}) selects nothing"
                ),
            });
        }
        Ok(TimeRange { start, end })
    }

    fn reduce_dimension(
        &self,
        scope: &mut Scope<'a>,
        node: &'a str,
    ) -> Result<Value, CompileError> {
        let cube = self.cube_arg(scope, node, "data")?;
        let sources = cube.sources.clone();
        let bands = Self::unscaled_multi(cube, node, "reduce_dimension")?;

        let dimension = Self::require(scope, node, "dimension")?;
        if dimension.as_str() != Some("bands") {
            return Err(CompileError::InvalidArgument {
                node: node.into(),
                process: "reduce_dimension".into(),
                argument: "dimension".into(),
                detail: format!(
                    "only the \"bands\" dimension can be reduced in v0, got {dimension}"
                ),
            });
        }

        let reducer = Self::require(scope, node, "reducer")?;
        let sub = Graph::parse(reducer).map_err(|err| match err {
            CompileError::Malformed { detail } => CompileError::InvalidArgument {
                node: node.into(),
                process: "reduce_dimension".into(),
                argument: "reducer".into(),
                detail: format!("not a child process graph: {detail}"),
            },
            other => other,
        })?;
        let mut params = BTreeMap::new();
        params.insert("data", Value::Bands(bands));
        let mut sub_scope = Scope {
            graph: sub,
            params,
            states: BTreeMap::new(),
        };
        let result_id = sub_scope.graph.result;
        let value = self.eval_node(&mut sub_scope, result_id)?;
        match value {
            Value::Scalar(expr) => Ok(Value::Cube(Cube {
                kind: CubeKind::Gray(expr),
                rescale: None,
                colormap: None,
                sources,
            })),
            other => Err(CompileError::TypeMismatch {
                node: node.into(),
                process: "reduce_dimension".into(),
                expected: "a reducer producing a scalar per pixel".into(),
                got: other.kind().into(),
            }),
        }
    }

    fn ndvi(&self, scope: &mut Scope<'a>, node: &'a str) -> Result<Value, CompileError> {
        let cube = self.cube_arg(scope, node, "data")?;
        let sources = cube.sources.clone();
        let bands = Self::unscaled_multi(cube, node, "ndvi")?;
        let n = &scope.graph.nodes[node];
        if let Some(target) = Self::arg(n, "target_band")
            && !target.is_null()
        {
            return Err(CompileError::InvalidArgument {
                node: node.into(),
                process: "ndvi".into(),
                argument: "target_band".into(),
                detail: "keeping the bands dimension is not supported in v0 \
                             (omit target_band; the result is gray)"
                    .into(),
            });
        }
        let band_param = |name: &str, default: &str| -> Result<String, CompileError> {
            let requested = match Self::arg(n, name) {
                None | Some(Json::Null) => default.to_owned(),
                Some(v) => v
                    .as_str()
                    .ok_or_else(|| CompileError::InvalidArgument {
                        node: node.into(),
                        process: "ndvi".into(),
                        argument: name.into(),
                        detail: format!("expected a band name string, got {v}"),
                    })?
                    .to_owned(),
            };
            self.find_band(&bands, &requested, node)
        };
        let nir = band_param("nir", "nir")?;
        let red = band_param("red", "red")?;
        let expr = ndvi_expr(nir, red);
        Ok(Value::Cube(Cube {
            kind: CubeKind::Gray(expr),
            rescale: None,
            colormap: None,
            sources,
        }))
    }

    /// `merge_cubes` (ADR 0022): two gray, unscaled cubes from different
    /// `load_collection` nodes, joined per pixel pair by the required
    /// `overlap_resolver` child graph over `x` / `y` — the module docs
    /// carry the profile's narrowing.
    fn merge_cubes(&self, scope: &mut Scope<'a>, node: &'a str) -> Result<Value, CompileError> {
        let n = &scope.graph.nodes[node];
        if let Some(context) = Self::arg(n, "context")
            && !context.is_null()
        {
            return Err(CompileError::InvalidArgument {
                node: node.into(),
                process: "merge_cubes".into(),
                argument: "context".into(),
                detail: "not admitted in the bounded profile: the resolver sees only x and y"
                    .into(),
            });
        }
        let cube1 = self.cube_arg(scope, node, "cube1")?;
        let cube2 = self.cube_arg(scope, node, "cube2")?;
        let (x, mut sources) = Self::gray_unscaled(cube1, node, "cube1")?;
        let (y, more) = Self::gray_unscaled(cube2, node, "cube2")?;
        if let Some(shared) = more
            .iter()
            .find(|s| sources.iter().any(|t| t.node == s.node))
        {
            return Err(CompileError::InvalidArgument {
                node: node.into(),
                process: "merge_cubes".into(),
                argument: "cube2".into(),
                detail: format!(
                    "both cubes load the collection through node `{}` — load it once per \
                     frame (a second load_collection with its own temporal_extent) so each \
                     branch resolves its own granule",
                    shared.node
                ),
            });
        }
        sources.extend(more);
        let n = &scope.graph.nodes[node];
        let resolver = match Self::arg(n, "overlap_resolver") {
            None | Some(Json::Null) => {
                return Err(CompileError::MissingResolver { node: node.into() });
            }
            Some(resolver) => resolver,
        };
        let sub = Graph::parse(resolver).map_err(|err| match err {
            CompileError::Malformed { detail } => CompileError::InvalidArgument {
                node: node.into(),
                process: "merge_cubes".into(),
                argument: "overlap_resolver".into(),
                detail: format!("not a child process graph: {detail}"),
            },
            other => other,
        })?;
        let mut params = BTreeMap::new();
        params.insert("x", Value::Scalar(x));
        params.insert("y", Value::Scalar(y));
        let mut sub_scope = Scope {
            graph: sub,
            params,
            states: BTreeMap::new(),
        };
        let result_id = sub_scope.graph.result;
        match self.eval_node(&mut sub_scope, result_id)? {
            Value::Scalar(expr) => Ok(Value::Cube(Cube {
                kind: CubeKind::Gray(expr),
                rescale: None,
                colormap: None,
                sources,
            })),
            other => Err(CompileError::TypeMismatch {
                node: node.into(),
                process: "merge_cubes".into(),
                expected: "an overlap_resolver producing a scalar per pixel pair".into(),
                got: other.kind().into(),
            }),
        }
    }

    /// A `merge_cubes` input: gray (one value per pixel) and not yet
    /// scaled, with the sources it reads through.
    fn gray_unscaled(
        cube: Cube,
        node: &'a str,
        argument: &str,
    ) -> Result<(Expr, Vec<SourceWindow>), CompileError> {
        let reject = |detail: String| CompileError::InvalidArgument {
            node: node.into(),
            process: "merge_cubes".into(),
            argument: argument.into(),
            detail,
        };
        if cube.rescale.is_some() {
            return Err(reject(
                "expected an unscaled data cube, got an already-scaled one — apply \
                 linear_scale_range to the merged result instead"
                    .into(),
            ));
        }
        match cube.kind {
            CubeKind::Gray(expr) => Ok((expr, cube.sources)),
            CubeKind::Multi(bands) => Err(reject(format!(
                "expected a gray (reduced) data cube, got a data cube with {} bands — reduce \
                 to one value per pixel first (ndvi or reduce_dimension)",
                bands.len()
            ))),
            CubeKind::Udf { .. } => Err(reject(
                "expected a gray band-math cube, got a UDF result — a UDF result cannot feed \
                 merge_cubes in v1"
                    .into(),
            )),
        }
    }

    fn linear_scale_range(
        &self,
        scope: &mut Scope<'a>,
        node: &'a str,
    ) -> Result<Value, CompileError> {
        let mut cube = self.cube_arg(scope, node, "x")?;
        if cube.rescale.is_some() {
            return Err(CompileError::InvalidArgument {
                node: node.into(),
                process: "linear_scale_range".into(),
                argument: "x".into(),
                detail: "the cube is already scaled; chained linear_scale_range is not \
                         supported in v0"
                    .into(),
            });
        }
        let min = Self::number_arg(scope, node, "inputMin")?;
        let max = Self::number_arg(scope, node, "inputMax")?;
        if min.partial_cmp(&max) != Some(std::cmp::Ordering::Less) {
            return Err(CompileError::InvalidArgument {
                node: node.into(),
                process: "linear_scale_range".into(),
                argument: "inputMin".into(),
                detail: format!("degenerate input range: inputMin {min} >= inputMax {max}"),
            });
        }
        let n = &scope.graph.nodes[node];
        let out = |name: &str, default: f64| -> Result<f64, CompileError> {
            match Self::arg(n, name) {
                None => Ok(default),
                Some(v) => v.as_f64().ok_or_else(|| CompileError::InvalidArgument {
                    node: node.into(),
                    process: "linear_scale_range".into(),
                    argument: name.into(),
                    detail: format!("expected a number, got {v}"),
                }),
            }
        };
        let (out_min, out_max) = (out("outputMin", 0.0)?, out("outputMax", 1.0)?);
        if (out_min, out_max) != (0.0, 255.0) {
            return Err(CompileError::InvalidArgument {
                node: node.into(),
                process: "linear_scale_range".into(),
                argument: "outputMin".into(),
                detail: format!(
                    "the Render IR quantizes to 8-bit; the output range must be exactly \
                     0..255, got {out_min}..{out_max}"
                ),
            });
        }
        cube.rescale = Some((min, max));
        Ok(Value::Cube(cube))
    }

    /// `run_udf` (ADR 0018): registers the module through the context's
    /// [`UdfRegistrar`] and types the result cube by the pinned output
    /// arity — the module docs carry the profile's narrowing.
    fn run_udf(&self, scope: &mut Scope<'a>, node: &'a str) -> Result<Value, CompileError> {
        let cube = self.cube_arg(scope, node, "data")?;
        let sources = cube.sources.clone();
        let loaded = Self::unscaled_multi(cube, node, "run_udf")?;
        let n = &scope.graph.nodes[node];
        let invalid = |argument: &str, detail: String| CompileError::InvalidArgument {
            node: node.into(),
            process: "run_udf".into(),
            argument: argument.into(),
            detail,
        };

        // Runtime "wasm", version "1", only (ADR 0018) — the spec's
        // InvalidRuntime / InvalidVersion exceptions.
        let runtime = Self::require(scope, node, "runtime")?;
        if runtime.as_str() != Some("wasm") {
            return Err(invalid(
                "runtime",
                format!("only the \"wasm\" runtime is supported (InvalidRuntime), got {runtime}"),
            ));
        }
        if let Some(version) = Self::arg(n, "version")
            && !version.is_null()
            && version.as_str() != Some("1")
        {
            return Err(invalid(
                "version",
                format!(
                    "the wasm runtime speaks version \"1\" only (InvalidVersion), got {version}"
                ),
            ));
        }
        // `context` rides through verbatim as the stage's params.
        let params = match Self::arg(n, "context") {
            None | Some(Json::Null) => Json::Null,
            Some(context) => context.clone(),
        };

        let Some(registrar) = self.ctx.udf.as_ref() else {
            return Err(CompileError::UdfUnavailable { node: node.into() });
        };
        let udf = Self::require(scope, node, "udf")?;
        let udf = udf
            .as_str()
            .ok_or_else(|| invalid("udf", format!("expected a string, got {udf}")))?;
        let module = match UdfSource::parse(udf).map_err(|detail| invalid("udf", detail))? {
            UdfSource::Inline(bytes) => Arc::from(bytes),
            UdfSource::Remote(url) => self
                .ctx
                .modules
                .iter()
                .find(|(key, _)| *key == url)
                .map(|(_, bytes)| Arc::clone(bytes))
                .ok_or_else(|| {
                    invalid(
                        "udf",
                        format!("remote module `{url}` was not fetched for this compile motion"),
                    )
                })?,
        };
        if module.len() > MODULE_MAX_BYTES {
            return Err(invalid(
                "udf",
                format!(
                    "module of {} bytes exceeds the {MODULE_MAX_BYTES}-byte limit",
                    module.len()
                ),
            ));
        }

        // One request plane per loaded dataset band, deduplicated in load
        // order — exactly the plan inputs `plan_for` derives, so the arity
        // the module is registered against is the arity it will see.
        let mut bands: Vec<String> = Vec::with_capacity(loaded.len());
        for band in &loaded {
            if !bands.contains(&band.dataset) {
                bands.push(band.dataset.clone());
            }
        }
        let input_planes = u32::try_from(bands.len()).map_err(|_| {
            invalid(
                "data",
                format!("{} input bands cannot form a request", bands.len()),
            )
        })?;
        let registration = registrar
            .register(&module, input_planes)
            .map_err(|source| CompileError::UdfModule {
                node: node.into(),
                source,
            })?;
        if !matches!(registration.output_planes, 1 | 3) {
            return Err(CompileError::UdfOutputPlanes {
                node: node.into(),
                input_planes,
                output_planes: registration.output_planes,
            });
        }
        let stage = UdfStage::new(registration.code_hash, registration.output_planes, params);
        Ok(Value::Cube(Cube {
            kind: CubeKind::Udf {
                bands,
                stage,
                module,
            },
            rescale: None,
            colormap: None,
            sources,
        }))
    }

    fn save_result(&self, scope: &mut Scope<'a>, node: &'a str) -> Result<Value, CompileError> {
        let cube = self.cube_arg(scope, node, "data")?;
        let format = Self::require(scope, node, "format")?;
        let format_str = format.as_str().unwrap_or_default();
        // The spec makes format matching case-insensitive.
        if !format_str.eq_ignore_ascii_case("png") {
            return Err(CompileError::InvalidArgument {
                node: node.into(),
                process: "save_result".into(),
                argument: "format".into(),
                detail: format!("only \"png\" is supported in v0, got {format}"),
            });
        }
        let n = &scope.graph.nodes[node];
        let mut cube = cube;
        if let Some(options) = Self::arg(n, "options") {
            cube.colormap = Self::save_options(options, &cube, node)?;
        }
        Ok(Value::Cube(cube))
    }

    /// Parses `save_result`'s `options`: only `colormap` is supported
    /// (module docs), and only on a gray result.
    fn save_options(
        options: &Json,
        cube: &Cube,
        node: &'a str,
    ) -> Result<Option<Colormap>, CompileError> {
        let invalid = |detail: String| CompileError::InvalidArgument {
            node: node.into(),
            process: "save_result".into(),
            argument: "options".into(),
            detail,
        };
        let object = options.as_object().ok_or_else(|| {
            invalid(format!(
                "the only supported format option is \"colormap\", got {options}"
            ))
        })?;
        if let Some(key) = object.keys().find(|key| key.as_str() != "colormap") {
            return Err(invalid(format!(
                "the only supported format option is \"colormap\", got key \"{key}\""
            )));
        }
        let Some(requested) = object.get("colormap") else {
            return Ok(None);
        };
        let colormap = match requested.as_str() {
            Some("grayscale") => Colormap::Grayscale,
            Some("viridis") => Colormap::Viridis,
            Some("magma") => Colormap::Magma,
            Some("rdylgn") => Colormap::RdYlGn,
            _ => {
                return Err(invalid(format!(
                    "unknown colormap {requested}: expected one of \"grayscale\", \
                     \"viridis\", \"magma\", \"rdylgn\""
                )));
            }
        };
        match cube.kind {
            CubeKind::Multi(_) => Err(invalid(
                "a colormap maps one gray value per pixel; it cannot apply to a \
                 multi-band (composite) result — reduce to gray first"
                    .into(),
            )),
            CubeKind::Udf { .. } => Err(invalid(
                "a colormap cannot apply to a run_udf result in v1 — UDF output renders \
                 directly (1 plane gray, 3 planes RGB)"
                    .into(),
            )),
            CubeKind::Gray(_) => Ok(Some(colormap)),
        }
    }

    fn array_element(&self, scope: &mut Scope<'a>, node: &'a str) -> Result<Value, CompileError> {
        let data = Self::require(scope, node, "data")?;
        let bands = match self.resolve_ref(scope, node, data)? {
            Some(Value::Bands(bands)) => bands,
            Some(other) => {
                return Err(CompileError::TypeMismatch {
                    node: node.into(),
                    process: "array_element".into(),
                    expected: "the reducer's band array (from_parameter \"data\")".into(),
                    got: other.kind().into(),
                });
            }
            None => {
                return Err(CompileError::TypeMismatch {
                    node: node.into(),
                    process: "array_element".into(),
                    expected: "the reducer's band array (from_parameter \"data\")".into(),
                    got: format!("a JSON literal ({data})"),
                });
            }
        };
        let n = &scope.graph.nodes[node];
        let index = Self::arg(n, "index").filter(|v| !v.is_null());
        let label = Self::arg(n, "label").filter(|v| !v.is_null());
        let dataset = match (index, label) {
            (Some(_), Some(_)) => {
                return Err(CompileError::InvalidArgument {
                    node: node.into(),
                    process: "array_element".into(),
                    argument: "index".into(),
                    detail: "only one of index and label may be set \
                             (ArrayElementParameterConflict)"
                        .into(),
                });
            }
            (None, None) => {
                return Err(CompileError::MissingArgument {
                    node: node.into(),
                    process: "array_element".into(),
                    argument: "index (or label)".into(),
                });
            }
            (Some(index), None) => {
                let i = index.as_u64().and_then(|i| usize::try_from(i).ok());
                let i = i.ok_or_else(|| CompileError::InvalidArgument {
                    node: node.into(),
                    process: "array_element".into(),
                    argument: "index".into(),
                    detail: format!("expected a non-negative integer, got {index}"),
                })?;
                let band = bands.get(i).ok_or_else(|| CompileError::InvalidArgument {
                    node: node.into(),
                    process: "array_element".into(),
                    argument: "index".into(),
                    detail: format!(
                        "index {i} is out of bounds for the {} loaded bands",
                        bands.len()
                    ),
                })?;
                band.dataset.clone()
            }
            (None, Some(label)) => {
                let label = label
                    .as_str()
                    .ok_or_else(|| CompileError::InvalidArgument {
                        node: node.into(),
                        process: "array_element".into(),
                        argument: "label".into(),
                        detail: format!("expected a band label string, got {label}"),
                    })?;
                self.find_band(&bands, label, node)?
            }
        };
        Ok(Value::Scalar(Expr::Band(dataset)))
    }

    fn arithmetic(
        &self,
        scope: &mut Scope<'a>,
        node: &'a str,
        process: &str,
    ) -> Result<Value, CompileError> {
        let x = self.scalar_arg(scope, node, "x")?;
        let y = self.scalar_arg(scope, node, "y")?;
        let op = match process {
            "add" => BinaryOp::Add,
            "subtract" => BinaryOp::Sub,
            "multiply" => BinaryOp::Mul,
            _ => BinaryOp::Div,
        };
        Ok(Value::Scalar(Expr::Binary {
            op,
            lhs: Box::new(x),
            rhs: Box::new(y),
        }))
    }

    // --- shared checks -----------------------------------------------------

    /// The cube must still carry its bands dimension and must not have been
    /// scaled yet (scaling happens after reduction/composition in v0).
    fn unscaled_multi(
        cube: Cube,
        node: &'a str,
        process: &str,
    ) -> Result<Vec<LoadedBand>, CompileError> {
        if cube.rescale.is_some() {
            return Err(CompileError::TypeMismatch {
                node: node.into(),
                process: process.into(),
                expected: "an unscaled data cube (apply linear_scale_range after reducing)".into(),
                got: "an already-scaled data cube".into(),
            });
        }
        match cube.kind {
            CubeKind::Multi(bands) => Ok(bands),
            CubeKind::Gray(_) => Err(CompileError::TypeMismatch {
                node: node.into(),
                process: process.into(),
                expected: "a data cube with a bands dimension".into(),
                got: "a gray (reduced) data cube".into(),
            }),
            // One run_udf per graph in v1: its result feeds nothing but
            // linear_scale_range and save_result.
            CubeKind::Udf { .. } => Err(CompileError::TypeMismatch {
                node: node.into(),
                process: process.into(),
                expected: "a loaded (multi-band) data cube — v1 runs one run_udf per graph, \
                           after any reduction"
                    .into(),
                got: "a UDF result data cube".into(),
            }),
        }
    }

    /// Finds a loaded band by openEO label, matching the label itself or
    /// anything the context resolves to the same dataset band (common
    /// names, dataset band names).
    fn find_band(
        &self,
        bands: &[LoadedBand],
        name: &str,
        node: &str,
    ) -> Result<String, CompileError> {
        let direct = bands.iter().find(|b| b.label == name);
        let via_ctx = || {
            let dataset = self.ctx.resolve(name)?;
            bands.iter().find(|b| {
                b.dataset
                    .rsplit_once('@')
                    .map_or(b.dataset.as_str(), |(d, _)| d)
                    == dataset
            })
        };
        direct
            .or_else(via_ctx)
            .map(|b| b.dataset.clone())
            .ok_or_else(|| CompileError::UnknownBand {
                node: node.into(),
                band: name.into(),
                available: bands.iter().map(|b| b.label.clone()).collect(),
            })
    }
}

/// Compiles an openEO process graph against `ctx` into an executable
/// [`RenderPlan`] plus its serving metadata. See the module docs for the
/// exact conformance statement (the supported v0 subset).
///
/// # Errors
///
/// Any [`CompileError`]: a malformed or cyclic graph, missing/duplicate
/// result nodes, dangling references, unsupported processes, unknown
/// collections/bands/parameters, or per-process argument and type errors —
/// each naming the offending node.
pub fn compile(graph: &Json, ctx: &CompileContext) -> Result<CompiledProduct, CompileError> {
    let parsed = Graph::parse(graph)?;
    let result_id = parsed.result;
    let result_process = parsed.nodes[result_id].process_id;
    if result_process != "save_result" {
        return Err(CompileError::UnsavedResult {
            node: result_id.into(),
            process: result_process.into(),
        });
    }
    let mut scope = Scope {
        graph: parsed,
        params: BTreeMap::new(),
        states: BTreeMap::new(),
    };
    let multi_source = scope
        .graph
        .nodes
        .values()
        .filter(|n| n.process_id == "load_collection")
        .count()
        > 1;
    let compiler = Compiler { ctx, multi_source };
    let value = compiler.eval_node(&mut scope, result_id)?;
    let Value::Cube(cube) = value else {
        // save_result always returns its data cube; anything else is a bug
        // upstream in this module, but keep it a typed error.
        return Err(CompileError::TypeMismatch {
            node: result_id.into(),
            process: "save_result".into(),
            expected: "a data cube".into(),
            got: value.kind().into(),
        });
    };

    let mut udf_module = None;
    let spec = match cube.kind {
        CubeKind::Gray(expr) => PlanSpec::BandMath {
            expr,
            rescale: cube.rescale,
            colormap: cube.colormap.unwrap_or(Colormap::Grayscale),
        },
        CubeKind::Udf {
            bands,
            stage,
            module,
        } => {
            udf_module = Some(module.to_vec());
            PlanSpec::Udf {
                bands,
                stage,
                rescale: cube.rescale,
            }
        }
        CubeKind::Multi(loaded) => {
            let [r, g, b] = loaded.as_slice() else {
                return Err(CompileError::TypeMismatch {
                    node: result_id.into(),
                    process: "save_result".into(),
                    expected: "exactly 3 bands for an RGB composite (or reduce to gray first)"
                        .into(),
                    got: format!("a data cube with {} bands", loaded.len()),
                });
            };
            PlanSpec::Composite {
                r: r.dataset.clone(),
                g: g.dataset.clone(),
                b: b.dataset.clone(),
                rescale: cube.rescale,
            }
        }
    };

    let (plan, _) = plan_for(&spec);
    let bands = plan.inputs.iter().map(|input| input.name.clone()).collect();
    let window = hull(cube.sources.iter().map(|source| &source.window));
    Ok(CompiledProduct {
        plan,
        collection: ctx.collection.clone(),
        bands,
        spec,
        window,
        sources: cube.sources,
        udf_module,
    })
}

/// The hull of resolution windows: the earliest start and the latest end,
/// a side open as soon as any window is open there. What a product over
/// more than one source reports as its window (ADR 0022) — a frame either
/// branch can show lies inside it.
fn hull<'w>(windows: impl Iterator<Item = &'w TimeRange>) -> TimeRange {
    let mut windows = windows.peekable();
    let Some(first) = windows.next() else {
        return TimeRange::default();
    };
    let mut start = first.start.clone();
    let mut end = first.end.clone();
    for window in windows {
        start = match (start, &window.start) {
            (Some(a), Some(b)) if a.to_unix_millis() <= b.to_unix_millis() => Some(a),
            (Some(_), Some(b)) => Some(b.clone()),
            _ => None,
        };
        end = match (end, &window.end) {
            (Some(a), Some(b)) if a.to_unix_millis() >= b.to_unix_millis() => Some(a),
            (Some(_), Some(b)) => Some(b.clone()),
            _ => None,
        };
    }
    TimeRange { start, end }
}
