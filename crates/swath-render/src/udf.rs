// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `run_udf` executor **port** (ADR 0018, issue #201): the seam
//! between the Render IR's [`PixelOp::Udf`](crate::ir::PixelOp::Udf)
//! stage and whatever actually runs the sandboxed WASM module.
//!
//! This crate defines only the trait; the wasmtime adapter (#203) lives in
//! `swath-udf-wasmtime` behind it, so **swath-render never depends on
//! wasmtime** — the same port/adapter posture every other boundary in the
//! engine keeps (ARCHITECTURE.md, ADR 0013). The seam is also ADR 0018's
//! rollback lever: moving UDF execution onto a bounded worker pool, or
//! withdrawing it from the live tile path entirely, is a wiring change at
//! this trait, not an IR redesign.
//!
//! A [`UdfStage`] names the module by **content hash** — the module bytes
//! never enter the IR; the module store (`swath_core::udf`, #204) owns
//! hash → bytes. [`NoUdf`] is the default executor for deployments with
//! no UDF support wired: it refuses every stage, and since
//! [`eval`](crate::ir::eval) consults the executor only when it reaches a
//! `Udf` op, plans without UDF stages never touch it.
//!
//! The compile motion has its own seam, [`UdfRegistrar`] (#204): the
//! process compiler hands it module bytes and gets back the content hash
//! and the pinned output arity — registration (zero-import check, the
//! four v1 exports, the ABI version probe, `swath_udf_output_planes`) is
//! the adapter's, so the compiler validates modules without knowing
//! wasmtime exists. [`UdfSource`] is the grammar of `run_udf`'s `udf`
//! argument in the Swath profile: inline `data:application/wasm;base64`
//! or an `http(s)` URL.

use serde::{Deserialize, Serialize};
use swath_core::udf::MODULE_MAX_BYTES;

use crate::warp::WarpedBuffer;

/// One `run_udf` stage of a [`RenderPlan`](crate::ir::RenderPlan): the
/// sandboxed module (by content hash), its pinned output arity, and its
/// opaque parameters.
///
/// This is IR **data** — serde round-trippable, snapshot-pinned — and the
/// argument [`UdfExecutor::run`] receives. The module bytes themselves are
/// deliberately absent (plans stay small, cacheable, and hash-addressed);
/// the module store (#204) owns hash → bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UdfStage {
    /// Lowercase sha256 hex of the registered module bytes — the
    /// content-addressed module identity (ADR 0018). Registration (#204)
    /// computes it; serving resolves it; the IR only carries it.
    pub code_hash: String,
    /// How many planes the module produces per tile — the
    /// `swath_udf_output_planes` answer pinned at registration
    /// (`docs/udf-abi/v1.md`). [`eval`](crate::ir::eval) renders 1 plane
    /// as gray and 3 as RGB; other counts are a plan error.
    pub output_planes: u32,
    /// Opaque UDF parameters (openEO `run_udf`'s `context` argument),
    /// carried verbatim for the executor. `Null` when the caller passed
    /// none. Part of the plan — and therefore of cache identity (#205).
    pub params: serde_json::Value,
}

impl UdfStage {
    /// A stage running the module named by `code_hash`, producing
    /// `output_planes` planes, with `params` as its opaque parameters.
    #[must_use]
    pub fn new(
        code_hash: impl Into<String>,
        output_planes: u32,
        params: serde_json::Value,
    ) -> Self {
        Self {
            code_hash: code_hash.into(),
            output_planes,
            params,
        }
    }
}

