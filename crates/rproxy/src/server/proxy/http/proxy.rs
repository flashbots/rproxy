use std::{
    borrow::Cow,
    fmt::Debug,
    marker::PhantomData,
    mem,
    pin::Pin,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use actix::{Actor, AsyncContext, WrapFuture};
use actix_web::{
    self,
    App,
    HttpRequest,
    HttpResponse,
    HttpResponseBuilder,
    HttpServer,
    body::BodySize,
    http::{StatusCode, header},
    middleware::{NormalizePath, TrailingSlash},
    web,
};
use awc::{
    Client,
    ClientRequest,
    ClientResponse,
    Connector,
    body::MessageBody,
    error::HeaderValue,
    http::{Method, header::HeaderMap},
};
use bytes::Bytes;
use futures::TryStreamExt;
use futures_core::Stream;
use pin_project::pin_project;
use scc::HashMap;
use time::{UtcDateTime, format_description::well_known::Iso8601};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};
use url::Url;
use uuid::Uuid;
use x509_parser::asn1_rs::ToStatic;

use crate::{
    config::PARALLELISM,
    jrpc::JrpcRequestMetaMaybeBatch,
    server::{
        metrics::{LabelsProxy, LabelsProxyClientInfo, LabelsProxyHttpJrpc, Metrics},
        proxy::{
            Proxy,
            ProxyConnectionGuard,
            config::ConfigTls,
            http::{
                ProxyHttpInner,
                config::{ConfigProxyHttp, ConfigProxyHttpMirroringStrategy},
            },
        },
    },
    utils::{Loggable, decompress, is_hop_by_hop_header, raw_transaction_to_hash},
};

const TCP_KEEPALIVE_ATTEMPTS: u32 = 8;

// ProxyHttp -----------------------------------------------------------

pub(crate) struct ProxyHttp<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    id: Uuid,

    shared: ProxyHttpSharedState<C, P>,

    backend: ProxyHttpBackendEndpoint<C, P>,
    requests: HashMap<Uuid, ProxiedHttpRequest>,
    postprocessor: actix::Addr<ProxyHttpPostprocessor<C, P>>,
}

impl<C, P> ProxyHttp<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    fn new(shared: ProxyHttpSharedState<C, P>, connections_limit: usize) -> Self {
        let id = Uuid::now_v7();

        debug!(proxy = P::name(), worker_id = %id, "Creating http-proxy worker...");

        let config = shared.config();
        let inner = shared.inner();

        let backend = ProxyHttpBackendEndpoint::new(
            inner.clone(),
            id,
            shared.metrics.clone(),
            config.backend_url(),
            connections_limit,
            config.backend_timeout(),
        );

        let peers: Arc<Vec<actix::Addr<ProxyHttpBackendEndpoint<C, P>>>> = Arc::new(
            config
                .mirroring_peer_urls()
                .iter()
                .map(|peer_url| {
                    ProxyHttpBackendEndpoint::new(
                        shared.inner(),
                        id,
                        shared.metrics.clone(),
                        peer_url.to_owned(),
                        config.backend_max_concurrent_requests(),
                        config.backend_timeout(),
                    )
                    .start()
                })
                .collect(),
        );

        let postprocessor = ProxyHttpPostprocessor::<C, P> {
            worker_id: id,
            inner: inner.clone(),
            metrics: shared.metrics.clone(),
            mirroring_peers: peers.clone(),
            mirroring_peer_round_robin_index: AtomicUsize::new(0),
        }
        .start();

        Self { id, shared, backend, requests: HashMap::default(), postprocessor }
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

        let workers_count =
            std::cmp::min(PARALLELISM.to_static(), config.backend_max_concurrent_requests());
        let max_concurrent_requests_per_worker =
            config.backend_max_concurrent_requests() / workers_count;
        if workers_count * max_concurrent_requests_per_worker <
            config.backend_max_concurrent_requests()
        {
            warn!(
                "Max backend concurrent requests must be a round of available parallelism ({}), therefore it's clamped at {} (instead of {})",
                PARALLELISM.to_static(),
                workers_count * max_concurrent_requests_per_worker,
                config.backend_max_concurrent_requests()
            );
        }

        let shared = ProxyHttpSharedState::<C, P>::new(config, &metrics);
        let client_connections_count = shared.client_connections_count.clone();

        info!(
            proxy = P::name(),
            listen_address = %listen_address,
            workers_count = workers_count,
            max_concurrent_requests_per_worker = max_concurrent_requests_per_worker,
            "Starting http-proxy..."
        );

        let proxy = HttpServer::new(move || {
            let this =
                web::Data::new(Self::new(shared.clone(), max_concurrent_requests_per_worker));

            App::new()
                .app_data(this)
                .wrap(NormalizePath::new(TrailingSlash::Trim))
                .default_service(web::route().to(Self::receive))
        })
        .on_connect(Self::on_connect(P::name(), metrics, client_connections_count))
        .shutdown_signal(canceller.cancelled_owned())
        .workers(workers_count);

