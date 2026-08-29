// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

#![doc = include_str!("../README.md")]
//!
//! # Writing a module
//!
//! A real module is one `#![no_std]` crate with `crate-type = ["cdylib"]`,
//! built for `wasm32-unknown-unknown` (worked examples: the swath repo's
//! `examples/udf/`); everything below the attributes is identical there,
//! and compiles — and runs — on the host too:
//!
//! ```
//! extern crate alloc;
//! use swath_udf_guest::{swath_udf, Plane, Request, Response};
//!
//! swath_udf! {
//!     output_planes: |input_planes| if input_planes == 1 { 1 } else { 0 },
//!     run: double,
//! }
//!
//! fn double(request: &Request) -> Option<Response> {
//!     let input = &request.planes[0];
//!     let mut out = Plane::invalid(request.pixels());
//!     for i in 0..request.pixels() {
//!         out.values[i] = input.values[i] * 2.0;
//!         out.validity[i] = input.validity[i];
//!     }
//!     Some(Response { planes: alloc::vec![out] })
//! }
//!
//! # fn main() {
//! // The macro produced the ABI exports; two are plain callable functions.
//! assert_eq!(swath_udf_abi(), 1);
//! assert_eq!(swath_udf_output_planes(1), 1);
//! assert_eq!(swath_udf_output_planes(2), 0);
//! # }
//! ```
//!
//! The kit is strict on the wire (manifest-v1 discipline): unknown header
//! fields, a wrong `abi`, or a payload length disagreeing with the header
//! are decode errors, and `swath_udf_run` answers any of them with `0` —
//! the guest-declared failure the host turns into a loud per-tile error.
//!

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

/// The ABI version this kit implements (`swath_udf_abi` returns it).
pub const ABI_VERSION: i32 = 1;

/// One plane: `width x height` row-major `f64` samples plus a parallel
/// `u8` validity mask (`1` valid, `0` invalid). Invalid pixels hold `0.0`
/// by the `WarpedBuffer` convention.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    /// Row-major sample values.
    pub values: Vec<f64>,
    /// Parallel validity flags (`1` valid, `0` invalid).
    pub validity: Vec<u8>,
}

impl Plane {
    /// An all-invalid plane of `len` pixels (values `0.0`, validity `0`) —
    /// the natural starting point for an output plane.
    #[must_use]
    pub fn invalid(len: usize) -> Self {
        Self {
            values: alloc::vec![0.0; len],
            validity: alloc::vec![0; len],
        }
    }
}

/// A decoded request buffer: the tile dimensions and the input planes,
/// in header order.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Tile width in pixels.
    pub width: u32,
    /// Tile height in pixels.
    pub height: u32,
    /// Input planes, in header order.
    pub planes: Vec<Plane>,
}

impl Request {
    /// Pixels per plane (`width x height`).
    #[must_use]
    pub fn pixels(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }
}

/// The guest's answer: output planes at the request's dimensions (v1 is
/// dimension-preserving). The plane count must equal the module's pinned
/// `swath_udf_output_planes` answer — the host enforces it.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// Output planes.
    pub planes: Vec<Plane>,
}

/// What can go wrong on the wire. Every variant is a loud failure:
/// `swath_udf_run` answers `0` and the host raises a per-tile UDF error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AbiError {
    /// The buffer is shorter than its own length prefix or header claims.
    Truncated,
    /// The JSON header is not the strict shape v1 defines.
    MalformedHeader,
    /// A header field this ABI version does not define (deny-unknown).
    UnknownField,
    /// A required header field is absent.
    MissingField,
    /// A header field appears twice.
    DuplicateField,
    /// `abi` is not `1`.
    AbiVersion,
    /// A dimension or plane count is zero or does not fit the platform.
    BadDimensions,
    /// The payload length disagrees with the header.
    PayloadLength,
    /// A plane's buffer lengths disagree with the stated dimensions.
    PlaneShape,
}

