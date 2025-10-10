use std::{net::TcpListener, sync::Arc, time::Duration};

use actix_web::{
    App,
    HttpRequest,
    HttpResponse,
    HttpServer,
    middleware::{NormalizePath, TrailingSlash},
    web,
};
use awc::http::Method;
use prometheus_client::{
    metrics::{counter::Counter, family::Family, gauge::Gauge},
    registry::{Registry, Unit},
};
use socket2::{SockAddr, Socket, TcpKeepalive};
use tracing::{error, info};

use crate::{
    config::ConfigMetrics,
    metrics::{
        Candlestick,
        LabelsProxy,
        LabelsProxyClientInfo,
        LabelsProxyHttpJrpc,
        LabelsProxyWs,
    },
};

// Metrics -------------------------------------------------------------

pub(crate) struct Metrics {
    config: ConfigMetrics,
    registry: Registry,

    pub(crate) client_connections_active_count: Family<LabelsProxy, Gauge>,
    pub(crate) client_connections_established_count: Family<LabelsProxy, Counter>,
    pub(crate) client_connections_closed_count: Family<LabelsProxy, Counter>,
    pub(crate) client_info: Family<LabelsProxyClientInfo, Counter>,

    pub(crate) http_latency_backend: Family<LabelsProxyHttpJrpc, Candlestick>,
    pub(crate) http_latency_delta: Family<LabelsProxyHttpJrpc, Candlestick>,
    pub(crate) http_latency_total: Family<LabelsProxyHttpJrpc, Candlestick>,

    pub(crate) http_mirror_success_count: Family<LabelsProxyHttpJrpc, Counter>,
    pub(crate) http_mirror_failure_count: Family<LabelsProxy, Counter>,

    pub(crate) http_proxy_success_count: Family<LabelsProxyHttpJrpc, Counter>,
    pub(crate) http_proxy_failure_count: Family<LabelsProxy, Counter>,

    pub(crate) http_request_size: Family<LabelsProxyHttpJrpc, Candlestick>,
    pub(crate) http_response_size: Family<LabelsProxyHttpJrpc, Candlestick>,

    pub(crate) http_request_decompressed_size: Family<LabelsProxyHttpJrpc, Candlestick>,
    pub(crate) http_response_decompressed_size: Family<LabelsProxyHttpJrpc, Candlestick>,

    pub(crate) tls_certificate_valid_not_before: Gauge,
    pub(crate) tls_certificate_valid_not_after: Gauge,

    pub(crate) ws_latency_backend: Family<LabelsProxyWs, Candlestick>,
    pub(crate) ws_latency_client: Family<LabelsProxyWs, Candlestick>,
    pub(crate) ws_latency_proxy: Family<LabelsProxyWs, Candlestick>,

    pub(crate) ws_message_size: Family<LabelsProxyWs, Candlestick>,

    pub(crate) ws_proxy_success_count: Family<LabelsProxyWs, Counter>,
    pub(crate) ws_proxy_failure_count: Family<LabelsProxyWs, Counter>,
}