        let server = match if tls.enabled() {
            let cert = tls.certificate().clone();
            let key = tls.key().clone_key();

            proxy.listen_rustls_0_23(
                listener,
                rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(cert, key)
                    .unwrap(), // safety: verified on start
            )
        } else {
            proxy.listen(listener)
        } {
            Ok(server) => server,
            Err(err) => {
                error!(
                    proxy = P::name(),
                    error = ?err,
                    "Failed to initialise http-proxy",
                );
                return Err(Box::new(err));
            }
        }
        .run();

        let handler = server.handle();
        let mut resetter = resetter.subscribe();
        tokio::spawn(async move {
            if resetter.recv().await.is_ok() {
                info!(proxy = P::name(), "Reset signal received, stopping http-proxy...");
                handler.stop(true).await;
            }
        });

        if let Err(err) = server.await {
            error!(proxy = P::name(), error = ?err, "Failure while running http-proxy")
        }

        info!(proxy = P::name(), "Stopped http-proxy");

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

        // allow binding while there are still residual connections in TIME_WAIT
        socket.set_reuse_address(true)?;

        if !config.idle_connection_timeout().is_zero() {
            socket.set_tcp_keepalive(
                &socket2::TcpKeepalive::new()
                    .with_time(
                        config.idle_connection_timeout().div_f64(f64::from(TCP_KEEPALIVE_ATTEMPTS)),
                    )
                    .with_interval(
                        config.idle_connection_timeout().div_f64(f64::from(TCP_KEEPALIVE_ATTEMPTS)),
                    )
                    .with_retries(TCP_KEEPALIVE_ATTEMPTS - 1),
            )?;
        }

        socket.bind(&socket2::SockAddr::from(config.listen_address()))?;

        socket.listen(1024)?;