impl core::fmt::Display for AbiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            Self::Truncated => "buffer truncated",
            Self::MalformedHeader => "malformed JSON header",
            Self::UnknownField => "unknown header field (deny-unknown)",
            Self::MissingField => "missing required header field",
            Self::DuplicateField => "duplicate header field",
            Self::AbiVersion => "header abi is not 1",
            Self::BadDimensions => "zero or oversize dimensions",
            Self::PayloadLength => "payload length disagrees with header",
            Self::PlaneShape => "plane buffers disagree with dimensions",
        };
        f.write_str(msg)
    }
}

impl core::error::Error for AbiError {}

// --- strict header parsing ------------------------------------------------

/// Parses a flat JSON object of unsigned-integer fields, strictly: exactly
/// the keys in `keys` may appear (deny-unknown), each at most once, values
/// are plain non-negative decimal integers, and nothing may follow the
/// closing brace. Returns the parsed values in `keys` order.
fn parse_uint_object<const N: usize>(
    bytes: &[u8],
    keys: [&str; N],
) -> Result<[Option<u64>; N], AbiError> {
    let mut out = [None; N];
    let mut pos = 0usize;
    let skip_ws = |pos: &mut usize| {
        while let Some(b) = bytes.get(*pos) {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                *pos += 1;
            } else {
                break;
            }
        }
    };
    let expect = |pos: &mut usize, ch: u8| -> Result<(), AbiError> {
        if bytes.get(*pos) == Some(&ch) {
            *pos += 1;
            Ok(())
        } else {
            Err(AbiError::MalformedHeader)
        }
    };
    skip_ws(&mut pos);
    expect(&mut pos, b'{')?;
    loop {
        skip_ws(&mut pos);
        expect(&mut pos, b'"')?;
        let key_start = pos;
        while let Some(&b) = bytes.get(pos) {
            if b == b'"' {
                break;
            }
            // Known keys are plain ASCII; escapes cannot appear in them.
            if b == b'\\' {
                return Err(AbiError::MalformedHeader);
            }
            pos += 1;
        }
        let key = &bytes[key_start..pos];
        expect(&mut pos, b'"')?;
        skip_ws(&mut pos);
        expect(&mut pos, b':')?;
        skip_ws(&mut pos);
        let digit_start = pos;
        while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
            pos += 1;
        }
        if pos == digit_start || pos - digit_start > 19 {
            return Err(AbiError::MalformedHeader);
        }
        let mut value: u64 = 0;
        for &d in &bytes[digit_start..pos] {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(u64::from(d - b'0')))
                .ok_or(AbiError::MalformedHeader)?;
        }
        let slot = keys
            .iter()
            .position(|k| k.as_bytes() == key)
            .ok_or(AbiError::UnknownField)?;
        if out[slot].replace(value).is_some() {
            return Err(AbiError::DuplicateField);
        }
        skip_ws(&mut pos);
        match bytes.get(pos) {
            Some(b',') => pos += 1,
            Some(b'}') => {
                pos += 1;
                break;
            }
            _ => return Err(AbiError::MalformedHeader),
        }
    }
    skip_ws(&mut pos);
    if pos != bytes.len() {
        return Err(AbiError::MalformedHeader);
    }
    Ok(out)
}

/// Splits `[u32 header_len (LE)][header][payload]`.
fn split_frame(buf: &[u8]) -> Result<(&[u8], &[u8]), AbiError> {
    let prefix: [u8; 4] = buf
        .get(..4)
        .and_then(|b| b.try_into().ok())
        .ok_or(AbiError::Truncated)?;
    let header_len = u32::from_le_bytes(prefix) as usize;
    let header = buf
        .get(4..4usize.checked_add(header_len).ok_or(AbiError::Truncated)?)
        .ok_or(AbiError::Truncated)?;
    Ok((header, &buf[4 + header_len..]))
}

