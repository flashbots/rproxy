#![cfg(feature = "actix")]

// ---------------------------------------------------------------------

use std::{
    convert::Infallible,
    io::{self, IoSlice},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use actix_rt::{
    net::{ActixStream, Ready},
    time::{Sleep, sleep},
};
use actix_service::{Service, ServiceFactory};
use actix_tls::accept::TlsError;
use actix_utils::{
    counter::{Counter, CounterGuard},
    future::{Ready as FutReady, ready},
};
use pin_project_lite::pin_project;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::server::{
    ACTIX_DEFAULT_TLS_HANDSHAKE_TIMEOUT,
    ACTIX_MAX_CONN_COUNTER,
    NestingTlsAccept,
    NestingTlsStream,
};

// ActixNestingTlsStream -----------------------------------------------

pub struct ActixNestingTlsStream<IO>(pub NestingTlsStream<IO>);

impl_more::impl_from!(<IO> in NestingTlsStream<IO> => ActixNestingTlsStream<IO>);
impl_more::impl_deref_and_mut!(<IO> in ActixNestingTlsStream<IO> => NestingTlsStream<IO> );

impl<IO: ActixStream> AsyncRead for ActixNestingTlsStream<IO> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut **self.get_mut()).poll_read(cx, buf)
    }
}

impl<IO: ActixStream> AsyncWrite for ActixNestingTlsStream<IO> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut **self.get_mut()).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut **self.get_mut()).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut **self.get_mut()).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut **self.get_mut()).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        (**self).is_write_vectored()
    }
}

impl<IO: ActixStream> ActixStream for ActixNestingTlsStream<IO> {
    fn poll_read_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        IO::poll_read_ready((**self).get_ref().0.get_ref().0, cx)
    }

    fn poll_write_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        IO::poll_write_ready((**self).get_ref().0.get_ref().0, cx)
    }
}

// ActixNestingTlsAcceptor ---------------------------------------------

#[derive(Clone)]
pub struct ActixNestingTlsAcceptor {
    outer: Arc<rustls::ServerConfig>,
    inner: Arc<rustls::ServerConfig>,
    handshake_timeout: Duration,
}

impl ActixNestingTlsAcceptor {
    pub fn new(outer: rustls::ServerConfig, inner: rustls::ServerConfig) -> Self {
        Self {
            outer: Arc::new(outer),
            inner: Arc::new(inner),
            handshake_timeout: ACTIX_DEFAULT_TLS_HANDSHAKE_TIMEOUT,
        }
    }

    pub fn set_handshake_timeout(&mut self, handshake_timeout: Duration) -> &mut Self {
        self.handshake_timeout = handshake_timeout;
        self
    }
}

impl<IO: ActixStream> ServiceFactory<IO> for ActixNestingTlsAcceptor {
    type Response = ActixNestingTlsStream<IO>;
    type Error = TlsError<io::Error, Infallible>;
    type Config = ();
    type Service = ActixNestingTlsAcceptorService;
    type InitError = ();
    type Future = FutReady<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let res = ACTIX_MAX_CONN_COUNTER.with(|conns| {
            Ok(ActixNestingTlsAcceptorService {
                acceptor: (self.outer.clone(), self.inner.clone()).into(),
                conns: conns.clone(),
                handshake_timeout: self.handshake_timeout,
            })
        });

        ready(res)
    }
}

// ActixNestingTlsAcceptorService --------------------------------------

pub struct ActixNestingTlsAcceptorService {
    acceptor: crate::server::NestingTlsAcceptor,
    conns: Counter,
    handshake_timeout: Duration,
}

impl<IO: ActixStream> Service<IO> for ActixNestingTlsAcceptorService {
    type Response = ActixNestingTlsStream<IO>;
    type Error = TlsError<io::Error, Infallible>;
    type Future = ActixNestingTlsAcceptFut<IO>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(cx) { Poll::Ready(Ok(())) } else { Poll::Pending }
    }

    fn call(&self, req: IO) -> Self::Future {
        ActixNestingTlsAcceptFut {
            fut: self.acceptor.accept(req),
            timeout: sleep(self.handshake_timeout),
            _guard: self.conns.get(),
        }
    }
}

// ActixNestingTlsAcceptFut --------------------------------------------

pin_project! {
    pub struct ActixNestingTlsAcceptFut<IO: ActixStream> {
        #[pin]
        fut: NestingTlsAccept<IO>,
        #[pin]
        timeout: Sleep,
        _guard: CounterGuard,
    }
}

impl<IO: ActixStream> Future for ActixNestingTlsAcceptFut<IO> {
    type Output = Result<ActixNestingTlsStream<IO>, TlsError<io::Error, Infallible>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.fut.poll(cx) {
            Poll::Ready(Ok(stream)) => Poll::Ready(Ok(ActixNestingTlsStream(stream))),
            Poll::Ready(Err(err)) => Poll::Ready(Err(TlsError::Tls(err))),
            Poll::Pending => this.timeout.poll(cx).map(|_| Err(TlsError::Timeout)),
        }
    }
}

// tests ===============================================================

#[cfg(test)]
mod tests {
    use std::{sync::OnceLock, task::Waker};

    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio::io::ReadBuf;

    use super::*;

    #[derive(Default)]
    struct PendingIo;

    impl AsyncRead for PendingIo {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for PendingIo {
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

    fn test_server_config() -> rustls::ServerConfig {
        ensure_crypto_provider_installed();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let key_der =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()));

        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert.cert.der().clone()], key_der)
            .unwrap()
    }

    #[actix_rt::test]
    async fn set_handshake_timeout_propagates_to_service() {
        let mut acceptor = ActixNestingTlsAcceptor::new(test_server_config(), test_server_config());
        let timeout = Duration::from_millis(25);

        acceptor.set_handshake_timeout(timeout);
        let service =
            <ActixNestingTlsAcceptor as ServiceFactory<PendingIo>>::new_service(&acceptor, ())
                .await
                .unwrap();

        assert_eq!(service.handshake_timeout, timeout);
    }

    #[actix_rt::test]
    async fn nested_tls_handshake_times_out_when_stream_never_progresses() {
        let mut acceptor = ActixNestingTlsAcceptor::new(test_server_config(), test_server_config());
        acceptor.set_handshake_timeout(Duration::from_millis(5));
        let service =
            <ActixNestingTlsAcceptor as ServiceFactory<PendingIo>>::new_service(&acceptor, ())
                .await
                .unwrap();

        let result = service.call(PendingIo).await;
        assert!(matches!(result, Err(TlsError::Timeout)));
    }

    #[actix_rt::test]
    async fn poll_ready_reflects_connection_counter_capacity() {
        let service = ActixNestingTlsAcceptorService {
            acceptor: (Arc::new(test_server_config()), Arc::new(test_server_config())).into(),
            conns: Counter::new(1),
            handshake_timeout: Duration::from_millis(50),
        };

        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert!(matches!(
            <ActixNestingTlsAcceptorService as Service<PendingIo>>::poll_ready(&service, &mut cx),
            Poll::Ready(Ok(()))
        ));
        let fut = service.call(PendingIo);
        assert!(matches!(
            <ActixNestingTlsAcceptorService as Service<PendingIo>>::poll_ready(&service, &mut cx),
            Poll::Pending
        ));
        drop(fut);
        assert!(matches!(
            <ActixNestingTlsAcceptorService as Service<PendingIo>>::poll_ready(&service, &mut cx),
            Poll::Ready(Ok(()))
        ));
    }
}
