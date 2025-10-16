use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    time::Duration,
};

use clap::Args;
use thiserror::Error;
use tracing::warn;
use url::Url;

use crate::{
    config::{ALREADY_VALIDATED, ConfigProxyHttp, ConfigProxyHttpMirroringStrategy},
    utils::get_all_local_ip_addresses,
};

// ConfigAuthrpc -------------------------------------------------------

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigAuthrpc {
    /// url of authrpc backend
    #[arg(
        default_value = "http://127.0.0.1:18651",
        env = "RPROXY_AUTHRPC_BACKEND",
        help_heading = "authrpc",
        long("authrpc-backend"),
        name("authrpc_backend"),
        value_name = "url"
    )]
    pub(crate) backend_url: String,

    /// max concurrent requests per authrpc backend
    #[arg(
        default_value = "1",
        env = "RPROXY_AUTHRPC_BACKEND_MAX_CONCURRENT_REQUESTS",
        help_heading = "authrpc",
        long("authrpc-backend-max-concurrent-requests"),
        name("authrpc_backend_max_concurrent_requests"),
        value_name = "count"
    )]
    pub(crate) backend_max_concurrent_requests: usize,

    /// max duration for authrpc backend requests
    #[arg(
        default_value = "30s",
        env = "RPROXY_AUTHRPC_BACKEND_TIMEOUT",
        help_heading = "authrpc",
        long("authrpc-backend-timeout"),
        name("authrpc_backend_timeout"),
        value_name = "duration",
        value_parser = humantime::parse_duration
    )]
    pub(crate) backend_timeout: Duration,

    /// enable authrpc proxy
    #[arg(
        env = "RPROXY_AUTHRPC_ENABLED",
        help_heading = "authrpc",
        long("authrpc-enabled"),
        name("authrpc_enabled")
    )]
    pub(crate) enabled: bool,

    /// duration to keep idle authrpc connections open (0 means no
    /// keep-alive)
    #[arg(
        default_value = "30s",
        env = "RPROXY_AUTHRPC_IDLE_CONNECTION_TIMEOUT",
        help_heading = "authrpc",
        long("authrpc-idle-connection-timeout"),
        name("authrpc_idle_connection_timeout"),
        value_name = "duration",
        value_parser = humantime::parse_duration
    )]
    pub(crate) idle_connection_timeout: Duration,

    /// host:port for authrpc proxy
    #[arg(
        default_value = "0.0.0.0:8651",
        env = "RPROXY_AUTHRPC_LISTEN_ADDRESS",
        help_heading = "authrpc",
        long("authrpc-listen-address"),
        name("authrpc_listen_address"),
        value_name = "socket"
    )]
    pub(crate) listen_address: String,

    /// whether to log proxied authrpc requests
    #[arg(
        env = "RPROXY_AUTHRPC_LOG_MIRRORED_REQUESTS",
        help_heading = "authrpc",
        long("authrpc-log-mirrored-requests"),
        name("authrpc_log_mirrored_requests")
    )]
    pub(crate) log_mirrored_requests: bool,

    /// whether to log responses to proxied authrpc requests
    #[arg(
        env = "RPROXY_AUTHRPC_LOG_MIRRORED_RESPONSES",
        help_heading = "authrpc",
        long("authrpc-log-mirrored-responses"),
        name("authrpc_log_mirrored_responses")
    )]
    pub(crate) log_mirrored_responses: bool,

    /// whether to log proxied authrpc requests
    #[arg(
        env = "RPROXY_AUTHRPC_LOG_PROXIED_REQUESTS",
        help_heading = "authrpc",
        long("authrpc-log-proxied-requests"),
        name("authrpc_log_proxied_requests")
    )]
    pub(crate) log_proxied_requests: bool,

    /// whether to log responses to proxied authrpc requests
    #[arg(
        env = "RPROXY_AUTHRPC_LOG_PROXIED_RESPONSES",
        help_heading = "authrpc",
        long("authrpc-log-proxied-responses"),
        name("authrpc_log_proxied_responses")
    )]
    pub(crate) log_proxied_responses: bool,

    /// sanitise logs of proxied authrpc requests/responses (e.g. don't log
    /// raw transactions)
    #[arg(
        help_heading = "authrpc",
        env = "RPROXY_AUTHRPC_LOG_SANITISE",
        long("authrpc-log-sanitise"),
        name("authrpc_log_sanitise")
    )]
    pub(crate) log_sanitise: bool,

    /// list of authrpc peers urls to mirror the requests to
    #[arg(
        env="RPROXY_AUTHRPC_MIRRORING_PEERS",
        help_heading = "authrpc",
        long("authrpc-mirroring-peer"),
        name("authrpc_mirroring_peer"),
        num_args = 1..,
        value_name="url"
    )]
    pub(crate) mirroring_peer_urls: Vec<String>,

    #[arg(
        default_value = "fan-out",
        env = "RPROXY_AUTHRPC_MIRRORING_STRATEGY",
        help_heading = "authrpc",
        long("authrpc-mirroring-strategy"),
        name("authrpc_mirroring_strategy"),
        value_name = "strategy"
    )]
    #[clap(value_enum)]
    pub(crate) mirroring_strategy: ConfigProxyHttpMirroringStrategy,

    /// remove authrpc backend from mirroring peers
    #[arg(
        env = "RPROXY_AUTHRPC_REMOVE_BACKEND_FROM_MIRRORING_PEERS",
        help_heading = "authrpc",
        long("authrpc-remove-backend-from-mirroring-peers"),
        name("authrpc_remove_backend_from_mirroring_peers")
    )]
    pub(crate) remove_backend_from_mirroring_peers: bool,
}

