use std::{process, sync::LazyLock};

use clap::Parser;
use sysctl::Sysctl;
use thiserror::Error;
use x509_parser::asn1_rs::ToStatic;

use crate::server::{
    config::{ConfigLogging, ConfigLoggingError, ConfigMetrics, ConfigMetricsError},
    proxy::config::{
        ConfigAuthrpc,
        ConfigAuthrpcError,
        ConfigCircuitBreaker,
        ConfigCircuitBreakerError,
        ConfigFlashblocks,
        ConfigFlashblocksError,
        ConfigRpc,
        ConfigRpcError,
        ConfigTls,
        ConfigTlsError,
    },
};

pub(crate) const ALREADY_VALIDATED: &str = "parameter must have been validated already";

pub(crate) static PARALLELISM: LazyLock<usize> =
    LazyLock::new(|| std::thread::available_parallelism().map_or(2, std::num::NonZero::get));

pub(crate) static PARALLELISM_STRING: LazyLock<String> = LazyLock::new(|| PARALLELISM.to_string());

pub(crate) static TCP_KEEPALIVE_INTERVAL: LazyLock<libc::c_int> = LazyLock::new(|| {
    #[cfg(target_os = "linux")]
    {
        let mut res: libc::c_int = 75;
        if let Ok(ctl) = sysctl::Ctl::new("net.ipv4.tcp_keepalive_intvl") &&
            let Ok(value) = ctl.value() &&
            let Ok(value) = value.into_int()
        {
            res = value
        }
        return res;
    }

    #[cfg(target_os = "macos")]
    {
        let mut res: libc::c_int = 75000;
        if let Ok(ctl) = sysctl::Ctl::new("net.inet.tcp.keepintvl") &&
            let Ok(value) = ctl.value() &&
            let Ok(value) = value.into_int()
        {
            res = value / 1000 // millis on macos
        }

        return res;
    }

    #[allow(unreachable_code)]
    75
});

pub(crate) static TCP_KEEPALIVE_INTERVAL_STRING: LazyLock<String> =
    LazyLock::new(|| format!("{}s", TCP_KEEPALIVE_INTERVAL.to_static()));

pub(crate) static TCP_KEEPALIVE_PROBES: LazyLock<libc::c_int> = LazyLock::new(|| {
    #[cfg(target_os = "linux")]
    {
        let mut res: libc::c_int = 9;
        if let Ok(ctl) = sysctl::Ctl::new("net.ipv4.tcp_keepalive_probes") &&
            let Ok(value) = ctl.value() &&
            let Ok(value) = value.into_int()
        {
            res = value
        }
        return res;
    }

    #[cfg(target_os = "macos")]
    {
        let mut res: libc::c_int = 8;
        if let Ok(ctl) = sysctl::Ctl::new("net.inet.tcp.keepcnt") &&
            let Ok(value) = ctl.value() &&
            let Ok(value) = value.into_int()
        {
            res = value
        }

        return res;
    }

    #[allow(unreachable_code)]
    8
});

pub(crate) static TCP_KEEPALIVE_PROBES_STRING: LazyLock<String> =
    LazyLock::new(|| TCP_KEEPALIVE_PROBES.to_string());

// Config --------------------------------------------------------------

#[derive(Clone, Parser)]
#[command(about, author, long_about = None, term_width = 90, version)]
pub struct Config {
    #[command(flatten)]
    pub(crate) authrpc: ConfigAuthrpc,

    #[command(flatten)]
    pub(crate) circuit_breaker: ConfigCircuitBreaker,

    #[command(flatten)]
    pub(crate) flashblocks: ConfigFlashblocks,

    #[command(flatten)]
    pub(crate) logging: ConfigLogging,

    #[command(flatten)]
    pub(crate) metrics: ConfigMetrics,

    #[command(flatten)]
    pub(crate) rpc: ConfigRpc,

    #[command(flatten)]
    pub(crate) tls: ConfigTls,
}

impl Config {
    pub fn setup() -> Self {
        let mut res = Config::parse();

        if let Some(errs) = res.clone().validate() {
            for err in errs.iter() {
                eprintln!("fatal: {err}");
            }
            process::exit(1);
        };

        res.logging.setup_logging();

        res.preprocess();

        res
    }

