#![cfg(feature = "actix")]

// ---------------------------------------------------------------------

use std::{
    convert::Infallible,
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use actix_rt::{
    net::ActixStream,
    time::{Sleep, sleep},
};
use actix_service::{Service, ServiceFactory};
use actix_tls::accept::TlsError;
use actix_utils::{
    counter::{Counter, CounterGuard},
    future::{Ready as FutReady, ready},
};
use attested_tls_proxy::attested_tls::AttestedTlsError;
use pin_project_lite::pin_project;

use crate::server::{
    ACTIX_DEFAULT_TLS_HANDSHAKE_TIMEOUT,
    ACTIX_MAX_CONN_COUNTER,
    ActixNestingTlsStream,
    NestingAttestedTlsAccept,
    NestingAttestedTlsServer,
};

// ActixNestingAttestedTlsAcceptor -------------------------------------

#[derive(Clone)]
pub struct ActixNestingAttestedTlsAcceptor {
    server: Arc<NestingAttestedTlsServer>,
    handshake_timeout: Duration,
}

impl ActixNestingAttestedTlsAcceptor {
    pub fn new(server: NestingAttestedTlsServer) -> Self {
        Self { server: Arc::new(server), handshake_timeout: ACTIX_DEFAULT_TLS_HANDSHAKE_TIMEOUT }
    }

    pub fn set_handshake_timeout(&mut self, handshake_timeout: Duration) -> &mut Self {
        self.handshake_timeout = handshake_timeout;
        self
    }
}

impl From<NestingAttestedTlsServer> for ActixNestingAttestedTlsAcceptor {
    fn from(server: NestingAttestedTlsServer) -> Self {
        Self::new(server)
    }
}

impl<IO: ActixStream + 'static> ServiceFactory<IO> for ActixNestingAttestedTlsAcceptor {
    type Response = ActixNestingTlsStream<IO>;
    type Error = TlsError<io::Error, Infallible>;
    type Config = ();
    type Service = ActixNestingAttestedTlsAcceptorService;
    type InitError = ();
    type Future = FutReady<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let res = ACTIX_MAX_CONN_COUNTER.with(|conns| {
            Ok(ActixNestingAttestedTlsAcceptorService {
                server: self.server.clone(),
                conns: conns.clone(),
                handshake_timeout: self.handshake_timeout,
            })
        });
        ready(res)
    }
}

// ActixNestingAttestedTlsAcceptorService ------------------------------

pub struct ActixNestingAttestedTlsAcceptorService {
    server: Arc<NestingAttestedTlsServer>,
    conns: Counter,
    handshake_timeout: Duration,
}

impl<IO: ActixStream + 'static> Service<IO> for ActixNestingAttestedTlsAcceptorService {
    type Response = ActixNestingTlsStream<IO>;
    type Error = TlsError<io::Error, Infallible>;
    type Future = ActixNestingAttestedTlsAcceptFut<IO>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(cx) { Poll::Ready(Ok(())) } else { Poll::Pending }
    }

    fn call(&self, req: IO) -> Self::Future {
        let server = self.server.clone();
        ActixNestingAttestedTlsAcceptFut {
            fut: Box::pin(async move { server.accept_attesting(req).await }),
            timeout: sleep(self.handshake_timeout),
            _guard: self.conns.get(),
        }
    }
}

pin_project! {
    pub struct ActixNestingAttestedTlsAcceptFut<IO: ActixStream> {
        fut: NestingAttestedTlsAccept<IO>,
        #[pin]
        timeout: Sleep,
        _guard: CounterGuard,
    }
}

impl<IO: ActixStream> Future for ActixNestingAttestedTlsAcceptFut<IO> {
    type Output = Result<ActixNestingTlsStream<IO>, TlsError<io::Error, Infallible>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.fut.as_mut().poll(cx) {
            Poll::Ready(Ok(stream)) => Poll::Ready(Ok(ActixNestingTlsStream(stream))),
            Poll::Ready(Err(err)) => Poll::Ready(Err(match err {
                AttestedTlsError::Io(err) => TlsError::Tls(err),
                err => TlsError::Tls(io::Error::other(err)),
            })),
            Poll::Pending => this.timeout.poll(cx).map(|_| Err(TlsError::Timeout)),
        }
    }
}

