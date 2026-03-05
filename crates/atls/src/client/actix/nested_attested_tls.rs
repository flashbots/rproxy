#![cfg(feature = "actix")]

// ---------------------------------------------------------------------

use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use actix_http::Uri;
use actix_rt::net::TcpStream;
use actix_service::Service;
use actix_tls::connect::{
    ConnectError,
    ConnectInfo,
    Connection,
    ConnectorService,
    rustls_0_23::webpki_roots_cert_store,
};
use attested_tls_proxy::{
    AttestationGenerator,
    attestation::AttestationVerifier,
    attested_tls::AttestedTlsClient,
};
use rustls::{ClientConfig, pki_types::ServerName};
use tokio_rustls::TlsConnector;

use super::ActixNestingTlsStream;
use crate::client::AtlsServerCertVerifier;

// ActixNestingAttestedTlsConnectorService -----------------------------

#[derive(Clone)]
pub struct ActixNestingAttestedTlsConnectorService {
    tcp: ConnectorService,
    outer: TlsConnector,
    inner: AttestedTlsClient,
}

impl ActixNestingAttestedTlsConnectorService {
    pub fn new(
        attestation_generator: AttestationGenerator,
        attestation_verifier: AttestationVerifier,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let outer = TlsConnector::from(Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(webpki_roots_cert_store())
                .with_no_client_auth(),
        ));

        let verifier = AtlsServerCertVerifier::new()?;

        let client_config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();

        let inner = AttestedTlsClient::new_with_tls_config(
            client_config,
            attestation_generator,
            attestation_verifier,
            None,
        )?;

        Ok(Self { tcp: ConnectorService::default(), outer, inner })
    }
}

impl Service<ConnectInfo<Uri>> for ActixNestingAttestedTlsConnectorService {
    type Response = Connection<Uri, ActixNestingTlsStream<TcpStream>>;
    type Error = ConnectError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + 'static>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        <ConnectorService as Service<ConnectInfo<Uri>>>::poll_ready(&self.tcp, cx)
            .map_err(|err| ConnectError::Resolver(Box::new(err)))
    }

    fn call(&self, req: ConnectInfo<Uri>) -> Self::Future {
        let host = req.hostname().to_string();

        let tcp = <ConnectorService as Service<ConnectInfo<Uri>>>::call(&self.tcp, req);
        let outer = self.outer.clone();
        let inner = self.inner.clone();

        Box::pin(async move {
            let conn = tcp.await.map_err(|err| ConnectError::Resolver(Box::new(err)))?;
            let (tcp_stream, uri) = conn.into_parts();

            let domain = ServerName::try_from(host.clone()).map_err(
                // ServerName::try_from only returns InvalidDnsNameError
                |_| ConnectError::Unresolved,
            )?;

            let outer_stream = outer.connect(domain, tcp_stream).await.map_err(ConnectError::Io)?;

            let (stream, _, _) = inner // TODO: verify measurements
                .connect(&host, outer_stream)
                .await
                .map_err(|err| ConnectError::Io(io::Error::other(err)))?;

            Ok(Connection::new(uri, ActixNestingTlsStream::from(stream)))
        })
    }
}
