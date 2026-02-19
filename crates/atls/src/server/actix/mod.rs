#![cfg(feature = "actix")]

// ---------------------------------------------------------------------

use std::sync::atomic::{AtomicUsize, Ordering};

use actix_utils::counter::Counter;

// ---------------------------------------------------------------------

mod nested_tls;
pub use nested_tls::*;

mod nested_attested_tls;
pub use nested_attested_tls::*;

// ---------------------------------------------------------------------

pub(crate) static ACTIX_MAX_CONN: AtomicUsize = AtomicUsize::new(256);

pub(crate) const ACTIX_DEFAULT_TLS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(3);

thread_local! {
    pub(crate) static ACTIX_MAX_CONN_COUNTER: Counter = Counter::new(ACTIX_MAX_CONN.load(Ordering::Relaxed));
}
