// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// "double" — the AssemblyScript conformance fixture for the Swath UDF ABI v1
// (docs/udf-abi/v1.md): 1 input plane in, 1 output plane out, every sample
// value doubled, validity passed through. The point is language neutrality,
// not the math: any toolchain that can emit a zero-import
// wasm32-unknown-unknown module with the four exports can author UDFs.
//
// Build (pinned toolchain, see ../README.md):
//   npx --yes --package assemblyscript@0.28.20 asc index.ts \
//     -o double.wasm -O3 --runtime stub --use abort= --noAssert
//
// `--runtime stub` is the arena allocator (no GC machinery) and
// `--use abort=` replaces the abort import with a trap — both required for
// the zero-import rule; a module importing anything is rejected at
// registration.

const RESPONSE_HEADER = '{"abi":1,"planes":1}';

// Scans the request header for `"<key>":` and parses the unsigned integer
// after it; -1 when absent. A scanner, not a JSON parser: the fixture
// trusts the host's fixed v1 header shape (the Rust kit parses strictly).
function readUintField(headerPtr: i32, headerLen: i32, key: string): i64 {
  const pattern = '"' + key + '":';
  for (let at = 0; at + pattern.length < headerLen; at++) {
    let hit = true;
    for (let k = 0; k < pattern.length; k++) {
      if (load<u8>(headerPtr + at + k) != (<u8>pattern.charCodeAt(k))) {
        hit = false;
        break;
      }
    }
    if (!hit) continue;
    let pos = at + pattern.length;
    while (pos < headerLen && load<u8>(headerPtr + pos) == 0x20) pos++;
    let value: i64 = -1;
    while (pos < headerLen) {
      const digit = load<u8>(headerPtr + pos);
      if (digit < 0x30 || digit > 0x39) break;
      value = (value < 0 ? 0 : value) * 10 + (digit - 0x30);
      pos++;
    }
    return value;
  }
  return -1;
}

export function swath_udf_abi(): i32 {
  return 1;
}

export function swath_udf_output_planes(inputPlanes: i32): i32 {
  return inputPlanes == 1 ? 1 : 0;
}

export function swath_udf_alloc(len: i32): i32 {
  if (len <= 0) return 0;
  return <i32>heap.alloc(len);
}

export function swath_udf_run(ptr: i32, len: i32): i64 {
  if (ptr <= 0 || len < 4) return 0;
  const headerLen = <i32>load<u32>(ptr);
  if (headerLen < 0 || 4 + headerLen > len) return 0;
  const headerPtr = ptr + 4;
  const abi = readUintField(headerPtr, headerLen, "abi");
  const width = readUintField(headerPtr, headerLen, "width");
  const height = readUintField(headerPtr, headerLen, "height");
  const planes = readUintField(headerPtr, headerLen, "planes");
  if (abi != 1 || width < 1 || height < 1 || planes != 1) return 0;
  const pixels = <i32>width * <i32>height;
  const payload = ptr + 4 + headerLen;
  if (len - 4 - headerLen != pixels * 9) return 0;

  const outLen = 4 + RESPONSE_HEADER.length + pixels * 9;
  const out = <i32>heap.alloc(outLen);
  if (out == 0) return 0;
  store<u32>(out, RESPONSE_HEADER.length);
  for (let i = 0; i < RESPONSE_HEADER.length; i++) {
    store<u8>(out + 4 + i, <u8>RESPONSE_HEADER.charCodeAt(i));
  }
  const outPayload = out + 4 + RESPONSE_HEADER.length;
  for (let i = 0; i < pixels; i++) {
    store<f64>(outPayload + i * 8, load<f64>(payload + i * 8) * 2.0);
  }
  memory.copy(outPayload + pixels * 8, payload + pixels * 8, pixels);
  return (<i64>out << 32) | <i64>outLen;
}
