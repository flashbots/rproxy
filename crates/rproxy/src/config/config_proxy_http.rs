use std::{net::SocketAddr, time::Duration};

use url::Url;

// ConfigProxyHttp -----------------------------------------------------

pub(crate) trait ConfigProxyHttp: Clone + Send + Unpin + 'static {
    fn backend_max_concurrent_requests(&self) -> usize;
    fn backend_timeout(&self) -> Duration;
    fn backend_url(&self) -> Url;
    fn idle_connection_timeout(&self) -> Duration;
    fn listen_address(&self) -> SocketAddr;
    fn log_mirrored_requests(&self) -> bool;
    fn log_mirrored_responses(&self) -> bool;
    fn log_proxied_requests(&self) -> bool;
    fn log_proxied_responses(&self) -> bool;
    fn log_sanitise(&self) -> bool;
    fn mirroring_peer_urls(&self) -> Vec<Url>;
}
