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
    middleware::NormalizePath,
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
use tracing::{error, info, trace, warn};
use tungstenite::Utf8Bytes;
use uuid::Uuid;
use x509_parser::asn1_rs::ToStatic;

use crate::{
    config::{ConfigProxyWs, ConfigTls, PARALLELISM},
    metrics::{LabelsProxyWs, Metrics},
    proxy::{Proxy, ProxyConnectionGuard},
    proxy_http::ProxyHttpRequestInfo,
    proxy_ws::ProxyWsInner,
    utils::{Loggable, raw_transaction_to_hash},
};

const WS_PING_INTERVAL_SECONDS: u64 = 1;

const WS_CLI_ERROR: &'static str = "client error";
const WS_BCK_ERROR: &'static str = "backend error";
const WS_BCK_TIMEOUT: &'static str = "backend error";
const WS_CLOSE_OK: &'static str = "";

const WS_LABEL_BACKEND: &'static str = "backend";
const WS_LABEL_CLIENT: &'static str = "client";

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

    backend: ProxyWsBackendEndpoint<C, P>,

    pings: HashMap<Uuid, ProxyWsPing>,
    ping_balance_cli: AtomicI64,
    ping_balance_bck: AtomicI64,

    _config: PhantomData<C>,
    _proxy: PhantomData<P>,
}

