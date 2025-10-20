use clap::Args;
use thiserror::Error;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigLogging {
    /// logging format
    #[arg(
        default_value = "json",
        env = "RPROXY_LOG_FORMAT",
        help_heading = "log",
        long("log-format"),
        name("log_format"),
        value_name = "format"
    )]
    #[clap(value_enum)]
    pub(crate) format: ConfigLoggingFormat,

    /// logging level
    #[arg(
        help_heading = "log",
        default_value = "info",
        env = "RPROXY_LOG_LEVEL",
        long("log-level"),
        name("log_level"),
        value_name = "level"
    )]
    pub(crate) level: String,
}

impl ConfigLogging {
    pub(crate) fn validate(self) -> Option<Vec<ConfigLoggingError>> {
        let mut errs: Vec<ConfigLoggingError> = vec![];

        // level
        let _ = EnvFilter::builder().parse(self.level).map_err(|err| {
            errs.push(ConfigLoggingError::LevelInvalid { err: err.to_string() });
        });

        match errs.len() {
            0 => None,
            _ => Some(errs),
        }
    }

    pub(crate) fn setup_logging(&self) {
        match self.format {
            ConfigLoggingFormat::Json => {
                tracing_subscriber::registry()
                    .with(EnvFilter::from(self.level.clone()))
                    .with(fmt::layer().json().flatten_event(true))
                    .init();
            }

            ConfigLoggingFormat::Text => {
                tracing_subscriber::registry()
                    .with(EnvFilter::from(self.level.clone()))
                    .with(fmt::layer())
                    .init();
            }
        }
    }
}

// ConfigLogFormat -----------------------------------------------------

#[derive(Clone, Debug, clap::ValueEnum)]
pub(crate) enum ConfigLoggingFormat {
    Json,
    Text,
}

// ConfigLogError ------------------------------------------------------

#[derive(Debug, Clone, Error)]
pub(crate) enum ConfigLoggingError {
    #[error("invalid log level filter: {err}")]
    LevelInvalid { err: String },
}