fn checked_dims(width: u64, height: u64, planes: u64) -> Result<(u32, u32, usize), AbiError> {
    if width == 0 || height == 0 || planes == 0 {
        return Err(AbiError::BadDimensions);
    }
    let width = u32::try_from(width).map_err(|_| AbiError::BadDimensions)?;
    let height = u32::try_from(height).map_err(|_| AbiError::BadDimensions)?;
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(AbiError::BadDimensions)?;
    // 8 value bytes + 1 validity byte per pixel, per plane.
    usize::try_from(planes)
        .ok()
        .and_then(|p| pixels.checked_mul(9)?.checked_mul(p))
        .ok_or(AbiError::BadDimensions)?;
    Ok((width, height, pixels))
}

fn decode_planes(payload: &[u8], pixels: usize, planes: usize) -> Result<Vec<Plane>, AbiError> {
    if payload.len() != planes * pixels * 9 {
        return Err(AbiError::PayloadLength);
    }
    let mut out = Vec::with_capacity(planes);
    for chunk in payload.chunks_exact(pixels * 9) {
        let (value_bytes, validity) = chunk.split_at(pixels * 8);
        let values = value_bytes
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().expect("chunks_exact(8)")))
            .collect();
        out.push(Plane {
            values,
            validity: validity.to_vec(),
        });
    }
    Ok(out)
}

fn encode_planes(out: &mut Vec<u8>, planes: &[Plane], pixels: usize) -> Result<(), AbiError> {
    for plane in planes {
        if plane.values.len() != pixels || plane.validity.len() != pixels {
            return Err(AbiError::PlaneShape);
        }
    }
    for plane in planes {
        for value in &plane.values {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&plane.validity);
    }
    Ok(())
}

fn frame(header: &str, payload_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + header.len() + payload_len);
    out.extend_from_slice(
        &u32::try_from(header.len())
            .expect("tiny header")
            .to_le_bytes(),
    );
    out.extend_from_slice(header.as_bytes());
    out
}

// --- request --------------------------------------------------------------

/// Decodes a request buffer (host -> guest), strictly per the ABI: exact
/// header fields, `abi` = 1, payload length agreeing with the header.
///
/// ```
/// use swath_udf_guest::{Plane, decode_request, encode_request};
///
/// let plane = Plane { values: vec![1.5, -2.0], validity: vec![1, 0] };
/// let buf = encode_request(2, 1, &[plane.clone()]).unwrap();
/// let request = decode_request(&buf).unwrap();
/// assert_eq!((request.width, request.height), (2, 1));
/// assert_eq!(request.planes, vec![plane]);
/// ```
pub fn decode_request(buf: &[u8]) -> Result<Request, AbiError> {
    let (header, payload) = split_frame(buf)?;
    let [abi, width, height, planes] =
        parse_uint_object(header, ["abi", "width", "height", "planes"])?;
    let (abi, width, height, planes) = (
        abi.ok_or(AbiError::MissingField)?,
        width.ok_or(AbiError::MissingField)?,
        height.ok_or(AbiError::MissingField)?,
        planes.ok_or(AbiError::MissingField)?,
    );
    if abi != 1 {
        return Err(AbiError::AbiVersion);
    }
    let (width, height, pixels) = checked_dims(width, height, planes)?;
    let planes = decode_planes(payload, pixels, usize::try_from(planes).expect("checked"))?;
    Ok(Request {
        width,
        height,
        planes,
    })
}

/// Encodes a request buffer (host -> guest). The host-side half of the
/// contract, here so fixture tests and future adapters build requests with
/// the same code the guest kit round-trips against.
pub fn encode_request(width: u32, height: u32, planes: &[Plane]) -> Result<Vec<u8>, AbiError> {
    let (_, _, pixels) = checked_dims(u64::from(width), u64::from(height), planes.len() as u64)?;
    let mut header = String::new();
    write!(
        header,
        "{{\"abi\":1,\"width\":{width},\"height\":{height},\"planes\":{}}}",
        planes.len()
    )
    .expect("write to String cannot fail");
    let mut out = frame(&header, planes.len() * pixels * 9);
    encode_planes(&mut out, planes, pixels)?;
    Ok(out)
}

// --- response -------------------------------------------------------------

