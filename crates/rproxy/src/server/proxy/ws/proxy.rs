use std::{
    io::Write,
    marker::PhantomData,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use actix::{Actor, AsyncContext, WrapFuture};
use actix_web::{
    App,
    HttpRequest,
    HttpResponse,
    HttpServer,
    middleware::{NormalizePath, TrailingSlash},
    web,
};
use actix_ws::{MessageStream, Session};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{
    SinkExt,
    StreamExt,
    stream::{SplitSink, SplitStream},
};
use prometheus_client::metrics::gauge::Atomic;
use scc::HashMap;
use time::{UtcDateTime, format_description::well_known::Iso8601};
use tokio::{net::TcpStream, sync::broadcast};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use x509_parser::asn1_rs::ToStatic;

use crate::{
    config::PARALLELISM,
    server::{
        metrics::{LabelsProxyWs, Metrics},
        proxy::{
            ConnectionGuard,
            TCP_KEEPALIVE_ATTEMPTS,
            config::ConfigTls,
            http::ProxyHttpRequestInfo,
            ws::{ProxyWsInner, config::ConfigProxyWs},
        },
    },
    utils::{Loggable, raw_transaction_to_hash, setup_keepalive},
};

const WS_PING_INTERVAL_SECONDS: u64 = 1;

const WS_CLNT_ERROR: &str = "client error";
const WS_BKND_ERROR: &str = "backend error";
const WS_CLOSE_OK: &str = "";

const WS_CLOSE_REASON_NORMAL: &str = "normal-close";
const WS_CLOSE_REASON_UNSPECIFIED: &str = "unexpected-close";

const WS_LABEL_BKND: &str = "backend";
const WS_LABEL_CLNT: &str = "client";

// ProxyWs -------------------------------------------------------------

pub(crate) struct ProxyWs<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    id: Uuid,

    shared: ProxyWsSharedState<C, P>,
    postprocessor: actix::Addr<ProxyWsPostprocessor<C, P>>,
    canceller: tokio_util::sync::CancellationToken,
    resetter: broadcast::Sender<()>,

    backend: ProxyWsBackendEndpoint<C, P>,
}

