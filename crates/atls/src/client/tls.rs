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
    tokio_rustls::client::TlsStream<tokio_rustls::client::TlsStream<IO>>;

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
                        // put back what we just mem::replaced
                        self.state = NestingConnectState::Outer(fut);
                        return Poll::Pending;
                    }

                    Poll::Ready(Ok(outer)) => {
                        // start inner handshake
                        self.state = NestingConnectState::Inner(Box::pin(
                            self.inner.connect(self.domain.clone(), outer),
                        ));
                        continue;
                    }

                    Poll::Ready(Err(err)) => {
                        // bail out on error
                        return Poll::Ready(Err(err))
                    }
                },

                // handle inner handshake
                NestingConnectState::Inner(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Pending => {
                        // put back what we just mem::replaced
                        self.state = NestingConnectState::Inner(fut);
                        return Poll::Pending;
                    }

                    Poll::Ready(res) => {
                        // done
                        return Poll::Ready(res)
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
    Outer(Pin<Box<tokio_rustls::client::Connect<IO>>>),
    Inner(Pin<Box<tokio_rustls::client::Connect<tokio_rustls::client::TlsStream<IO>>>>),
    Done,
}