/// Encodes a response buffer (guest -> host): header `{"abi":1,"planes":N}`
/// then the planes at the request's `width`/`height`.
pub fn encode_response(width: u32, height: u32, response: &Response) -> Result<Vec<u8>, AbiError> {
    let (_, _, pixels) = checked_dims(
        u64::from(width),
        u64::from(height),
        response.planes.len() as u64,
    )?;
    let mut header = String::new();
    write!(header, "{{\"abi\":1,\"planes\":{}}}", response.planes.len())
        .expect("write to String cannot fail");
    let mut out = frame(&header, response.planes.len() * pixels * 9);
    encode_planes(&mut out, &response.planes, pixels)?;
    Ok(out)
}

/// Decodes a response buffer (guest -> host) at known dimensions — the
/// host-side half, for fixture tests and future adapters.
pub fn decode_response(width: u32, height: u32, buf: &[u8]) -> Result<Response, AbiError> {
    let (header, payload) = split_frame(buf)?;
    let [abi, planes] = parse_uint_object(header, ["abi", "planes"])?;
    let (abi, planes) = (
        abi.ok_or(AbiError::MissingField)?,
        planes.ok_or(AbiError::MissingField)?,
    );
    if abi != 1 {
        return Err(AbiError::AbiVersion);
    }
    let (_, _, pixels) = checked_dims(u64::from(width), u64::from(height), planes)?;
    let planes = decode_planes(payload, pixels, usize::try_from(planes).expect("checked"))?;
    Ok(Response { planes })
}

// --- export plumbing (used by the swath_udf! macro) -----------------------

/// `swath_udf_alloc`'s implementation: `len` writable bytes for the host's
/// request buffer, `0` on failure. Meaningful on `wasm32` (pointers fit
/// `i32`); host builds compile it but must not call it.
#[doc(hidden)]
#[allow(unsafe_code)] // the ABI hands out raw guest pointers by design
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // wasm32 pointers are 32-bit
#[must_use]
pub fn __alloc(len: i32) -> i32 {
    let Ok(len) = usize::try_from(len) else {
        return 0;
    };
    if len == 0 {
        return 0;
    }
    let Ok(layout) = core::alloc::Layout::from_size_align(len, 8) else {
        return 0;
    };
    // SAFETY: layout has non-zero size (len > 0 checked above).
    let ptr = unsafe { alloc::alloc::alloc(layout) };
    ptr as usize as i32
}

/// `swath_udf_run`'s implementation: decode, run the user's function,
/// encode, and answer `(out_ptr << 32) | out_len` — or `0` on any failure
/// (the guest-declared error the host reports per-tile). Meaningful on
/// `wasm32`; host tests exercise the codec halves directly instead.
#[doc(hidden)]
#[allow(unsafe_code)] // trusts the host-provided (ptr, len) pair, per the ABI
#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)] // i32/i64 are the wasm ABI types
#[must_use]
pub fn __run(ptr: i32, len: i32, run: fn(&Request) -> Option<Response>) -> i64 {
    if ptr <= 0 || len <= 0 {
        return 0;
    }
    // SAFETY: the host wrote `len` bytes at the pointer it obtained from
    // `swath_udf_alloc(len)`; the ABI makes that region ours to read.
    let buf = unsafe {
        core::slice::from_raw_parts(ptr as u32 as usize as *const u8, len as u32 as usize)
    };
    let Ok(request) = decode_request(buf) else {
        return 0;
    };
    let Some(response) = run(&request) else {
        return 0;
    };
    let Ok(out) = encode_response(request.width, request.height, &response) else {
        return 0;
    };
    let out_len = out.len() as u64;
    if out_len > u64::from(u32::MAX) {
        return 0;
    }
    // Leak on purpose: the buffer must outlive this call for the host to
    // read it; the instance is disposable (pooled, per-request — ADR 0018).
    let out_ptr = out.leak().as_ptr() as usize as u64;
    ((out_ptr << 32) | out_len) as i64
}