impl<C, P> ProxyWs<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    fn new(
        shared: ProxyWsSharedState<C, P>,
        canceller: tokio_util::sync::CancellationToken,
        resetter: broadcast::Sender<()>,
    ) -> Self {
        let id = Uuid::now_v7();

        let config = shared.config();

        let backend = ProxyWsBackendEndpoint::new(id, config.backend_url());

        let postprocessor = ProxyWsPostprocessor::<C, P> {
            inner: shared.inner.clone(),
            metrics: shared.metrics.clone(),
            worker_id: id,
            _config: PhantomData,
        }
        .start();

        Self { id, shared, postprocessor, canceller, resetter, backend }
    }

    fn config(&self) -> &C {
        self.shared.config()
    }

    pub(crate) async fn run(
        config: C,
        tls: ConfigTls,
        metrics: Arc<Metrics>,
        canceller: tokio_util::sync::CancellationToken,
        resetter: broadcast::Sender<()>,
    ) -> Result<(), Box<dyn std::error::Error + Send>> {
        let listen_address = config.listen_address();

        let listener = match Self::listen(&config) {
            Ok(listener) => listener,
            Err(err) => {
                error!(
                    proxy = P::name(),
                    addr = %config.listen_address(),
                    error = ?err,
                    "Failed to initialise a socket"
                );
                return Err(Box::new(err));
            }
        };

        let workers_count = PARALLELISM.to_static();

        let shared = ProxyWsSharedState::<C, P>::new(config.clone(), &metrics);
        let client_connections_count = shared.client_connections_count.clone();
        let worker_canceller = canceller.clone();
        let worker_resetter = resetter.clone();

        info!(
            proxy = P::name(),
            listen_address = %listen_address,
            workers_count = workers_count,
            "Starting websocket-proxy...",
        );

        let server = HttpServer::new(move || {
            let this = web::Data::new(Self::new(
                shared.clone(),
                worker_canceller.clone(),
                worker_resetter.clone(),
            ));

            App::new()
                .app_data(this)
                .wrap(NormalizePath::new(TrailingSlash::Trim))
                .default_service(web::route().to(Self::receive))
        })
        .keep_alive(config.keep_alive_interval())
        .on_connect(ConnectionGuard::on_connect(
            P::name(),
            metrics,
            client_connections_count,
            config.keep_alive_interval(),
        ))
        .shutdown_signal(canceller.cancelled_owned())
        .workers(workers_count);

        let proxy = match if tls.enabled() {
            let cert = tls.certificate().clone();
            let key = tls.key().clone_key();

            server.listen_rustls_0_23(
                listener,
                rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(cert, key)
                    .unwrap(),
            )
        } else {
            server.listen(listener)
        } {
            Ok(server) => server,
            Err(err) => {
                error!(proxy = P::name(), error = ?err, "Failed to initialise websocket-proxy");
                return Err(Box::new(err));
            }
        }
        .run();

        let handler = proxy.handle();
        let mut resetter = resetter.subscribe();
        tokio::spawn(async move {
            if resetter.recv().await.is_ok() {
                info!(proxy = P::name(), "Reset signal received, stopping websocket-proxy...");
                handler.stop(true).await;
            }
        });

        if let Err(err) = proxy.await {
            error!(proxy = P::name(), error = ?err, "Failure while running websocket-proxy")
        }

        info!(proxy = P::name(), "Stopped websocket-proxy");

        Ok(())
    }

    fn listen(config: &C) -> std::io::Result<std::net::TcpListener> {
        let socket = socket2::Socket::new(
            socket2::Domain::for_address(config.listen_address()),
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;

        // allow keep-alive packets
        let keep_alive_timeout = config.backend_timeout().mul_f64(2.0);
        let keep_alive_interval = keep_alive_timeout.div_f64(f64::from(*TCP_KEEPALIVE_ATTEMPTS));
        socket.set_keepalive(true)?;
        socket.set_tcp_keepalive(
            &socket2::TcpKeepalive::new()
                .with_time(keep_alive_interval)
                .with_interval(keep_alive_interval)
                .with_retries(*TCP_KEEPALIVE_ATTEMPTS as u32),
        )?;

        // must use non-blocking with tokio
        socket.set_nonblocking(true)?;

        // allow time to flush buffers on close
        socket.set_linger(Some(config.backend_timeout()))?;

        // allow binding to the socket while there are still TIME_WAIT connections
        socket.set_reuse_address(true)?;

        socket.bind(&socket2::SockAddr::from(config.listen_address()))?;

        socket.listen(1024)?;

        Ok(socket.into())
    }

    #[expect(clippy::unused_async, reason = "required by the actix framework")]
    async fn receive(
        clnt_req: HttpRequest,
        clnt_req_body: web::Payload,
        this: web::Data<Self>,
    ) -> Result<HttpResponse, actix_web::Error> {
        let info = ProxyHttpRequestInfo::new(&clnt_req, clnt_req.conn_data::<ConnectionGuard>());

        let (res, clnt_tx, clnt_rx) = match actix_ws::handle(&clnt_req, clnt_req_body) {
            Ok(res) => res,
            Err(err) => {
                error!(
                    proxy = P::name(),
                    request_id = %info.req_id(),
                    connection_id = %info.conn_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to upgrade to websocket",
                );
                return Err(err);
            }
        };

        actix_web::rt::spawn(Self::handshake(this, clnt_tx, clnt_rx, info));

        Ok(res)
    }

    async fn handshake(
        this: web::Data<Self>,
        clnt_tx: Session,
        clnt_rx: MessageStream,
        info: ProxyHttpRequestInfo,
    ) {
        let bknd_uri = this.backend.new_backend_uri(&info);
        debug!(
            proxy = P::name(),
            request_id = %info.req_id(),
            connection_id = %info.conn_id(),
            worker_id = %this.id,
            backend_uri = %bknd_uri,
            "Starting websocket handshake...",
        );

        let (bknd_stream, _) = match tokio::time::timeout(
            this.config().backend_timeout(),
            tokio_tungstenite::connect_async(bknd_uri),
        )
        .await
        {
            Ok(Ok(res)) => res,

            Ok(Err(err)) => {
                error!(
                    proxy = P::name(),
                    request_id = %info.req_id(),
                    connection_id = %info.conn_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to establish backend websocket session"
                );
                let _ = clnt_tx // only 1 possible error (i.e. "already closed")
                    .close(Some(actix_ws::CloseReason {
                        code: awc::ws::CloseCode::Error,
                        description: Some(String::from(WS_BKND_ERROR)),
                    }))
                    .await;
                return;
            }

            Err(_) => {
                // only 1 possible error (timed out)
                error!(
                    proxy = P::name(),
                    request_id = %info.req_id(),
                    connection_id = %info.conn_id(),
                    worker_id = %this.id,
                    "Timed out to establish backend websocket session"
                );
                let _ = clnt_tx // only 1 possible error (i.e. "already closed")
                    .close(Some(actix_ws::CloseReason {
                        code: awc::ws::CloseCode::Error,
                        description: Some(String::from(WS_BKND_ERROR)),
                    }))
                    .await;
                return;
            }
        };

        match bknd_stream.get_ref() {
            tokio_tungstenite::MaybeTlsStream::Plain(tcp_stream) => {
                if let Err(err) = setup_keepalive(tcp_stream, this.config().keep_alive_interval()) {
                    warn!(
                        proxy = P::name(),
                        request_id = %info.req_id(),
                        connection_id = %info.conn_id(),
                        worker_id = %this.id,
                        error = ?err,
                        "Failed to set keepalive interval"
                    );
                }
            }

            tokio_tungstenite::MaybeTlsStream::Rustls(tls_stream) => {
                let (tcp_stream, _) = tls_stream.get_ref();
                if let Err(err) = setup_keepalive(tcp_stream, this.config().keep_alive_interval()) {
                    warn!(
                        proxy = P::name(),
                        request_id = %info.req_id(),
                        connection_id = %info.conn_id(),
                        worker_id = %this.id,
                        error = ?err,
                        "Failed to set keepalive interval"
                    );
                }
            }

            &_ => {} // there's nothing else
        }

        let (bknd_tx, bknd_rx) = bknd_stream.split();

        let mut pump = ProxyWsPump {
            info: Arc::new(info),
            worker_id: this.id,
            shared: this.shared.clone(),
            postprocessor: this.postprocessor.clone(),
            canceller: this.canceller.clone(),
            resetter: this.resetter.clone(),
            clnt_tx,
            clnt_rx,
            bknd_tx,
            bknd_rx,
            pings: HashMap::new(),
            ping_balance_bknd: AtomicI64::new(0),
            ping_balance_clnt: AtomicI64::new(0),

            #[cfg(feature = "chaos")]
            chaos: ProxyWsPumpChaos {
                stream_is_blocked: std::sync::atomic::AtomicBool::new(false),
            },
        };

        pump.run().await;
    }

    fn finalise_proxying(
        msg: ProxyWsMessage,
        inner: Arc<P>,
        metrics: Arc<Metrics>,
        worker_id: Uuid,
    ) {
        Self::maybe_log_proxied_message(&msg, inner.clone(), worker_id);

        Self::emit_metrics_on_proxy_success(&msg, metrics.clone());
    }

    fn maybe_log_proxied_message(msg: &ProxyWsMessage, inner: Arc<P>, worker_id: Uuid) {
        let config = inner.config();

        match msg {
            ProxyWsMessage::BackendToClientBinary { msg, info, start, end } => {
                let json_msg = if config.log_backend_messages() {
                    Loggable(&Self::maybe_sanitise(
                        config.log_sanitise(),
                        serde_json::from_slice(msg).unwrap_or_default(),
                    ))
                } else {
                    Loggable(&serde_json::Value::Null)
                };

                info!(
                    proxy = P::name(),
                    connection_id = %info.conn_id(),
                    worker_id = %worker_id,
                    remote_addr = info.remote_addr(),
                    ts_message_received = start.format(&Iso8601::DEFAULT).unwrap_or_default(),
                    latency_proxying = (*end - *start).as_seconds_f64(),
                    json_msg = tracing::field::valuable(&json_msg),
                    "Proxied binary message to client",
                );
            }

            ProxyWsMessage::BackendToClientText { msg, info, start, end } => {
                let json_msg = if config.log_backend_messages() {
                    Loggable(&Self::maybe_sanitise(
                        config.log_sanitise(),
                        serde_json::from_str(msg).unwrap_or_default(),
                    ))
                } else {
                    Loggable(&serde_json::Value::Null)
                };

                info!(
                    proxy = P::name(),
                    connection_id = %info.conn_id(),
                    worker_id = %worker_id,
                    remote_addr = info.remote_addr(),
                    ts_message_received = start.format(&Iso8601::DEFAULT).unwrap_or_default(),
                    latency_proxying = (*end - *start).as_seconds_f64(),
                    json_msg = tracing::field::valuable(&json_msg),
                    "Proxied text message to client",
                );
            }

            ProxyWsMessage::ClientToBackendBinary { msg, info, start, end } => {
                let json_msg = if config.log_client_messages() {
                    Loggable(&Self::maybe_sanitise(
                        config.log_sanitise(),
                        serde_json::from_slice(msg).unwrap_or_default(),
                    ))
                } else {
                    Loggable(&serde_json::Value::Null)
                };

                info!(
                    proxy = P::name(),
                    connection_id = %info.conn_id(),
                    worker_id = %worker_id,
                    remote_addr = info.remote_addr(),
                    ts_message_received = start.format(&Iso8601::DEFAULT).unwrap_or_default(),
                    latency_proxying = (*end - *start).as_seconds_f64(),
                    json_msg = tracing::field::valuable(&json_msg),
                    "Proxied binary message to backend",
                );
            }

            ProxyWsMessage::ClientToBackendText { msg, info, start, end } => {
                let json_msg = if config.log_client_messages() {
                    Loggable(&Self::maybe_sanitise(
                        config.log_sanitise(),
                        serde_json::from_str(msg).unwrap_or_default(),
                    ))
                } else {
                    Loggable(&serde_json::Value::Null)
                };

                info!(
                    proxy = P::name(),
                    connection_id = %info.conn_id(),
                    worker_id = %worker_id,
                    remote_addr = info.remote_addr(),
                    ts_message_received = start.format(&Iso8601::DEFAULT).unwrap_or_default(),
                    latency_proxying = (*end - *start).as_seconds_f64(),
                    json_msg = tracing::field::valuable(&json_msg),
                    "Proxied text message to backend",
                );
            }
        }
    }

    fn maybe_sanitise(do_sanitise: bool, mut message: serde_json::Value) -> serde_json::Value {
        if !do_sanitise {
            return message;
        }

        if let Some(object) = message.as_object_mut() &&
            let Some(diff) = object.get_mut("diff") &&
            let Some(transactions) = diff.get_mut("transactions") &&
            let Some(transactions) = transactions.as_array_mut()
        {
            for transaction in transactions {
                raw_transaction_to_hash(transaction);
            }
        }

        message
    }

    fn emit_metrics_on_proxy_success(msg: &ProxyWsMessage, metrics: Arc<Metrics>) {
        match msg {
            ProxyWsMessage::BackendToClientBinary { msg, info: _, start, end } => {
                let labels = LabelsProxyWs { proxy: P::name(), destination: WS_LABEL_CLNT };
                metrics
                    .ws_latency_proxy
                    .get_or_create(&labels)
                    .record((1000000.0 * (*end - *start).as_seconds_f64()) as i64);
                metrics.ws_proxy_success_count.get_or_create_owned(&labels).inc();
                metrics.ws_message_size.get_or_create_owned(&labels).record(msg.len() as i64);
            }

            ProxyWsMessage::BackendToClientText { msg, info: _, start, end } => {
                let labels = LabelsProxyWs { proxy: P::name(), destination: WS_LABEL_CLNT };
                metrics
                    .ws_latency_proxy
                    .get_or_create(&labels)
                    .record((1000000.0 * (*end - *start).as_seconds_f64()) as i64);
                metrics.ws_proxy_success_count.get_or_create_owned(&labels).inc();
                metrics.ws_message_size.get_or_create_owned(&labels).record(msg.len() as i64);
            }

            ProxyWsMessage::ClientToBackendBinary { msg, info: _, start, end } => {
                let labels = LabelsProxyWs { proxy: P::name(), destination: WS_LABEL_BKND };
                metrics
                    .ws_latency_proxy
                    .get_or_create(&labels)
                    .record((1000000.0 * (*end - *start).as_seconds_f64()) as i64);
                metrics.ws_proxy_success_count.get_or_create_owned(&labels).inc();
                metrics.ws_message_size.get_or_create_owned(&labels).record(msg.len() as i64);
            }

            ProxyWsMessage::ClientToBackendText { msg, info: _, start, end } => {
                let labels = LabelsProxyWs { proxy: P::name(), destination: WS_LABEL_BKND };
                metrics
                    .ws_latency_proxy
                    .get_or_create(&labels)
                    .record((1000000.0 * (*end - *start).as_seconds_f64()) as i64);
                metrics.ws_proxy_success_count.get_or_create_owned(&labels).inc();
                metrics.ws_message_size.get_or_create_owned(&labels).record(msg.len() as i64);
            }
        }
    }
}