/// Why a UDF stage could not be executed. Distinct from
/// [`PlanError`](crate::ir::PlanError)'s structural variants: these are
/// the executor port's failures, wrapped into the plan taxonomy as
/// [`PlanError::Udf`](crate::ir::PlanError::Udf).
///
/// The taxonomy is pinned by issue #203 and implemented by the wasmtime
/// adapter (`swath-udf-wasmtime`); every ADR 0018 failure mode is a
/// distinct variant — a loud per-tile error, never a hung worker and
/// never a stringly-typed catch-all. Registration-motion failures
/// ([`InvalidModule`](Self::InvalidModule) through
/// [`UnsupportedAbiVersion`](Self::UnsupportedAbiVersion)) and tile-path
/// failures ([`UnknownModule`](Self::UnknownModule) onward) share the one
/// enum because the port has one error channel. `#[non_exhaustive]`: a
/// future adapter may still add variants without breaking consumers.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum UdfError {
    /// No UDF executor is wired into this deployment ([`NoUdf`]): the
    /// plan names a module, but nothing can run it. Serve wiring arrives
    /// with #205; until then every UDF plan refuses loudly here.
    #[error("no UDF executor is configured: plan names module `{code_hash}` (ADR 0018)")]
    NotConfigured {
        /// The module the plan asked for.
        code_hash: String,
    },
    /// The WASM runtime is unavailable on this host: the deterministic
    /// engine configuration (ADR 0018) was rejected at executor
    /// construction. A startup-time failure — the configuration is
    /// static, so this only trips on an unsupported host.
    #[error("WASM runtime unavailable: {detail}")]
    NoRuntime {
        /// The runtime's explanation.
        detail: String,
    },
    /// Registration: the bytes are not a valid WASM module the engine
    /// can compile.
    #[error("module does not compile: {detail}")]
    InvalidModule {
        /// The compiler's explanation.
        detail: String,
    },
    /// Registration: the module imports something. Zero-import modules
    /// are ADR 0018's structural determinism guarantee — with no imports
    /// there is nothing nondeterministic to call.
    #[error("module imports `{module}`.`{name}`: zero-import rule (ADR 0018)")]
    ForbiddenImport {
        /// The import's module namespace.
        module: String,
        /// The imported symbol.
        name: String,
    },
    /// Registration: a required ABI v1 export (`swath_udf_abi`,
    /// `swath_udf_output_planes`, `swath_udf_alloc`, `swath_udf_run`, or
    /// the linear `memory`) is absent or has the wrong signature.
    #[error("module export `{export}` missing or mis-typed: {detail}")]
    MissingExport {
        /// The export that failed the check.
        export: String,
        /// What was found instead.
        detail: String,
    },
    /// Registration: `swath_udf_abi` answered something other than `1`
    /// (`docs/udf-abi/v1.md`: the next incompatible contract is a new
    /// version, never a silent blend).
    #[error("module speaks UDF ABI {got}, this host speaks 1")]
    UnsupportedAbiVersion {
        /// The version the module claimed.
        got: i32,
    },
    /// Registration: `swath_udf_output_planes(input_planes)` answered
    /// `<= 0` — the module refuses this input arity (`docs/udf-abi/v1.md`:
    /// rejected at registration). Which *positive* arities render is the
    /// IR's rule (1 or 3), checked by the compiler, not here.
    #[error("module answers {output_planes} output planes for {input_planes} input planes")]
    UnsupportedArity {
        /// The input arity the module was asked about.
        input_planes: u32,
        /// Its answer.
        output_planes: i32,
    },
    /// Tile path: the stage names a module hash the executor has not
    /// compiled. Compilation happens at the publish/preview motion, never
    /// the tile path — an unknown hash is refused, not compiled inline.
    #[error("module `{code_hash}` is not registered with this executor")]
    UnknownModule {
        /// The hash the plan asked for.
        code_hash: String,
    },
    /// Tile path: the input planes cannot be encoded as an ABI v1
    /// request (no planes, zero dimensions, mismatched plane shapes, or
    /// a request too large for the wire). Host-side and unreachable
    /// through a validated plan — kept loud rather than panicking.
    #[error("input planes cannot form a v1 request: {detail}")]
    InvalidRequest {
        /// Which precondition failed.
        detail: String,
    },
    /// The deterministic fuel budget — ADR 0018's primary bound — ran
    /// out. Reproducible: identical inputs consume identical fuel, so
    /// this either always trips for a given tile or never does.
    #[error("UDF exhausted its fuel budget of {budget}")]
    FuelExhausted {
        /// The budget the call was given.
        budget: u64,
    },
    /// The wall-clock epoch deadline — the backstop that keeps ADR
    /// 0012's inline-render posture alive under a pathological module —
    /// interrupted the call.
    #[error("UDF exceeded the {deadline_ms} ms epoch deadline")]
    EpochDeadline {
        /// The deadline, in milliseconds.
        deadline_ms: u64,
    },
    /// The 64 MiB per-instance memory cap (ADR 0018): the module
    /// declares more than the cap, instantiation failed, or the guest
    /// could not allocate the request buffer (`swath_udf_alloc` answered
    /// `0` — growth past the cap is denied, so allocation failure is the
    /// shape a memory overrun takes inside a conforming guest).
    #[error("UDF memory limit: {detail}")]
    MemoryLimit {
        /// Which allocation failed, and how.
        detail: String,
    },
    /// The module trapped for any reason other than fuel or the epoch
    /// deadline (unreachable, out-of-bounds access, stack overflow, a
    /// guest panic — the guest kit's panic handler traps deliberately).
    #[error("UDF trapped: {detail}")]
    Trap {
        /// The runtime's trap description.
        detail: String,
    },
    /// The guest declared failure: `swath_udf_run` answered `0` (the
    /// ABI's own error signal — e.g. the module refuses the input
    /// arity, or the UDF itself returned an error).
    #[error("module `{code_hash}` declared failure (swath_udf_run answered 0)")]
    GuestFailure {
        /// The module that refused.
        code_hash: String,
    },
    /// The guest's answer violated the ABI framing: an out-of-bounds
    /// allocation or response pointer, or a response buffer that does
    /// not decode as a v1 response. Always a typed error, never UB —
    /// every guest byte is bounds-checked and strictly parsed.
    #[error("malformed UDF response: {detail}")]
    MalformedOutput {
        /// What failed to parse or bounds-check.
        detail: String,
    },
    /// The response header's plane count disagrees with the stage's
    /// pinned `swath_udf_output_planes` answer.
    #[error("UDF answered {actual} output planes, stage pins {declared}")]
    OutputPlanes {
        /// Planes the stage declares (pinned at registration).
        declared: u32,
        /// Planes the response header claimed.
        actual: u32,
    },
}