        Ok(socket.into())
    }

    fn to_client_response<S>(bck_res: &ClientResponse<S>) -> HttpResponseBuilder {
        let mut cli_res = HttpResponse::build(bck_res.status());

        for (name, header) in bck_res.headers().iter() {
            if is_hop_by_hop_header(name) {
                continue;
            }
            if let Ok(hname) = header::HeaderName::from_str(name.as_str()) {
                cli_res.append_header((hname, header.clone()));
            }
        }

        cli_res
    }

    /// receive accepts client's (frontend) request and proxies it to
    /// backend
    async fn receive(
        cli_req: HttpRequest,
        cli_req_body: web::Payload,
        this: web::Data<Self>,
    ) -> Result<HttpResponse, actix_web::Error> {
        let timestamp = UtcDateTime::now();

        if let Some(user_agent) = cli_req.headers().get(header::USER_AGENT) &&
            !user_agent.is_empty() &&
            let Ok(user_agent) = user_agent.to_str()
        {
            this.shared
                .metrics
                .client_info
                .get_or_create(&LabelsProxyClientInfo {
                    proxy: P::name(),
                    user_agent: user_agent.to_string(),
                })
                .inc();
        }

        let info = ProxyHttpRequestInfo::new(&cli_req, cli_req.conn_data::<ProxyConnectionGuard>());

        let id = info.id;
        let connection_id = info.connection_id;

        let bck_req = this.backend.new_backend_request(&info);
        let bck_req_body = ProxyHttpRequestBody::new(this.clone(), info, cli_req_body, timestamp);

        let bck_res = match bck_req.send_stream(bck_req_body).await {
            Ok(res) => res,
            Err(err) => {
                warn!(
                    proxy = P::name(),
                    request_id = %id,
                    connection_id = %connection_id,
                    worker_id = %this.id,
                    backend_url = %this.backend.url,
                    error = ?err,
                    "Failed to proxy a request",
                );
                this.shared
                    .metrics
                    .http_proxy_failure_count
                    .get_or_create(&LabelsProxy { proxy: P::name() })
                    .inc();
                return Ok(HttpResponse::BadGateway().body(format!("Backend error: {err:?}")));
            }
        };

        let timestamp = UtcDateTime::now();
        let status = bck_res.status();
        let mut cli_res = Self::to_client_response(&bck_res);

        let bck_body = ProxyHttpResponseBody::new(
            this,
            id,
            status,
            bck_res.headers().clone(),
            bck_res.into_stream(),
            timestamp,
        );

        Ok(cli_res.streaming(bck_body))
    }

    fn postprocess_client_request(&self, req: ProxiedHttpRequest) {
        let id = req.info.id;
        let connection_id = req.info.connection_id;

        if self.requests.insert_sync(id, req).is_err() {
            error!(
                proxy = P::name(),
                request_id = %id,
                connection_id = %connection_id,
                worker_id = %self.id,
                "Duplicate request id",
            );
        };
    }

    fn postprocess_backend_response(&self, bck_res: ProxiedHttpResponse) {
        let Some((_, cli_req)) = self.requests.remove_sync(&bck_res.info.id) else {
            error!(
                proxy = P::name(),
                request_id = %bck_res.info.id,
                worker_id = %self.id,
                "Proxied http response for unmatching request",
            );
            return;
        };

        // hand over to postprocessor asynchronously so that we can return the
        // response to the client as early as possible
        self.postprocessor.do_send(ProxiedHttpCombo { req: cli_req, res: bck_res });
    }

    fn finalise_proxying(
        mut cli_req: ProxiedHttpRequest,
        mut bck_res: ProxiedHttpResponse,
        inner: Arc<P>,
        worker_id: Uuid,
        metrics: Arc<Metrics>,
        mirroring_peers: Arc<Vec<actix::Addr<ProxyHttpBackendEndpoint<C, P>>>>,
        mut mirroring_peer_round_robin_index: usize,
    ) {
        if cli_req.decompressed_size < cli_req.size {
            (cli_req.decompressed_body, cli_req.decompressed_size) =
                decompress(cli_req.body.clone(), cli_req.size, cli_req.info.content_encoding());
        }

        if bck_res.decompressed_size < bck_res.size {
            (bck_res.decompressed_body, bck_res.decompressed_size) =
                decompress(bck_res.body.clone(), bck_res.size, bck_res.info.content_encoding());
        }

        match serde_json::from_slice::<JrpcRequestMetaMaybeBatch>(&cli_req.decompressed_body) {
            Ok(jrpc) => {
                if inner.should_mirror(&jrpc, &cli_req, &bck_res) {
                    let mirrors_count = match inner.config().mirroring_strategy() {
                        ConfigProxyHttpMirroringStrategy::FanOut => mirroring_peers.len(),
                        ConfigProxyHttpMirroringStrategy::RoundRobin => 1,
                        ConfigProxyHttpMirroringStrategy::RoundRobinPairs => 2,
                    };

                    for _ in 0..mirrors_count {
                        let mirroring_peer = &mirroring_peers[mirroring_peer_round_robin_index];
                        mirroring_peer_round_robin_index += 1;
                        if mirroring_peer_round_robin_index >= mirroring_peers.len() {
                            mirroring_peer_round_robin_index = 0;
                        }

                        let mut req = cli_req.clone();
                        req.info.jrpc_method_enriched = jrpc.method_enriched();
                        mirroring_peer.do_send(req.clone());
                    }
                }

                Self::maybe_log_proxied_request_and_response(
                    &jrpc,
                    &cli_req,
                    &bck_res,
                    inner.clone(),
                    worker_id,
                );

                Self::emit_metrics_on_proxy_success(&jrpc, &cli_req, &bck_res, metrics.clone());
            }

            Err(err) => {
                warn!(
                    proxy = P::name(),
                    request_id = %cli_req.info.id,
                    connection_id = %cli_req.info.connection_id,
                    worker_id = %worker_id,
                    error = ?err,
                    "Failed to parse json-rpc request",
                );
            }
        }
    }

    fn postprocess_mirrored_response(
        mut cli_req: ProxiedHttpRequest,
        mut mrr_res: ProxiedHttpResponse,
        inner: Arc<P>,
        metrics: Arc<Metrics>,
        worker_id: Uuid,
    ) {
        if cli_req.decompressed_size < cli_req.size {
            (cli_req.decompressed_body, cli_req.decompressed_size) =
                decompress(cli_req.body.clone(), cli_req.size, cli_req.info.content_encoding());
        }

        if mrr_res.decompressed_size < mrr_res.size {
            (mrr_res.decompressed_body, mrr_res.decompressed_size) =
                decompress(mrr_res.body.clone(), mrr_res.size, mrr_res.info.content_encoding());
        }

        Self::maybe_log_mirrored_request(&cli_req, &mrr_res, worker_id, inner.config());

        metrics
            .http_mirror_success_count
            .get_or_create(&LabelsProxyHttpJrpc {
                proxy: P::name(),
                jrpc_method: cli_req.info.jrpc_method_enriched,
            })
            .inc();
    }

    fn maybe_log_proxied_request_and_response(
        jrpc: &JrpcRequestMetaMaybeBatch,
        req: &ProxiedHttpRequest,
        res: &ProxiedHttpResponse,
        inner: Arc<P>,
        worker_id: Uuid,
    ) {
        let config = inner.config();

        let json_req = if config.log_proxied_requests() {
            Loggable(&Self::maybe_sanitise(
                config.log_sanitise(),
                serde_json::from_slice(&req.decompressed_body).unwrap_or_default(),
            ))
        } else {
            Loggable(&serde_json::Value::Null)
        };

        let json_res = if config.log_proxied_responses() {
            Loggable(&Self::maybe_sanitise(
                config.log_sanitise(),
                serde_json::from_slice(&res.decompressed_body).unwrap_or_default(),
            ))
        } else {
            Loggable(&serde_json::Value::Null)
        };

        info!(
            proxy = P::name(),
            request_id = %req.info.id,
            connection_id = %req.info.connection_id,
            worker_id = %worker_id,
            jrpc_method = %jrpc.method_enriched(),
            http_status = res.status(),
            remote_addr = req.info().remote_addr,
            ts_request_received = req.start().format(&Iso8601::DEFAULT).unwrap_or_default(),
            latency_backend = (res.start() - req.end()).as_seconds_f64(),
            latency_total = (res.end() - req.start()).as_seconds_f64(),
            json_request = tracing::field::valuable(&json_req),
            json_response = tracing::field::valuable(&json_res),
            "Proxied request"
        );
    }

    fn maybe_log_mirrored_request(
        req: &ProxiedHttpRequest,
        res: &ProxiedHttpResponse,
        worker_id: Uuid,
        config: &C,
    ) {
        let json_req = if config.log_mirrored_requests() {
            Loggable(&Self::maybe_sanitise(
                config.log_sanitise(),
                serde_json::from_slice(&req.decompressed_body).unwrap_or_default(),
            ))
        } else {
            Loggable(&serde_json::Value::Null)
        };

        let json_res = if config.log_mirrored_responses() {
            Loggable(&Self::maybe_sanitise(
                config.log_sanitise(),
                serde_json::from_slice(&res.decompressed_body).unwrap_or_default(),
            ))
        } else {
            Loggable(&serde_json::Value::Null)
        };

        info!(
            proxy = P::name(),
            request_id = %req.info.id,
            connection_id = %req.info.connection_id,
            worker_id = %worker_id,
            jrpc_method = %req.info.jrpc_method_enriched,
            http_status = res.status(),
            remote_addr = req.info().remote_addr,
            ts_request_received = req.start().format(&Iso8601::DEFAULT).unwrap_or_default(),
            latency_backend = (res.start() - req.end()).as_seconds_f64(),
            latency_total = (res.end() - req.start()).as_seconds_f64(),
            json_request = tracing::field::valuable(&json_req),
            json_response = tracing::field::valuable(&json_res),
            "Mirrored request"
        );
    }

    fn maybe_sanitise(do_sanitise: bool, mut message: serde_json::Value) -> serde_json::Value {
        if do_sanitise {
            sanitise(&mut message);
        }
        return message;

        fn sanitise(message: &mut serde_json::Value) {
            if let Some(batch) = message.as_array_mut() {
                for item in batch {
                    sanitise(item);
                }
                return;
            }

            let Some(message) = message.as_object_mut() else { return };

            let method = (match message.get_key_value("method") {
                Some((_, method)) => method.as_str(),
                None => None,
            })
            .unwrap_or_default()
            .to_owned();

            if !method.is_empty() {
                // single-shot request

                let Some(params) = match message.get_mut("params") {
                    Some(params) => params,
                    None => return,
                }
                .as_array_mut() else {
                    return
                };

                match method.as_str() {
                    "engine_forkchoiceUpdatedV3" => {
                        if params.len() < 2 {
                            return;
                        }

                        let Some(execution_payload) = params[1].as_object_mut() else { return };

                        let Some(transactions) = match execution_payload.get_mut("transactions") {
                            Some(transactions) => transactions,
                            None => return,
                        }
                        .as_array_mut() else {
                            return
                        };

                        for transaction in transactions {
                            raw_transaction_to_hash(transaction);
                        }
                    }

                    "engine_newPayloadV4" => {
                        if params.is_empty() {
                            return;
                        }

                        let Some(execution_payload) = params[0].as_object_mut() else { return };

                        let Some(transactions) = match execution_payload.get_mut("transactions") {
                            Some(transactions) => transactions,
                            None => return,
                        }
                        .as_array_mut() else {
                            return
                        };

                        for transaction in transactions {
                            raw_transaction_to_hash(transaction);
                        }
                    }

                    "eth_sendBundle" => {
                        if params.is_empty() {
                            return;
                        }

                        let Some(execution_payload) = params[0].as_object_mut() else { return };

                        let Some(transactions) = match execution_payload.get_mut("txs") {
                            Some(transactions) => transactions,
                            None => return,
                        }
                        .as_array_mut() else {
                            return
                        };

                        for transaction in transactions {
                            raw_transaction_to_hash(transaction);
                        }
                    }

                    "eth_sendRawTransaction" => {
                        for transaction in params {
                            raw_transaction_to_hash(transaction);
                        }
                    }

                    _ => {
                        return;
                    }
                }
            }

            let Some(result) = (match message.get_mut("result") {
                Some(result) => result.as_object_mut(),
                None => return,
            }) else {
                return
            };

            if let Some(execution_payload) = result.get_mut("executionPayload") &&
                let Some(transactions) = execution_payload.get_mut("transactions") &&
                let Some(transactions) = transactions.as_array_mut()
            {
                // engine_getPayloadV4

                for transaction in transactions {
                    raw_transaction_to_hash(transaction);
                }
            }
        }
    }

    fn emit_metrics_on_proxy_success(
        jrpc: &JrpcRequestMetaMaybeBatch,
        req: &ProxiedHttpRequest,
        res: &ProxiedHttpResponse,
        metrics: Arc<Metrics>,
    ) {
        let metric_labels_jrpc = match jrpc {
            JrpcRequestMetaMaybeBatch::Single(jrpc) => {
                LabelsProxyHttpJrpc { jrpc_method: jrpc.method_enriched(), proxy: P::name() }
            }

            JrpcRequestMetaMaybeBatch::Batch(_) => {
                LabelsProxyHttpJrpc { jrpc_method: Cow::Borrowed("batch"), proxy: P::name() }
            }
        };

        let latency_backend = 1000000.0 * (res.start() - req.end()).as_seconds_f64();
        let latency_total = 1000000.0 * (res.end() - req.start()).as_seconds_f64();

        // latency_backend
        metrics
            .http_latency_backend
            .get_or_create(&metric_labels_jrpc)
            .record(latency_backend.round() as i64);

        // latency_delta
        metrics
            .http_latency_delta
            .get_or_create(&metric_labels_jrpc)
            .record((latency_total - latency_backend).round() as i64);

        // latency_total
        metrics
            .http_latency_total
            .get_or_create(&metric_labels_jrpc)
            .record(latency_total.round() as i64);

        // proxy_success_count
        match jrpc {
            JrpcRequestMetaMaybeBatch::Single(_) => {
                metrics.http_proxy_success_count.get_or_create(&metric_labels_jrpc).inc();
            }

            JrpcRequestMetaMaybeBatch::Batch(batch) => {
                for jrpc in batch.iter() {
                    let metric_labels_jrpc = LabelsProxyHttpJrpc {
                        jrpc_method: jrpc.method_enriched(),
                        proxy: P::name(),
                    };
                    metrics.http_proxy_success_count.get_or_create(&metric_labels_jrpc).inc();
                }
            }
        }

        // proxied_request_size
        metrics.http_request_size.get_or_create_owned(&metric_labels_jrpc).record(req.size as i64);

        // proxied_response_size
        metrics.http_response_size.get_or_create_owned(&metric_labels_jrpc).record(res.size as i64);

        // proxied_request_decompressed_size
        metrics
            .http_request_decompressed_size
            .get_or_create_owned(&metric_labels_jrpc)
            .record(req.decompressed_size as i64);

        // proxied_response_decompressed_size
        metrics
            .http_response_decompressed_size
            .get_or_create_owned(&metric_labels_jrpc)
            .record(res.decompressed_size as i64);
    }
}