// ProxyWsSharedState --------------------------------------------------

#[derive(Clone)]
struct ProxyWsSharedState<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    inner: Arc<P>,
    metrics: Arc<Metrics>,

    client_connections_count: Arc<AtomicI64>,

    _config: PhantomData<C>,
}

impl<C, P> ProxyWsSharedState<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    fn new(config: C, metrics: &Arc<Metrics>) -> Self {
        Self {
            inner: Arc::new(P::new(config)),
            metrics: metrics.clone(),
            client_connections_count: Arc::new(AtomicI64::new(0)),
            _config: PhantomData,
        }
    }

    #[inline]
    fn config(&self) -> &C {
        self.inner.config()
    }
}

// ProxyWsBackendEndpoint ----------------------------------------------

struct ProxyWsBackendEndpoint<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    worker_id: Uuid,

    url: tungstenite::http::Uri,

    _config: PhantomData<C>,
    _inner: PhantomData<P>,
}

impl<C, P> ProxyWsBackendEndpoint<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    fn new(worker_id: Uuid, url: tungstenite::http::Uri) -> Self {
        Self { worker_id, url, _config: PhantomData, _inner: PhantomData }
    }

    fn new_backend_uri(&self, info: &ProxyHttpRequestInfo) -> tungstenite::http::Uri {
        let mut parts = self.url.clone().into_parts();
        let pq = tungstenite::http::uri::PathAndQuery::from_str(info.path_and_query())
            .inspect_err(|err| {
                error!(
                    proxy = P::name(),
                    request_id = %info.req_id(),
                    connection_id = %info.conn_id(),
                    worker_id = %self.worker_id,
                    error = ?err,
                    "Failed to re-parse client request's path and query",
                );
            })
            .ok();
        parts.path_and_query = pq;

        tungstenite::http::Uri::from_parts(parts)
            .inspect_err(|err| {
                error!(
                    proxy = P::name(),
                    request_id = %info.req_id(),
                    connection_id = %info.conn_id(),
                    worker_id = %self.worker_id,
                    error = ?err, "Failed to construct backend URI, defaulting to the base one",
                );
            })
            .unwrap_or(self.url.clone())
    }
}

