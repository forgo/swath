// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Host-side proof of the wire contract (`docs/udf-abi/v1.md`): encode and
//! decode are inverse, the parser is strict (deny-unknown, exact payload
//! lengths, `abi` = 1), and the `swath_udf!` macro's callable exports
//! forward as declared. The `wasm32` half of the story (the exports under
//! a real runtime) is pinned by `swath-udf-wasmtime`'s fixture tests.

use swath_udf_guest::{
    AbiError, Plane, Request, Response, decode_request, decode_response, encode_request,
    encode_response, swath_udf,
};

fn planes_2x3() -> Vec<Plane> {
    vec![
        Plane {
            values: vec![1.0, 2.5, -3.0, 0.0, f64::MAX, 6.25],
            validity: vec![1, 1, 1, 0, 1, 1],
        },
        Plane {
            values: vec![0.5; 6],
            validity: vec![1; 6],
        },
    ]
}

#[test]
fn request_roundtrips() {
    let planes = planes_2x3();
    let buf = encode_request(2, 3, &planes).expect("encodes");
    // The frame is the ABI shape: length prefix + exact JSON header.
    let header_len = u32::from_le_bytes(buf[..4].try_into().unwrap()) as usize;
    assert_eq!(
        &buf[4..4 + header_len],
        br#"{"abi":1,"width":2,"height":3,"planes":2}"#
    );
    let decoded = decode_request(&buf).expect("decodes");
    assert_eq!(
        decoded,
        Request {
            width: 2,
            height: 3,
            planes,
        }
    );
}

#[test]
fn response_roundtrips() {
    let response = Response {
        planes: planes_2x3(),
    };
    let buf = encode_response(2, 3, &response).expect("encodes");
    let header_len = u32::from_le_bytes(buf[..4].try_into().unwrap()) as usize;
    assert_eq!(&buf[4..4 + header_len], br#"{"abi":1,"planes":2}"#);
    assert_eq!(decode_response(2, 3, &buf).expect("decodes"), response);
}

fn request_with_header(header: &str, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&u32::try_from(header.len()).unwrap().to_le_bytes());
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// One valid pixel's payload for a 1x1, single-plane request.
fn payload_1x1() -> Vec<u8> {
    let mut payload = 7.5f64.to_le_bytes().to_vec();
    payload.push(1);
    payload
}

#[test]
fn whitespace_tolerant_but_strict() {
    let buf = request_with_header(
        "{ \"abi\": 1,\n  \"width\": 1, \"height\": 1, \"planes\": 1 }",
        &payload_1x1(),
    );
    assert_eq!(decode_request(&buf).expect("decodes").pixels(), 1);
}

#[test]
fn unknown_header_field_is_denied() {
    let buf = request_with_header(
        r#"{"abi":1,"width":1,"height":1,"planes":1,"halo":1}"#,
        &payload_1x1(),
    );
    assert_eq!(decode_request(&buf), Err(AbiError::UnknownField));
}

#[test]
fn missing_and_duplicate_fields_are_errors() {
    let buf = request_with_header(r#"{"abi":1,"width":1,"height":1}"#, &payload_1x1());
    assert_eq!(decode_request(&buf), Err(AbiError::MissingField));
    let buf = request_with_header(
        r#"{"abi":1,"width":1,"width":1,"height":1,"planes":1}"#,
        &payload_1x1(),
    );
    assert_eq!(decode_request(&buf), Err(AbiError::DuplicateField));
}

#[test]
fn wrong_abi_version_is_an_error() {
    let buf = request_with_header(
        r#"{"abi":2,"width":1,"height":1,"planes":1}"#,
        &payload_1x1(),
    );
    assert_eq!(decode_request(&buf), Err(AbiError::AbiVersion));
}

#[test]
fn payload_length_must_agree_with_header() {
    let mut short = payload_1x1();
    short.pop();
    let header = r#"{"abi":1,"width":1,"height":1,"planes":1}"#;
    assert_eq!(
        decode_request(&request_with_header(header, &short)),
        Err(AbiError::PayloadLength)
    );
    let mut long = payload_1x1();
    long.push(0);
    assert_eq!(
        decode_request(&request_with_header(header, &long)),
        Err(AbiError::PayloadLength)
    );
}

#[test]
fn truncated_and_malformed_frames_are_errors() {
    assert_eq!(decode_request(&[1, 0]), Err(AbiError::Truncated));
    // Length prefix claims more header than exists.
    assert_eq!(
        decode_request(&[200, 0, 0, 0, b'{']),
        Err(AbiError::Truncated)
    );
    for header in [
        "",
        "{}",
        "[1]",
        r#"{"abi":-1,"width":1,"height":1,"planes":1}"#,
        r#"{"abi":1.5,"width":1,"height":1,"planes":1}"#,
        r#"{"abi":1,"width":1,"height":1,"planes":1} extra"#,
    ] {
        assert_eq!(
            decode_request(&request_with_header(header, &payload_1x1())),
            Err(AbiError::MalformedHeader),
            "header: {header}"
        );
    }
}

#[test]
fn zero_dimensions_are_errors() {
    for header in [
        r#"{"abi":1,"width":0,"height":1,"planes":1}"#,
        r#"{"abi":1,"width":1,"height":0,"planes":1}"#,
        r#"{"abi":1,"width":1,"height":1,"planes":0}"#,
    ] {
        assert_eq!(
            decode_request(&request_with_header(header, &[])),
            Err(AbiError::BadDimensions),
            "header: {header}"
        );
    }
}

#[test]
fn plane_shape_mismatch_is_an_error() {
    let response = Response {
        planes: vec![Plane {
            values: vec![1.0; 5],
            validity: vec![1; 6],
        }],
    };
    assert_eq!(encode_response(2, 3, &response), Err(AbiError::PlaneShape));
}

// --- the macro's callable surface ----------------------------------------

swath_udf! {
    output_planes: |input_planes| i32::from(input_planes == 2),
    run: passthrough_first,
}

fn passthrough_first(request: &Request) -> Option<Response> {
    Some(Response {
        planes: vec![request.planes.first()?.clone()],
    })
}

#[test]
fn macro_exports_forward_as_declared() {
    assert_eq!(swath_udf_abi(), 1);
    assert_eq!(swath_udf_output_planes(2), 1);
    assert_eq!(swath_udf_output_planes(1), 0, "unsupported arity answers 0");
    assert_eq!(swath_udf_output_planes(-7), 0);
}
