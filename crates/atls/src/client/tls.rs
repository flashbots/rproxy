use std::{
    io,
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::client::{Connect, TlsStream};
use tracing::trace;

// NestingTlsStream ----------------------------------------------------

pub type NestingTlsStream<IO> = TlsStream<TlsStream<IO>>;

// NestingTlsConnector -------------------------------------------------

#[derive(Clone)]
pub struct NestingTlsConnector {
    outer: tokio_rustls::TlsConnector,
    inner: tokio_rustls::TlsConnector,
}

impl NestingTlsConnector {
    pub fn new(outer: Arc<rustls::ClientConfig>, inner: Arc<rustls::ClientConfig>) -> Self {
        Self {
            outer: tokio_rustls::TlsConnector::from(outer),
            inner: tokio_rustls::TlsConnector::from(inner),
        }
    }

    #[inline]
    pub fn connect<IO>(
        &self,
        domain: rustls::pki_types::ServerName<'static>,
        stream: IO,
    ) -> NestingConnect<IO>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        trace!(domain = ?domain, "starting outer handshake");
        NestingConnect {
            domain: domain.clone(),
            inner: self.inner.clone(),
            state: NestingConnectState::Outer(Box::pin(self.outer.connect(domain, stream))),
        }
    }
}

// NestingConnect ------------------------------------------------------

pub struct NestingConnect<IO> {
    domain: rustls::pki_types::ServerName<'static>,

    inner: tokio_rustls::TlsConnector,
    state: NestingConnectState<IO>,
}

impl<IO> Future for NestingConnect<IO>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    type Output = io::Result<NestingTlsStream<IO>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match mem::replace(&mut self.state, NestingConnectState::Done) {
                // handle outer handshake
                NestingConnectState::Outer(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Pending => {
                        trace!(domain = ?self.domain, "Waiting for outer handshake");
                        // put back what we just mem::replaced
                        self.state = NestingConnectState::Outer(fut);
                        return Poll::Pending;
                    }

                    Poll::Ready(Err(err)) => {
                        trace!(domain = ?self.domain, error = ?err, "Outer handshake failed");
                        // bail out on error
                        return Poll::Ready(Err(err))
                    }

                    Poll::Ready(Ok(outer)) => {
                        trace!(domain = ?self.domain, "Starting inner handshake");
                        // start inner handshake
                        self.state = NestingConnectState::Inner(Box::pin(
                            self.inner.connect(self.domain.clone(), outer),
                        ));
                        continue;
                    }
                },

                // handle inner handshake
                NestingConnectState::Inner(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Pending => {
                        trace!(domain = ?self.domain, "Waiting for inner handshake");
                        // put back what we just mem::replaced
                        self.state = NestingConnectState::Inner(fut);
                        return Poll::Pending;
                    }

                    Poll::Ready(Err(err)) => {
                        trace!(domain = ?self.domain, error = ?err, "Inner handshake failed");
                        // bail out on error
                        return Poll::Ready(Err(err));
                    }

                    Poll::Ready(Ok(inner)) => {
                        trace!(domain = ?self.domain, "Finished both handshakes");
                        // done
                        return Poll::Ready(Ok(inner))
                    }
                },

                NestingConnectState::Done => {
                    panic!("unexpected polling after handshake")
                }
            }
        }
    }
}

// NestingConnectState -------------------------------------------------

enum NestingConnectState<IO> {
    Outer(Pin<Box<Connect<IO>>>),
    Inner(Pin<Box<Connect<TlsStream<IO>>>>),
    Done,
}