impl<C, P> ProxyWs<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
    fn new(
        shared: ProxyWsSharedState<C, P>,
        canceller: tokio_util::sync::CancellationToken,
    ) -> Self {
        let id = Uuid::now_v7();

        let config = shared.config();

        let backend = ProxyWsBackendEndpoint::new(id.clone(), config.backend_url());

        let postprocessor = ProxyWsPostprocessor::<C, P> {
            inner: shared.inner.clone(),
            metrics: shared.metrics.clone(),
            worker_id: id,
            _config: PhantomData,
        }
        .start();

        Self {
            id,
            shared,
            postprocessor,
            canceller,
            backend,
            pings: HashMap::new(),
            ping_balance_bck: AtomicI64::new(0),
            ping_balance_cli: AtomicI64::new(0),
            _config: PhantomData,
            _proxy: PhantomData,
        }
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
        let listen_address = config.listen_address().clone();

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

        let shared = ProxyWsSharedState::<C, P>::new(config, &metrics);
        let client_connections_count = shared.client_connections_count.clone();
        let worker_canceller = canceller.clone();

        info!(
            proxy = P::name(),
            listen_address = %listen_address,
            workers_count = workers_count,
            "Starting websocket-proxy...",
        );

        let server = HttpServer::new(move || {
            let this = web::Data::new(Self::new(shared.clone(), worker_canceller.clone()));

            App::new()
                .app_data(this)
                .wrap(NormalizePath::trim())
                .default_service(web::route().to(Self::receive))
        })
        .on_connect(Self::on_connect(metrics, client_connections_count))
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
            if let Ok(_) = resetter.recv().await {
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

        // must use non-blocking with tokio
        socket.set_nonblocking(true)?;

        // allow time to flush buffers on close
        socket.set_linger(Some(config.backend_timeout()))?;

        // allow binding to the socket whlie there are still TIME_WAIT conns
        socket.set_reuse_address(true)?;

        socket.bind(&socket2::SockAddr::from(config.listen_address()))?;

        socket.listen(1024)?;

        Ok(socket.into())
    }

    async fn receive(
        cli_req: HttpRequest,
        cli_req_body: web::Payload,
        this: web::Data<Self>,
    ) -> Result<HttpResponse, actix_web::Error> {
        let info = ProxyHttpRequestInfo::new(&cli_req, cli_req.conn_data::<ProxyConnectionGuard>());

        let (res, cli_tx, cli_rx) = match actix_ws::handle(&cli_req, cli_req_body) {
            Ok(res) => res,
            Err(err) => {
                error!(
                    proxy = P::name(),
                    request_id = %info.id(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to upgrade to websocket",
                );
                return Err(err);
            }
        };

        actix_web::rt::spawn(Self::handshake(this, cli_tx, cli_rx, info));

        Ok(res)
    }

    async fn handshake(
        this: web::Data<Self>,
        cli_tx: Session,
        cli_rx: MessageStream,
        info: ProxyHttpRequestInfo,
    ) {
        let bck_uri = this.backend.new_backend_uri(&info);
        trace!(
            proxy = P::name(),
            request_id = %info.id(),
            connection_id = %info.connection_id(),
            worker_id = %this.id,
            backend_uri = %bck_uri,
            "Starting websocket handshake...",
        );

        let (bck_stream, _) = match tokio::time::timeout(
            this.config().backend_timeout(),
            tokio_tungstenite::connect_async(bck_uri),
        )
        .await
        {
            Ok(Ok(res)) => res,

            Ok(Err(err)) => {
                error!(
                    proxy = P::name(),
                    request_id = %info.id(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to establish backend websocket session"
                );

                if let Err(err) = cli_tx
                    .close(Some(actix_ws::CloseReason {
                        code: awc::ws::CloseCode::Error,
                        description: Some(String::from(WS_BCK_ERROR)),
                    }))
                    .await
                {
                    error!(
                        proxy = P::name(),
                        request_id = %info.id(),
                        connection_id = %info.connection_id(),
                        worker_id = %this.id,
                        error = ?err,
                        "Failed to close client websocket session"
                    );
                };
                return;
            }

            Err(_) => {
                error!(
                    proxy = P::name(),
                    request_id = %info.id(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    "Timed out to establish backend websocket session"
                );

                if let Err(err) = cli_tx
                    .close(Some(actix_ws::CloseReason {
                        code: awc::ws::CloseCode::Again,
                        description: Some(String::from(WS_BCK_TIMEOUT)),
                    }))
                    .await
                {
                    error!(
                        proxy = P::name(),
                        request_id = %info.id(),
                        connection_id = %info.connection_id(),
                        worker_id = %this.id,
                        error = ?err,
                        "Failed to close client websocket session"
                    );
                }
                return;
            }
        };

        let (bck_tx, bck_rx) = bck_stream.split();

        Self::pump(this, info, cli_tx, cli_rx, bck_tx, bck_rx).await;
    }

    async fn pump(
        this: web::Data<Self>,
        info: ProxyHttpRequestInfo,
        mut cli_tx: Session,
        mut cli_rx: MessageStream,
        mut bck_tx: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, tungstenite::Message>,
        mut bck_rx: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    ) {
        info!(
            proxy = P::name(),
            connection_id = %info.connection_id(),
            worker_id = %this.id,
            "Starting websocket pump..."
        );

        let info = Arc::new(info);

        let mut heartbeat = tokio::time::interval(Duration::from_secs(WS_PING_INTERVAL_SECONDS));

        let mut pumping: Result<(), &str> = Ok(());

        while pumping.is_ok() && !this.canceller.is_cancelled() {
            tokio::select! {
                _ = this.canceller.cancelled() => {
                    break;
                }

                // ping both sides
                _ = heartbeat.tick() => {
                    pumping = Self::heartbeat(&this, info.clone(), &mut cli_tx, &mut bck_tx).await;
                }

                // client => backend
                cli_msg = cli_rx.next() => {
                    pumping = Self::pump_cli_to_bck(
                        &this,
                        info.clone(),
                        UtcDateTime::now(),
                        cli_msg,
                        &mut bck_tx,
                        &mut cli_tx
                    ).await;
                }

                // backend => client
                bck_msg = bck_rx.next() => {
                    pumping = Self::pump_bck_to_cli(
                        &this,
                        info.clone(),
                        UtcDateTime::now(),
                        bck_msg,
                        &mut cli_tx,
                        &mut bck_tx
                    ).await;
                }
            }
        }

        if let Err(msg) = pumping &&
            msg != WS_CLOSE_OK
        {
            if let Err(err) = cli_tx
                .close(Some(actix_ws::CloseReason {
                    code: awc::ws::CloseCode::Error,
                    description: Some(String::from(msg)),
                }))
                .await
            {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to close client websocket session"
                );
            }

            if let Err(err) = bck_tx
                .send(tungstenite::Message::Close(Some(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Error,
                    reason: msg.into(),
                })))
                .await
            {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to close backend websocket session"
                );
            }
        } else {
            if let Err(err) = cli_tx
                .close(Some(actix_ws::CloseReason {
                    code: awc::ws::CloseCode::Normal,
                    description: None,
                }))
                .await
            {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to close client websocket session"
                );
            }

            if let Err(err) = bck_tx
                .send(tungstenite::Message::Close(Some(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                    reason: Utf8Bytes::default(),
                })))
                .await
            {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to close backend websocket session"
                );
            }
        }

        info!(
            proxy = P::name(),
            connection_id = %info.connection_id(),
            worker_id = %this.id,
            "Stopped websocket pump"
        );
    }

    async fn heartbeat(
        this: &web::Data<Self>,
        info: Arc<ProxyHttpRequestInfo>,
        cli_tx: &mut Session,
        bck_tx: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, tungstenite::Message>,
    ) -> Result<(), &'static str> {
        let ping_threshold =
            (1 + this.config().backend_timeout().as_secs() / WS_PING_INTERVAL_SECONDS) as i64;

        {
            // ping -> client

            if this.ping_balance_cli.load(Ordering::Relaxed) > ping_threshold {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    "More than {} websocket pings sent to client didn't return, terminating the pump...", ping_threshold,
                );
                return Err(WS_CLI_ERROR);
            }

            let cli_ping = ProxyWsPing::new(info.connection_id());
            if let Err(err) = cli_tx.ping(&cli_ping.to_slice()).await {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to send ping websocket message to client"
                );
                return Err(WS_CLI_ERROR);
            }
            let _ = this.pings.insert_sync(cli_ping.id, cli_ping);
            this.ping_balance_cli.inc();
        }

        {
            // ping -> backend

            if this.ping_balance_bck.load(Ordering::Relaxed) > ping_threshold {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    "More than {} websocket pings sent to backend didn't return, terminating the pump...", ping_threshold,
                );
                return Err(WS_BCK_ERROR);
            }

            let bck_ping = ProxyWsPing::new(info.connection_id());
            if let Err(err) = bck_tx.send(tungstenite::Message::Ping(bck_ping.to_bytes())).await {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Failed to send ping websocket message to backend"
                );
                return Err(WS_BCK_ERROR);
            }
            let _ = this.pings.insert_sync(bck_ping.id, bck_ping);
            this.ping_balance_bck.inc();
        }
        Ok(())
    }

    async fn pump_cli_to_bck(
        this: &web::Data<Self>,
        info: Arc<ProxyHttpRequestInfo>,
        timestamp: UtcDateTime,
        cli_msg: Option<Result<actix_ws::Message, actix_http::ws::ProtocolError>>,
        bck_tx: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, tungstenite::Message>,
        cli_tx: &mut Session,
    ) -> Result<(), &'static str> {
        match cli_msg {
            Some(Ok(msg)) => {
                match msg {
                    // binary
                    actix_ws::Message::Binary(bytes) => {
                        if let Err(err) =
                            bck_tx.send(tungstenite::Message::Binary(bytes.clone())).await
                        {
                            error!(
                                proxy = P::name(),
                                connection_id = %info.connection_id(),
                                worker_id = %this.id,
                                error = ?err,
                                "Failed to proxy binary websocket message to backend"
                            );
                            this.shared
                                .metrics
                                .ws_proxy_failure_count
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_BACKEND,
                                })
                                .inc();
                            return Err(WS_BCK_ERROR);
                        }
                        this.postprocessor.do_send(ProxyWsMessage::ClientToBackendBinary {
                            msg: bytes,
                            info,
                            start: timestamp,
                            end: UtcDateTime::now(),
                        });
                        return Ok(());
                    }

                    // text
                    actix_ws::Message::Text(text) => {
                        if let Err(err) = bck_tx
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
                                connection_id = %info.connection_id(),
                                worker_id = %this.id,
                                error = ?err,
                                "Failed to proxy text websocket message to backend"
                            );
                            this.shared
                                .metrics
                                .ws_proxy_failure_count
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_BACKEND,
                                })
                                .inc();
                            return Err(WS_BCK_ERROR);
                        }
                        this.postprocessor.do_send(ProxyWsMessage::ClientToBackendText {
                            msg: text,
                            info,
                            start: timestamp,
                            end: UtcDateTime::now(),
                        });
                        return Ok(());
                    }

                    // ping
                    actix_ws::Message::Ping(bytes) => {
                        if let Err(err) = cli_tx.pong(&bytes).await {
                            error!(
                                proxy = P::name(),
                                connection_id = %info.connection_id(),
                                worker_id = %this.id,
                                error = ?err,
                                "Failed to return pong message to client"
                            );
                            return Err(WS_CLI_ERROR);
                        }
                        return Ok(());
                    }

                    // pong
                    actix_ws::Message::Pong(bytes) => {
                        if let Some(pong) = ProxyWsPing::from_bytes(bytes) {
                            if let Some((_, ping)) = this.pings.remove_sync(&pong.id) {
                                if pong == ping {
                                    this.ping_balance_cli.dec();
                                    this.shared
                                        .metrics
                                        .ws_latency_client
                                        .get_or_create(&LabelsProxyWs {
                                            proxy: P::name(),
                                            destination: WS_LABEL_BACKEND,
                                        })
                                        .record(
                                            (1000000.0 *
                                                (timestamp - pong.timestamp).as_seconds_f64() /
                                                2.0)
                                                as i64,
                                        );
                                    return Ok(());
                                }
                            }
                        }
                        warn!(
                            proxy = P::name(),
                            connection_id = %info.connection_id(),
                            worker_id = %this.id,
                            "Unexpected websocket pong received from client",
                        );
                        return Ok(());
                    }

                    // close
                    actix_ws::Message::Close(reason) => {
                        if let Err(err) = bck_tx
                            .send(tungstenite::Message::Close(reason.map(|r| {
                                tungstenite::protocol::CloseFrame {
                                    code: tungstenite::protocol::frame::coding::CloseCode::from(
                                        u16::from(r.code),
                                    ),
                                    reason: r.description.unwrap_or_default().into(),
                                }
                            })))
                            .await
                        {
                            error!(
                                proxy = P::name(),
                                connection_id = %info.connection_id(),
                                worker_id = %this.id,
                                error = ?err,
                                "Failed to proxy close websocket message to backend"
                            );
                            return Err(WS_BCK_ERROR);
                        }
                        return Err(WS_CLOSE_OK);
                    }

                    _ => {
                        return Ok(());
                    }
                }
            }

            Some(Err(err)) => {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Client websocket stream error"
                );
                return Err(WS_CLI_ERROR);
            }

            None => {
                info!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    "Client had closed websocket stream"
                );
                return Err(WS_CLOSE_OK);
            }
        }
    }

    async fn pump_bck_to_cli(
        this: &web::Data<Self>,
        info: Arc<ProxyHttpRequestInfo>,
        timestamp: UtcDateTime,
        bck_msg: Option<Result<tungstenite::Message, tungstenite::Error>>,
        cli_tx: &mut Session,
        bck_tx: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, tungstenite::Message>,
    ) -> Result<(), &'static str> {
        match bck_msg {
            Some(Ok(msg)) => {
                match msg {
                    // binary
                    tungstenite::Message::Binary(bytes) => {
                        if let Err(err) = cli_tx.binary(bytes.clone()).await {
                            error!(
                                proxy = P::name(),
                                connection_id = %info.connection_id(),
                                worker_id = %this.id,
                                error = ?err,
                                "Failed to proxy binary websocket message to client"
                            );
                            this.shared
                                .metrics
                                .ws_proxy_failure_count
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_CLIENT,
                                })
                                .inc();
                            return Err(WS_CLI_ERROR);
                        }
                        this.postprocessor.do_send(ProxyWsMessage::BackendToClientBinary {
                            msg: bytes,
                            info,
                            start: timestamp,
                            end: UtcDateTime::now(),
                        });
                        return Ok(());
                    }

                    // text
                    tungstenite::Message::Text(text) => {
                        if let Err(err) = cli_tx.text(text.clone().as_str()).await {
                            error!(
                                proxy = P::name(),
                                connection_id = %info.connection_id(),
                                worker_id = %this.id,
                                error = ?err,
                                "Failed to proxy text websocket message to client"
                            );
                            this.shared
                                .metrics
                                .ws_proxy_failure_count
                                .get_or_create(&LabelsProxyWs {
                                    proxy: P::name(),
                                    destination: WS_LABEL_CLIENT,
                                })
                                .inc();
                            return Err(WS_CLI_ERROR);
                        }
                        this.postprocessor.do_send(ProxyWsMessage::BackendToClientText {
                            msg: text,
                            info,
                            start: timestamp,
                            end: UtcDateTime::now(),
                        });
                        return Ok(());
                    }

                    // ping
                    tungstenite::Message::Ping(bytes) => {
                        if let Err(err) = bck_tx.send(tungstenite::Message::Pong(bytes)).await {
                            error!(
                                proxy = P::name(),
                                connection_id = %info.connection_id(),
                                worker_id = %this.id,
                                error = ?err,
                                "Failed to return pong message to backend"
                            );
                            return Err(WS_BCK_ERROR);
                        }
                        return Ok(());
                    }

                    // pong
                    tungstenite::Message::Pong(bytes) => {
                        if let Some(pong) = ProxyWsPing::from_bytes(bytes) {
                            if let Some((_, ping)) = this.pings.remove_sync(&pong.id) {
                                if pong == ping {
                                    this.ping_balance_bck.dec();
                                    this.shared
                                        .metrics
                                        .ws_latency_backend
                                        .get_or_create(&LabelsProxyWs {
                                            proxy: P::name(),
                                            destination: WS_LABEL_BACKEND,
                                        })
                                        .record(
                                            (1000000.0 *
                                                (timestamp - pong.timestamp).as_seconds_f64() /
                                                2.0)
                                                as i64,
                                        );
                                    return Ok(());
                                }
                            }
                        }
                        warn!(
                            proxy = P::name(),
                            connection_id = %info.connection_id(),
                            worker_id = %this.id,
                            "Unexpected websocket pong received from backend",
                        );
                        return Ok(());
                    }

                    // close
                    tungstenite::Message::Close(reason) => {
                        if let Err(err) = cli_tx
                            .clone() // .close() consumes it
                            .close(reason.map(|reason| actix_ws::CloseReason {
                                code: u16::from(reason.code).into(),
                                description: reason.reason.to_string().into(),
                            }))
                            .await
                        {
                            error!(
                                proxy = P::name(),
                                connection_id = %info.connection_id(),
                                worker_id = %this.id,
                                error = ?err,
                                "Failed to proxy close websocket message to client"
                            );
                            return Err(WS_CLI_ERROR);
                        }
                        return Err(WS_CLOSE_OK);
                    }

                    _ => {
                        return Ok(());
                    }
                }
            }

            Some(Err(err)) => {
                error!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    error = ?err,
                    "Backend websocket stream error"
                );
                return Err(WS_BCK_ERROR);
            }

            None => {
                info!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %this.id,
                    "Backend had closed websocket stream"
                );
                return Err(WS_CLOSE_OK);
            }
        }
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
                        serde_json::from_slice(&msg).unwrap_or_default(),
                    ))
                } else {
                    Loggable(&serde_json::Value::Null)
                };

                info!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %worker_id,
                    remote_addr = info.remote_addr(),
                    ts_message_received = start.format(&Iso8601::DEFAULT).unwrap_or_default(),
                    latency_proxying = (*end - *start).as_seconds_f64(),
                    message = tracing::field::valuable(&json_msg),
                    "Proxied binary message to client",
                );
            }

            ProxyWsMessage::BackendToClientText { msg, info, start, end } => {
                let json_msg = if config.log_backend_messages() {
                    Loggable(&Self::maybe_sanitise(
                        config.log_sanitise(),
                        serde_json::from_str(&msg).unwrap_or_default(),
                    ))
                } else {
                    Loggable(&serde_json::Value::Null)
                };

                info!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %worker_id,
                    remote_addr = info.remote_addr(),
                    ts_message_received = start.format(&Iso8601::DEFAULT).unwrap_or_default(),
                    latency_proxying = (*end - *start).as_seconds_f64(),
                    message = tracing::field::valuable(&json_msg),
                    "Proxied text message to client",
                );
            }

            ProxyWsMessage::ClientToBackendBinary { msg, info, start, end } => {
                let json_msg = if config.log_client_messages() {
                    Loggable(&Self::maybe_sanitise(
                        config.log_sanitise(),
                        serde_json::from_slice(&msg).unwrap_or_default(),
                    ))
                } else {
                    Loggable(&serde_json::Value::Null)
                };

                info!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %worker_id,
                    remote_addr = info.remote_addr(),
                    ts_message_received = start.format(&Iso8601::DEFAULT).unwrap_or_default(),
                    latency_proxying = (*end - *start).as_seconds_f64(),
                    message = tracing::field::valuable(&json_msg),
                    "Proxied binary message to backend",
                );
            }

            ProxyWsMessage::ClientToBackendText { msg, info, start, end } => {
                let json_msg = if config.log_client_messages() {
                    Loggable(&Self::maybe_sanitise(
                        config.log_sanitise(),
                        serde_json::from_str(&msg).unwrap_or_default(),
                    ))
                } else {
                    Loggable(&serde_json::Value::Null)
                };

                info!(
                    proxy = P::name(),
                    connection_id = %info.connection_id(),
                    worker_id = %worker_id,
                    remote_addr = info.remote_addr(),
                    ts_message_received = start.format(&Iso8601::DEFAULT).unwrap_or_default(),
                    latency_proxying = (*end - *start).as_seconds_f64(),
                    message = tracing::field::valuable(&json_msg),
                    "Proxied text message to backend",
                );
            }
        }
    }

    fn maybe_sanitise(do_sanitise: bool, mut message: serde_json::Value) -> serde_json::Value {
        if !do_sanitise {
            return message;
        }

        if let Some(object) = message.as_object_mut() {
            if let Some(diff) = object.get_mut("diff") {
                if let Some(transactions) = diff.get_mut("transactions") {
                    if let Some(transactions) = transactions.as_array_mut() {
                        for transaction in transactions {
                            raw_transaction_to_hash(transaction);
                        }
                    }
                }
            }
        }

        message
    }

    fn emit_metrics_on_proxy_success(msg: &ProxyWsMessage, metrics: Arc<Metrics>) {
        match msg {
            ProxyWsMessage::BackendToClientBinary { msg, info: _, start, end } => {
                let labels = LabelsProxyWs { proxy: P::name(), destination: WS_LABEL_CLIENT };
                metrics
                    .ws_latency_proxy
                    .get_or_create(&labels)
                    .record((1000000.0 * (*end - *start).as_seconds_f64()) as i64);
                metrics.ws_proxy_success_count.get_or_create_owned(&labels).inc();
                metrics.ws_message_size.get_or_create_owned(&labels).record(msg.len() as i64);
            }

            ProxyWsMessage::BackendToClientText { msg, info: _, start, end } => {
                let labels = LabelsProxyWs { proxy: P::name(), destination: WS_LABEL_CLIENT };
                metrics
                    .ws_latency_proxy
                    .get_or_create(&labels)
                    .record((1000000.0 * (*end - *start).as_seconds_f64()) as i64);
                metrics.ws_proxy_success_count.get_or_create_owned(&labels).inc();
                metrics.ws_message_size.get_or_create_owned(&labels).record(msg.len() as i64);
            }

            ProxyWsMessage::ClientToBackendBinary { msg, info: _, start, end } => {
                let labels = LabelsProxyWs { proxy: P::name(), destination: WS_LABEL_BACKEND };
                metrics
                    .ws_latency_proxy
                    .get_or_create(&labels)
                    .record((1000000.0 * (*end - *start).as_seconds_f64()) as i64);
                metrics.ws_proxy_success_count.get_or_create_owned(&labels).inc();
                metrics.ws_message_size.get_or_create_owned(&labels).record(msg.len() as i64);
            }

            ProxyWsMessage::ClientToBackendText { msg, info: _, start, end } => {
                let labels = LabelsProxyWs { proxy: P::name(), destination: WS_LABEL_BACKEND };
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

impl<C, P> Proxy<P> for ProxyWs<C, P>
where
    C: ConfigProxyWs,
    P: ProxyWsInner<C>,
{
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
                    request_id = %info.id(),
                    connection_id = %info.connection_id(),
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
                    request_id = %info.id(),
                    connection_id = %info.connection_id(),
                    worker_id = %self.worker_id,
                    error = ?err, "Failed to construct backend URI, defaulting to the base one",
                );
            })
            .unwrap_or(self.url.clone())
    }
}