impl<C, P> Proxy for ProxyHttp<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
}

impl<C, P> Drop for ProxyHttp<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    fn drop(&mut self) {
        debug!(
            proxy = P::name(),
            worker_id = %self.id,
            "Destroying http-proxy worker...",
        );
    }
}

// ProxyHttpSharedState ------------------------------------------------

#[derive(Clone)]
struct ProxyHttpSharedState<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    inner: Arc<P>,
    metrics: Arc<Metrics>,

    client_connections_count: Arc<AtomicI64>,

    _config: PhantomData<C>,
}

impl<C, P> ProxyHttpSharedState<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
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

    #[inline]
    fn inner(&self) -> Arc<P> {
        self.inner.clone()
    }
}

// ProxyHttpPostprocessor ----------------------------------------------

struct ProxyHttpPostprocessor<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    inner: Arc<P>,
    worker_id: Uuid,
    metrics: Arc<Metrics>,

    /// mirroring_peers is the vector of endpoints for mirroring peers.
    mirroring_peers: Arc<Vec<actix::Addr<ProxyHttpBackendEndpoint<C, P>>>>,

    /// mirroring_peer_round_robin_index is used for round-robin mirroring
    /// strategy.  it holds the index of the mirroring peers that will be
    /// used for the next round of mirroring.
    mirroring_peer_round_robin_index: AtomicUsize,
}

