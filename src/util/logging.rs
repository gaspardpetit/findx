use crate::cli::LogFormat;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init(format: LogFormat) {
    if std::env::var("RUST_LOG").is_err() {
        if let Ok(level) = std::env::var("LOG_LEVEL") {
            std::env::set_var("RUST_LOG", level);
        }
    }
    let filter = EnvFilter::from_default_env();
    match format {
        LogFormat::Json => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json())
                .init();
        }
        LogFormat::Text => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
}
