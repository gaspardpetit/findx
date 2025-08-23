use crate::cli::LogFormat;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init(format: LogFormat) {
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