    pub(crate) fn validate(self) -> Option<Vec<ConfigError>> {
        let mut errs: Vec<ConfigError> = vec![];

        // authrpc proxy
        if self.rpc.enabled &&
            let Some(_errs) = self.authrpc.validate()
        {
            errs.append(&mut _errs.into_iter().map(|err| err.into()).collect());
        }

        // circuit-breaker
        if let Some(_errs) = self.circuit_breaker.validate() {
            errs.append(&mut _errs.into_iter().map(|err| err.into()).collect());
        }

        // flashblocks proxy
        if self.flashblocks.enabled &&
            let Some(_errs) = self.flashblocks.validate()
        {
            errs.append(&mut _errs.into_iter().map(|err| err.into()).collect());
        }

        // logging
        if let Some(_errs) = self.logging.validate() {
            errs.append(&mut _errs.into_iter().map(|err| err.into()).collect());
        }

        // metrics
        if let Some(_errs) = self.metrics.validate() {
            errs.append(&mut _errs.into_iter().map(|err| err.into()).collect());
        }

        // rpc proxy
        if self.rpc.enabled &&
            let Some(_errs) = self.rpc.validate()
        {
            errs.append(&mut _errs.into_iter().map(|err| err.into()).collect());
        }

        // tls
        if (!self.tls.certificate.is_empty() || !self.tls.key.is_empty()) &&
            let Some(_errs) = self.tls.validate()
        {
            errs.append(&mut _errs.into_iter().map(|err| err.into()).collect());
        }

        if !self.authrpc.enabled && !self.flashblocks.enabled && !self.rpc.enabled {
            errs.push(ConfigError::NoEnabledProxies);
        }

        match errs.len() {
            0 => None,
            _ => Some(errs),
        }
    }

    pub(crate) fn preprocess(&mut self) {
        self.authrpc.preprocess();
        self.rpc.preprocess();
    }
}

// ConfigError ---------------------------------------------------------

#[derive(Debug, Error)]
pub(crate) enum ConfigError {
    #[error("invalid authrpc proxy configuration: {0}")]
    ConfigAuthrpcInvalid(ConfigAuthrpcError),

    #[error("invalid circuit-breaker configuration: {0}")]
    ConfigCircuitBreakerInvalid(ConfigCircuitBreakerError),

    #[error("invalid flashblocks proxy configuration: {0}")]
    ConfigFlashblocksInvalid(ConfigFlashblocksError),

    #[error("invalid logging configuration: {0}")]
    ConfigLoggingInvalid(ConfigLoggingError),

    #[error("invalid metrics configuration: {0}")]
    ConfigMetricsInvalid(ConfigMetricsError),

    #[error("invalid rpc proxy configuration: {0}")]
    ConfigRpcInvalid(ConfigRpcError),

    #[error("invalid tls configuration: {0}")]
    ConfigTlsInvalid(ConfigTlsError),

    #[error("no enabled proxies")]
    NoEnabledProxies,
}

impl From<ConfigAuthrpcError> for ConfigError {
    fn from(err: ConfigAuthrpcError) -> Self {
        Self::ConfigAuthrpcInvalid(err)
    }
}

impl From<ConfigCircuitBreakerError> for ConfigError {
    fn from(err: ConfigCircuitBreakerError) -> Self {
        Self::ConfigCircuitBreakerInvalid(err)
    }
}

impl From<ConfigFlashblocksError> for ConfigError {
    fn from(err: ConfigFlashblocksError) -> Self {
        Self::ConfigFlashblocksInvalid(err)
    }
}

impl From<ConfigLoggingError> for ConfigError {
    fn from(err: ConfigLoggingError) -> Self {
        Self::ConfigLoggingInvalid(err)
    }
}

impl From<ConfigMetricsError> for ConfigError {
    fn from(err: ConfigMetricsError) -> Self {
        Self::ConfigMetricsInvalid(err)
    }
}

impl From<ConfigRpcError> for ConfigError {
    fn from(err: ConfigRpcError) -> Self {
        Self::ConfigRpcInvalid(err)
    }
}

impl From<ConfigTlsError> for ConfigError {
    fn from(err: ConfigTlsError) -> Self {
        Self::ConfigTlsInvalid(err)
    }
}