impl<C, P> actix::Actor for ProxyHttpPostprocessor<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    type Context = actix::Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.set_mailbox_capacity(1024);
    }
}

impl<C, P> actix::Handler<ProxiedHttpCombo> for ProxyHttpPostprocessor<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    type Result = ();

    fn handle(&mut self, msg: ProxiedHttpCombo, ctx: &mut Self::Context) -> Self::Result {
        let inner = self.inner.clone();
        let metrics = self.metrics.clone();
        let worker_id = self.worker_id;
        let mirroring_peers = self.mirroring_peers.clone();
        let mut mirroring_peer_round_robin_index =
            self.mirroring_peer_round_robin_index.load(Ordering::Relaxed);

        ctx.spawn(
            async move {
                ProxyHttp::<C, P>::finalise_proxying(
                    msg.req,
                    msg.res,
                    inner,
                    worker_id,
                    metrics,
                    mirroring_peers,
                    mirroring_peer_round_robin_index,
                );
            }
            .into_actor(self),
        );

        mirroring_peer_round_robin_index += 1;
        if mirroring_peer_round_robin_index >= self.mirroring_peers.len() {
            mirroring_peer_round_robin_index = 0;
        }
        self.mirroring_peer_round_robin_index
            .store(mirroring_peer_round_robin_index, Ordering::Relaxed);
    }
}

// ProxyHttpBackendEndpoint --------------------------------------------

