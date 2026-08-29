// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** A hand-assembled ABI v1 module, one failure mode (issue #208). The
 * same no-toolchain posture as `crates/swath-api/tests/common/wasm.rs`
 * (which this mirrors byte for byte): structurally conforming — the four
 * v1 exports plus memory — so it registers, with a `swath_udf_run` that
 * does whatever `run` says. */

function uleb(value: number): number[] {
  const out: number[] = [];
  let v = value;
  for (;;) {
    const byte = v & 0x7f;
    v = Math.floor(v / 128);
    if (v === 0) {
      out.push(byte);
      return out;
    }
    out.push(byte | 0x80);
  }
}

function sleb(value: number): number[] {
  const out: number[] = [];
  let v = value;
  for (;;) {
    const byte = v & 0x7f;
    v >>= 7;
    const sign = (byte & 0x40) !== 0;
    if ((v === 0 && !sign) || (v === -1 && sign)) {
      out.push(byte);
      return out;
    }
    out.push(byte | 0x80);
  }
}

function section(id: number, payload: number[]): number[] {
  return [id, ...uleb(payload.length), ...payload];
}

function counted(items: number[][]): number[] {
  return [...uleb(items.length), ...items.flat()];
}

function name(text: string): number[] {
  const bytes = [...new TextEncoder().encode(text)];
  return [...uleb(bytes.length), ...bytes];
}

/** `i32.const k` then `end`. */
function retI32(k: number): number[] {
  return [0x41, ...sleb(k), 0x0b];
}

/** A structurally conforming ABI v1 module (abi = 1, one output plane,
 * `swath_udf_alloc` answering 8 inside a 4 MiB memory) whose
 * `swath_udf_run` body is `run`. */
export function abiModule(run: number[]): Buffer {
  const exportEntry = (n: string, kind: number, index: number): number[] => [
    ...name(n),
    kind,
    ...uleb(index),
  ];
  const body = (code: number[]): number[] => {
    const entry = [0x00, ...code]; // zero locals
    return [...uleb(entry.length), ...entry];
  };
  return Buffer.from([
    0x00,
    0x61,
    0x73,
    0x6d,
    0x01,
    0x00,
    0x00,
    0x00,
    // Types: 0 = () -> i32, 1 = (i32) -> i32, 2 = (i32, i32) -> i64.
    ...section(
      1,
      counted([
        [0x60, 0x00, 0x01, 0x7f],
        [0x60, 0x01, 0x7f, 0x01, 0x7f],
        [0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e],
      ]),
    ),
    // Functions: abi, output_planes, alloc, run.
    ...section(3, counted([[0], [1], [1], [2]])),
    // Memory: 64 pages (4 MiB), no max.
    ...section(5, counted([[0x00, 0x40]])),
    ...section(
      7,
      counted([
        exportEntry("memory", 0x02, 0),
        exportEntry("swath_udf_abi", 0x00, 0),
        exportEntry("swath_udf_output_planes", 0x00, 1),
        exportEntry("swath_udf_alloc", 0x00, 2),
        exportEntry("swath_udf_run", 0x00, 3),
      ]),
    ),
    ...section(10, counted([body(retI32(1)), body(retI32(1)), body(retI32(8)), body(run)])),
  ]);
}

/** The fuel bomb: `swath_udf_run` is `(loop (br 0))`. */
export const FUEL_BOMB = abiModule([0x03, 0x40, 0x0c, 0x00, 0x0b, 0x42, 0x00, 0x0b]);
