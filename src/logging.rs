use std::io;
use std::fs;
use std::path::PathBuf;

use time::macros::format_description;
use time::UtcOffset;
use tracing_subscriber::{self, fmt, prelude::*};
use tracing_appender::non_blocking::WorkerGuard;

use crate::wire_logger::WireLogger;

const LOG_RETENTION_DAYS: u64 = 7;

/// Get the log directory path in the user-specific OS cache directory
/// - Linux: ~/.cache/f1r3fly-io/rholang-language-server/
/// - macOS: ~/Library/Caches/f1r3fly-io/rholang-language-server/
/// - Windows: %LOCALAPPDATA%\f1r3fly-io\rholang-language-server\
fn get_log_dir() -> io::Result<PathBuf> {
    let cache_dir = dirs::cache_dir()
        .ok_or_else(|| io::Error::new(
            io::ErrorKind::NotFound,
            "Unable to determine user cache directory"
        ))?;

    let mut log_dir = cache_dir;
    log_dir.push("f1r3fly-io");
    log_dir.push("rholang-language-server");

    // Create directory if it doesn't exist (creates parent directories as needed)
    if !log_dir.exists() {
        fs::create_dir_all(&log_dir)?;
    }

    Ok(log_dir)
}

/// Clean up log files older than LOG_RETENTION_DAYS
fn cleanup_old_logs(log_dir: &PathBuf) -> io::Result<()> {
    let now = std::time::SystemTime::now();
    let retention = std::time::Duration::from_secs(LOG_RETENTION_DAYS * 24 * 60 * 60);

    if let Ok(entries) = fs::read_dir(log_dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        // Clean up both session and wire logs
                        if (name.starts_with("session-") || name.starts_with("wire-")) && name.ends_with(".log") {
                            if let Ok(modified) = metadata.modified() {
                                if let Ok(age) = now.duration_since(modified) {
                                    if age > retention {
                                        if let Err(e) = fs::remove_file(entry.path()) {
                                            eprintln!("Failed to remove old log file {:?}: {}", entry.path(), e);
                                        } else {
                                            eprintln!("Removed old log file: {:?}", entry.path());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Initialize logger with both stderr and file output
/// Returns a tuple of (WorkerGuard, WireLogger) that must be kept alive for the duration of the program
///
/// # Arguments
/// * `no_color` - Disable ANSI colors in stderr output
/// * `log_level` - Override log level (otherwise uses RUST_LOG or defaults to "info")
/// * `enable_file_logging` - Enable file logging to temp directory (disable for tests)
/// * `enable_wire_logging` - Enable wire protocol logging to separate file
///
/// # Logging Behavior
/// - **Stderr/Console**: Logs at the configured level (default "info") - shows method names and key identifiers, NOT full payloads
/// - **Session File**: Logs at DEBUG level - includes detailed diagnostics with full parameters
/// - **Wire Log**: If enabled, logs all LSP JSON-RPC messages with Content-Length headers (LSP framing format)
pub fn init_logger(no_color: bool, log_level: Option<&str>, enable_file_logging: bool, enable_wire_logging: bool) -> io::Result<(WorkerGuard, WireLogger)> {
    let timer = fmt::time::OffsetTime::new(
        UtcOffset::UTC,
        format_description!("[[[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z]"),
    );

    // Configure the stderr log level based on whether --log-level was provided
    let stderr_filter = match log_level {
        Some(level) => {
            // If --log-level is provided, use it directly
            tracing_subscriber::EnvFilter::new(level)
        }
        None => {
            // If --log-level is not provided, fall back to RUST_LOG or default to "info"
            // This provides cleaner logs by default while still allowing verbose debugging via RUST_LOG
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        }
    };

    // Log to stderr with the configured filter level
    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_timer(timer.clone())
        .with_ansi(!no_color)
        .with_filter(stderr_filter);

    // File logs at DEBUG level by default
    let file_filter = tracing_subscriber::EnvFilter::new("debug");

    // Conditionally enable file logging and wire logging
    if enable_file_logging {
        // Get log directory and clean up old logs
        let log_dir = get_log_dir()?;
        cleanup_old_logs(&log_dir)?;

        // Create session-specific identifier (shared between session log and wire log)
        let timestamp = time::OffsetDateTime::now_utc()
            .format(&time::format_description::parse(
                "[year][month][day]-[hour][minute][second]"
            ).unwrap())
            .unwrap();
        let pid = std::process::id();
        let session_id = format!("{}-{}", timestamp, pid);

        // Create wire logger with matching session identifier
        let wire_logger = if enable_wire_logging {
            WireLogger::new_with_session_id(true, Some(log_dir.clone()), session_id.clone())?
        } else {
            WireLogger::new(false, None)?
        };

        // Create session-specific log filename
        let log_filename = format!("session-{}.log", session_id);
        let log_path = log_dir.join(&log_filename);

        // Log to file with non-blocking writer
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;

        let (non_blocking, guard) = tracing_appender::non_blocking(file);
        let file_layer = fmt::layer()
            .with_writer(non_blocking)
            .with_timer(timer)
            .with_ansi(false) // No ANSI colors in file
            .with_filter(file_filter);

        // Combine the layers using a registry
        // Note: Each layer has its own filter, so no global filter needed
        let result = tracing_subscriber::registry()
            .with(stderr_layer)
            .with(file_layer)
            .try_init();

        match result {
            Ok(()) => {
                eprintln!("Logging to file: {:?}", log_path);
                Ok((guard, wire_logger))
            }
            Err(e) => {
                // Ignore errors due to the subscriber or logger already being set
                if e.to_string().contains("already been set") || e.to_string().contains("SetLoggerError") {
                    eprintln!("Logging to file: {:?}", log_path);
                    Ok((guard, wire_logger))
                } else {
                    // Propagate unexpected errors
                    Err(io::Error::new(io::ErrorKind::Other, e))
                }
            }
        }
    } else {
        // No file logging - use a dummy guard and disabled wire logger
        let (_, guard) = tracing_appender::non_blocking(std::io::sink());
        let wire_logger = WireLogger::new(false, None)?;

        // Combine the layers using a registry (stderr only)
        // Note: stderr_layer already has its own filter
        let result = tracing_subscriber::registry()
            .with(stderr_layer)
            .try_init();

        match result {
            Ok(()) => Ok((guard, wire_logger)),
            Err(e) => {
                // Ignore errors due to the subscriber or logger already being set
                if e.to_string().contains("already been set") || e.to_string().contains("SetLoggerError") {
                    Ok((guard, wire_logger))
                } else {
                    // Propagate unexpected errors
                    Err(io::Error::new(io::ErrorKind::Other, e))
                }
            }
        }
    }
}