/// Produces the four ABI v1 exports from two pieces of user code:
///
/// - `output_planes`: `fn(i32) -> i32` — output planes for a given input
///   arity; answer `0` (or negative) for arities the UDF does not support
///   (the host rejects the module loudly at registration).
/// - `run`: `fn(&Request) -> Option<Response>` — the UDF itself; `None`
///   is a guest-declared failure (a per-tile UDF error host-side).
///
/// Also installs the `wasm32-unknown-unknown` runtime a `no_std` module
/// needs (bump allocator over `memory.grow`, aborting panic handler) —
/// the consuming crate must be `#![no_std]` with `crate-type = ["cdylib"]`
/// and must not define its own.
#[macro_export]
macro_rules! swath_udf {
    (output_planes: $output_planes:expr, run: $run:expr $(,)?) => {
        #[doc = "Swath UDF ABI v1 export (generated by `swath_udf!`)."]
        #[allow(unsafe_code)] // `#[unsafe(no_mangle)]` is what makes it an export
        #[unsafe(no_mangle)]
        pub extern "C" fn swath_udf_abi() -> i32 {
            $crate::ABI_VERSION
        }

        #[doc = "Swath UDF ABI v1 export (generated by `swath_udf!`)."]
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn swath_udf_output_planes(input_planes: i32) -> i32 {
            let output_planes: fn(i32) -> i32 = $output_planes;
            output_planes(input_planes)
        }

        #[doc = "Swath UDF ABI v1 export (generated by `swath_udf!`)."]
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn swath_udf_alloc(len: i32) -> i32 {
            $crate::__alloc(len)
        }

        #[doc = "Swath UDF ABI v1 export (generated by `swath_udf!`)."]
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn swath_udf_run(ptr: i32, len: i32) -> i64 {
            let run: fn(&$crate::Request) -> ::core::option::Option<$crate::Response> = $run;
            $crate::__run(ptr, len, run)
        }
    };
}

// --- wasm32 runtime (allocator + panic) -----------------------------------

/// The `no_std` `wasm32-unknown-unknown` runtime: a bump allocator over
/// `memory.grow` (never frees — instances are pooled and disposable,
/// per-request, so a run's garbage dies with the instance; ADR 0018) and
/// an aborting panic handler (a guest panic traps, which the host reports
/// as a per-tile UDF error).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(unsafe_code)] // a global allocator is unsafe to implement by nature
mod wasm_runtime {
    use core::alloc::{GlobalAlloc, Layout};
    use core::sync::atomic::{AtomicUsize, Ordering};

    const PAGE: usize = 65536;

    unsafe extern "C" {
        // Provided by rust-lld for wasm targets: the first address past
        // static data — where the bump heap starts.
        static __heap_base: u8;
    }

    /// Next free address; `0` means "not yet initialized from `__heap_base`".
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Bump;

    unsafe impl GlobalAlloc for Bump {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // Guest code is single-threaded (no threads proposal compiled
            // in host-side), so Relaxed load/store is a plain read/write.
            let mut next = NEXT.load(Ordering::Relaxed);
            if next == 0 {
                // Taking the raw address of the linker-provided symbol is
                // safe; it is never dereferenced.
                next = &raw const __heap_base as usize;
            }
            let Some(start) = next.checked_add(layout.align() - 1) else {
                return core::ptr::null_mut();
            };
            let start = start & !(layout.align() - 1);
            let Some(end) = start.checked_add(layout.size()) else {
                return core::ptr::null_mut();
            };
            let have = core::arch::wasm32::memory_size(0) * PAGE;
            if end > have {
                let need_pages = (end - have).div_ceil(PAGE);
                if core::arch::wasm32::memory_grow(0, need_pages) == usize::MAX {
                    // Over the declared/host cap (64 MiB, ADR 0018):
                    // allocation failure, which the kit reports as 0.
                    return core::ptr::null_mut();
                }
            }
            NEXT.store(end, Ordering::Relaxed);
            start as *mut u8
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
            // Never freed: the pooled instance is dropped after the run.
        }
    }

    #[global_allocator]
    static ALLOC: Bump = Bump;

    #[panic_handler]
    fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
        core::arch::wasm32::unreachable()
    }
}
