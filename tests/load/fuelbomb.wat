;; SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
;; SPDX-License-Identifier: Apache-2.0
;;
;; The runaway-loop UDF for `just load-udf` (issue #207): a STRUCTURALLY
;; VALID Swath UDF ABI v1 module (the four exports, exported memory, zero
;; imports) whose `swath_udf_run` spins a billion-iteration no-op loop.
;;
;; It registers cleanly — `swath_udf_abi`, `swath_udf_output_planes`, and
;; `swath_udf_alloc` are all constant-time, so `POST /services` accepts it
;; — and only bites on the tile path, where the loop exhausts the layer's
;; `max_udf_fuel_per_tile` long before it finishes (fuel is charged per
;; basic block; the 100 M default trips after ~100 M iterations). That is
;; the fuel-bomb the load harness proves the engine refuses with the
;; RFC 7807 fuel problem (tiles) / `ProcessGraphComplexity` (preview),
;; with zero collateral to /healthz or the SSE stream.
;;
;; Deliberately hand-authored and committed as bytes (like the
;; AssemblyScript/TinyGo example fixtures): build once with
;;   wat2wasm tests/load/fuelbomb.wat -o tests/load/fuelbomb.wasm
;; It is a load instrument, not a conformance fixture — its honesty is the
;; harness assertion (published-then-refused), not a CI rebuild.
(module
  ;; 48 pages (3 MiB) so a full 256x256 two-plane request (~1.18 MiB of
  ;; f64 samples + validity) written at the alloc pointer stays in bounds:
  ;; the module must actually REACH swath_udf_run and burn fuel, not fail
  ;; the host's bounds check on the copy-in and never spin at all.
  (memory (export "memory") 48)

  ;; swath_udf_abi() -> 1
  (func (export "swath_udf_abi") (result i32)
    (i32.const 1))

  ;; swath_udf_output_planes(input_planes) -> 1 (accepts any arity so the
  ;; NDVI two-band load lowers to one output plane and publish succeeds).
  (func (export "swath_udf_output_planes") (param i32) (result i32)
    (i32.const 1))

  ;; swath_udf_alloc(len) -> a fixed in-bounds pointer past the header; the
  ;; declared 48 pages hold the whole request at this offset.
  (func (export "swath_udf_alloc") (param i32) (result i32)
    (i32.const 1024))

  ;; swath_udf_run(ptr, len) -> i64: spin 1e9 no-op iterations. Fuel runs
  ;; out first; the call never reaches the return.
  (func (export "swath_udf_run") (param i32 i32) (result i64)
    (local $i i64)
    (local.set $i (i64.const 1000000000))
    (block $done
      (loop $spin
        (br_if $done (i64.eqz (local.get $i)))
        (local.set $i (i64.sub (local.get $i) (i64.const 1)))
        (br $spin)))
    (i64.const 0)))