impl ConfigAuthrpc {
    pub(crate) fn validate(&self) -> Option<Vec<ConfigAuthrpcError>> {
        let mut errs: Vec<ConfigAuthrpcError> = vec![];

        // backend_url
        match Url::parse(&self.backend_url) {
            Ok(url) => {
                if url.host().is_none() {
                    errs.push(ConfigAuthrpcError::BackendUrlMissesHost {
                        url: self.backend_url.clone(),
                    });
                }
            }

            Err(err) => {
                errs.push(ConfigAuthrpcError::BackendUrlInvalid {
                    url: self.backend_url.clone(),
                    err,
                });
            }
        }

        // listen_address
        let _ = self.listen_address.parse::<SocketAddr>().map_err(|err| {
            errs.push(ConfigAuthrpcError::ListenAddressInvalid {
                addr: self.listen_address.clone(),
                err,
            })
        });

        // mirroring_peer_urls
        for peer_url in self.mirroring_peer_urls.iter() {
            match Url::parse(peer_url) {
                Ok(url) => {
                    if url.host().is_none() {
                        errs.push(ConfigAuthrpcError::PeerUrlMissesHost { url: peer_url.clone() });
                    }
                }

                Err(err) => {
                    errs.push(ConfigAuthrpcError::PeerUrlInvalid { url: peer_url.clone(), err });
                }
            }
        }

        match errs.len() {
            0 => None,
            _ => Some(errs),
        }
    }