pub(crate) struct ProxyHttpBackendEndpoint<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    inner: Arc<P>,
    worker_id: Uuid,
    metrics: Arc<Metrics>,

    client: Client,
    url: Url,

    _config: PhantomData<C>,
}

impl<C, P> ProxyHttpBackendEndpoint<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    fn new(
        inner: Arc<P>,
        worker_id: Uuid,
        metrics: Arc<Metrics>,
        url: Url,
        connections_limit: usize,
        timeout: std::time::Duration,
    ) -> Self {
        let host = url
            .host()
            .unwrap() // safety: verified on start
            .to_string();

        let client = Client::builder()
            .add_default_header((header::HOST, host))
            .connector(Connector::new().conn_keep_alive(2 * timeout).limit(connections_limit))
            .timeout(timeout)
            .finish();

        Self { inner, worker_id, metrics, client, url, _config: PhantomData }
    }

    fn new_backend_request(&self, info: &ProxyHttpRequestInfo) -> ClientRequest {
        let mut url = self.url.clone();
        url.set_path(&info.path);

        let mut req = self.client.request(info.method.clone(), url.as_str()).no_decompress();

        for (header, value) in info.headers.iter() {
            req = req.insert_header((header.clone(), value.clone()));
        }

        req
    }
}

impl<C, P> actix::Actor for ProxyHttpBackendEndpoint<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    type Context = actix::Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.set_mailbox_capacity(1024);
    }
}

impl<C, P> actix::Handler<ProxiedHttpRequest> for ProxyHttpBackendEndpoint<C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    type Result = ();

    fn handle(&mut self, cli_req: ProxiedHttpRequest, ctx: &mut Self::Context) -> Self::Result {
        let start = UtcDateTime::now();

        let inner = self.inner.clone();
        let worker_id = self.worker_id;
        let metrics = self.metrics.clone();

        let mrr_req = self.new_backend_request(&cli_req.info);
        let mrr_req_body = cli_req.body.clone();

        ctx.spawn(
            async move {
                match mrr_req.send_body(mrr_req_body).await {
                    Ok(mut bck_res) => {
                        let end = UtcDateTime::now();

                        match bck_res.body().await {
                            Ok(mrr_res_body) => {
                                let size = match mrr_res_body.size() {
                                    BodySize::Sized(size) => size, // Body is always sized
                                    BodySize::None | BodySize::Stream => 0,
                                };
                                let info = ProxyHttpResponseInfo::new(
                                    cli_req.info.id,
                                    bck_res.status(),
                                    bck_res.headers().clone(),
                                );
                                let mrr_res = ProxiedHttpResponse {
                                    info,
                                    body: mrr_res_body,
                                    size: size as usize,
                                    decompressed_body: Bytes::new(),
                                    decompressed_size: 0,
                                    start,
                                    end,
                                };
                                ProxyHttp::<C, P>::postprocess_mirrored_response(
                                    cli_req, mrr_res, inner, metrics, worker_id,
                                );
                            }
                            Err(err) => {
                                warn!(
                                    proxy = P::name(),
                                    request_id = %cli_req.info.id,
                                    connection_id = %cli_req.info.connection_id,
                                    error = ?err,
                                    "Failed to mirror a request",
                                );
                                metrics
                                    .http_mirror_failure_count
                                    .get_or_create(&LabelsProxy { proxy: P::name() })
                                    .inc();
                            }
                        };
                    }

                    Err(err) => {
                        warn!(
                            proxy = P::name(),
                            request_id = %cli_req.info.id,
                            connection_id = %cli_req.info.connection_id,
                            error = ?err,
                            "Failed to mirror a request",
                        );
                        metrics
                            .http_mirror_failure_count
                            .get_or_create(&LabelsProxy { proxy: P::name() })
                            .inc();
                    }
                }
            }
            .into_actor(self),
        );
    }
}

// ProxyHttpRequestInfo ------------------------------------------------

#[derive(Clone)]
pub(crate) struct ProxyHttpRequestInfo {
    id: Uuid,
    connection_id: Uuid,
    remote_addr: Option<String>,
    method: Method,
    path: String,
    path_and_query: String,
    headers: HeaderMap,
    jrpc_method_enriched: Cow<'static, str>,
}

