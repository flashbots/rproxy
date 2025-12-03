use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    time::Duration,
};

use clap::Args;
use thiserror::Error;
use tracing::warn;
use url::Url;

use crate::{
    config::{ALREADY_VALIDATED, PARALLELISM_STRING},
    server::proxy::http::config::{ConfigProxyHttp, ConfigProxyHttpMirroringStrategy},
    utils::get_all_local_ip_addresses,
};

// ConfigRpc -----------------------------------------------------------

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigRpc {
    /// url of rpc backend
    #[arg(
        default_value = "http://127.0.0.1:18645",
        env = "RPROXY_RPC_BACKEND",
        help_heading = "rpc",
        long("rpc-backend"),
        name("rpc_backend"),
        value_name = "url"
    )]
    pub(crate) backend_url: String,

    /// max concurrent requests per backend
    #[arg(
        default_value = PARALLELISM_STRING.as_str(),
        env = "RPROXY_RPC_BACKEND_MAX_CONCURRENT_REQUESTS",
        help_heading = "rpc",
        long("rpc-backend-max-concurrent-requests"),
        name("rpc_backend_max_concurrent_requests"),
        value_name = "count"
    )]
    pub(crate) backend_max_concurrent_requests: usize,

    /// max duration for backend requests
    #[arg(
        default_value = "30s",
        env = "RPROXY_RPC_BACKEND_TIMEOUT",
        help_heading = "rpc",
        long("rpc-backend-timeout"),
        name("rpc_backend_timeout"),
        value_name = "duration",
        value_parser = humantime::parse_duration
    )]
    pub(crate) backend_timeout: Duration,

    /// enable rpc proxy
    #[arg(
        env = "RPROXY_RPC_ENABLED",
        help_heading = "rpc",
        long("rpc-enabled"),
        name("rpc_enabled")
    )]
    pub(crate) enabled: bool,

    /// duration to keep idle rpc connections open (0 means no keep-alive)
    #[arg(
        default_value = "30s",
        env = "RPROXY_RPC_IDLE_CONNECTION_TIMEOUT",
        help_heading = "rpc",
        long("rpc-idle-connection-timeout"),
        name("rpc_idle_connection_timeout"),
        value_name = "duration",
        value_parser = humantime::parse_duration
    )]
    pub(crate) idle_connection_timeout: Duration,

    /// interval between tcp keepalive packets on rpc connections
    #[arg(
        default_value = "5s",
        env = "RPROXY_RPC_KEEPALIVE_INTERVAL",
        help_heading = "rpc",
        long("rpc-keepalive-interval"),
        name("rpc_keepalive_interval"),
        value_name = "duration",
        value_parser = humantime::parse_duration
    )]
    pub(crate) keepalive_interval: Duration,

    /// host:port for rpc proxy
    #[arg(
        default_value = "0.0.0.0:8645",
        env = "RPROXY_RPC_LISTEN_ADDRESS",
        help_heading = "rpc",
        long("rpc-listen-address"),
        name("rpc_listen_address"),
        value_name = "socket"
    )]
    pub(crate) listen_address: String,

    /// whether to log proxied rpc requests
    #[arg(
        env = "RPROXY_RPC_LOG_MIRRORED_REQUESTS",
        help_heading = "rpc",
        long("rpc-log-mirrored-requests"),
        name("rpc_log_mirrored_requests")
    )]
    pub(crate) log_mirrored_requests: bool,

    /// whether to log responses to proxied rpc requests
    #[arg(
        env = "RPROXY_RPC_LOG_MIRRORED_RESPONSES",
        help_heading = "rpc",
        long("rpc-log-mirrored-responses"),
        name("rpc_log_mirrored_responses")
    )]
    pub(crate) log_mirrored_responses: bool,

    /// whether to log proxied rpc requests
    #[arg(
        env = "RPROXY_RPC_LOG_PROXIED_REQUESTS",
        help_heading = "rpc",
        long("rpc-log-proxied-requests"),
        name("rpc_log_proxied_requests")
    )]
    pub(crate) log_proxied_requests: bool,

    /// whether to log responses to proxied rpc requests
    #[arg(
        env = "RPROXY_RPC_LOG_PROXIED_RESPONSES",
        help_heading = "rpc",
        long("rpc-log-proxied-responses"),
        name("rpc_log_proxied_responses")
    )]
    pub(crate) log_proxied_responses: bool,

    /// sanitise logs of proxied rpc requests/responses (e.g. don't log raw
    /// transactions)
    #[arg(
        help_heading = "rpc",
        env = "RPROXY_RPC_LOG_SANITISE",
        long("rpc-log-sanitise"),
        name("rpc-log_sanitise")
    )]
    pub(crate) log_sanitise: bool,

    /// max size of rpc requests
    #[arg(
        default_value = "16",
        env = "RPROXY_RPC_MAX_REQUEST_SIZE_MB",
        help_heading = "rpc",
        long("rpc-max-request-size-mb"),
        name("rpc_max_request_size_mb"),
        value_name = "megabytes"
    )]
    pub(crate) max_request_size_mb: usize,

    /// max size of rpc responses
    #[arg(
        default_value = "256",
        env = "RPROXY_RPC_MAX_RESPONSE_SIZE_MB",
        help_heading = "rpc",
        long("rpc-max-response-size-mb"),
        name("rpc_max_response_size_mb"),
        value_name = "megabytes"
    )]
    pub(crate) max_response_size_mb: usize,

    /// whether the requests that returned an error from rpc backend should
    /// be mirrored to peers
    #[arg(
        help_heading = "rpc",
        env = "RPROXY_RPC_MIRROR_ERRORED_REQUESTS",
        long("rpc-mirror-errored-requests"),
        name("rpc_mirror_errored_requests")
    )]
    pub(crate) mirror_errored_requests: bool,

    /// list of rpc peers urls to mirror the requests to
    #[arg(
        env="RPROXY_RPC_MIRRORING_PEERS",
        help_heading = "rpc",
        long("rpc-mirroring-peer"),
        name("rpc_mirroring_peer"),
        num_args = 1..,
        value_name="url"
    )]
    pub(crate) mirroring_peer_urls: Vec<String>,

    #[arg(
        default_value = "fan-out",
        env = "RPROXY_RPC_MIRRORING_STRATEGY",
        help_heading = "rpc",
        long("rpc-mirroring-strategy"),
        name("rpc_mirroring_strategy"),
        value_name = "strategy"
    )]
    #[clap(value_enum)]
    pub(crate) mirroring_strategy: ConfigProxyHttpMirroringStrategy,

    /// size of preallocated rpc request buffers
    #[arg(
        default_value = "1",
        env = "RPROXY_RPC_PREALLOCATED_RESPONSE_BUFFER_SIZE_KB",
        help_heading = "rpc",
        long("rpc-preallocated-request-buffer-size-kb"),
        name("rpc_preallocated_request_buffer_size_kb"),
        value_name = "kilobytes"
    )]
    pub(crate) prealloacated_request_buffer_size_kb: usize,

    /// size of preallocated rpc response buffers
    #[arg(
        default_value = "256",
        env = "RPROXY_RPC_PREALLOCATED_RESPONSE_BUFFER_SIZE_KB",
        help_heading = "rpc",
        long("rpc-preallocated-response-buffer-size-kb"),
        name("rpc_preallocated_response_buffer_size_kb"),
        value_name = "kilobytes"
    )]
    pub(crate) prealloacated_response_buffer_size_kb: usize,

    /// remove rpc backend from peers
    #[arg(
        env = "RPROXY_RPC_REMOVE_BACKEND_FROM_MIRRORING_PEERS",
        help_heading = "rpc",
        long("rpc-remove-backend-from-mirroring-peers"),
        name("rpc_remove_backend_from_mirroring_peers")
    )]
    pub(crate) remove_backend_from_mirroring_peers: bool,
}

