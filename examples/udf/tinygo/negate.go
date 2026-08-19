// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// "negate" — the TinyGo conformance fixture for the Swath UDF ABI v1
// (docs/udf-abi/v1.md): 1 input plane in, 1 output plane out, every sample
// value negated, validity passed through. The point is language
// neutrality, not the math: any toolchain that can emit a zero-import
// wasm32-unknown-unknown module with the four exports can author UDFs.
//
// Build (pinned toolchains, full pipeline in ../README.md):
//
//	tinygo build -o negate-raw.wasm -target=wasm-unknown -no-debug -panic=trap .
//	wasm-ctor-eval negate-raw.wasm --ctors=_initialize -o negate-ctor.wasm
//	wasm2wat negate-ctor.wasm | grep -vE '\(export "f(min|max)imumf?"' > negate.wat
//	wat2wasm negate.wat -o negate-stripped.wasm
//	wasm-opt -O2 --remove-unused-module-elements negate-stripped.wasm -o negate.wasm
//
// `wasm-unknown` is TinyGo's freestanding target (no WASI, no JS glue —
// zero imports, as registration requires) and `-panic=trap` keeps panic
// formatting machinery out of the module. The post-processing exists
// because TinyGo emits a runtime-init export (`_initialize`) the ABI has
// no hook to call — `wasm-ctor-eval` evaluates it at build time and
// snapshots the initialized memory — plus a few LLVM helper exports
// (`fminimum` etc.) beyond the ABI's export set, stripped above.
package main

import "unsafe"

const responseHeader = `{"abi":1,"planes":1}`

// Kept alive so the leaking collector's allocations backing handed-out
// pointers are never reclaimed or moved.
var arenas [][]byte

func main() {}

func loadU8(addr int32) uint8 {
	return *(*uint8)(unsafe.Pointer(uintptr(addr)))
}

func loadF64(addr int32) float64 {
	return *(*float64)(unsafe.Pointer(uintptr(addr)))
}

// readUintField scans the request header for `"<key>":` and parses the
// unsigned integer after it; -1 when absent. A scanner, not a JSON
// parser: the fixture trusts the host's fixed v1 header shape (the Rust
// kit parses strictly).
func readUintField(headerPtr, headerLen int32, key string) int64 {
	pattern := `"` + key + `":`
	for at := int32(0); at+int32(len(pattern)) < headerLen; at++ {
		hit := true
		for k := 0; k < len(pattern); k++ {
			if loadU8(headerPtr+at+int32(k)) != pattern[k] {
				hit = false
				break
			}
		}
		if !hit {
			continue
		}
		pos := at + int32(len(pattern))
		for pos < headerLen && loadU8(headerPtr+pos) == ' ' {
			pos++
		}
		value := int64(-1)
		for pos < headerLen {
			digit := loadU8(headerPtr + pos)
			if digit < '0' || digit > '9' {
				break
			}
			if value < 0 {
				value = 0
			}
			value = value*10 + int64(digit-'0')
			pos++
		}
		return value
	}
	return -1
}

//go:wasmexport swath_udf_abi
func swathUdfAbi() int32 {
	return 1
}

//go:wasmexport swath_udf_output_planes
func swathUdfOutputPlanes(inputPlanes int32) int32 {
	if inputPlanes == 1 {
		return 1
	}
	return 0
}

//go:wasmexport swath_udf_alloc
func swathUdfAlloc(length int32) int32 {
	if length <= 0 {
		return 0
	}
	buf := make([]byte, length)
	arenas = append(arenas, buf)
	return int32(uintptr(unsafe.Pointer(&buf[0])))
}

//go:wasmexport swath_udf_run
func swathUdfRun(ptr, length int32) int64 {
	if ptr <= 0 || length < 4 {
		return 0
	}
	headerLen := int32(*(*uint32)(unsafe.Pointer(uintptr(ptr))))
	if headerLen < 0 || 4+headerLen > length {
		return 0
	}
	headerPtr := ptr + 4
	abi := readUintField(headerPtr, headerLen, "abi")
	width := readUintField(headerPtr, headerLen, "width")
	height := readUintField(headerPtr, headerLen, "height")
	planes := readUintField(headerPtr, headerLen, "planes")
	if abi != 1 || width < 1 || height < 1 || planes != 1 {
		return 0
	}
	pixels := int32(width) * int32(height)
	payload := ptr + 4 + headerLen
	if length-4-headerLen != pixels*9 {
		return 0
	}

	outLen := int32(4+len(responseHeader)) + pixels*9
	out := make([]byte, outLen)
	arenas = append(arenas, out)
	outPtr := int32(uintptr(unsafe.Pointer(&out[0])))
	out[0] = byte(len(responseHeader))
	out[1], out[2], out[3] = 0, 0, 0
	copy(out[4:], responseHeader)
	values := out[4+len(responseHeader):]
	for i := int32(0); i < pixels; i++ {
		negated := -loadF64(payload + i*8)
		bits := *(*uint64)(unsafe.Pointer(&negated))
		for b := 0; b < 8; b++ {
			values[i*8+int32(b)] = byte(bits >> (8 * b))
		}
	}
	validity := values[pixels*8:]
	for i := int32(0); i < pixels; i++ {
		validity[i] = loadU8(payload + pixels*8 + i)
	}
	return int64(outPtr)<<32 | int64(outLen)
}
