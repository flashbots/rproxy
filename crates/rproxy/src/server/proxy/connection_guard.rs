use std::{
    any::Any,
    sync::{
        Arc,
        LazyLock,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use actix_web::dev::Extensions;
use sysctl::Sysctl;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    server::metrics::{LabelsProxy, Metrics},
    utils::setup_keepalive,
};

pub(crate) static TCP_KEEPALIVE_ATTEMPTS: LazyLock<libc::c_int> = LazyLock::new(|| {
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
        if let Ok(ctl) = sysctl::Ctl::new("net.ipv4.tcp_keepalive_probes") &&
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

// ProxyConnectionGuard ------------------------------------------------

pub(crate) struct ConnectionGuard {
    pub id: Uuid,
    pub remote_addr: Option<String>,
    pub local_addr: Option<String>,

    proxy: &'static str,
    metrics: Arc<Metrics>,
    client_connections_count: Arc<AtomicI64>,
}

impl ConnectionGuard {
    fn new(
        id: Uuid,
        proxy: &'static str,
        remote_addr: Option<String>,
        local_addr: Option<String>,
        metrics: Arc<Metrics>,
        client_connections_count: Arc<AtomicI64>,
    ) -> Self {
        Self {
            id,
            remote_addr,
            local_addr,
            proxy,
            metrics: metrics.clone(),
            client_connections_count,
        }
    }

    pub(crate) fn on_connect(
        proxy: &'static str,
        metrics: Arc<Metrics>,
        client_connections_count: Arc<AtomicI64>,
        keep_alive_interval: Duration,
    ) -> impl Fn(&dyn Any, &mut Extensions) {
        move |connection, extensions| {
            {
                let val = client_connections_count.fetch_add(1, Ordering::Relaxed) + 1;
                let metric_labels = LabelsProxy { proxy };

                metrics.client_connections_active_count.get_or_create(&metric_labels).set(val);
                metrics.client_connections_established_count.get_or_create(&metric_labels).inc();
            }

            let stream: Option<&actix_web::rt::net::TcpStream> = if let Some(stream) = connection.downcast_ref::<actix_tls::accept::rustls_0_23::TlsStream<actix_web::rt::net::TcpStream>>() {
                let (stream, _) = stream.get_ref();
                Some(stream)
            } else if let Some(stream) = connection.downcast_ref::<actix_web::rt::net::TcpStream>() {
                Some(stream)
            } else {
                warn!(
                    proxy = proxy,
                    connection_type = std::any::type_name_of_val(connection),
                    "Unexpected connection type",
                );
                None
            };

            if let Some(stream) = stream &&
                let Err(err) = setup_keepalive(stream, keep_alive_interval)
            {
                warn!(
                    proxy = proxy,
                    error = ?err,
                    "Failed to set keepalive interval",
                );
            }

            if let Some(stream) = stream {
                let id = Uuid::now_v7();

                let remote_addr = match stream.peer_addr() {
                    Ok(local_addr) => Some(local_addr.to_string()),
                    Err(err) => {
                        warn!(
                            proxy = proxy,
                            error = ?err,
                            "Failed to get remote peer address of incoming connection",
                        );
                        None
                    }
                };
                let local_addr = match stream.local_addr() {
                    Ok(local_addr) => Some(local_addr.to_string()),
                    Err(err) => {
                        warn!(
                            proxy = proxy,
                            error = ?err,
                            "Failed to get local peer address for incoming connection",
                        );
                        None
                    }
                };

                #[cfg(debug_assertions)]
                debug!(
                    proxy = proxy,
                    connection_id = %id,
                    remote_addr = remote_addr.as_ref().map_or("unknown", |v| v.as_str()),
                    local_addr = local_addr.as_ref().map_or("unknown", |v| v.as_str()),
                    "Client connection open"
                );

                extensions.insert(ConnectionGuard::new(
                    id,
                    proxy,
                    remote_addr,
                    local_addr,
                    metrics.clone(),
                    client_connections_count.clone(),
                ));
            }
        }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let val = self.client_connections_count.fetch_sub(1, Ordering::Relaxed) - 1;

        let metric_labels = LabelsProxy { proxy: self.proxy };

        self.metrics.client_connections_active_count.get_or_create(&metric_labels).set(val);
        self.metrics.client_connections_closed_count.get_or_create(&metric_labels).inc();

        debug!(
            proxy = self.proxy,
            connection_id = %self.id,
            remote_addr = self.remote_addr.as_ref().map_or("unknown", |v| v.as_str()),
            local_addr = self.local_addr.as_ref().map_or("unknown", |v| v.as_str()),
            "Client connection closed"
        );
    }
}