impl Metrics {
    pub(crate) fn new(config: ConfigMetrics) -> Self {
        let mut this = Metrics {
            config,
            registry: Registry::with_prefix("rproxy"),

            client_connections_active_count: Family::default(),
            client_connections_established_count: Family::default(),
            client_connections_closed_count: Family::default(),

            client_info: Family::default(),

            http_latency_backend: Family::default(),
            http_latency_delta: Family::default(),
            http_latency_total: Family::default(),

            http_mirror_success_count: Family::default(),
            http_mirror_failure_count: Family::default(),

            http_proxy_success_count: Family::default(),
            http_proxy_failure_count: Family::default(),

            http_request_size: Family::default(),
            http_response_size: Family::default(),

            http_request_decompressed_size: Family::default(),
            http_response_decompressed_size: Family::default(),

            tls_certificate_valid_not_before: Gauge::default(),
            tls_certificate_valid_not_after: Gauge::default(),

            ws_latency_backend: Family::default(),
            ws_latency_client: Family::default(),
            ws_latency_proxy: Family::default(),

            ws_message_size: Family::default(),

            ws_proxy_success_count: Family::default(),
            ws_proxy_failure_count: Family::default(),
        };

        this.registry.register(
            "client_connections_active_count",
            "count of active client connections",
            this.client_connections_active_count.clone(),
        );

        this.registry.register(
            "client_connections_established_count",
            "count of client connections established",
            this.client_connections_established_count.clone(),
        );

        this.registry.register(
            "client_info",
            "general information about the client",
            this.client_info.clone(),
        );

        this.registry.register(
            "client_connections_closed_count",
            "count of client connections closed",
            this.client_connections_closed_count.clone(),
        );

        this.registry.register_with_unit(
            "http_latency_backend",
            "latency of backend http responses (interval b/w end of client's request and begin of backend's response)",
            Unit::Other(String::from("nanoseconds")),
            this.http_latency_backend.clone(),
        );

        this.registry.register_with_unit(
            "http_latency_delta",
            "latency delta (http_latency_total - http_latency_backend)",
            Unit::Other(String::from("nanoseconds")),
            this.http_latency_delta.clone(),
        );

        this.registry.register_with_unit(
            "http_latency_total",
            "overall latency of http requests (interval b/w begin of client's request and end of forwarded response)",
            Unit::Other(String::from("nanoseconds")),
            this.http_latency_total.clone(),
        );

        this.registry.register(
            "http_mirror_success_count",
            "count of successfully mirrored http requests/responses",
            this.http_mirror_success_count.clone(),
        );

        this.registry.register(
            "http_mirror_failure_count",
            "count of failures to mirror http request/response",
            this.http_mirror_failure_count.clone(),
        );

        this.registry.register(
            "http_proxy_success_count",
            "count of successfully proxied http requests/responses",
            this.http_proxy_success_count.clone(),
        );

        this.registry.register(
            "http_proxy_failure_count",
            "count of failures to proxy http request/response",
            this.http_proxy_failure_count.clone(),
        );

        this.registry.register_with_unit(
            "http_request_size",
            "sizes of incoming http requests",
            Unit::Bytes,
            this.http_request_size.clone(),
        );

        this.registry.register_with_unit(
            "http_response_size",
            "sizes of proxied http responses",
            Unit::Bytes,
            this.http_response_size.clone(),
        );

        this.registry.register_with_unit(
            "http_request_decompressed_size",
            "decompressed sizes of incoming http requests",
            Unit::Bytes,
            this.http_request_decompressed_size.clone(),
        );

        this.registry.register_with_unit(
            "http_response_decompressed_size",
            "decompressed sizes of proxied http responses",
            Unit::Bytes,
            this.http_response_decompressed_size.clone(),
        );

        this.registry.register(
            "tls_certificate_valid_not_before",
            "tls certificate's not-valid-before timestamp",
            this.tls_certificate_valid_not_before.clone(),
        );

        this.registry.register(
            "tls_certificate_valid_not_after",
            "tls certificate's not-valid-after timestamp",
            this.tls_certificate_valid_not_after.clone(),
        );

        this.registry.register_with_unit(
            "ws_latency_backend",
            "round-trip-time of websocket pings to backend divided by 2",
            Unit::Other(String::from("nanoseconds")),
            this.ws_latency_backend.clone(),
        );

        this.registry.register_with_unit(
            "ws_latency_client",
            "round-trip-time of websocket pings to backend divided by 2",
            Unit::Other(String::from("nanoseconds")),
            this.ws_latency_client.clone(),
        );

        this.registry.register_with_unit(
            "ws_latency_proxy",
            "time to process the websocket message by the proxy",
            Unit::Other(String::from("nanoseconds")),
            this.ws_latency_proxy.clone(),
        );

        this.registry.register_with_unit(
            "ws_message_size",
            "sizes of proxied websocket messages",
            Unit::Bytes,
            this.ws_message_size.clone(),
        );

        this.registry.register(
            "ws_proxy_success_count",
            "count of successfully proxied websocket messages",
            this.ws_proxy_success_count.clone(),
        );

        this.registry.register(
            "ws_proxy_failure_count",
            "count of failures to proxy websocket message",
            this.ws_proxy_failure_count.clone(),
        );

        this
    }

    #[inline]
    pub(crate) fn name() -> &'static str {
        "metrics"
    }

    pub(crate) async fn run(
        self: Arc<Self>,
        canceller: tokio_util::sync::CancellationToken,
    ) -> Result<(), Box<dyn std::error::Error + Send>> {
        let listen_address = self.config.listen_address().clone();

        let listener = match self.listen() {
            Ok(listener) => listener,
            Err(err) => {
                error!(
                    service = Self::name(),
                    addr = %&self.config.listen_address(),
                    error = ?err,
                    "Failed to initialise a socket"
                );
                return Err(Box::new(err));
            }
        };

        let server = match HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(self.clone()))
                .wrap(NormalizePath::new(TrailingSlash::Trim))
                .default_service(web::route().to(Self::receive))
        })
        .workers(1)
        .shutdown_signal(canceller.cancelled_owned())
        .listen(listener)
        {
            Ok(metrics) => metrics,
            Err(err) => {
                error!(service = Self::name(), error = ?err, "Failed to initialise http service");
                return Err(Box::new(err));
            }
        };

        info!(
            service = Self::name(),
            listen_address = %listen_address,
            "Starting http service...",
        );

        if let Err(err) = server.run().await {
            error!(service = Self::name(), error = ?err, "Failure while running http service")
        }

        info!(service = Self::name(), "Stopped http service");

        Ok(())
    }

    fn listen(&self) -> std::io::Result<TcpListener> {
        let socket = Socket::new(
            socket2::Domain::for_address(self.config.listen_address()),
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;

        // must use non-blocking with tokio
        socket.set_nonblocking(true)?;

        // allow time to flush buffers on close
        socket.set_linger(Some(Duration::from_secs(1)))?;

        // allow binding to the socket whlie there are still TIME_WAIT conns
        socket.set_reuse_address(true)?;

        socket.set_tcp_keepalive(
            &TcpKeepalive::new()
                .with_time(Duration::from_secs(15))
                .with_interval(Duration::from_secs(15))
                .with_retries(4),
        )?;

        socket.bind(&SockAddr::from(self.config.listen_address()))?;

        socket.listen(16)?;

        Ok(socket.into())
    }

    async fn receive(
        req: HttpRequest,
        _: web::Payload,
        this: web::Data<Arc<Self>>,
    ) -> Result<HttpResponse, actix_web::Error> {
        if req.method() != Method::GET {
            return Ok(HttpResponse::BadRequest().finish());
        }

        let mut body = String::new();

        if let Err(err) = prometheus_client::encoding::text::encode(&mut body, &this.registry) {
            error!(service = Self::name(), error = ?err, "Failed to encode metrics");
            return Ok(HttpResponse::InternalServerError().finish());
        }

        Ok(HttpResponse::Ok()
            .content_type("application/openmetrics-text; version=1.0.0; charset=utf-8")
            .body(body))
    }
}
