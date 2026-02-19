use std::sync::Arc;

use rustls::{
    DistinguishedName,
    SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::VerifierBuilderError,
};
use thiserror::Error;
use tracing::debug;
use x509_certificate::certificate::X509Certificate;

// AtlsServerCertVerifier ----------------------------------------------

#[derive(Debug)]
pub struct AtlsServerCertVerifier {
    inner: Arc<WebPkiServerVerifier>,
}

impl AtlsServerCertVerifier {
    pub fn new() -> Result<Self, AtlsServerCertVerifierError> {
        let roots = {
            let mut root_certs = tokio_rustls::rustls::RootCertStore::empty();
            root_certs.extend(webpki_roots::TLS_SERVER_ROOTS.to_owned());
            Arc::new(root_certs)
        };

        let inner = WebPkiServerVerifier::builder(roots).build()?;

        Ok(Self { inner })
    }
}

impl ServerCertVerifier for AtlsServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer)) => {
                // handle self-signed certs differently
            }
            Err(err) => return Err(err),
            Ok(res) => return Ok(res),
        };

        let cert = X509Certificate::from_der(end_entity.as_ref())
            .map_err(|err| rustls::Error::General(format!("invalid cert der: {err}")))?;

        let pubkey =
            &cert.as_ref().tbs_certificate.subject_public_key_info.subject_public_key.octet_bytes(); // TODO: verify against allow-list

        debug!(pubkey = hex::encode(pubkey), "Allowing self-signed certificate from the server");

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        self.inner.root_hint_subjects()
    }
}

// AtlsServerCertVerifierError -----------------------------------------

#[derive(Debug, Error)]
pub enum AtlsServerCertVerifierError {
    #[error("inner verifier builder error: {0}")]
    VerifierBuilderError(#[from] VerifierBuilderError),
}
