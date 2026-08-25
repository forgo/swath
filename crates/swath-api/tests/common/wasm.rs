// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Minimal hand-assembled ABI v1 modules, one failure mode each, for the
//! preview-as-validation-loop tests (#206): the same no-`wat`-dependency
//! posture as the wasmtime adapter's own `executor.rs` suite (which this
//! mirrors — the supply-chain tree stays exactly wasmtime's). Each module
//! is structurally conforming (the four v1 exports plus memory), so it
//! registers at publish/preview time; only `swath_udf_run` misbehaves.

/// Unsigned LEB128.
fn uleb(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// Signed LEB128.
#[allow(
    clippy::cast_sign_loss,
    reason = "LEB128 emits the low 7 bits of each signed chunk by design"
)]
fn sleb(mut v: i64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        let sign = byte & 0x40 != 0;
        if (v == 0 && !sign) || (v == -1 && sign) {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn section(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![id];
    out.extend(uleb(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn counted(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = uleb(items.len() as u64);
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn name(s: &str) -> Vec<u8> {
    let mut out = uleb(s.len() as u64);
    out.extend_from_slice(s.as_bytes());
    out
}

/// `i32.const k` then `end`.
fn ret_i32(k: i32) -> Vec<u8> {
    let mut body = vec![0x41];
    body.extend(sleb(i64::from(k)));
    body.push(0x0b);
    body
}

/// `i64.const k` then `end`.
fn ret_i64(k: i64) -> Vec<u8> {
    let mut body = vec![0x42];
    body.extend(sleb(k));
    body.push(0x0b);
    body
}

/// A structurally conforming ABI v1 module (abi = 1, one output plane,
/// `swath_udf_alloc` answering 8 inside a 4 MiB memory — room for a
/// full 256-px two-plane request, so the host's bounds check admits the
/// request and `run` is what fails) whose `swath_udf_run` body is `run`.
fn abi_module(run: &[u8]) -> Vec<u8> {
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    // Types: 0 = () -> i32, 1 = (i32) -> i32, 2 = (i32, i32) -> i64.
    module.extend(section(
        1,
        &counted(&[
            vec![0x60, 0x00, 0x01, 0x7f],
            vec![0x60, 0x01, 0x7f, 0x01, 0x7f],
            vec![0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e],
        ]),
    ));
    // Functions: abi, output_planes, alloc, run.
    module.extend(section(3, &counted(&[vec![0], vec![1], vec![1], vec![2]])));
    // Memory: 64 pages (4 MiB), no max.
    module.extend(section(5, &counted(&[vec![0x00, 0x40]])));
    let export = |n: &str, kind: u8, index: u64| {
        let mut out = name(n);
        out.push(kind);
        out.extend(uleb(index));
        out
    };
    module.extend(section(
        7,
        &counted(&[
            export("memory", 0x02, 0),
            export("swath_udf_abi", 0x00, 0),
            export("swath_udf_output_planes", 0x00, 1),
            export("swath_udf_alloc", 0x00, 2),
            export("swath_udf_run", 0x00, 3),
        ]),
    ));
    // Code: every body has zero locals.
    let body = |code: &[u8]| {
        let mut entry = vec![0x00];
        entry.extend_from_slice(code);
        let mut out = uleb(entry.len() as u64);
        out.extend(entry);
        out
    };
    module.extend(section(
        10,
        &counted(&[
            body(&ret_i32(1)),
            body(&ret_i32(1)),
            body(&ret_i32(8)),
            body(run),
        ]),
    ));
    module
}

/// The fuel bomb: `swath_udf_run` is `(loop (br 0))` — spins until the
/// fuel budget (or the epoch backstop) stops it.
pub(crate) fn fuel_bomb() -> Vec<u8> {
    abi_module(&[0x03, 0x40, 0x0c, 0x00, 0x0b, 0x42, 0x00, 0x0b])
}

/// The trapper: `swath_udf_run` executes `unreachable`.
pub(crate) fn trapper() -> Vec<u8> {
    abi_module(&[0x00, 0x0b])
}

/// The liar: `swath_udf_run` claims a response at 3 GiB with a 1 GiB
/// length — a packed pointer the host's bounds check refuses.
pub(crate) fn malformed_output() -> Vec<u8> {
    abi_module(&ret_i64((0xC000_0000_i64 << 32) | 0x4000_0000))
}