impl ConfigRpc {
    pub(crate) fn validate(&self) -> Option<Vec<ConfigRpcError>> {
        let mut errs: Vec<ConfigRpcError> = vec![];

        // backend_url
        match Url::parse(&self.backend_url) {
            Ok(url) => {
                if url.host().is_none() {
                    errs.push(ConfigRpcError::BackendUrlMissesHost {
                        url: self.backend_url.clone(),
                    });
                }
            }

            Err(err) => {
                errs.push(ConfigRpcError::BackendUrlInvalid { url: self.backend_url.clone(), err });
            }
        }

        // listen_address
        let _ = self.listen_address.parse::<SocketAddr>().map_err(|err| {
            errs.push(ConfigRpcError::ListenAddressInvalid {
                addr: self.listen_address.clone(),
                err,
            })
        });

        // mirroring_peer_urls
        for peer_url in self.mirroring_peer_urls.iter() {
            match Url::parse(peer_url) {
                Ok(url) => {
                    if url.host().is_none() {
                        errs.push(ConfigRpcError::PeerUrlMissesHost { url: peer_url.clone() });
                    }
                }

                Err(err) => {
                    errs.push(ConfigRpcError::PeerUrlInvalid { url: peer_url.clone(), err });
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

impl ConfigProxyHttp for ConfigRpc {
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
    fn keepalive_interval(&self) -> Duration {
        self.keepalive_interval
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
    fn max_request_size(&self) -> usize {
        1024 * 1024 * self.max_request_size_mb
    }

    #[inline]
    fn max_response_size(&self) -> usize {
        1024 * 1024 * self.max_response_size_mb
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

    #[inline]
    fn prealloacated_request_buffer_size(&self) -> usize {
        1024 * self.prealloacated_request_buffer_size_kb
    }

    #[inline]
    fn prealloacated_response_buffer_size(&self) -> usize {
        1024 * self.prealloacated_response_buffer_size_kb
    }
}

// ConfigRpcError ------------------------------------------------------

#[derive(Debug, Clone, Error)]
pub(crate) enum ConfigRpcError {
    #[error("invalid rpc backend url '{url}': {err}")]
    BackendUrlInvalid { url: String, err: url::ParseError },

    #[error("invalid rpc backend url '{url}': host is missing")]
    BackendUrlMissesHost { url: String },

    #[error("invalid rpc proxy listen address '{addr}': {err}")]
    ListenAddressInvalid { addr: String, err: std::net::AddrParseError },

    #[error("invalid rpc peer url '{url}': {err}")]
    PeerUrlInvalid { url: String, err: url::ParseError },

    #[error("invalid rpc peer url '{url}': host is missing")]
    PeerUrlMissesHost { url: String },
}