/// The executor port: runs one [`UdfStage`] over the plan's warped input
/// planes, returning the module's output planes.
///
/// # Contract (checked by the caller)
///
/// [`eval`](crate::ir::eval) verifies the returned planes — exactly
/// [`UdfStage::output_planes`] buffers, each tile-shaped — and enforces
/// the ABI's host post-conditions (`docs/udf-abi/v1.md`): output validity
/// is `ANDed` with input validity, and non-finite values the executor
/// claims valid are canonicalized to invalid. An adapter may enforce them
/// too, but the IR never trusts it to.
///
/// Synchronous by design: render compute runs inline on the calling task
/// (ADR 0012); the fuel/epoch budgets bounding a call are the adapter's
/// job (#203).
pub trait UdfExecutor {
    /// Runs `stage`'s module over `inputs` (one request plane per buffer,
    /// in plan-input order), returning its output planes in order.
    ///
    /// # Errors
    ///
    /// Any [`UdfError`]: the executor could not run the module at all, or
    /// the module failed. Per-pixel data conditions are never errors —
    /// they belong in the returned buffers' validity masks.
    fn run(&self, stage: &UdfStage, inputs: &[WarpedBuffer])
    -> Result<Vec<WarpedBuffer>, UdfError>;
}

/// The default executor: **no UDF support**. Every stage is refused with
/// [`UdfError::NotConfigured`]; plans without UDF stages evaluate exactly
/// as before and never consult it.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoUdf;

impl UdfExecutor for NoUdf {
    fn run(
        &self,
        stage: &UdfStage,
        _inputs: &[WarpedBuffer],
    ) -> Result<Vec<WarpedBuffer>, UdfError> {
        Err(UdfError::NotConfigured {
            code_hash: stage.code_hash.clone(),
        })
    }
}

/// What registering a module pins (`docs/udf-abi/v1.md`): its content
/// hash and its output arity for the input arity it was registered
/// against — exactly the two facts a [`UdfStage`] carries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UdfRegistration {
    /// Lowercase sha256 hex of the module bytes ([`swath_core::udf::code_hash`]).
    pub code_hash: String,
    /// The module's `swath_udf_output_planes` answer for the registered
    /// input arity, `> 0` by contract.
    pub output_planes: u32,
}

impl UdfRegistration {
    /// A registration of the module hashing to `code_hash` that pins
    /// `output_planes`.
    #[must_use]
    pub fn new(code_hash: impl Into<String>, output_planes: u32) -> Self {
        Self {
            code_hash: code_hash.into(),
            output_planes,
        }
    }
}

/// The compile-motion port (#204): validates and registers module bytes
/// so a [`UdfStage`] can be built. The process compiler
/// ([`crate::process`]) calls it once per `run_udf` node; the tile path
/// never does. Registration is the ADR 0018 gate — zero imports, the four
/// v1 exports with the v1 signatures, an exported memory within the cap,
/// `swath_udf_abi() == 1` — plus the output-arity probe for the graph's
/// input band count. The wasmtime adapter (`swath-udf-wasmtime`)
/// implements it over its module LRU, so a registered module is also
/// runnable by the same executor.
pub trait UdfRegistrar: Send + Sync {
    /// Registers `bytes` for a plan feeding it `input_planes` planes.
    ///
    /// # Errors
    ///
    /// Any registration [`UdfError`] (`InvalidModule`, `ForbiddenImport`,
    /// `MissingExport`, `MemoryLimit`, `UnsupportedAbiVersion`,
    /// `UnsupportedArity`), or an execution error if the probe calls
    /// themselves misbehave.
    fn register(&self, bytes: &[u8], input_planes: u32) -> Result<UdfRegistration, UdfError>;
}

/// Where a graph's `run_udf` node says its module comes from — the Swath
/// profile of the `udf` argument (openEO says "source code, URL, or
/// workspace path"; the profile narrows to exactly these two forms).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UdfSource {
    /// `data:application/wasm;base64,…` — the module bytes, decoded here.
    Inline(Vec<u8>),
    /// An absolute `http://` / `https://` URL, fetched once at the compile
    /// motion by the caller ([`swath_core::udf::ModuleFetcher`]) — the
    /// compiler itself never fetches.
    Remote(String),
}

