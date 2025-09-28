use std::net::SocketAddr;

use clap::Args;
use thiserror::Error;

use crate::config::ALREADY_VALIDATED;

// ConfigMetrics -------------------------------------------------------

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigMetrics {
    /// host:port for metrics
    #[arg(
        default_value = "0.0.0.0:6785",
        env = "RPROXY_METRICS_LISTEN_ADDRESS",
        help_heading = "metrics",
        long("metrics-listen-address"),
        name("metrics_listen_address"),
        value_name = "socket"
    )]
    pub(crate) listen_address: String,
}

impl ConfigMetrics {
    pub(crate) fn validate(&self) -> Option<Vec<ConfigMetricsError>> {
        let mut errs: Vec<ConfigMetricsError> = vec![];

        // listen_address
        let _ = self.listen_address.parse::<SocketAddr>().map_err(|err| {
            errs.push(ConfigMetricsError::ListenAddressInvalid {
                addr: self.listen_address.clone(),
                err,
            })
        });

        match errs.len() {
            0 => None,
            _ => Some(errs),
        }
    }

    #[inline]
    pub(crate) fn listen_address(&self) -> SocketAddr {
        self.listen_address.parse::<SocketAddr>().expect(ALREADY_VALIDATED)
    }
}

// ConfigMetricsError --------------------------------------------------

#[derive(Debug, Clone, Error)]
pub(crate) enum ConfigMetricsError {
    #[error("invalid metrics listen address '{addr}': {err}")]
    ListenAddressInvalid { addr: String, err: std::net::AddrParseError },
}
