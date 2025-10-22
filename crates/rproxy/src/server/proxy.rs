pub(crate) mod circuit_breaker;
pub(crate) mod config;
pub(crate) mod http;
pub(crate) mod ws;

// ---------------------------------------------------------------------

use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use actix_web::dev::Extensions;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::server::metrics::{LabelsProxy, Metrics};

// Proxy ---------------------------------------------------------------

pub(crate) trait Proxy {
    fn on_connect(
        metrics: Arc<Metrics>,
        client_connections_count: Arc<AtomicI64>,
        proxy_name: String,
    ) -> impl Fn(&dyn Any, &mut Extensions) {
        move |connection, extensions| {
            {
                let val = client_connections_count.fetch_add(1, Ordering::Relaxed) + 1;
                let metric_labels = LabelsProxy { proxy: proxy_name.clone() };

                metrics.client_connections_active_count.get_or_create(&metric_labels).set(val);
                metrics.client_connections_established_count.get_or_create(&metric_labels).inc();
            }

            let stream: Option<&actix_web::rt::net::TcpStream> = if let Some(stream) = connection.downcast_ref::<actix_tls::accept::rustls_0_23::TlsStream<actix_web::rt::net::TcpStream>>() {
                let (stream, _) = stream.get_ref();
                Some(stream)
            } else if let Some(stream) = connection.downcast_ref::<actix_web::rt::net::TcpStream>() {
                Some(stream)
            } else {
                warn!("Unexpected stream type");
                None
            };

            if let Some(stream) = stream {
                let id = Uuid::now_v7();

                let remote_addr = match stream.peer_addr() {
                    Ok(local_addr) => Some(local_addr.to_string()),
                    Err(err) => {
                        warn!(proxy = P::name(), error = ?err, "Failed to get remote address");
                        None
                    }
                };
                let local_addr = match stream.local_addr() {
                    Ok(local_addr) => Some(local_addr.to_string()),
                    Err(err) => {
                        warn!(proxy = P::name(), error = ?err, "Failed to get remote address");
                        None
                    }
                };

                debug!(
                    proxy = P::name(),
                    connection_id = %id,
                    remote_addr = remote_addr.as_ref().map_or("unknown", |v| v.as_str()),
                    local_addr = local_addr.as_ref().map_or("unknown", |v| v.as_str()),
                    "Client connection open"
                );

                extensions.insert(ProxyConnectionGuard::new(
                    id,
                    &proxy_name,
                    remote_addr,
                    local_addr,
                    &metrics,
                    client_connections_count.clone(),
                ));
            }
        }
    }
}

// ProxyConnectionGuard ------------------------------------------------

pub struct ProxyConnectionGuard {
    pub id: Uuid,
    pub remote_addr: Option<String>,
    pub local_addr: Option<String>,

    proxy_name: String,
    metrics: Arc<Metrics>,
    client_connections_count: Arc<AtomicI64>,
}

impl ProxyConnectionGuard {
    fn new(
        id: Uuid,
        proxy_name: &str,
        remote_addr: Option<String>,
        local_addr: Option<String>,
        metrics: &Arc<Metrics>,
        client_connections_count: Arc<AtomicI64>,
    ) -> Self {
        Self {
            id,
            remote_addr,
            local_addr,
            proxy_name: proxy_name.to_string(),
            metrics: metrics.clone(),
            client_connections_count,
        }
    }
}

impl Drop for ProxyConnectionGuard {
    fn drop(&mut self) {
        let val = self.client_connections_count.fetch_sub(1, Ordering::Relaxed) - 1;

        let metric_labels = LabelsProxy { proxy: self.proxy_name.clone() };

        self.metrics.client_connections_active_count.get_or_create(&metric_labels).set(val);
        self.metrics.client_connections_closed_count.get_or_create(&metric_labels).inc();

        debug!(
            proxy = self.proxy_name,
            connection_id = %self.id,
            remote_addr = self.remote_addr.as_ref().map_or("unknown", |v| v.as_str()),
            local_addr = self.local_addr.as_ref().map_or("unknown", |v| v.as_str()),
            "Client connection closed"
        );
    }
}
