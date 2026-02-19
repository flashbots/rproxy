use std::{
    io,
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{
    TlsAcceptor,
    server::{Accept, TlsStream},
};
use tracing::trace;

// NestingTlsStream ----------------------------------------------------

pub type NestingTlsStream<IO> = TlsStream<TlsStream<IO>>;

// NestingTlsAcceptor --------------------------------------------------

pub struct NestingTlsAcceptor {
    outer: TlsAcceptor,
    inner: TlsAcceptor,
}

impl NestingTlsAcceptor {
    pub fn new(outer: Arc<ServerConfig>, inner: Arc<ServerConfig>) -> Self {
        Self { outer: TlsAcceptor::from(outer), inner: TlsAcceptor::from(inner) }
    }

    #[inline]
    pub fn accept<IO>(&self, stream: IO) -> NestingTlsAccept<IO>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        trace!("Starting outer handshake");
        NestingTlsAccept {
            inner: self.inner.clone(),
            state: NestingAcceptState::Outer(Box::pin(self.outer.accept(stream))),
        }
    }
}

impl From<(Arc<ServerConfig>, Arc<ServerConfig>)> for NestingTlsAcceptor {
    fn from(config: (Arc<ServerConfig>, Arc<ServerConfig>)) -> Self {
        NestingTlsAcceptor::new(config.0, config.1)
    }
}

// NestingAccept -------------------------------------------------------

pub struct NestingTlsAccept<IO> {
    inner: TlsAcceptor,
    state: NestingAcceptState<IO>,
}

impl<IO> Future for NestingTlsAccept<IO>
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
                        trace!("Waiting for outer handshake");
                        // put back what we just mem::replaced
                        self.state = NestingAcceptState::Outer(fut);
                        return Poll::Pending;
                    }

                    Poll::Ready(Err(err)) => {
                        trace!(error = ?err, "Outer handshake failed");
                        // bail out on error
                        return Poll::Ready(Err(err));
                    }

                    Poll::Ready(Ok(outer)) => {
                        trace!("Starting inner handshake");
                        // start inner handshake
                        self.state = NestingAcceptState::Inner(Box::pin(self.inner.accept(outer)));
                        continue;
                    }
                },

                // handle inner handshake
                NestingAcceptState::Inner(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Pending => {
                        trace!("Waiting for inner handshake");
                        // put back what we just mem::replaced
                        self.state = NestingAcceptState::Inner(fut);
                        return Poll::Pending;
                    }

                    Poll::Ready(Err(err)) => {
                        trace!(error = ?err, "Inner handshake failed");
                        // bail out on error
                        return Poll::Ready(Err(err));
                    }

                    Poll::Ready(Ok(inner)) => {
                        trace!("Finished both handshakes");
                        // done
                        return Poll::Ready(Ok(inner));
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
    Outer(Pin<Box<Accept<IO>>>),
    Inner(Pin<Box<Accept<TlsStream<IO>>>>),
    Done,
}