// ProxyWsPump ---------------------------------------------------------

struct ProxyWsPump<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    info: Arc<ProxyHttpRequestInfo>,
    worker_id: Uuid,

    shared: ProxyWsSharedState<C, P>,
    postprocessor: actix::Addr<ProxyWsPostprocessor<C, P>>,
    canceller: tokio_util::sync::CancellationToken,
    resetter: broadcast::Sender<()>,

    clnt_tx: Session,
    clnt_rx: MessageStream,
    bknd_tx: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, tungstenite::Message>,
    bknd_rx: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,

    pings: HashMap<Uuid, ProxyWsPing>,
    ping_balance_clnt: AtomicI64,
    ping_balance_bknd: AtomicI64,

    #[cfg(feature = "chaos")]
    chaos: ProxyWsPumpChaos,
}

impl<C, P> ProxyWsPump<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    async fn run(&mut self) {
        info!(
            proxy = P::name(),
            connection_id = %self.info.conn_id(),
            worker_id = %self.worker_id,
            "Starting websocket pump..."
        );

        let mut heartbeat = tokio::time::interval(Duration::from_secs(WS_PING_INTERVAL_SECONDS));

        let mut pumping: Result<(), &str> = Ok(());

        let mut resetter = self.resetter.subscribe();

        while pumping.is_ok() && !self.canceller.is_cancelled() && !resetter.is_closed() {
            #[cfg(feature = "chaos")]
            if self.chaos.stream_is_blocked.load(Ordering::Relaxed) {
                tokio::select! {
                    _ = self.canceller.cancelled() => {
                        break;
                    }

                    _ = resetter.recv() => {
                        break;
                    }

                    // client => backend
                    clnt_msg = self.clnt_rx.next() => {
                        pumping = self.pump_clnt_to_bknd(UtcDateTime::now(), clnt_msg).await;
                    }
                }

                continue;
            }

            tokio::select! {
                _ = self.canceller.cancelled() => {
                    break;
                }

                _ = resetter.recv() => {
                    break;
                }

                // ping both sides
                _ = heartbeat.tick() => {
                    pumping = self.heartbeat().await;
                }

                // client => backend
                clnt_msg = self.clnt_rx.next() => {
                    pumping = self.pump_clnt_to_bknd(UtcDateTime::now(), clnt_msg).await;
                }

                // backend => client
                bknd_msg = self.bknd_rx.next() => {
                    pumping = self.pump_bknd_to_cli(UtcDateTime::now(), bknd_msg).await;
                }
            }
        }