// ProxyWsPostprocessor

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
        let worker_id = self.worker_id.clone();

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
    connection_id: Uuid,
    timestamp: UtcDateTime,
}

impl ProxyWsPing {
    fn new(connection_id: Uuid) -> Self {
        Self { id: Uuid::now_v7(), connection_id, timestamp: UtcDateTime::now() }
    }

    fn to_bytes(&self) -> Bytes {
        let mut bytes = BytesMut::with_capacity(48);
        bytes.put_u128(self.id.as_u128());
        bytes.put_u128(self.connection_id.as_u128());
        bytes.put_i128(self.timestamp.unix_timestamp_nanos());
        bytes.freeze()
    }

    fn from_bytes(mut bytes: Bytes) -> Option<Self> {
        if bytes.len() != 48 {
            return None;
        }

        let id = Uuid::from_u128(bytes.get_u128());
        let connection_id = Uuid::from_u128(bytes.get_u128());
        let timestamp = match UtcDateTime::from_unix_timestamp_nanos(bytes.get_i128()) {
            Ok(timestamp) => timestamp,
            Err(_) => return None,
        };

        Some(Self { id, connection_id, timestamp })
    }

    fn to_slice(&self) -> [u8; 48] {
        let res: [u8; 48] = [0; 48];
        let mut cur = std::io::Cursor::new(res);

        let _ = cur.write(self.id.as_bytes());
        let _ = cur.write(self.connection_id.as_bytes());
        let _ = cur.write(&self.timestamp.unix_timestamp_nanos().to_be_bytes());

        cur.into_inner()
    }
}

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