// tests ===============================================================

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use actix_rt::net::Ready;
    use actix_tls::accept::TlsError;
    use attested_tls_proxy::{AttestationGenerator, attestation::AttestationVerifier};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    use super::*;

    #[derive(Default)]
    struct PendingIo;

    impl tokio::io::AsyncRead for PendingIo {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl tokio::io::AsyncWrite for PendingIo {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Pending
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl ActixStream for PendingIo {
        fn poll_read_ready(&self, _cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            Poll::Pending
        }

        fn poll_write_ready(&self, _cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
            Poll::Pending
        }
    }

    fn ensure_crypto_provider_installed() {
        static PROVIDER: OnceLock<()> = OnceLock::new();
        PROVIDER.get_or_init(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
    }

    fn ca_cert_and_key() -> (PrivateKeyDer<'static>, Vec<CertificateDer<'static>>, Vec<u8>) {
        ensure_crypto_provider_installed();
        let signing_key = rcgen::KeyPair::generate().unwrap();
        let key_der = signing_key.serialize_der();
        let cert = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
            .unwrap()
            .self_signed(&signing_key)
            .unwrap();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der.clone()));
        let cert_chain = vec![cert.der().clone()];
        (key, cert_chain, key_der)
    }

    fn test_server() -> NestingAttestedTlsServer {
        let (ca_key, ca_cert, key_der) = ca_cert_and_key();
        let attested_tls_key = rcgen::KeyPair::try_from(key_der.as_slice()).unwrap();
        let attestation_generator = AttestationGenerator::with_no_attestation();
        let attestation_verifier = AttestationVerifier::expect_none();

        NestingAttestedTlsServer::new(
            &ca_key,
            &ca_cert,
            &attested_tls_key,
            attestation_generator,
            attestation_verifier,
        )
        .unwrap()
    }

    #[actix_rt::test]
    async fn set_handshake_timeout_propagates_to_service() {
        let mut acceptor = ActixNestingAttestedTlsAcceptor::new(test_server());
        let timeout = Duration::from_millis(25);
        acceptor.set_handshake_timeout(timeout);

        let service = <ActixNestingAttestedTlsAcceptor as ServiceFactory<PendingIo>>::new_service(
            &acceptor,
            (),
        )
        .await
        .unwrap();

        assert_eq!(service.handshake_timeout, timeout);
    }

    #[actix_rt::test]
    async fn nested_attested_tls_handshake_times_out_when_stream_never_progresses() {
        let mut acceptor = ActixNestingAttestedTlsAcceptor::new(test_server());
        acceptor.set_handshake_timeout(Duration::from_millis(5));
        let service = <ActixNestingAttestedTlsAcceptor as ServiceFactory<PendingIo>>::new_service(
            &acceptor,
            (),
        )
        .await
        .unwrap();

        let result = service.call(PendingIo).await;
        assert!(matches!(result, Err(TlsError::Timeout)));
    }

    #[actix_rt::test]
    async fn poll_ready_reflects_connection_counter_capacity() {
        let service = ActixNestingAttestedTlsAcceptorService {
            server: Arc::new(test_server()),
            conns: Counter::new(1),
            handshake_timeout: Duration::from_millis(50),
        };
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert!(matches!(
            <ActixNestingAttestedTlsAcceptorService as Service<PendingIo>>::poll_ready(
                &service, &mut cx
            ),
            Poll::Ready(Ok(()))
        ));
        let fut = service.call(PendingIo);
        assert!(matches!(
            <ActixNestingAttestedTlsAcceptorService as Service<PendingIo>>::poll_ready(
                &service, &mut cx
            ),
            Poll::Pending
        ));
        drop(fut);
        assert!(matches!(
            <ActixNestingAttestedTlsAcceptorService as Service<PendingIo>>::poll_ready(
                &service, &mut cx
            ),
            Poll::Ready(Ok(()))
        ));
    }
}