impl ProxyHttpRequestInfo {
    pub(crate) fn new(req: &HttpRequest, guard: Option<&ProxyConnectionGuard>) -> Self {
        // copy over only non hop-by-hop headers
        let mut headers = HeaderMap::new();
        for (header, value) in req.headers().iter() {
            if !is_hop_by_hop_header(header) {
                headers.insert(header.clone(), value.clone());
            }
        }

        // append remote ip to x-forwarded-for
        if let Some(peer_addr) = req.connection_info().peer_addr() {
            let mut forwarded_for = String::new();
            if let Some(ff) = req.headers().get(header::X_FORWARDED_FOR) &&
                let Ok(ff) = ff.to_str()
            {
                forwarded_for.push_str(ff);
                forwarded_for.push_str(", ");
            }
            forwarded_for.push_str(peer_addr);
            if let Ok(forwarded_for) = HeaderValue::from_str(&forwarded_for) {
                headers.insert(header::X_FORWARDED_FOR, forwarded_for);
            }
        }

        // set x-forwarded-proto if it's not already set
        if req.connection_info().scheme() != "" &&
            req.headers().get(header::X_FORWARDED_PROTO).is_none() &&
            let Ok(forwarded_proto) = HeaderValue::from_str(req.connection_info().scheme())
        {
            headers.insert(header::X_FORWARDED_PROTO, forwarded_proto);
        }

        // set x-forwarded-host if it's not already set
        if req.connection_info().scheme() != "" &&
            req.headers().get(header::X_FORWARDED_HOST).is_none() &&
            let Ok(forwarded_host) = HeaderValue::from_str(req.connection_info().scheme())
        {
            headers.insert(header::X_FORWARDED_HOST, forwarded_host);
        }

        // remote address from the guard has port, and connection info has ip
        // address only => we prefer the guard
        let remote_addr = match guard {
            Some(guard) => match guard.remote_addr.clone() {
                Some(remote_addr) => Some(remote_addr),
                None => req.connection_info().peer_addr().map(String::from),
            },
            None => req.connection_info().peer_addr().map(String::from),
        };

        let path = match req.path() {
            "" => "/",
            val => val,
        }
        .to_string();

        let path_and_query = match req.query_string() {
            "" => path.clone(),
            val => format!("{path}?{val}"),
        };

        Self {
            id: Uuid::now_v7(),
            connection_id: Uuid::now_v7(),
            remote_addr,
            method: req.method().clone(),
            path,
            path_and_query,
            headers,
            jrpc_method_enriched: Cow::Borrowed(""),
        }
    }

    #[inline]
    pub(crate) fn id(&self) -> Uuid {
        self.id
    }

    #[inline]
    pub(crate) fn connection_id(&self) -> Uuid {
        self.connection_id
    }

    #[inline]
    fn content_encoding(&self) -> String {
        self.headers
            .get(header::CONTENT_ENCODING)
            .map(|h| h.to_str().unwrap_or_default())
            .map(|h| h.to_string())
            .unwrap_or_default()
    }

    #[inline]
    pub(crate) fn path_and_query(&self) -> &str {
        &self.path_and_query
    }

    #[inline]
    pub(crate) fn remote_addr(&self) -> &Option<String> {
        &self.remote_addr
    }
}

// ProxyHttpResponseInfo -----------------------------------------------

#[derive(Clone)]
pub(crate) struct ProxyHttpResponseInfo {
    id: Uuid,
    status: StatusCode,
    headers: HeaderMap, // TODO: perhaps we don't need all headers, just select ones
}

impl ProxyHttpResponseInfo {
    pub(crate) fn new(id: Uuid, status: StatusCode, headers: HeaderMap) -> Self {
        Self { id, status, headers }
    }

    #[inline]
    pub(crate) fn id(&self) -> Uuid {
        self.id
    }

    fn content_encoding(&self) -> String {
        self.headers
            .get(header::CONTENT_ENCODING)
            .map(|h| h.to_str().unwrap_or_default())
            .map(|h| h.to_string())
            .unwrap_or_default()
    }
}

// ProxyHttpRequestBody ------------------------------------------------

#[pin_project]
struct ProxyHttpRequestBody<S, C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    proxy: web::Data<ProxyHttp<C, P>>,

    info: Option<ProxyHttpRequestInfo>,
    start: UtcDateTime,
    body: Vec<u8>,

    #[pin]
    stream: S,
}

impl<S, C, P> ProxyHttpRequestBody<S, C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    fn new(
        worker: web::Data<ProxyHttp<C, P>>,
        info: ProxyHttpRequestInfo,
        body: S,
        timestamp: UtcDateTime,
    ) -> Self {
        Self {
            proxy: worker,
            info: Some(info),
            stream: body,
            start: timestamp,
            body: Vec::new(), // TODO: preallocate reasonable size
        }
    }
}

impl<S, E, C, P> Stream for ProxyHttpRequestBody<S, C, P>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Debug,
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        match this.stream.poll_next(cx) {
            Poll::Pending => Poll::Pending,

            Poll::Ready(Some(Ok(chunk))) => {
                this.body.extend_from_slice(&chunk);
                Poll::Ready(Some(Ok(chunk)))
            }

            Poll::Ready(Some(Err(err))) => {
                if let Some(info) = mem::take(this.info) {
                    warn!(
                        proxy = P::name(),
                        request_id = %info.id(),
                        connection_id = %info.connection_id(),
                        error = ?err,
                        "Proxy http request stream error",
                    );
                } else {
                    warn!(
                        proxy = P::name(),
                        error = ?err,
                        request_id = "unknown",
                        "Proxy http request stream error",
                    );
                }
                Poll::Ready(Some(Err(err)))
            }

            Poll::Ready(None) => {
                let end = UtcDateTime::now();

                if let Some(info) = mem::take(this.info) {
                    let proxy = this.proxy.clone();

                    let req = ProxiedHttpRequest::new(info, mem::take(this.body), *this.start, end);

                    proxy.postprocess_client_request(req);
                }

                Poll::Ready(None)
            }
        }
    }
}

