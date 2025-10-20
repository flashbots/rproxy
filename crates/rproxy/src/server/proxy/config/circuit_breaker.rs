use std::time::Duration;

use clap::Args;
use thiserror::Error;
use url::Url;

use crate::config::ALREADY_VALIDATED;

// ConfigCircuitBreaker ------------------------------------------------

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigCircuitBreaker {
    /// reset proxies continuously at each poll interval as long as
    /// circuit-breaker url reports unhealthy
    #[arg(
        default_value = "false",
        env = "RPROXY_CIRCUIT_BREAKER_RESET_CONTINUOUSLY",
        help_heading = "circuit-breaker",
        long("circuit-breaker-reset-continuously"),
        name("circuit_breaker_reset_continuously")
    )]
    pub(crate) reset_continuously: bool,

    /// circuit breaker's poll interval
    #[arg(
        default_value = "5s",
        env = "RPROXY_CIRCUIT_BREAKER_POLL_INTERVAL",
        help_heading =  "circuit-breaker",
        long("circuit-breaker-poll-interval"),
        name("circuit_breaker_poll_interval"),
        value_name = "duration",
        value_parser = humantime::parse_duration
    )]
    pub(crate) poll_interval: Duration,

    /// healthy threshold for circuit-breaker
    #[arg(
        default_value = "2",
        env = "RPROXY_CIRCUIT_BREAKER_THRESHOLD_HEALTHY",
        help_heading = "circuit-breaker",
        long("circuit-breaker-threshold-healthy"),
        name("circuit_breaker_threshold_healthy"),
        value_name = "count"
    )]
    pub(crate) threshold_healthy: usize,

    /// unhealthy threshold for circuit-breaker
    #[arg(
        default_value = "3",
        env = "RPROXY_CIRCUIT_BREAKER_THRESHOLD_UNHEALTHY",
        help_heading = "circuit-breaker",
        long("circuit-breaker-threshold-unhealthy"),
        name("circuit_breaker_threshold_unhealthy"),
        value_name = "count"
    )]
    pub(crate) threshold_unhealthy: usize,

    /// url of circuit-breaker (e.g. backend healthcheck)
    #[arg(
        default_value = "",
        env = "RPROXY_CIRCUIT_BREAKER_URL",
        help_heading = "circuit-breaker",
        long("circuit-breaker-url"),
        name("circuit_breaker_url"),
        value_name = "url"
    )]
    pub(crate) url: String,
}

impl ConfigCircuitBreaker {
    pub(crate) fn url(&self) -> Url {
        self.url.parse::<Url>().expect(ALREADY_VALIDATED)
    }

    pub(crate) fn validate(&self) -> Option<Vec<ConfigCircuitBreakerError>> {
        let mut errs: Vec<ConfigCircuitBreakerError> = vec![];

        // threshold_healthy
        if self.threshold_healthy < 1 {
            errs.push(ConfigCircuitBreakerError::ThresholdHealthyInvalid {
                threshold: self.threshold_healthy,
            });
        }

        // threshold_unhealthy
        if self.threshold_unhealthy < 1 {
            errs.push(ConfigCircuitBreakerError::ThresholdUnhealthyInvalid {
                threshold: self.threshold_unhealthy,
            });
        }

        // url
        if !self.url.is_empty() {
            match Url::parse(&self.url) {
                Ok(url) => {
                    if url.host().is_none() {
                        errs.push(ConfigCircuitBreakerError::UrlMissesHost {
                            url: self.url.clone(),
                        });
                    }
                }

                Err(err) => {
                    errs.push(ConfigCircuitBreakerError::UrlInvalid { url: self.url.clone(), err });
                }
            }
        }

        match errs.len() {
            0 => None,
            _ => Some(errs),
        }
    }
}

// ConfigCircuitBreakerError -------------------------------------------

#[derive(Debug, Clone, Error)]
pub(crate) enum ConfigCircuitBreakerError {
    #[error("healthy threshold '{threshold}' is invalid: must be greater than 0")]
    ThresholdHealthyInvalid { threshold: usize },

    #[error("unhealthy threshold '{threshold}' is invalid: must be greater than 0")]
    ThresholdUnhealthyInvalid { threshold: usize },

    #[error("invalid circuit breaker url '{url}': {err}")]
    UrlInvalid { url: String, err: url::ParseError },

    #[error("invalid circuit breaker url '{url}': host is missing")]
    UrlMissesHost { url: String },
}