        if let Err(reason) = pumping {
            let (frame_clnt, frame_bknd) = match reason {
                WS_CLOSE_OK => (
                    actix_ws::CloseReason {
                        code: awc::ws::CloseCode::Normal,
                        description: WS_CLOSE_REASON_NORMAL.to_string().into(),
                    },
                    tungstenite::protocol::CloseFrame {
                        code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                        reason: WS_CLOSE_REASON_NORMAL.into(),
                    },
                ),

                _ => (
                    actix_ws::CloseReason {
                        code: awc::ws::CloseCode::Error,
                        description: reason.to_string().into(),
                    },
                    tungstenite::protocol::CloseFrame {
                        code: tungstenite::protocol::frame::coding::CloseCode::Error,
                        reason: reason.into(),
                    },
                ),
            };

            _ = self.close_clnt_session(frame_clnt).await;
            _ = self.close_bknd_session(frame_bknd).await;
        }

        info!(
            proxy = P::name(),
            connection_id = %self.info.conn_id(),
            worker_id = %self.worker_id,
            "Stopped websocket pump"
        );
    }

    async fn heartbeat(&mut self) -> Result<(), &'static str> {
        #[cfg(feature = "chaos")]
        if self.chaos.stream_is_blocked.load(Ordering::Relaxed) {
            return Ok(());
        }

        let ping_threshold = (1 + self.shared.config().backend_timeout().as_secs() /
            WS_PING_INTERVAL_SECONDS) as i64;

        {
            // ping -> client

            if self.ping_balance_clnt.load(Ordering::Relaxed) > ping_threshold {
                error!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    "More than {} websocket pings sent to client didn't return, terminating the pump...", ping_threshold,
                );
                return Err(WS_CLNT_ERROR);
            }

            let clnt_ping = ProxyWsPing::new(self.info.conn_id());
            if let Err(err) = self.clnt_tx.ping(&clnt_ping.to_slice()).await {
                error!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    error = ?err,
                    "Failed to send ping websocket message to client"
                );
                return Err(WS_CLNT_ERROR);
            }
            let _ = self.pings.insert_sync(clnt_ping.id, clnt_ping);
            self.ping_balance_clnt.inc();
        }

        {
            // ping -> backend

            if self.ping_balance_bknd.load(Ordering::Relaxed) > ping_threshold {
                error!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    "More than {} websocket pings sent to backend didn't return, terminating the pump...", ping_threshold,
                );
                return Err(WS_BKND_ERROR);
            }

            let bknd_ping = ProxyWsPing::new(self.info.conn_id());
            if let Err(err) =
                self.bknd_tx.send(tungstenite::Message::Ping(bknd_ping.to_bytes())).await
            {
                error!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    error = ?err,
                    "Failed to send ping websocket message to backend"
                );
                return Err(WS_BKND_ERROR);
            }
            let _ = self.pings.insert_sync(bknd_ping.id, bknd_ping);
            self.ping_balance_bknd.inc();
        }
        Ok(())
    }

    async fn pump_clnt_to_bknd(
        &mut self,
        timestamp: UtcDateTime,
        clnt_msg: Option<Result<actix_ws::Message, actix_http::ws::ProtocolError>>,
    ) -> Result<(), &'static str> {
        #[cfg(feature = "chaos")]
        if !self.chaos.stream_is_blocked.load(Ordering::Relaxed) &&
            rand::random::<f64>() < self.shared.config().chaos_probability_stream_blocked()
        {
            warn!(
                proxy = P::name(),
                connection_id = %self.info.conn_id(),
                worker_id = %self.worker_id,
                "Blocking the stream (chaos)"
            );
            self.chaos.stream_is_blocked.store(true, Ordering::Relaxed);
            _ = self
                .close_bknd_session(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                    reason: WS_CLOSE_REASON_NORMAL.into(),
                })
                .await;
        }

        match clnt_msg {
            Some(Ok(msg)) => {
                match msg {
                    // binary msg from client
                    actix_ws::Message::Binary(bytes) => {
                        #[cfg(feature = "chaos")]
                        if self.chaos.stream_is_blocked.load(Ordering::Relaxed) {
                            return Ok(());
                        }

                        if let Err(err) =
                            self.bknd_tx.send(tungstenite::Message::Binary(bytes.clone())).await
                        {
                            error!(
                                proxy = P::name(),
                                connection_id = %self.info.conn_id(),
                                worker_id = %self.worker_id,
                                error = ?err,
                                "Failed to proxy binary websocket message to backend"
                            );
                            self.shared
                                .metrics
                                .ws_proxy_failure_count
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_BKND,
                                })
                                .inc();
                            return Err(WS_BKND_ERROR);
                        }
                        self.postprocessor.do_send(ProxyWsMessage::ClientToBackendBinary {
                            msg: bytes,
                            info: self.info.clone(),
                            start: timestamp,
                            end: UtcDateTime::now(),
                        });
                        Ok(())
                    }

                    // text msg from client
                    actix_ws::Message::Text(text) => {
                        #[cfg(feature = "chaos")]
                        if self.chaos.stream_is_blocked.load(Ordering::Relaxed) {
                            return Ok(());
                        }

                        if let Err(err) = self
                            .bknd_tx
                            .send(tungstenite::Message::Text(unsafe {
                                // safety: it's from client's ws message => must be valid utf-8
                                tungstenite::protocol::frame::Utf8Bytes::from_bytes_unchecked(
                                    text.clone().into_bytes(),
                                )
                            }))
                            .await
                        {
                            error!(
                                proxy = P::name(),
                                connection_id = %self.info.conn_id(),
                                worker_id = %self.worker_id,
                                error = ?err,
                                "Failed to proxy text websocket message to backend"
                            );
                            self.shared
                                .metrics
                                .ws_proxy_failure_count
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_BKND,
                                })
                                .inc();
                            return Err(WS_BKND_ERROR);
                        }
                        self.postprocessor.do_send(ProxyWsMessage::ClientToBackendText {
                            msg: text,
                            info: self.info.clone(),
                            start: timestamp,
                            end: UtcDateTime::now(),
                        });
                        Ok(())
                    }

                    // ping msg from client
                    actix_ws::Message::Ping(bytes) => {
                        #[cfg(feature = "chaos")]
                        if self.chaos.stream_is_blocked.load(Ordering::Relaxed) {
                            return Ok(());
                        }

                        #[cfg(debug_assertions)]
                        debug!(
                            proxy = P::name(),
                            connection_id = %self.info.conn_id(),
                            worker_id = %self.worker_id,
                            "Handling client's ping..."
                        );

                        #[cfg(feature = "chaos")]
                        if rand::random::<f64>() <
                            self.shared.config().chaos_probability_client_ping_ignored()
                        {
                            debug!(
                                proxy = P::name(),
                                connection_id = %self.info.conn_id(),
                                worker_id = %self.worker_id,
                                "Ignored client's ping (chaos)"
                            );
                            return Ok(());
                        }

                        if let Err(err) = self.clnt_tx.pong(&bytes).await {
                            error!(
                                proxy = P::name(),
                                connection_id = %self.info.conn_id(),
                                worker_id = %self.worker_id,
                                error = ?err,
                                "Failed to return pong message to client"
                            );
                            return Err(WS_CLNT_ERROR);
                        }
                        Ok(())
                    }

                    // pong msg from client
                    actix_ws::Message::Pong(bytes) => {
                        #[cfg(debug_assertions)]
                        debug!(
                            proxy = P::name(),
                            connection_id = %self.info.conn_id(),
                            worker_id = %self.worker_id,
                            "Received pong from client"
                        );

                        if let Some(pong) = ProxyWsPing::from_bytes(bytes) &&
                            let Some((_, ping)) = self.pings.remove_sync(&pong.id) &&
                            pong == ping
                        {
                            self.ping_balance_clnt.dec();
                            self.shared
                                .metrics
                                .ws_latency_client
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_CLNT,
                                })
                                .record(
                                    (1000000.0 * (timestamp - pong.timestamp).as_seconds_f64() /
                                        2.0) as i64,
                                );
                            return Ok(());
                        }
                        warn!(
                            proxy = P::name(),
                            connection_id = %self.info.conn_id(),
                            worker_id = %self.worker_id,
                            "Unexpected websocket pong received from client",
                        );
                        Ok(())
                    }

                    // close msg from client
                    actix_ws::Message::Close(reason) => {
                        return self
                            .close_bknd_session(
                                reason
                                    .map(|r| tungstenite::protocol::CloseFrame {
                                        code: tungstenite::protocol::frame::coding::CloseCode::from(
                                            u16::from(r.code),
                                        ),
                                        reason: r
                                            .description
                                            .map_or(WS_CLOSE_REASON_NORMAL.into(), |r| r.into()),
                                    })
                                    .unwrap_or(tungstenite::protocol::CloseFrame {
                                        code:
                                            tungstenite::protocol::frame::coding::CloseCode::Normal,
                                        reason: WS_CLOSE_REASON_NORMAL.into(),
                                    }),
                            )
                            .await;
                    }

                    _ => Ok(()),
                }
            }

            Some(Err(err)) => {
                error!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    error = ?err,
                    "Client websocket stream error"
                );
                Err(WS_CLNT_ERROR)
            }

            None => {
                info!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    "Client had closed websocket stream"
                );
                Err(WS_CLOSE_OK)
            }
        }
    }

    async fn pump_bknd_to_cli(
        &mut self,
        timestamp: UtcDateTime,
        bknd_msg: Option<Result<tungstenite::Message, tungstenite::Error>>,
    ) -> Result<(), &'static str> {
        #[cfg(feature = "chaos")]
        if !self.chaos.stream_is_blocked.load(Ordering::Relaxed) &&
            rand::random::<f64>() < self.shared.config().chaos_probability_stream_blocked()
        {
            warn!(
                proxy = P::name(),
                connection_id = %self.info.conn_id(),
                worker_id = %self.worker_id,
                "Blocking the stream (chaos)"
            );
            self.chaos.stream_is_blocked.store(true, Ordering::Relaxed);
            _ = self
                .close_bknd_session(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                    reason: WS_CLOSE_REASON_NORMAL.into(),
                })
                .await;
        }

        match bknd_msg {
            Some(Ok(msg)) => {
                match msg {
                    // binary
                    tungstenite::Message::Binary(bytes) => {
                        #[cfg(feature = "chaos")]
                        if self.chaos.stream_is_blocked.load(Ordering::Relaxed) {
                            return Ok(());
                        }

                        if let Err(err) = self.clnt_tx.binary(bytes.clone()).await {
                            error!(
                                proxy = P::name(),
                                connection_id = %self.info.conn_id(),
                                worker_id = %self.worker_id,
                                error = ?err,
                                "Failed to proxy binary websocket message to client"
                            );
                            self.shared
                                .metrics
                                .ws_proxy_failure_count
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_CLNT,
                                })
                                .inc();
                            return Err(WS_CLNT_ERROR);
                        }
                        self.postprocessor.do_send(ProxyWsMessage::BackendToClientBinary {
                            msg: bytes,
                            info: self.info.clone(),
                            start: timestamp,
                            end: UtcDateTime::now(),
                        });
                        Ok(())
                    }

                    // text
                    tungstenite::Message::Text(text) => {
                        #[cfg(feature = "chaos")]
                        if self.chaos.stream_is_blocked.load(Ordering::Relaxed) {
                            return Ok(());
                        }

                        if let Err(err) = self.clnt_tx.text(text.clone().as_str()).await {
                            error!(
                                proxy = P::name(),
                                connection_id = %self.info.conn_id(),
                                worker_id = %self.worker_id,
                                error = ?err,
                                "Failed to proxy text websocket message to client"
                            );
                            self.shared
                                .metrics
                                .ws_proxy_failure_count
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_CLNT,
                                })
                                .inc();
                            return Err(WS_CLNT_ERROR);
                        }
                        self.postprocessor.do_send(ProxyWsMessage::BackendToClientText {
                            msg: text,
                            info: self.info.clone(),
                            start: timestamp,
                            end: UtcDateTime::now(),
                        });
                        Ok(())
                    }

                    // ping
                    tungstenite::Message::Ping(bytes) => {
                        #[cfg(feature = "chaos")]
                        if self.chaos.stream_is_blocked.load(Ordering::Relaxed) {
                            return Ok(());
                        }

                        #[cfg(debug_assertions)]
                        debug!(
                            proxy = P::name(),
                            connection_id = %self.info.conn_id(),
                            worker_id = %self.worker_id,
                            "Handling backend's ping..."
                        );

                        #[cfg(feature = "chaos")]
                        if rand::random::<f64>() <
                            self.shared.config().chaos_probability_backend_ping_ignored()
                        {
                            debug!(
                                proxy = P::name(),
                                connection_id = %self.info.conn_id(),
                                worker_id = %self.worker_id,
                                "Ignored backend's ping (chaos)"
                            );
                            return Ok(());
                        }

                        if let Err(err) = self.bknd_tx.send(tungstenite::Message::Pong(bytes)).await
                        {
                            error!(
                                proxy = P::name(),
                                connection_id = %self.info.conn_id(),
                                worker_id = %self.worker_id,
                                error = ?err,
                                "Failed to return pong message to backend"
                            );
                            return Err(WS_BKND_ERROR);
                        }
                        Ok(())
                    }

                    // pong
                    tungstenite::Message::Pong(bytes) => {
                        #[cfg(debug_assertions)]
                        debug!(
                            proxy = P::name(),
                            connection_id = %self.info.conn_id(),
                            worker_id = %self.worker_id,
                            "Received pong from backend"
                        );

                        if let Some(pong) = ProxyWsPing::from_bytes(bytes) &&
                            let Some((_, ping)) = self.pings.remove_sync(&pong.id) &&
                            pong == ping
                        {
                            self.ping_balance_bknd.dec();
                            self.shared
                                .metrics
                                .ws_latency_backend
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_BKND,
                                })
                                .record(
                                    (1000000.0 * (timestamp - pong.timestamp).as_seconds_f64() /
                                        2.0) as i64,
                                );
                            return Ok(());
                        }
                        warn!(
                            proxy = P::name(),
                            connection_id = %self.info.conn_id(),
                            worker_id = %self.worker_id,
                            "Unexpected websocket pong received from backend",
                        );
                        Ok(())
                    }

                    // close
                    tungstenite::Message::Close(reason) => {
                        return self
                            .close_clnt_session(
                                reason
                                    .map(|reason| actix_ws::CloseReason {
                                        code: u16::from(reason.code).into(),
                                        description: reason.reason.to_string().into(),
                                    })
                                    .unwrap_or(actix_ws::CloseReason {
                                        code: awc::ws::CloseCode::Normal,
                                        description: WS_CLOSE_REASON_UNSPECIFIED.to_string().into(),
                                    }),
                            )
                            .await;
                    }

                    _ => Ok(()),
                }
            }

            Some(Err(err)) => {
                error!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    error = ?err,
                    "Backend websocket stream error"
                );
                Err(WS_BKND_ERROR)
            }

            None => {
                info!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    "Backend had closed websocket stream"
                );
                Err(WS_CLOSE_OK)
            }
        }
    }

    async fn close_clnt_session(
        &mut self,
        frame: actix_ws::CloseReason,
    ) -> Result<(), &'static str> {
        debug!(
            proxy = P::name(),
            connection_id = %self.info.conn_id(),
            worker_id = %self.worker_id,
            msg = ?frame.description,
            "Closing client websocket session..."
        );
        let _ = self // only 1 possible "already closed" error (which we ignore)
            .clnt_tx
            .clone() // .close() consumes it
            .close(Some(frame))
            .await;
        Ok(())
    }

    async fn close_bknd_session(
        &mut self,
        frame: tungstenite::protocol::CloseFrame,
    ) -> Result<(), &'static str> {
        debug!(
            proxy = P::name(),
            connection_id = %self.info.conn_id(),
            worker_id = %self.worker_id,
            msg = %frame.reason,
            "Closing backend websocket session..."
        );

        if let Err(err) = self
            .bknd_tx
            .send(tungstenite::Message::Close(Some(
                frame.clone(), // it's cheap to clone
            )))
            .await
        {
            if let tungstenite::error::Error::AlreadyClosed = err {
                return Ok(());
            }
            if let tungstenite::error::Error::Protocol(protocol_err) = err {
                if protocol_err == tungstenite::error::ProtocolError::SendAfterClosing {
                    return Ok(());
                }
                error!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    msg = %frame.reason,
                    error = ?protocol_err,
                    "Failed to close backend websocket session"
                );
            } else {
                error!(
                    proxy = P::name(),
                    connection_id = %self.info.conn_id(),
                    worker_id = %self.worker_id,
                    msg = %frame.reason,
                    error = ?err,
                    "Failed to close backend websocket session"
                );
            }
            return Err(WS_BKND_ERROR);
        }

        if let Err(err) = self.bknd_tx.close().await {
            error!(
                proxy = P::name(),
                connection_id = %self.info.conn_id(),
                worker_id = %self.worker_id,
                msg = %frame.reason,
                error = ?err,
                "Failed to close backend websocket session"
            );
        }

        Ok(())
    }
}

