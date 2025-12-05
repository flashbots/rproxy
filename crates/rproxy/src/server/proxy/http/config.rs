use std::{net::SocketAddr, time::Duration};

use url::Url;

// ConfigProxyHttp -----------------------------------------------------

pub(crate) trait ConfigProxyHttp: Clone + Send + Unpin + 'static {
    fn backend_max_concurrent_requests(&self) -> usize;
    fn backend_timeout(&self) -> Duration;
    fn backend_url(&self) -> Url;
    fn idle_connection_timeout(&self) -> Duration;
    fn keepalive_interval(&self) -> Duration;
    fn keepalive_retries(&self) -> u32;
    fn listen_address(&self) -> SocketAddr;
    fn log_mirrored_requests(&self) -> bool;
    fn log_mirrored_responses(&self) -> bool;
    fn log_proxied_requests(&self) -> bool;
    fn log_proxied_responses(&self) -> bool;
    fn log_sanitise(&self) -> bool;
    fn max_request_size(&self) -> usize;
    fn max_response_size(&self) -> usize;
    fn mirroring_peer_urls(&self) -> Vec<Url>;
    fn mirroring_strategy(&self) -> &ConfigProxyHttpMirroringStrategy;
    fn prealloacated_request_buffer_size(&self) -> usize;
    fn prealloacated_response_buffer_size(&self) -> usize;
}

// ConfigProxyHttpMirroringStrategy ------------------------------------

#[derive(Clone, Debug, clap::ValueEnum)]
pub(crate) enum ConfigProxyHttpMirroringStrategy {
    /// mirror to all configured peers
    #[value(name = "fan-out")]
    FanOut,

    /// mirror to only 1 peer at a time, in round-robin fashion
    #[value(name = "round-robin")]
    RoundRobin,

    /// mirror to 2 peers at a time, in round-robin fashion
    #[value(name = "round-robin-pairs")]
    RoundRobinPairs,
}
