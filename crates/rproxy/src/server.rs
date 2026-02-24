pub mod config;
pub(crate) mod metrics;
pub(crate) mod proxy;

// ---------------------------------------------------------------------

use std::{error::Error, sync::Arc};

use tokio::{
    signal::unix::{SignalKind, signal},
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
        let shutdown_signal = Server::wait_for_shutdown_signal();
        let reset_signal = Server::wait_for_reset_signal(shutdown_signal.clone());

        Self::_run(config, shutdown_signal, reset_signal).await
    }

    async fn _run(
        config: Config,
        shutdown_signal: CancellationToken,
        reset_signal: CancellationToken,
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
            let canceller = shutdown_signal.clone();
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
            let shutdown_signal = shutdown_signal.clone();
            let reset_signal = reset_signal.clone();

            let _ = std::thread::spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(err) => {
                        error!(error = ?err, "Failed to initialise a single-threaded runtime for circuit-breaker");
                        std::process::exit(-1);
                    }
                };

                let circuit_breaker = CircuitBreaker::new(config.circuit_breaker.clone());

                tokio::task::LocalSet::new().block_on(&rt, async move {
                    circuit_breaker.run(shutdown_signal, reset_signal).await
                })
            });
        }

        while !shutdown_signal.is_cancelled() {
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
                let canceller = shutdown_signal.clone();
                let reset_signal = reset_signal.clone();

                services.push(tokio::spawn(async move {
                    ProxyHttp::<ConfigAuthrpc, ProxyHttpInnerAuthrpc>::run(
                        config,
                        tls,
                        metrics,
                        canceller.clone(),
                        reset_signal,
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
                let canceller = shutdown_signal.clone();
                let reset_signal = reset_signal.clone();

                services.push(tokio::spawn(async move {
                    ProxyHttp::<ConfigRpc, ProxyHttpInnerRpc>::run(
                        config,
                        tls,
                        metrics,
                        canceller.clone(),
                        reset_signal,
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
                let canceller = shutdown_signal.clone();
                let reset_signal = reset_signal.clone();

                services.push(tokio::spawn(async move {
                    ProxyWs::<ConfigFlashblocks, ProxyWsInnerFlashblocks>::run(
                        config,
                        tls,
                        metrics,
                        canceller.clone(),
                        reset_signal,
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

    fn wait_for_reset_signal(shutdown_signal: CancellationToken) -> CancellationToken {
        let reset_signal = shutdown_signal.child_token();

        tokio::spawn({
            let reset_signal = reset_signal.clone();
            let shutdown_signal = shutdown_signal.clone();
            async move {
                let mut hangup =
                    signal(SignalKind::hangup()).expect("failed to install sighup handler");

                loop {
                    tokio::select! {
                        _ = shutdown_signal.cancelled() => break,
                        _ = hangup.recv() => {
                            info!("Hangup signal received, resetting...");
                            reset_signal.cancel();
                        }
                    }
                }
            }
        });

        reset_signal
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
    use tracing::debug;

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

        let shutdown_signal = CancellationToken::new();
        let reset_signal = Server::wait_for_reset_signal(shutdown_signal.clone());

        let server = {
            let shutdown_signal = shutdown_signal.clone();
            let reset_signal = reset_signal.clone();

            actix_rt::spawn(async move { Server::_run(cfg, shutdown_signal, reset_signal).await })
        };
        actix_rt::time::sleep(std::time::Duration::from_millis(100)).await;

        {
            let shutdown_signal = shutdown_signal.clone();
            let client = Client::builder().timeout(Duration::from_millis(10)).finish();

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

                        _ = shutdown_signal.cancelled() => {
                            break
                        }
                    }
                }
            });
        }

        {
            let shutdown_signal = shutdown_signal.clone();
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

                        _ = shutdown_signal.cancelled() => {
                            break
                        }
                    }
                }
            });
        }

        for _ in 0..10 {
            actix_rt::time::sleep(std::time::Duration::from_millis(1200)).await;
            reset_signal.cancel();
        }

        shutdown_signal.cancel();

        tokio::time::timeout(tokio::time::Duration::from_secs(5), server).await.ok();
    }
}
