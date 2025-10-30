//! Wire protocol logger for LSP messages
//!
//! This module provides structured logging of all LSP JSON-RPC messages (requests, responses, notifications)
//! to a separate file for debugging and analysis. The wire log is correlated with the main server log
//! via timestamps.
//!
//! ## Format
//!
//! Messages are logged with LSP framing (Content-Length headers), similar to HTTP wire logs:
//!
//! ```text
//! [2025-10-29T15:19:49.123Z] >>> REQUEST
//! Content-Length: 145
//!
//! {"jsonrpc":"2.0","id":1,"method":"textDocument/definition",...}
//!
//! [2025-10-29T15:19:49.125Z] <<< RESPONSE
//! Content-Length: 89
//!
//! {"jsonrpc":"2.0","id":1,"result":[{"uri":"file:///test.rho","range":{...}}]}
//! ```

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde_json::Value;

/// Wire logger that logs all LSP messages to a separate file
#[derive(Clone)]
pub struct WireLogger {
    writer: Arc<Mutex<Option<fs::File>>>,
    enabled: bool,
}

impl WireLogger {
    /// Create a new wire logger
    ///
    /// # Arguments
    /// * `enabled` - Whether wire logging is enabled
    /// * `log_dir` - Directory where wire log should be created
    pub fn new(enabled: bool, log_dir: Option<PathBuf>) -> io::Result<Self> {
        if !enabled {
            return Ok(WireLogger {
                writer: Arc::new(Mutex::new(None)),
                enabled: false,
            });
        }

        let log_dir = log_dir.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Log directory not provided")
        })?;

        // Create wire log filename with timestamp and PID
        let timestamp = time::OffsetDateTime::now_utc()
            .format(&time::format_description::parse(
                "[year][month][day]-[hour][minute][second]"
            ).unwrap())
            .unwrap();
        let pid = std::process::id();
        let wire_filename = format!("wire-{}-{}.log", timestamp, pid);
        let wire_path = log_dir.join(&wire_filename);

        // Create wire log file
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&wire_path)?;

        eprintln!("Wire logging to file: {:?}", wire_path);

        Ok(WireLogger {
            writer: Arc::new(Mutex::new(Some(file))),
            enabled: true,
        })
    }

    /// Create a new wire logger with a specific session ID
    ///
    /// # Arguments
    /// * `enabled` - Whether wire logging is enabled
    /// * `log_dir` - Directory where wire log should be created
    /// * `session_id` - Session identifier to use in filename (e.g., "20251029-151949-3043298")
    pub fn new_with_session_id(enabled: bool, log_dir: Option<PathBuf>, session_id: String) -> io::Result<Self> {
        if !enabled {
            return Ok(WireLogger {
                writer: Arc::new(Mutex::new(None)),
                enabled: false,
            });
        }

        let log_dir = log_dir.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Log directory not provided")
        })?;

        // Create wire log filename with matching session ID
        let wire_filename = format!("wire-{}.log", session_id);
        let wire_path = log_dir.join(&wire_filename);

        // Create wire log file
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&wire_path)?;

        eprintln!("Wire logging to file: {:?}", wire_path);

        Ok(WireLogger {
            writer: Arc::new(Mutex::new(Some(file))),
            enabled: true,
        })
    }

    /// Check if wire logging is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Log an outgoing LSP message (request or notification from server)
    pub fn log_outgoing(&self, message: &Value) {
        if !self.enabled {
            return;
        }

        if let Ok(mut writer_guard) = self.writer.lock() {
            if let Some(ref mut writer) = *writer_guard {
                let timestamp = time::OffsetDateTime::now_utc()
                    .format(&time::format_description::parse(
                        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
                    ).unwrap())
                    .unwrap();

                let message_type = if message.get("method").is_some() {
                    if message.get("id").is_some() {
                        "REQUEST"
                    } else {
                        "NOTIFICATION"
                    }
                } else {
                    "RESPONSE"
                };

                let json_body = serde_json::to_string(message).unwrap_or_else(|_| "<invalid JSON>".to_string());
                let content_length = json_body.len();

                // Log with LSP framing (Content-Length header)
                let _ = writeln!(writer, "[{}] >>> {} ", timestamp, message_type);
                let _ = writeln!(writer, "Content-Length: {}\r", content_length);
                let _ = writeln!(writer, "\r");
                let _ = writeln!(writer, "{}", json_body);
                let _ = writeln!(writer); // Blank line separator
                let _ = writer.flush();
            }
        }
    }

    /// Log an incoming LSP message (request or notification from client)
    pub fn log_incoming(&self, message: &Value) {
        if !self.enabled {
            return;
        }

        if let Ok(mut writer_guard) = self.writer.lock() {
            if let Some(ref mut writer) = *writer_guard {
                let timestamp = time::OffsetDateTime::now_utc()
                    .format(&time::format_description::parse(
                        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
                    ).unwrap())
                    .unwrap();

                let message_type = if message.get("method").is_some() {
                    if message.get("id").is_some() {
                        "REQUEST"
                    } else {
                        "NOTIFICATION"
                    }
                } else {
                    "RESPONSE"
                };

                let json_body = serde_json::to_string(message).unwrap_or_else(|_| "<invalid JSON>".to_string());
                let content_length = json_body.len();

                // Log with LSP framing (Content-Length header)
                let _ = writeln!(writer, "[{}] <<< {} ", timestamp, message_type);
                let _ = writeln!(writer, "Content-Length: {}\r", content_length);
                let _ = writeln!(writer, "\r");
                let _ = writeln!(writer, "{}", json_body);
                let _ = writeln!(writer); // Blank line separator
                let _ = writer.flush();
            }
        }
    }

    /// Log a summary message (e.g., method name only for less verbosity)
    pub fn log_summary(&self, direction: &str, method: &str, id: Option<&Value>) {
        if !self.enabled {
            return;
        }

        if let Ok(mut writer_guard) = self.writer.lock() {
            if let Some(ref mut writer) = *writer_guard {
                let timestamp = time::OffsetDateTime::now_utc()
                    .format(&time::format_description::parse(
                        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
                    ).unwrap())
                    .unwrap();

                let id_str = if let Some(id_val) = id {
                    format!(" (id: {})", id_val)
                } else {
                    String::new()
                };

                let _ = writeln!(
                    writer,
                    "[{}] {} {}{}",
                    timestamp,
                    direction,
                    method,
                    id_str
                );
                let _ = writer.flush();
            }
        }
    }
}

// Implement Debug to avoid exposing internal file handle
impl std::fmt::Debug for WireLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WireLogger")
            .field("enabled", &self.enabled)
            .finish()
    }
}