// ProxyHttpResponseBody -----------------------------------------------

#[pin_project]
struct ProxyHttpResponseBody<S, C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    proxy: web::Data<ProxyHttp<C, P>>,

    info: Option<ProxyHttpResponseInfo>,
    start: UtcDateTime,
    body: Vec<u8>,

    #[pin]
    stream: S,
}

impl<S, C, P> ProxyHttpResponseBody<S, C, P>
where
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    fn new(
        proxy: web::Data<ProxyHttp<C, P>>,
        id: Uuid,
        status: StatusCode,
        headers: HeaderMap,
        body: S,
        timestamp: UtcDateTime,
    ) -> Self {
        Self {
            proxy,
            stream: body,
            start: timestamp,
            body: Vec::new(), // TODO: preallocate reasonable size
            info: Some(ProxyHttpResponseInfo::new(id, status, headers)),
        }
    }
}

impl<S, E, C, P> Stream for ProxyHttpResponseBody<S, C, P>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Debug,
    C: ConfigProxyHttp,
    P: ProxyHttpInner<C>,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        match this.stream.poll_next(cx) {
            Poll::Pending => Poll::Pending,

            Poll::Ready(Some(Ok(chunk))) => {
                this.body.extend_from_slice(&chunk);
                Poll::Ready(Some(Ok(chunk)))
            }

            Poll::Ready(Some(Err(err))) => {
                if let Some(info) = mem::take(this.info) {
                    warn!(
                        proxy = P::name(),
                        request_id = %info.id(),
                        error = ?err,
                        "Proxy http response stream error",
                    );
                } else {
                    warn!(
                        proxy = P::name(),
                        error = ?err,
                        request_id = "unknown",
                        "Proxy http response stream error",
                    );
                }
                Poll::Ready(Some(Err(err)))
            }

            Poll::Ready(None) => {
                let end = UtcDateTime::now();

                if let Some(info) = mem::take(this.info) {
                    let proxy = this.proxy.clone();

                    let res =
                        ProxiedHttpResponse::new(info, mem::take(this.body), *this.start, end);

                    proxy.postprocess_backend_response(res);
                }

                Poll::Ready(None)
            }
        }
    }
}

// ProxiedHttpRequest --------------------------------------------------

#[derive(Clone, actix::Message)]
#[rtype(result = "()")]
pub(crate) struct ProxiedHttpRequest {
    info: ProxyHttpRequestInfo,
    body: Bytes,
    size: usize,
    decompressed_body: Bytes,
    decompressed_size: usize,
    start: UtcDateTime,
    end: UtcDateTime,
}

impl ProxiedHttpRequest {
    pub(crate) fn new(
        info: ProxyHttpRequestInfo,
        body: Vec<u8>,
        start: UtcDateTime,
        end: UtcDateTime,
    ) -> Self {
        let size = body.len();
        Self {
            info,
            body: Bytes::from(body),
            size,
            decompressed_body: Bytes::new(),
            decompressed_size: 0,
            start,
            end,
        }
    }

    #[inline]
    pub(crate) fn info(&self) -> &ProxyHttpRequestInfo {
        &self.info
    }

    #[inline]
    pub(crate) fn start(&self) -> UtcDateTime {
        self.start
    }

    #[inline]
    pub(crate) fn end(&self) -> UtcDateTime {
        self.end
    }
}

// ProxiedHttpResponse -------------------------------------------------

#[derive(Clone, actix::Message)]
#[rtype(result = "()")]
pub(crate) struct ProxiedHttpResponse {
    info: ProxyHttpResponseInfo,
    body: Bytes,
    size: usize,
    decompressed_body: Bytes,
    decompressed_size: usize,
    start: UtcDateTime,
    end: UtcDateTime,
}

impl ProxiedHttpResponse {
    pub(crate) fn new(
        info: ProxyHttpResponseInfo,
        body: Vec<u8>,
        start: UtcDateTime,
        end: UtcDateTime,
    ) -> Self {
        let size = body.len();
        Self {
            info,
            body: Bytes::from(body),
            size,
            decompressed_body: Bytes::new(),
            decompressed_size: 0,
            start,
            end,
        }
    }

    #[inline]
    pub(crate) fn status(&self) -> &str {
        self.info.status.as_str()
    }

    #[inline]
    pub(crate) fn decompressed_body(&self) -> Bytes {
        self.decompressed_body.clone()
    }

    #[inline]
    pub(crate) fn start(&self) -> UtcDateTime {
        self.start
    }

    #[inline]
    pub(crate) fn end(&self) -> UtcDateTime {
        self.end
    }
}

// ProxiedHttpCombo ----------------------------------------------------

#[derive(Clone, actix::Message)]
#[rtype(result = "()")]
struct ProxiedHttpCombo {
    req: ProxiedHttpRequest,
    res: ProxiedHttpResponse,
}
