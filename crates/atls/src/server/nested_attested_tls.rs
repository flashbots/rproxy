use std::{pin::Pin, sync::Arc};

use attested_tls_proxy::{
    AttestationGenerator,
    attestation::AttestationVerifier,
    attested_tls::{AttestedTlsError, AttestedTlsServer},
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tracing::trace;

use crate::utils::self_sign_shadow_cert;

// NestingAttestedTlsAccept --------------------------------------------

pub type NestingAttestedTlsAccept<IO> =
    Pin<Box<dyn Future<Output = Result<TlsStream<TlsStream<IO>>, AttestedTlsError>> + 'static>>;

// NestingAttestedTlsServer --------------------------------------------

#[derive(Clone)]
pub struct NestingAttestedTlsServer {
    outer: TlsAcceptor,
    inner: AttestedTlsServer,
}

impl NestingAttestedTlsServer {
    pub fn new(
        ca_signed_tls_key: &PrivateKeyDer<'static>,
        ca_signed_tls_cert: &Vec<CertificateDer<'static>>,
        attested_tls_key: &rcgen::KeyPair,
        attestation_generator: AttestationGenerator,
        attestation_verifier: AttestationVerifier,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let outer = TlsAcceptor::from(Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(ca_signed_tls_cert.clone(), ca_signed_tls_key.clone_key())?,
        ));

        let attested_tls_key =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(attested_tls_key.serialize_der()));
        let attested_tls_cert = self_sign_shadow_cert(&attested_tls_key, ca_signed_tls_cert)?;

        let inner = AttestedTlsServer::new(
            attested_tls_proxy::attested_tls::TlsCertAndKey {
                key: attested_tls_key,
                cert_chain: attested_tls_cert,
            },
            attestation_generator,
            attestation_verifier,
            false,
        )?;

        Ok(Self { outer, inner })
    }

    pub async fn accept_attesting<IO>(
        &self,
        stream: IO,
    ) -> Result<TlsStream<TlsStream<IO>>, AttestedTlsError>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        trace!("Starting outer handshake");
        let outer = self.outer.accept(stream).await?;

        trace!("Starting inner handshake");
        self.inner.accept(outer).await
    }
}