/// The `data:` URL prefix the profile accepts, verbatim.
const DATA_PREFIX: &str = "data:application/wasm;base64,";

impl UdfSource {
    /// Parses a `udf` argument. The decoded inline payload is bounded by
    /// [`MODULE_MAX_BYTES`], refused by *encoded* length before decoding.
    ///
    /// # Errors
    ///
    /// A human-readable reason: neither form, an inline payload that is
    /// not base64 or is over the limit.
    pub fn parse(udf: &str) -> Result<Self, String> {
        use base64::Engine as _;
        if let Some(encoded) = udf.strip_prefix(DATA_PREFIX) {
            // Every 4 base64 chars decode to at most 3 bytes; anything
            // longer than the limit's encoding cannot fit.
            let max_encoded = MODULE_MAX_BYTES.div_ceil(3) * 4;
            if encoded.len() > max_encoded {
                return Err(format!(
                    "inline module is over the {MODULE_MAX_BYTES}-byte limit \
                     ({} base64 characters)",
                    encoded.len()
                ));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|err| format!("inline module is not valid base64: {err}"))?;
            if bytes.len() > MODULE_MAX_BYTES {
                return Err(format!(
                    "inline module of {} bytes exceeds the {MODULE_MAX_BYTES}-byte limit",
                    bytes.len()
                ));
            }
            return Ok(Self::Inline(bytes));
        }
        if udf.starts_with("http://") || udf.starts_with("https://") {
            return Ok(Self::Remote(udf.to_owned()));
        }
        Err(format!(
            "expected `{DATA_PREFIX}…` or an absolute http(s) URL, got {}",
            summary(udf)
        ))
    }
}

/// A short, quotable rendering of an argument for diagnostics (a `data:`
/// URL can be megabytes; never echo it whole).
fn summary(udf: &str) -> String {
    const LIMIT: usize = 48;
    if udf.chars().count() <= LIMIT {
        format!("`{udf}`")
    } else {
        let head: String = udf.chars().take(LIMIT).collect();
        format!("`{head}…` ({} characters)", udf.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use swath_core::udf::MODULE_MAX_BYTES;

    use super::UdfSource;

    fn data_url(bytes: &[u8]) -> String {
        format!(
            "data:application/wasm;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    #[test]
    fn inline_data_urls_decode() {
        let magic = b"\0asm\x01\0\0\0";
        assert_eq!(
            UdfSource::parse(&data_url(magic)),
            Ok(UdfSource::Inline(magic.to_vec()))
        );
    }

    #[test]
    fn remote_urls_are_named_not_fetched() {
        for url in ["http://example.org/m.wasm", "https://example.org/m.wasm"] {
            assert_eq!(UdfSource::parse(url), Ok(UdfSource::Remote(url.to_owned())));
        }
    }

    #[test]
    fn other_forms_are_refused_with_the_grammar() {
        for bad in [
            "udf.py",
            "def apply(x):\n  return x",
            "ftp://example.org/m.wasm",
            "data:text/plain;base64,aGk=",
            "data:application/wasm,raw",
        ] {
            let err = UdfSource::parse(bad).expect_err(bad);
            assert!(err.contains("data:application/wasm;base64,"), "{err}");
            assert!(err.contains("http(s)"), "{err}");
        }
    }

    #[test]
    fn long_arguments_are_summarized_in_diagnostics() {
        let long = "x".repeat(10_000);
        let err = UdfSource::parse(&long).expect_err("refused");
        assert!(
            err.len() < 200,
            "diagnostic must not echo the argument: {err}"
        );
        assert!(err.contains("10000 characters"), "{err}");
    }

    #[test]
    fn invalid_base64_is_refused() {
        let err = UdfSource::parse("data:application/wasm;base64,***").expect_err("refused");
        assert!(err.contains("not valid base64"), "{err}");
    }

    /// The limit is enforced on the encoded length, before decoding: an
    /// oversized payload never allocates its decoded form.
    #[test]
    fn oversized_inline_modules_are_refused_by_encoded_length() {
        let over = "A".repeat(MODULE_MAX_BYTES.div_ceil(3) * 4 + 4);
        let err =
            UdfSource::parse(&format!("data:application/wasm;base64,{over}")).expect_err("refused");
        assert!(err.contains("over the"), "{err}");
        // Exactly at the limit decodes (8 MiB of zeros).
        let at_limit = data_url(&vec![0u8; MODULE_MAX_BYTES]);
        assert!(matches!(
            UdfSource::parse(&at_limit),
            Ok(UdfSource::Inline(bytes)) if bytes.len() == MODULE_MAX_BYTES
        ));
    }
}