#[cfg(debug_assertions)]
impl<C, P> Drop for ProxyWsPump<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    fn drop(&mut self) {
        debug!(
            proxy = P::name(),
            connection_id = %self.info.conn_id(),
            worker_id = %self.worker_id,
            "Dropping websocket pump"
        );
    }
}

// ProxyWsPostprocessor ------------------------------------------------

struct ProxyWsPostprocessor<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    inner: Arc<P>,
    worker_id: Uuid,
    metrics: Arc<Metrics>,

    _config: PhantomData<C>,
}

impl<C, P> actix::Actor for ProxyWsPostprocessor<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    type Context = actix::Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.set_mailbox_capacity(1024);
    }
}

impl<C, P> actix::Handler<ProxyWsMessage> for ProxyWsPostprocessor<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    type Result = ();

    fn handle(&mut self, msg: ProxyWsMessage, ctx: &mut Self::Context) -> Self::Result {
        let inner = self.inner.clone();
        let metrics = self.metrics.clone();
        let worker_id = self.worker_id;

        ctx.spawn(
            async move {
                ProxyWs::<C, P>::finalise_proxying(msg, inner, metrics, worker_id);
            }
            .into_actor(self),
        );
    }
}

// ProxyWsMessage ------------------------------------------------------

#[derive(Clone, actix::Message)]
#[rtype(result = "()")]
enum ProxyWsMessage {
    BackendToClientBinary {
        msg: bytes::Bytes,
        info: Arc<ProxyHttpRequestInfo>,
        start: UtcDateTime,
        end: UtcDateTime,
    },

