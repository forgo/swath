// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! A minimal synchronous HTTP/1.1 client for the e2e harness — the same
//! no-client-dependency stance as the hand-rolled SSE reader in
//! `swath-api/tests/trace_stream.rs`: one `TcpStream` per request,
//! `connection: close`, read to EOF, parse head + body. The harness talks
//! to exactly one server (the compose stack on loopback), so a general
//! HTTP client would be all supply-chain surface and no capability.

use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::time::Duration;

/// The compose stack's published API port (docker-compose.yml).
pub(crate) const HOST: &str = "localhost:8080";
const ADDR: &str = "127.0.0.1:8080";

/// Generous per-request ceiling: only ever reached on failure.
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// One complete HTTP response: status, headers (as received), body.
pub(crate) struct Response {
    pub(crate) status: u16,
    headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    /// First header value under `name` (case-insensitive), if present.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// `GET path` against the stack.
pub(crate) fn get(path: &str) -> Result<Response, String> {
    request("GET", path, None)
}

/// `POST path` with a JSON body against the stack.
pub(crate) fn post_json(path: &str, body: &serde_json::Value) -> Result<Response, String> {
    let payload = body.to_string();
    request("POST", path, Some(payload.as_bytes()))
}

/// Opens a connection to the stack with the harness's read/write timeouts.
pub(crate) fn connect() -> Result<TcpStream, String> {
    let stream = TcpStream::connect(ADDR).map_err(|e| format!("connect {ADDR}: {e}"))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|e| format!("socket timeouts: {e}"))?;
    Ok(stream)
}

fn request(method: &str, path: &str, body: Option<&[u8]>) -> Result<Response, String> {
    let mut stream = connect()?;
    let mut head = format!("{method} {path} HTTP/1.1\r\nhost: {HOST}\r\nconnection: close\r\n");
    if let Some(payload) = body {
        write!(
            head,
            "content-type: application/json\r\ncontent-length: {}\r\n",
            payload.len()
        )
        .expect("writing to a String is infallible");
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body.unwrap_or_default()))
        .map_err(|e| format!("{method} {path}: write: {e}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("{method} {path}: read: {e}"))?;
    parse(&raw).map_err(|e| format!("{method} {path}: {e}"))
}

/// Splits a raw `connection: close` response into status/headers/body,
/// de-chunking the body when the server chose chunked transfer.
fn parse(raw: &[u8]) -> Result<Response, String> {
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no header/body separator in response")?;
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|e| format!("head not UTF-8: {e}"))?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("empty response head")?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("malformed status line `{status_line}`"))?
        .parse()
        .map_err(|e| format!("non-numeric status in `{status_line}`: {e}"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let mut body = raw[head_end + 4..].to_vec();
    let chunked = headers
        .iter()
        .any(|(n, v)| n == "transfer-encoding" && v.eq_ignore_ascii_case("chunked"));
    if chunked {
        body = dechunk(&body)?;
    }
    Ok(Response {
        status,
        headers,
        body,
    })
}

/// Decodes a complete chunked-transfer body (the stream is already at EOF).
fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let line_end = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("chunk size line not terminated")?;
        let size_text = std::str::from_utf8(&rest[..line_end])
            .map_err(|e| format!("chunk size not UTF-8: {e}"))?
            .trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|e| format!("chunk size: {e}"))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(body);
        }
        if rest.len() < size + 2 {
            return Err("truncated chunk".to_owned());
        }
        body.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}
