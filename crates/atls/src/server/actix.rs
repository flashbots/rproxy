use std::{
    convert::Infallible,
    io::{self, IoSlice},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
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
use tokio::io::{AsyncRead, AsyncWrite};

// ---------------------------------------------------------------------

pub(crate) static MAX_CONN: AtomicUsize = AtomicUsize::new(256);

pub(crate) const DEFAULT_TLS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(3);

thread_local! {
    static MAX_CONN_COUNTER: Counter = Counter::new(MAX_CONN.load(Ordering::Relaxed));
}

// ActixNestingTlsStream -----------------------------------------------

pub struct ActixNestingTlsStream<IO>(crate::server::NestingTlsStream<IO>);

impl_more::impl_from!(<IO> in crate::server::NestingTlsStream<IO> => ActixNestingTlsStream<IO>);
impl_more::impl_deref_and_mut!(<IO> in ActixNestingTlsStream<IO> => crate::server::NestingTlsStream<IO> );

impl<IO: ActixStream> AsyncRead for ActixNestingTlsStream<IO> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
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
            handshake_timeout: DEFAULT_TLS_HANDSHAKE_TIMEOUT,
        }
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
        let res = MAX_CONN_COUNTER.with(|conns| {
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
        fut: crate::server::NestingAccept<IO>,
        #[pin]
        timeout: Sleep,
        _guard: CounterGuard,
    }
}

impl<IO: ActixStream> Future for ActixNestingTlsAcceptFut<IO> {
    type Output = Result<ActixNestingTlsStream<IO>, TlsError<io::Error, Infallible>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        match Pin::new(&mut this.fut).poll(cx) {
            Poll::Ready(Ok(stream)) => Poll::Ready(Ok(ActixNestingTlsStream(stream))),
            Poll::Ready(Err(err)) => Poll::Ready(Err(TlsError::Tls(err))),
            Poll::Pending => this.timeout.poll(cx).map(|_| Err(TlsError::Timeout)),
        }
    }
}