    BackendToClientText {
        msg: tungstenite::protocol::frame::Utf8Bytes,
        info: Arc<ProxyHttpRequestInfo>,
        start: UtcDateTime,
        end: UtcDateTime,
    },

    ClientToBackendBinary {
        msg: bytes::Bytes,
        info: Arc<ProxyHttpRequestInfo>,
        start: UtcDateTime,
        end: UtcDateTime,
    },

    ClientToBackendText {
        msg: bytestring::ByteString,
        info: Arc<ProxyHttpRequestInfo>,
        start: UtcDateTime,
        end: UtcDateTime,
    },
}

// ProxyWsPing ---------------------------------------------------------

#[derive(PartialEq, Eq)]
struct ProxyWsPing {
    id: Uuid,
    conn_id: Uuid,
    timestamp: UtcDateTime,
}

impl ProxyWsPing {
    fn new(conn_id: Uuid) -> Self {
        Self { id: Uuid::now_v7(), conn_id, timestamp: UtcDateTime::now() }
    }

    fn to_bytes(&self) -> Bytes {
        let mut bytes = BytesMut::with_capacity(48);
        bytes.put_u128(self.id.as_u128());
        bytes.put_u128(self.conn_id.as_u128());
        bytes.put_i128(self.timestamp.unix_timestamp_nanos());
        bytes.freeze()
    }

