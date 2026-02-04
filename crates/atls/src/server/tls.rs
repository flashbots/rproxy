use std::{
    io,
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite};

// NestingTlsStream ----------------------------------------------------

pub type NestingTlsStream<IO> =
    tokio_rustls::server::TlsStream<tokio_rustls::server::TlsStream<IO>>;

// NestingTlsAcceptor --------------------------------------------------

pub struct NestingTlsAcceptor {
    outer: tokio_rustls::TlsAcceptor,
    inner: tokio_rustls::TlsAcceptor,
}

impl NestingTlsAcceptor {
    pub fn new(outer: Arc<rustls::ServerConfig>, inner: Arc<rustls::ServerConfig>) -> Self {
        Self {
            outer: tokio_rustls::TlsAcceptor::from(outer),
            inner: tokio_rustls::TlsAcceptor::from(inner),
        }
    }

    #[inline]
    pub fn accept<IO>(&self, stream: IO) -> NestingAccept<IO>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        NestingAccept {
            inner: self.inner.clone(),
            state: NestingAcceptState::Outer(Box::pin(self.outer.accept(stream))),
        }
    }
}

impl From<(Arc<rustls::ServerConfig>, Arc<rustls::ServerConfig>)> for NestingTlsAcceptor {
    fn from(config: (Arc<rustls::ServerConfig>, Arc<rustls::ServerConfig>)) -> Self {
        NestingTlsAcceptor::new(config.0, config.1)
    }
}

// NestingAccept -------------------------------------------------------

pub struct NestingAccept<IO> {
    inner: tokio_rustls::TlsAcceptor,
    state: NestingAcceptState<IO>,
}

impl<IO> Future for NestingAccept<IO>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    type Output = io::Result<NestingTlsStream<IO>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match mem::replace(&mut self.state, NestingAcceptState::Done) {
                // handle outer handshake
                NestingAcceptState::Outer(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Pending => {
                        // put back what we just mem::replaced
                        self.state = NestingAcceptState::Outer(fut);
                        return Poll::Pending;
                    }

                    Poll::Ready(Ok(outer)) => {
                        // start inner handshake
                        self.state = NestingAcceptState::Inner(Box::pin(self.inner.accept(outer)));
                        continue;
                    }

                    Poll::Ready(Err(err)) => {
                        // bail out on error
                        return Poll::Ready(Err(err));
                    }
                },

                // handle inner handshake
                NestingAcceptState::Inner(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Pending => {
                        // put back what we just mem::replaced
                        self.state = NestingAcceptState::Inner(fut);
                        return Poll::Pending;
                    }

                    Poll::Ready(res) => {
                        // done
                        return Poll::Ready(res);
                    }
                },

                NestingAcceptState::Done => {
                    panic!("unexpected polling after handshake")
                }
            }
        }
    }
}

// NestingAcceptState --------------------------------------------------

enum NestingAcceptState<IO> {
    Outer(Pin<Box<tokio_rustls::server::Accept<IO>>>),
    Inner(Pin<Box<tokio_rustls::server::Accept<tokio_rustls::server::TlsStream<IO>>>>),
    Done,
}
