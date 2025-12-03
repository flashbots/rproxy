use std::{net::SocketAddr, time::Duration};

use tungstenite::http::Uri;

// ConfigProxyWs -------------------------------------------------------

pub(crate) trait ConfigProxyWs: Clone + Send + Unpin + 'static {
    fn backend_timeout(&self) -> Duration;
    fn backend_url(&self) -> Uri;
    fn keep_alive_interval(&self) -> Duration;
    fn listen_address(&self) -> SocketAddr;
    fn log_backend_messages(&self) -> bool;
    fn log_client_messages(&self) -> bool;
    fn log_sanitise(&self) -> bool;

    #[cfg(feature = "chaos")]
    fn chaos_probability_backend_ping_ignored(&self) -> f64;

    #[cfg(feature = "chaos")]
    fn chaos_probability_client_ping_ignored(&self) -> f64;

    #[cfg(feature = "chaos")]
    fn chaos_probability_stream_blocked(&self) -> f64;
}
