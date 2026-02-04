use std::{
    fmt,
    future::Future,
    io::{self, IoSlice},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use actix_http::Uri;
use actix_rt::net::{ActixStream, Ready, TcpStream};
use actix_service::Service;
use actix_tls::connect::{ConnectError, ConnectInfo, Connection, ConnectorService};
use tokio::io::{AsyncRead, AsyncWrite};

// ActixNestingTlsStream -----------------------------------------------

pub struct ActixNestingTlsStream<IO>(crate::client::NestingTlsStream<IO>);

impl_more::impl_from!(<IO> in crate::client::NestingTlsStream<IO> => ActixNestingTlsStream<IO>);
impl_more::impl_deref_and_mut!(<IO> in ActixNestingTlsStream<IO> => crate::client::NestingTlsStream<IO> );

impl<IO: fmt::Debug> fmt::Debug for ActixNestingTlsStream<IO> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NestingTlsConnection").finish()
    }
}

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

// NestingTlsConnectorService ------------------------------------------

pub struct ActixNestingTlsConnectorService {
    tcp: ConnectorService,
    tls: crate::client::NestingTlsConnector,
}

impl Clone for ActixNestingTlsConnectorService {
    fn clone(&self) -> Self {
        Self { tcp: self.tcp.clone(), tls: self.tls.clone() }
    }
}

impl ActixNestingTlsConnectorService {
    pub fn new(outer: Arc<rustls::ClientConfig>, inner: Arc<rustls::ClientConfig>) -> Self {
        Self {
            tcp: ConnectorService::default(),
            tls: crate::client::NestingTlsConnector::new(outer, inner),
        }
    }
}

impl Service<ConnectInfo<Uri>> for ActixNestingTlsConnectorService {
    type Response = Connection<Uri, ActixNestingTlsStream<TcpStream>>;
    type Error = ConnectError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + 'static>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        <ConnectorService as Service<ConnectInfo<Uri>>>::poll_ready(&self.tcp, cx)
            .map_err(|e| ConnectError::Resolver(Box::new(e)))
    }

    fn call(&self, req: ConnectInfo<Uri>) -> Self::Future {
        let host = req.hostname().to_string();

        let tcp = <ConnectorService as Service<ConnectInfo<Uri>>>::call(&self.tcp, req);
        let tls = self.tls.clone();

        Box::pin(async move {
            let conn = tcp.await.map_err(|e| ConnectError::Resolver(Box::new(e)))?;

            let (tcp_stream, uri) = conn.into_parts();

            let domain = rustls::pki_types::ServerName::try_from(host)
                .map_err(|_| ConnectError::Unresolved)?;

            let tls_stream = tls.connect(domain, tcp_stream).await.map_err(ConnectError::Io)?;

            Ok(Connection::new(uri, ActixNestingTlsStream(tls_stream)))
        })
    }
}
