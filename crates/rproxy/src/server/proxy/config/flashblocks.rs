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
        name("flashblocks_log_sanitise")
    )]
    pub(crate) log_sanitise: bool,

    /// the chance (between 0.0 and 1.0) that pings received from
    /// flashblocks backend would be ignored (no pong sent)
    #[arg(
        default_value = "0.0",
        help_heading = "chaos",
        env = "RPROXY_CHAOS_PROBABILITY_FLASHBLOCKS_BACKEND_PING_IGNORED",
        long("chaos-probability-flashblocks-backend-ping-ignored"),
        name("chaos_probability_flashblocks_backend_ping_ignored"),
        value_name = "probability"
    )]
    #[cfg(feature = "chaos")]
    pub(crate) chaos_probability_backend_ping_ignored: f64,

    /// the chance (between 0.0 and 1.0) that pings received from
    /// flashblocks client would be ignored (no pong sent)
    #[arg(
        default_value = "0.0",
        help_heading = "chaos",
        env = "RPROXY_CHAOS_PROBABILITY_FLASHBLOCKS_CLIENT_PING_IGNORED",
        long("chaos-probability-flashblocks-client-ping-ignored"),
        name("chaos_probability_flashblocks_client_ping_ignored"),
        value_name = "probability"
    )]
    #[cfg(feature = "chaos")]
    pub(crate) chaos_probability_client_ping_ignored: f64,

    /// the chance (between 0.0 and 1.0) that client's flashblocks stream
    /// would block (no more messages sent)
    #[arg(
        default_value = "0.0",
        help_heading = "chaos",
        env = "RPROXY_CHAOS_PROBABILITY_FLASHBLOCKS_STREAM_BLOCKED",
        long("chaos-probability-flashblocks-stream-blocked"),
        name("chaos_probability_flashblocks_stream_blocked"),
        value_name = "probability"
    )]
    #[cfg(feature = "chaos")]
    pub(crate) chaos_probability_stream_blocked: f64,
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

        #[cfg(feature = "chaos")]
        {
            // chaos_probability_flashblocks_backend_ping_ignored
            if self.chaos_probability_backend_ping_ignored < 0.0 ||
                self.chaos_probability_backend_ping_ignored > 1.0
            {
                errs.push(
                    ConfigFlashblocksError::ChaosProbabilityFlashblocksBackendPingIgnoredInvalid {
                        probability: self.chaos_probability_backend_ping_ignored,
                    },
                );
            }

            // chaos_probability_flashblocks_client_ping_ignored
            if self.chaos_probability_client_ping_ignored < 0.0 ||
                self.chaos_probability_client_ping_ignored > 1.0
            {
                errs.push(
                    ConfigFlashblocksError::ChaosProbabilityFlashblocksClientPingIgnoredInvalid {
                        probability: self.chaos_probability_client_ping_ignored,
                    },
                );
            }

            // chaos_probability_flashblocks_stream_blocked
            if self.chaos_probability_stream_blocked < 0.0 ||
                self.chaos_probability_stream_blocked > 1.0
            {
                errs.push(
                    ConfigFlashblocksError::ChaosProbabilityFlashblocksStreamBlockedInvalid {
                        probability: self.chaos_probability_stream_blocked,
                    },
                );
            }
        }

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

    #[inline]
    #[cfg(feature = "chaos")]
    fn chaos_probability_backend_ping_ignored(&self) -> f64 {
        self.chaos_probability_backend_ping_ignored
    }

    #[inline]
    #[cfg(feature = "chaos")]
    fn chaos_probability_client_ping_ignored(&self) -> f64 {
        self.chaos_probability_client_ping_ignored
    }

    #[inline]
    #[cfg(feature = "chaos")]
    fn chaos_probability_stream_blocked(&self) -> f64 {
        self.chaos_probability_stream_blocked
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

    #[error(
        "invalid flashblocks backend ping ignore probability (must be within [0.0 .. 1.0]: {probability}"
    )]
    #[cfg(feature = "chaos")]
    ChaosProbabilityFlashblocksBackendPingIgnoredInvalid { probability: f64 },

    #[error(
        "invalid flashblocks client ping ignore probability (must be within [0.0 .. 1.0]: {probability}"
    )]
    #[cfg(feature = "chaos")]
    ChaosProbabilityFlashblocksClientPingIgnoredInvalid { probability: f64 },

    #[error(
        "invalid flashblocks stream block probability (must be within [0.0 .. 1.0]: {probability}"
    )]
    #[cfg(feature = "chaos")]
    ChaosProbabilityFlashblocksStreamBlockedInvalid { probability: f64 },
}
