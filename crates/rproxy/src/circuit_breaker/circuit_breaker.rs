use std::{sync::Arc, time::Duration};

use awc::{
    Client,
    Connector,
    http::{self, Method, header},
};
use parking_lot::Mutex;
use tokio::sync::broadcast;
use tracing::{debug, error, warn};

use crate::config::ConfigCircuitBreaker;

// CircuitBreakerInner -------------------------------------------------

struct CircuitBreakerInner {
    curr_status: Status,
    last_status: Status,

    streak_length: usize,
}

// CircuitBreaker ------------------------------------------------------

pub(crate) struct CircuitBreaker {
    config: ConfigCircuitBreaker,
    inner: Arc<Mutex<CircuitBreakerInner>>,
    client: Client,
}

impl CircuitBreaker {
    pub(crate) fn new(config: ConfigCircuitBreaker) -> Self {
        let client = Self::client(&config);

        Self {
            config,
            client,
            inner: Arc::new(Mutex::new(CircuitBreakerInner {
                curr_status: Status::Healthy,
                last_status: Status::Healthy,

                streak_length: 0,
            })),
        }
    }

    #[inline]
    pub(crate) fn name() -> &'static str {
        "circuit-breaker"
    }

    #[inline]
    fn timeout(config: &ConfigCircuitBreaker) -> Duration {
        std::cmp::min(Duration::from_secs(5), config.poll_interval * 3 / 4)
    }

    #[inline]
    fn max_threshold(config: &ConfigCircuitBreaker) -> usize {
        std::cmp::max(config.threshold_healthy, config.threshold_unhealthy) + 1
    }

    #[inline]
    fn client(config: &ConfigCircuitBreaker) -> Client {
        let host = config
            .url()
            .host()
            .unwrap() // safety: verified on start
            .to_string();
        let timeout = Self::timeout(config);

        Client::builder()
            .add_default_header((header::HOST, host))
            .connector(Connector::new().timeout(timeout).handshake_timeout(timeout))
            .timeout(timeout)
            .finish()
    }

    pub(crate) async fn run(
        self,
        canceller: tokio_util::sync::CancellationToken,
        resetter: broadcast::Sender<()>,
    ) {
        let canceller = canceller.clone();
        let resetter = resetter.clone();

        let mut ticker = tokio::time::interval(self.config.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        if let Err(err) = tokio::task::spawn_local(async move {
            loop {
                let resetter = resetter.clone();

                tokio::select! {
                    _ = ticker.tick() => {
                        self.poll(resetter).await;
                    }

                    _ = canceller.cancelled() => {
                        break
                    }
                }
            }
        })
        .await
        {
            warn!(service = Self::name(), error = ?err, "Failure while running circuit-breaker");
        }
    }

    async fn poll(&self, resetter: broadcast::Sender<()>) {
        let req = self
            .client
            .request(Method::GET, self.config.url.clone())
            .timeout(Self::timeout(&self.config));

        let status = match req.send().await {
            Ok(res) => match res.status() {
                http::StatusCode::OK => Status::Healthy,
                _ => {
                    debug!(status = %res.status(), "Unexpected backend status");
                    Status::Unhealthy
                }
            },

            Err(err) => {
                debug!(error = ?err, "Failed to poll health-status of the backend");
                Status::Unhealthy
            }
        };

        let mut this = self.inner.lock();

        if this.last_status == status {
            if this.streak_length < Self::max_threshold(&self.config) {
                this.streak_length += 1; // prevent overflow on long healthy runs
            }
        } else {
            this.streak_length = 1
        }
        this.last_status = status;

        match (this.curr_status.clone(), this.last_status.clone()) {
            (Status::Healthy, Status::Healthy) => {}

            (Status::Healthy, Status::Unhealthy) => {
                if this.streak_length < self.config.threshold_unhealthy {
                    return;
                }
                this.curr_status = Status::Unhealthy;

                warn!(service = Self::name(), "Backend became unhealthy, resetting...");

                if let Err(err) = resetter.send(()) {
                    error!(
                        from = Self::name(),
                        error = ?err,
                        "Failed to broadcast reset signal",
                    );
                }
            }

            (Status::Unhealthy, Status::Unhealthy) => {
                warn!(service = Self::name(), "Backend is still unhealthy, resetting...");

                if let Err(err) = resetter.send(()) {
                    error!(
                        from = Self::name(),
                        error = ?err,
                        "Failed to broadcast reset signal",
                    );
                }
            }

            (Status::Unhealthy, Status::Healthy) => {
                if this.streak_length == self.config.threshold_healthy {
                    this.curr_status = Status::Healthy;
                }
            }
        }
    }
}

// Status --------------------------------------------------------------

#[derive(Clone, PartialEq)]
enum Status {
    Healthy,
    Unhealthy,
}
