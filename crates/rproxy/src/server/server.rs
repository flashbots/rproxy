use std::{error::Error, sync::Arc};

use tokio::{
    signal::unix::{SignalKind, signal},
    sync::broadcast,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    circuit_breaker::CircuitBreaker,
    config::{Config, ConfigAuthrpc, ConfigFlashblocks, ConfigRpc},
    metrics::Metrics,
    proxy::ProxyInner,
    proxy_http::{ProxyHttp, ProxyHttpInnerAuthrpc, ProxyHttpInnerRpc},
    proxy_ws::{ProxyWs, ProxyWsInnerFlashblocks},
    utils::tls_certificate_validity_timestamps,
};

// Proxy ---------------------------------------------------------------

pub struct Server {}

impl Server {
    pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error + Send>> {
        let canceller = Server::wait_for_shutdown_signal();
        let resetter = Server::wait_for_reset_signal(canceller.clone());

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
        if config.circuit_breaker.url != "" {
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
                    let res = ProxyHttp::<ConfigAuthrpc, ProxyHttpInnerAuthrpc>::run(
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
                    });
                    res
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
                    let res = ProxyHttp::<ConfigRpc, ProxyHttpInnerRpc>::run(
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
                    });
                    res
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
                    let res = ProxyWs::<ConfigFlashblocks, ProxyWsInnerFlashblocks>::run(
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
                            "Failed to start websocket-proxy, terminating...",
                        );
                        canceller.cancel();
                    });
                    res
                }));
            }

            futures::future::join_all(services).await;
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
        let (resetter, _) = broadcast::channel::<()>(2);

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
                                error!(from = "sighup", error = ?err, "Failed to broadcast reset signal");
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
