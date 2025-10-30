//! Wire logger middleware for intercepting LSP messages
//!
//! This module provides async wrappers around stdin/stdout that log all LSP messages
//! passing through the transport layer. It works by parsing the JSON-RPC messages
//! from the raw byte streams before/after they're processed by tower-lsp.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use serde_json::Value;

use crate::wire_logger::WireLogger;

/// Wrapper around AsyncRead that logs incoming LSP messages
pub struct LoggingReader<R> {
    inner: R,
    wire_logger: WireLogger,
    buffer: Vec<u8>,
}

impl<R> LoggingReader<R>
where
    R: AsyncRead + Unpin,
{
    pub fn new(inner: R, wire_logger: WireLogger) -> Self {
        Self {
            inner,
            wire_logger,
            buffer: Vec::new(),
        }
    }
}

impl<R> AsyncRead for LoggingReader<R>
where
    R: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);

        if let Poll::Ready(Ok(())) = &result {
            let after = buf.filled().len();
            if after > before {
                // Append new bytes to buffer
                let new_bytes = &buf.filled()[before..after];
                self.buffer.extend_from_slice(new_bytes);

                // Try to parse complete messages from buffer
                // LSP messages format: "Content-Length: N\r\n\r\n{json}"
                while let Some(message) = try_extract_message(&mut self.buffer) {
                    // Log the incoming message
                    if let Ok(json) = serde_json::from_str::<Value>(&message) {
                        self.wire_logger.log_incoming(&json);
                    }
                }
            }
        }

        result
    }
}

/// Wrapper around AsyncWrite that logs outgoing LSP messages
pub struct LoggingWriter<W> {
    inner: W,
    wire_logger: WireLogger,
    buffer: Vec<u8>,
}

impl<W> LoggingWriter<W>
where
    W: AsyncWrite + Unpin,
{
    pub fn new(inner: W, wire_logger: WireLogger) -> Self {
        Self {
            inner,
            wire_logger,
            buffer: Vec::new(),
        }
    }
}

impl<W> AsyncWrite for LoggingWriter<W>
where
    W: AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        // First try to write to the underlying writer
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);

        if let Poll::Ready(Ok(n)) = &result {
            // Buffer the written bytes for message extraction
            self.buffer.extend_from_slice(&buf[..*n]);

            // Try to parse complete messages from buffer
            while let Some(message) = try_extract_message(&mut self.buffer) {
                // Log the outgoing message
                if let Ok(json) = serde_json::from_str::<Value>(&message) {
                    self.wire_logger.log_outgoing(&json);
                }
            }
        }

        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Try to extract a complete LSP message from the buffer
/// Returns Some(json_string) if a complete message was found, None otherwise
/// Modifies the buffer to remove the extracted message
fn try_extract_message(buffer: &mut Vec<u8>) -> Option<String> {
    // Look for "Content-Length: " header
    let header_start = buffer.windows(16).position(|window| {
        window.starts_with(b"Content-Length: ")
    })?;

    // Find the end of the header line (\r\n)
    let header_end = buffer[header_start..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|pos| header_start + pos)?;

    // Parse the content length
    let length_str = std::str::from_utf8(&buffer[header_start + 16..header_end]).ok()?;
    let content_length: usize = length_str.trim().parse().ok()?;

    // Find the start of the JSON body (after \r\n\r\n)
    let body_start = buffer[header_end..]
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| header_end + pos + 4)?;

    // Check if we have the complete body
    let body_end = body_start + content_length;
    if buffer.len() < body_end {
        return None; // Incomplete message
    }

    // Extract the JSON body
    let json_bytes = buffer[body_start..body_end].to_vec();
    let json_str = String::from_utf8(json_bytes).ok()?;

    // Remove the complete message from buffer
    buffer.drain(..body_end);

    Some(json_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_extract_message_complete() {
        let mut buffer = b"Content-Length: 46\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}".to_vec();
        let message = try_extract_message(&mut buffer);
        assert_eq!(message, Some("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}".to_string()));
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn test_try_extract_message_incomplete() {
        let mut buffer = b"Content-Length: 45\r\n\r\n{\"jsonrpc\":\"2.0\"".to_vec();
        let message = try_extract_message(&mut buffer);
        assert_eq!(message, None);
        // Buffer should be unchanged (22 bytes header + 16 bytes partial message = 38)
        assert_eq!(buffer.len(), 38);
    }

    #[test]
    fn test_try_extract_message_multiple() {
        let mut buffer = b"Content-Length: 17\r\n\r\n{\"jsonrpc\":\"2.0\"}Content-Length: 17\r\n\r\n{\"jsonrpc\":\"2.0\"}".to_vec();

        let msg1 = try_extract_message(&mut buffer);
        assert_eq!(msg1, Some("{\"jsonrpc\":\"2.0\"}".to_string()));

        let msg2 = try_extract_message(&mut buffer);
        assert_eq!(msg2, Some("{\"jsonrpc\":\"2.0\"}".to_string()));

        assert_eq!(buffer.len(), 0);
    }
}
