use std::{net::SocketAddr, time::Duration};

use awc::http::Uri;
use clap::Args;
use thiserror::Error;

use crate::{config::ALREADY_VALIDATED, server::proxy::ws::config::ConfigProxyWs};

// ConfigFlashblocks ---------------------------------------------------

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigFlashblocks {
    /// url of flashblocks backend
    #[arg(
        default_value = "ws://127.0.0.1:11111",
        env = "RPROXY_FLASHBLOCKS_BACKEND",
        help_heading = "flashblocks",
        long("flashblocks-backend"),
        name("flashblocks_backend"),
        value_name = "url"
    )]
    pub(crate) backend_url: String,

    /// enable flashblocks proxy
    #[arg(
        env = "RPROXY_FLASHBLOCKS_ENABLED",
        help_heading = "flashblocks",
        long("flashblocks-enabled"),
        name("flashblocks_enabled")
    )]
    pub(crate) enabled: bool,

    /// timeout to establish backend connections of to receive pong
    /// websocket response
    #[arg(
        default_value = "30s",
        env = "RPROXY_FLASHBLOCKS_BACKEND_TIMEOUT",
        help_heading = "flashblocks",
        long("flashblocks-backend-timeout"),
        name("flashblocks_backend_timeout"),
        value_name = "duration",
        value_parser = humantime::parse_duration
    )]
    pub(crate) backend_timeout: Duration,

    /// host:port for flashblocks proxy
    #[arg(
        default_value = "0.0.0.0:1111",
        env = "RPROXY_FLASHBLOCKS_LISTEN_ADDRESS",
        help_heading = "flashblocks",
        long("flashblocks-listen-address"),
        name("flashblocks_listen_address"),
        value_name = "socket"
    )]
    pub(crate) listen_address: String,

    /// whether to log flashblocks backend messages
    #[arg(
        env = "RPROXY_FLASHBLOCKS_LOG_BACKEND_MESSAGES",
        help_heading = "flashblocks",
        long("flashblocks-log-backend-messages"),
        name("flashblocks_log_backend_messages")
    )]
    pub(crate) log_backend_messages: bool,

    /// whether to log flashblocks client messages
    #[arg(
        env = "RPROXY_FLASHBLOCKS_LOG_CLIENT_MESSAGES",
        help_heading = "flashblocks",
        long("flashblocks-log-client-messages"),
        name("flashblocks_log_client_messages")
    )]
    pub(crate) log_client_messages: bool,

    /// sanitise logs of proxied flashblocks messages (e.g. don't log raw
    /// transactions)
    #[arg(
        help_heading = "flashblocks",
        env = "RPROXY_FLASHBLOCKS_LOG_SANITISE",
        long("flashblocks-log-sanitise"),
        name("flashblocks-log_sanitise")
    )]
    pub(crate) log_sanitise: bool,
}

impl ConfigFlashblocks {
    pub(crate) fn validate(&self) -> Option<Vec<ConfigFlashblocksError>> {
        let mut errs: Vec<ConfigFlashblocksError> = vec![];

        // backend_url
        match self.backend_url.parse::<Uri>() {
            Ok(uri) => {
                if uri.authority().is_none() {
                    errs.push(ConfigFlashblocksError::BackendUrlMissesHost {
                        url: self.backend_url.clone(),
                    });
                }

                if uri.host().is_none() {
                    errs.push(ConfigFlashblocksError::BackendUrlMissesHost {
                        url: self.backend_url.clone(),
                    });
                }
            }

            Err(err) => {
                errs.push(ConfigFlashblocksError::BackendUrlInvalid {
                    url: self.backend_url.clone(),
                    err: err.to_string(),
                });
            }
        }

        // listen_address
        let _ = self.listen_address.parse::<SocketAddr>().map_err(|err| {
            errs.push(ConfigFlashblocksError::ListenAddressInvalid {
                addr: self.listen_address.clone(),
                err,
            })
        });

        match errs.len() {
            0 => None,
            _ => Some(errs),
        }
    }
}

impl ConfigProxyWs for ConfigFlashblocks {
    #[inline]
    fn backend_timeout(&self) -> Duration {
        self.backend_timeout
    }

    #[inline]
    fn backend_url(&self) -> tungstenite::http::Uri {
        self.backend_url.parse::<tungstenite::http::Uri>().expect(ALREADY_VALIDATED)
    }

    #[inline]
    fn listen_address(&self) -> SocketAddr {
        self.listen_address.parse::<SocketAddr>().expect(ALREADY_VALIDATED)
    }

    #[inline]
    fn log_backend_messages(&self) -> bool {
        self.log_backend_messages
    }

    #[inline]
    fn log_client_messages(&self) -> bool {
        self.log_client_messages
    }

    #[inline]
    fn log_sanitise(&self) -> bool {
        self.log_sanitise
    }
}

// ConfigFlashblocksError ----------------------------------------------

#[derive(Debug, Clone, Error)]
pub(crate) enum ConfigFlashblocksError {
    #[error("invalid flashblocks backend url '{url}': {err}")]
    BackendUrlInvalid { url: String, err: String },

    #[error("invalid flashblocks backend url '{url}': host is missing")]
    BackendUrlMissesHost { url: String },

    #[error("invalid flashblocks proxy listen address '{addr}': {err}")]
    ListenAddressInvalid { addr: String, err: std::net::AddrParseError },
}
