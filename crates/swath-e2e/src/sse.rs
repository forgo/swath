// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! A minimal synchronous SSE subscriber for `GET /traces` — the blocking
//! twin of the async client in `swath-api/tests/trace_stream.rs`: raw
//! HTTP/1.1 GET, incremental chunked-transfer decode, frame splitter.
//! Dropping it closes the connection.

use std::io::Read as _;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::http;

/// Per-`read` slice of patience: short so the deadline is honored
/// promptly, long enough to never busy-spin.
const READ_TIMEOUT: Duration = Duration::from_millis(500);

/// One parsed SSE event: field values plus any comment lines.
#[derive(Debug, Default)]
pub(crate) struct Frame {
    pub(crate) event: Option<String>,
    pub(crate) id: Option<String>,
    pub(crate) data: Vec<String>,
    comments: Vec<String>,
}

impl Frame {
    /// Whether this frame is only the server's keepalive comment.
    pub(crate) fn is_keepalive(&self) -> bool {
        self.event.is_none() && self.data.is_empty() && !self.comments.is_empty()
    }
}

/// The subscriber: owns the connection and the two decode buffers
/// (raw wire bytes -> de-chunked body -> frames).
pub(crate) struct Subscriber {
    stream: TcpStream,
    /// Wire bytes not yet consumed by the chunked decoder.
    raw: Vec<u8>,
    /// De-chunked body bytes not yet split into frames.
    decoded: Vec<u8>,
}

impl Subscriber {
    /// Connects and reads the response head. Returning implies the
    /// handler ran, so the broadcast subscription exists — renders
    /// published after this point are guaranteed visible to the stream
    /// (the same ordering argument `trace_stream.rs` relies on).
    pub(crate) fn connect() -> Result<Self, String> {
        use std::io::Write as _;
        let mut stream = http::connect()?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .map_err(|e| format!("SSE read timeout: {e}"))?;
        let request = format!(
            "GET /traces HTTP/1.1\r\nhost: {}\r\naccept: text/event-stream\r\n\r\n",
            http::HOST
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("GET /traces: write: {e}"))?;
        let mut this = Self {
            stream,
            raw: Vec::new(),
            decoded: Vec::new(),
        };
        this.read_head()?;
        Ok(this)
    }

    /// Reads until the response head is complete, verifies it, and
    /// leaves any body bytes already received in the raw buffer.
    fn read_head(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let head_end = loop {
            if let Some(pos) = self.raw.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos;
            }
            if Instant::now() >= deadline {
                return Err("GET /traces: response head not received in 10s".to_owned());
            }
            self.fill()?;
        };
        let head: Vec<u8> = self.raw.drain(..head_end + 4).collect();
        let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
        if !head.starts_with("http/1.1 200") {
            return Err(format!("GET /traces: expected 200, head was: {head}"));
        }
        if !head.contains("content-type: text/event-stream") {
            return Err(format!("GET /traces: not an event stream: {head}"));
        }
        Ok(())
    }

    /// The next SSE frame, or an error naming what arrived if the
    /// deadline passes first.
    pub(crate) fn next_frame(&mut self, deadline: Instant) -> Result<Frame, String> {
        loop {
            self.dechunk()?;
            if let Some(frame) = self.take_frame() {
                return Ok(frame);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "no further SSE frame before the deadline; undelivered body so far: {:?}",
                    String::from_utf8_lossy(&self.decoded)
                ));
            }
            self.fill()?;
        }
    }

    /// One socket read into the raw buffer; a timed-out read is not an
    /// error (the caller's deadline governs).
    fn fill(&mut self) -> Result<(), String> {
        let mut buf = [0_u8; 4096];
        match self.stream.read(&mut buf) {
            Ok(0) => Err("SSE connection closed by the server".to_owned()),
            Ok(n) => {
                self.raw.extend_from_slice(&buf[..n]);
                Ok(())
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(format!("SSE read: {e}")),
        }
    }

    /// Moves every complete chunk from the raw buffer into the decoded
    /// body buffer; partial chunks stay for the next read.
    fn dechunk(&mut self) -> Result<(), String> {
        loop {
            let Some(line_end) = self.raw.windows(2).position(|w| w == b"\r\n") else {
                return Ok(());
            };
            let size_text = std::str::from_utf8(&self.raw[..line_end])
                .map_err(|e| format!("chunk size not UTF-8: {e}"))?
                .trim()
                .to_owned();
            let size = usize::from_str_radix(&size_text, 16)
                .map_err(|e| format!("chunk size `{size_text}`: {e}"))?;
            if size == 0 {
                return Err("SSE stream ended (zero-length chunk)".to_owned());
            }
            let total = line_end + 2 + size + 2; // size line + data + CRLF
            if self.raw.len() < total {
                return Ok(());
            }
            self.decoded
                .extend_from_slice(&self.raw[line_end + 2..line_end + 2 + size]);
            self.raw.drain(..total);
        }
    }

    /// Splits one complete frame (terminated by a blank line) off the
    /// decoded body, if present.
    fn take_frame(&mut self) -> Option<Frame> {
        let end = self.decoded.windows(2).position(|w| w == b"\n\n")?;
        let raw: Vec<u8> = self.decoded.drain(..end + 2).collect();
        let text = String::from_utf8_lossy(&raw);

        let mut frame = Frame::default();
        for line in text.lines().filter(|line| !line.is_empty()) {
            if let Some(comment) = line.strip_prefix(':') {
                frame.comments.push(comment.trim_start().to_owned());
                continue;
            }
            let Some((field, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => frame.event = Some(value.to_owned()),
                "id" => frame.id = Some(value.to_owned()),
                "data" => frame.data.push(value.to_owned()),
                _ => {}
            }
        }
        Some(frame)
    }
}
