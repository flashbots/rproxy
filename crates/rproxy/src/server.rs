pub mod config;
pub(crate) mod metrics;
pub(crate) mod proxy;

// ---------------------------------------------------------------------

use std::{error::Error, sync::Arc};

use tokio::{
    signal::unix::{SignalKind, signal},
    sync::broadcast,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    config::Config,
    server::{
        metrics::Metrics,
        proxy::{
            circuit_breaker::CircuitBreaker,
            config::{ConfigAuthrpc, ConfigFlashblocks, ConfigRpc},
            http::{ProxyHttp, ProxyHttpInner, ProxyHttpInnerAuthrpc, ProxyHttpInnerRpc},
            ws::{ProxyWs, ProxyWsInner, ProxyWsInnerFlashblocks},
        },
    },
    utils::tls_certificate_validity_timestamps,
};

// Proxy ---------------------------------------------------------------

pub struct Server {}

impl Server {
    pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error + Send>> {
        let canceller = Server::wait_for_shutdown_signal();
        let resetter = Server::wait_for_reset_signal(canceller.clone());

        Self::_run(config, canceller, resetter).await
    }

    async fn _run(
        config: Config,
        canceller: CancellationToken,
        resetter: broadcast::Sender<()>,
    ) -> Result<(), Box<dyn std::error::Error + Send>> {
        // try to set system limits
        match rlimit::getrlimit(rlimit::Resource::NOFILE) {
            Ok((_, hard)) => {
                // raise soft limit to the max
                if let Err(err) = rlimit::setrlimit(rlimit::Resource::NOFILE, hard, hard) {
                    warn!(
                        error = ?err,
                        hard = hard,
                        soft = hard,
                        "Failed to set max open file limits",
                    );
                }
            }

            Err(err) => {
                warn!(
                    error = ?err,
                    "Failed to read max open file limits",
                );
            }
        }

        // spawn metrics service
        let metrics = Arc::new(Metrics::new(config.metrics.clone()));
        {
            let canceller = canceller.clone();
            let metrics = metrics.clone();

            tokio::spawn(async move {
                metrics.run(canceller).await.inspect_err(|err| {
                    error!(
                        service = Metrics::name(),
                        error = ?err,
                        "Failed to start metrics service",
                    );
                    std::process::exit(-1);
                })
            });
        }

        // spawn circuit-breaker
        if !config.circuit_breaker.url.is_empty() {
            let canceller = canceller.clone();
            let resetter = resetter.clone();

            let _ = std::thread::spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(err) => {
                        error!(error = ?err, "Failed to initialise a single-threaded runtime for circuit-breaker");
                        std::process::exit(-1);
                    }
                };

                let circuit_breaker = CircuitBreaker::new(config.circuit_breaker.clone());

                tokio::task::LocalSet::new()
                    .block_on(&rt, async move { circuit_breaker.run(canceller, resetter).await })
            });
        }

        while !canceller.is_cancelled() {
            if config.tls.enabled() {
                let metrics = metrics.clone();
                let (not_before, not_after) =
                    tls_certificate_validity_timestamps(config.tls.certificate());
                metrics.tls_certificate_valid_not_before.set(not_before);
                metrics.tls_certificate_valid_not_after.set(not_after);
            }

            let mut services: Vec<JoinHandle<Result<(), Box<dyn Error + Send>>>> = Vec::new();

            // spawn authrpc proxy
            if config.authrpc.enabled {
                let tls = config.tls.clone();
                let config = config.authrpc.clone();
                let metrics = metrics.clone();
                let canceller = canceller.clone();
                let resetter = resetter.clone();

                services.push(tokio::spawn(async move {
                    ProxyHttp::<ConfigAuthrpc, ProxyHttpInnerAuthrpc>::run(
                        config,
                        tls,
                        metrics,
                        canceller.clone(),
                        resetter,
                    )
                    .await
                    .inspect_err(|err| {
                        error!(
                            proxy = ProxyHttpInnerAuthrpc::name(),
                            error = ?err,
                            "Failed to start http-proxy, terminating...",
                        );
                        canceller.cancel();
                    })
                }));
            }

            // spawn rpc proxy
            if config.rpc.enabled {
                let tls = config.tls.clone();
                let config = config.rpc.clone();
                let metrics = metrics.clone();
                let canceller = canceller.clone();
                let resetter = resetter.clone();

                services.push(tokio::spawn(async move {
                    ProxyHttp::<ConfigRpc, ProxyHttpInnerRpc>::run(
                        config,
                        tls,
                        metrics,
                        canceller.clone(),
                        resetter,
                    )
                    .await
                    .inspect_err(|err| {
                        error!(
                            proxy = ProxyHttpInnerRpc::name(),
                            error = ?err,
                            "Failed to start http-proxy, terminating...",
                        );
                        canceller.cancel();
                    })
                }));
            }

            // spawn flashblocks proxy
            if config.flashblocks.enabled {
                let tls = config.tls.clone();
                let config = config.flashblocks.clone();
                let metrics = metrics.clone();
                let canceller = canceller.clone();
                let resetter = resetter.clone();

                services.push(tokio::spawn(async move {
                    ProxyWs::<ConfigFlashblocks, ProxyWsInnerFlashblocks>::run(
                        config,
                        tls,
                        metrics,
                        canceller.clone(),
                        resetter,
                    )
                    .await
                    .inspect_err(|err| {
                        error!(
                            proxy = ProxyWsInnerFlashblocks::name(),
                            error = ?err,
                            "Failed to start websocket-proxy, terminating...",
                        );
                        canceller.cancel();
                    })
                }));
            }

            for res in futures::future::join_all(services).await.iter() {
                if let Err(err) = res {
                    warn!(error = ?err, "One of the services had failed")
                }
            }
        }

        Ok(())
    }

    fn wait_for_shutdown_signal() -> CancellationToken {
        let canceller = tokio_util::sync::CancellationToken::new();

        {
            let canceller = canceller.clone();

            tokio::spawn(async move {
                let sigint = async {
                    signal(SignalKind::interrupt())
                        .expect("failed to install sigint handler")
                        .recv()
                        .await;
                };

                let sigterm = async {
                    signal(SignalKind::terminate())
                        .expect("failed to install sigterm handler")
                        .recv()
                        .await;
                };

                tokio::select! {
                    _ = sigint => {},
                    _ = sigterm => {},
                }

                info!("Shutdown signal received, stopping...");

                canceller.cancel();
            });
        }

        canceller
    }

    fn wait_for_reset_signal(canceller: CancellationToken) -> broadcast::Sender<()> {
        let (resetter, _) = broadcast::channel::<()>(1);

        {
            let resetter = resetter.clone();

            tokio::spawn(async move {
                let mut hangup =
                    signal(SignalKind::hangup()).expect("failed to install sighup handler");

                loop {
                    tokio::select! {
                        _ = hangup.recv() => {
                            info!("Hangup signal received, resetting...");

                            if let Err(err) = resetter.send(()) {
                                error!(from = "sighup", error = ?err, "Failed to broadcast reset signal, shutting down whole proxy...");
                                canceller.cancel();
                            }
                        }

                        _ = canceller.cancelled() => {
                            return
                        },
                    }
                }
            });
        }

        resetter
    }
}