    pub(crate) fn preprocess(&mut self) {
        if !self.enabled {
            return;
        }

        if self.remove_backend_from_mirroring_peers {
            let backend_url = Url::parse(&self.backend_url.clone()).expect(ALREADY_VALIDATED);
            let backend_host = backend_url.host_str().expect(ALREADY_VALIDATED);

            let backend_ips: Vec<IpAddr> = match format!("{backend_host}:0").to_socket_addrs() {
                Ok(res) => res,
                Err(err) => {
                    warn!(host = backend_host, error = ?err, "Failed to resolve backend host");
                    vec![].into_iter()
                }
            }
            .map(|addr| addr.ip())
            .collect();

            let local_ips = get_all_local_ip_addresses();

            self.mirroring_peer_urls.retain(|url| {
                let peer_url = Url::parse(url).expect(ALREADY_VALIDATED);
                let peer_host = peer_url.host_str().expect(ALREADY_VALIDATED);

                if !peer_url.port().eq(&backend_url.port()) {
                    // if ports don't match, keep the peer
                    return true;
                }

                if peer_url.host().eq(&backend_url.host()) {
                    // if backend's and peer's hostnames are exact match, drop the peer
                    return false;
                }

                let peer_ips: Vec<IpAddr> = match format!("{peer_host}:0").to_socket_addrs() {
                    Ok(res) => res,
                    Err(err) => {
                        warn!(host = peer_host, error = ?err, "Failed to resolve peer host");
                        vec![].into_iter()
                    }
                }
                .map(|addr| addr.ip())
                .collect();

                if peer_ips.iter().any(|peer_ip| backend_ips.contains(peer_ip)) {
                    // if peer's IPs overlap with backend's, drop the peer
                    return false;
                }

                if backend_ips.iter().any(|backend_ip| backend_ip.is_loopback()) &&
                    peer_ips.iter().any(|peer_ip| local_ips.contains(peer_ip))
                {
                    // if backend is loopback and peer resolves to one of the local addresses,
                    // drop the peer
                    return false;
                }

                true
            });
        }
    }
}

impl ConfigProxyHttp for ConfigAuthrpc {
    #[inline]
    fn backend_max_concurrent_requests(&self) -> usize {
        self.backend_max_concurrent_requests
    }

    #[inline]
    fn backend_timeout(&self) -> Duration {
        self.backend_timeout
    }

    #[inline]
    fn backend_url(&self) -> Url {
        self.backend_url.parse::<Url>().expect(ALREADY_VALIDATED)
    }

    #[inline]
    fn idle_connection_timeout(&self) -> Duration {
        self.idle_connection_timeout
    }

    #[inline]
    fn listen_address(&self) -> SocketAddr {
        self.listen_address.parse::<SocketAddr>().expect(ALREADY_VALIDATED)
    }

    #[inline]
    fn log_mirrored_requests(&self) -> bool {
        self.log_mirrored_requests
    }

    #[inline]
    fn log_mirrored_responses(&self) -> bool {
        self.log_mirrored_responses
    }

    #[inline]
    fn log_proxied_requests(&self) -> bool {
        self.log_proxied_requests
    }

    #[inline]
    fn log_proxied_responses(&self) -> bool {
        self.log_proxied_responses
    }

    #[inline]
    fn log_sanitise(&self) -> bool {
        self.log_sanitise
    }

    #[inline]
    fn mirroring_peer_urls(&self) -> Vec<Url> {
        self.mirroring_peer_urls
            .iter()
            .map(|peer_url| peer_url.parse::<Url>().expect(ALREADY_VALIDATED))
            .collect()
    }

    #[inline]
    fn mirroring_strategy(&self) -> &ConfigProxyHttpMirroringStrategy {
        &self.mirroring_strategy
    }
}

// ConfigAuthrpcError --------------------------------------------------

#[derive(Debug, Clone, Error)]
pub(crate) enum ConfigAuthrpcError {
    #[error("invalid authrpc backend url '{url}': {err}")]
    BackendUrlInvalid { url: String, err: url::ParseError },

    #[error("invalid authrpc backend url '{url}': host is missing")]
    BackendUrlMissesHost { url: String },

    #[error("invalid authrpc proxy listen address '{addr}': {err}")]
    ListenAddressInvalid { addr: String, err: std::net::AddrParseError },

    #[error("invalid authrpc peer url '{url}': {err}")]
    PeerUrlInvalid { url: String, err: url::ParseError },

    #[error("invalid authrpc peer url '{url}': host is missing")]
    PeerUrlMissesHost { url: String },
}