    fn from_bytes(mut bytes: Bytes) -> Option<Self> {
        if bytes.len() != 48 {
            return None;
        }

        let id = Uuid::from_u128(bytes.get_u128());
        let conn_id = Uuid::from_u128(bytes.get_u128());
        let Ok(timestamp) = UtcDateTime::from_unix_timestamp_nanos(bytes.get_i128()) else {
            return None;
        };

        Some(Self { id, conn_id, timestamp })
    }

    fn to_slice(&self) -> [u8; 48] {
        let res: [u8; 48] = [0; 48];
        let mut cur = std::io::Cursor::new(res);

        let _ = cur.write(self.id.as_bytes());
        let _ = cur.write(self.conn_id.as_bytes());
        let _ = cur.write(&self.timestamp.unix_timestamp_nanos().to_be_bytes());

        cur.into_inner()
    }
}

// ProxyWsPumpChaos --------------------------------------------------------

#[cfg(feature = "chaos")]
struct ProxyWsPumpChaos {
    stream_is_blocked: std::sync::atomic::AtomicBool,
}

// tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_ws_ping_encode_decode() {
        let ping = ProxyWsPing::new(Uuid::now_v7());

        {
            let pong = ProxyWsPing::from_bytes(ping.to_bytes());
            assert!(pong.is_some(), "must be some");
            let pong = pong.unwrap(); // safety: just verified
            assert!(pong == ping, "must be the same");
        }

        {
            let slice = ping.to_slice();
            let pong = ProxyWsPing::from_bytes(Bytes::copy_from_slice(&slice));
            assert!(pong.is_some(), "must be some");
            let pong = pong.unwrap(); // safety: just verified
            assert!(pong == ping, "must be the same");
        }
    }
}