// tests ===============================================================

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Duration};

    use awc::{Client, http::header};
    use clap::Parser;
    use jsonrpsee::{
        RpcModule,
        server::{ServerBuilder, ServerHandle},
    };
    use tracing::{debug, info};

    use super::*;
    use crate::config::Config;

    async fn spawn_rpc_backend() -> (SocketAddr, ServerHandle) {
        let server = ServerBuilder::default().build("127.0.0.1:0").await.unwrap();

        let addr: SocketAddr = server.local_addr().unwrap();

        let mut module = RpcModule::new(());

        module
            .register_async_method("eth_chainId", |_params, _ctx, _ext| async move {
                Ok::<_, jsonrpsee::types::ErrorObjectOwned>("0x1")
            })
            .unwrap();

        let handle = server.start(module);

        (addr, handle)
    }

    #[actix_web::test]
    async fn test_circuit_breaker() {
        let (backend, _handle) = spawn_rpc_backend().await;

        let cfg = {
            let mut cfg = Config::parse_from(["rproxy"]);

            cfg.authrpc.enabled = true;
            cfg.authrpc.backend_url = format!("http://{backend}");
            cfg.authrpc.listen_address = "127.0.0.1:18645".into();
            cfg.authrpc.shutdown_timeout_sec = 1;

            cfg.rpc.enabled = true;
            cfg.rpc.backend_url = format!("http://{backend}");
            cfg.rpc.listen_address = "127.0.0.1:18651".into();
            cfg.rpc.shutdown_timeout_sec = 1;

            cfg.logging.level = "warn,rproxy::server::tests=info".into();
            cfg.logging.setup_logging();

            cfg
        };

        let proxy_addr_authrpc = cfg.clone().authrpc.listen_address;
        let proxy_addr_rpc = cfg.clone().rpc.listen_address;

        let canceller = tokio_util::sync::CancellationToken::new();
        let resetter = Server::wait_for_reset_signal(canceller.clone());

        let server = {
            let canceller = canceller.clone();
            let resetter = resetter.clone();

            actix_rt::spawn(async move { Server::_run(cfg, canceller, resetter).await })
        };
        actix_rt::time::sleep(std::time::Duration::from_millis(100)).await;

        {
            let canceller = canceller.clone();
            let client = Client::builder().timeout(Duration::from_millis(10)).finish();
            let proxy_addr_authrpc = proxy_addr_authrpc.clone();

            actix_rt::spawn(async move {
                loop {
                    actix_rt::time::sleep(std::time::Duration::from_millis(10)).await;

                    let req = client
                        .post(format!("http://{proxy_addr_authrpc}"))
                        .insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
                        .send_body(
                            r#"{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}"#,
                        );

                    tokio::select! {
                        res = req => {
                            match res {
                                Ok(mut res) => {
                                    let _ = res.body().await;
                                }

                                Err(err) => {
                                    debug!(error = ?err, "Failed to send a request");
                                }
                            }
                        }

                        _ = canceller.cancelled() => {
                            break
                        }
                    }
                }
            });
        }

        {
            let canceller = canceller.clone();
            let client = Client::builder().timeout(Duration::from_millis(10)).finish();

            actix_rt::spawn(async move {
                loop {
                    actix_rt::time::sleep(std::time::Duration::from_millis(10)).await;

                    let req = client
                        .post(format!("http://{proxy_addr_rpc}"))
                        .insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
                        .send_body(
                            r#"{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}"#,
                        );

                    tokio::select! {
                        res = req => {
                            match res {
                                Ok(mut res) => {
                                    let _ = res.body().await;
                                }

                                Err(err) => {
                                    debug!(error = ?err, "Failed to send a request");
                                }
                            }
                        }

                        _ = canceller.cancelled() => {
                            break
                        }
                    }
                }
            });
        }

        let client = Client::builder().timeout(Duration::from_millis(10)).finish();

        for i in 0..10 {
            match resetter.send(()) {
                Err(err) => {
                    debug!(iteration = i, error = ?err, "Failed to send a reset");
                }

                Ok(proxies_count) => {
                    info!(iteration = i, proxies_count = proxies_count, "Sent a reset");
                    assert_eq!(
                        proxies_count, 2,
                        "sent reset wrong count of proxies: {proxies_count} != 2"
                    );
                }
            }

            actix_rt::time::sleep(std::time::Duration::from_millis(1200)).await;

            let req = client
                .post(format!("http://{proxy_addr_authrpc}"))
                .insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
                .send_body(r#"{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}"#);

            tokio::select! {
                res = req => {
                    match res {
                        Ok(mut res) => {
                            match res.body().await {
                                Err(err) => {
                                    panic!("Failed to send a request: {err}");
                                }
                                Ok(body) => {
                                    let body = String::from_utf8_lossy(&body).to_string();
                                    info!("Sent a request and got a response: {body}");
                                }
                            }
                        }

                        Err(err) => {
                            panic!("Failed to send a request: {err}");
                        }
                    }
                }

                _ = canceller.cancelled() => {
                    break
                }
            }
        }

        canceller.cancel();

        tokio::time::timeout(tokio::time::Duration::from_secs(5), server).await.ok();
    }
}
