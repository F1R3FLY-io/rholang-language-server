use std::io;

use time::macros::format_description;
use time::UtcOffset;
use tracing_subscriber::{self, fmt, prelude::*};

pub fn init_logger(no_color: bool, log_level: Option<&str>) -> io::Result<()> {
    let timer = fmt::time::OffsetTime::new(
        UtcOffset::UTC,
        format_description!("[[[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z]"),
    );

    // Log to stderr
    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_timer(timer)
        .with_ansi(!no_color);

    // Configure the log level based on whether --log-level was provided
    let env_filter = match log_level {
        Some(level) => {
            // If --log-level is provided, use it directly
            tracing_subscriber::EnvFilter::new(level)
        }
        None => {
            // If --log-level is not provided, fall back to RUST_LOG or default to "debug"
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"))
        }
    };

    // Combine the layers using a registry
    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .init();

    Ok(())
}
