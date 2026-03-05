use clap::Args;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigLogging {
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
}

impl ConfigLogging {
    pub(crate) fn setup_logging(&self) {
        match self.format {
            ConfigLoggingFormat::Json => {
                tracing_subscriber::registry()
                    .with(EnvFilter::from_default_env())
                    .with(fmt::layer().json().flatten_event(true))
                    .init();
            }

            ConfigLoggingFormat::Text => {
                tracing_subscriber::registry()
                    .with(EnvFilter::from_default_env())
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
