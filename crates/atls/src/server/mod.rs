use std::{
    ops::{Add, Sub},
    sync::Arc,
};

use parking_lot::RwLock;
use rcgen::{CertificateParams, KeyPair};
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    server::ResolvesServerCert,
    sign::CertifiedKey,
};
use scc::HashMap;
use thiserror::Error;
use time::UtcDateTime;
use tracing::{trace, warn};

mod actix;
pub use actix::*;

mod tls;
pub use tls::*;

// ---------------------------------------------------------------------

const CERTIFICATE_VALIDITY_DURATION_SEC: f64 = 3600.0;

// AtlsServerConfig ----------------------------------------------------

#[derive(Debug)]
pub struct AtlsServerConfig {
    subject_alt_names: Vec<rcgen::SanType>,

    key: &'static KeyPair,

    certs: Arc<RwLock<HashMap<String, Arc<ExpiringCertifiedKey>>>>,
}

impl AtlsServerConfig {
    pub fn new(key: &'static KeyPair, subject_alt_names: Vec<String>) -> Arc<Self> {
        let subject_alt_names =
            subject_alt_names.iter().flat_map(|name| Self::parse_san(name)).collect();

        Arc::new(Self { subject_alt_names, key, certs: Arc::new(RwLock::new(HashMap::new())) })
    }

    pub fn get(self: Arc<Self>) -> ServerConfig {
        ServerConfig::builder().with_no_client_auth().with_cert_resolver(self)
    }

    fn parse_san(s: &str) -> Result<rcgen::SanType, String> {
        if let Ok(ip) = s.parse::<std::net::IpAddr>() {
            return Ok(rcgen::SanType::IpAddress(ip));
        }

        if s.contains("://") {
            return Ok(rcgen::SanType::URI(
                s.try_into().map_err(|err: rcgen::Error| err.to_string())?,
            ));
        }

        if s.contains('@') {
            return Ok(rcgen::SanType::Rfc822Name(
                s.try_into().map_err(|err: rcgen::Error| err.to_string())?,
            ));
        }

        Ok(rcgen::SanType::DnsName(s.try_into().map_err(|err: rcgen::Error| err.to_string())?))
    }

    fn is_allowed(&self, server_name: &str) -> bool {
        let server_name = server_name.to_lowercase();

        for subject_alt_name in self.subject_alt_names.iter() {
            match subject_alt_name {
                rcgen::SanType::DnsName(dns_name) => {
                    if server_name == dns_name.as_str().to_lowercase() {
                        return true;
                    }
                }

                rcgen::SanType::IpAddress(ip_address) => {
                    if server_name == ip_address.to_string() {
                        return true;
                    }
                }

                _ => {}
            }
        }

        false
    }

    fn new_certified_key(
        key: &KeyPair,
        subject_alt_names: Vec<rcgen::SanType>,
    ) -> Result<(CertifiedKey, UtcDateTime), Error> {
        let mut params = CertificateParams::default();

        params.subject_alt_names = subject_alt_names;
        params.is_ca = rcgen::IsCa::NoCa;
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];

        params.not_before = time::OffsetDateTime::now_utc().sub(time::Duration::seconds(60));
        params.not_after =
            params.not_before.add(time::Duration::seconds_f64(CERTIFICATE_VALIDITY_DURATION_SEC));

        let expiry = params.not_after.to_utc();

        // TODO: https://github.com/CCC-Attestation/interoperable-ra-tls/blob/main/docs/Interoperable-RA-TLS-SGX-TDX-evidence-formats.md

        let cert = params.self_signed(key)?;

        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let cert_der = CertificateDer::from(cert.der().to_vec());

        let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der)?;
        let certified_key = CertifiedKey::new(vec![cert_der], signing_key);

        Ok((certified_key, expiry))
    }
}

impl ResolvesServerCert for AtlsServerConfig {
    fn resolve(&self, client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        trace!(client_hello = ?client_hello, "Resolving atls certificate");

        let server_name = client_hello.server_name().unwrap_or_default().to_lowercase();

        if !server_name.is_empty() && !self.is_allowed(&server_name) {
            warn!(server_name = server_name, "Rejecting unrecognised server_name");
            return None;
        }

        let certs = self.certs.read();
        if certs.contains_sync(&server_name) {
            let cert = certs.get_sync(&server_name)?;
            if (UtcDateTime::now() - cert.expiry).as_seconds_f64() >
                CERTIFICATE_VALIDITY_DURATION_SEC / 3.0
            {
                return Some(cert.key.clone());
            }
        }
        drop(certs);

        let certs = self.certs.write();
        let (cert, expiry) = match Self::new_certified_key(self.key, self.subject_alt_names.clone())
        {
            Ok(res) => res,
            Err(err) => {
                warn!(
                    server_name = server_name,
                    err = ?err,
                    "Failed to generate new self-signed certificate",
                );
                return None;
            }
        };

        let cert = Arc::new(ExpiringCertifiedKey { key: Arc::new(cert), expiry });
        if let Err(err) = certs.insert_sync(server_name, cert.clone()) {
            warn!(
                err = ?err,
                "Failed to cache new self-signed certificate",
            );
        };

        Some(cert.key.clone())
    }
}

// CertifiedKey --------------------------------------------------------

#[derive(Debug)]
struct ExpiringCertifiedKey {
    key: Arc<CertifiedKey>,
    expiry: UtcDateTime,
}

// Error ---------------------------------------------------------------

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {err}")]
    IO { err: std::io::Error },

    #[error("certificate error: {err}")]
    RCGen { err: rcgen::Error },

    #[error("certificate error: {err}")]
    Rustls { err: rustls::Error },

    #[error("no error")]
    None,
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::IO { err }
    }
}

impl From<rcgen::Error> for Error {
    fn from(err: rcgen::Error) -> Self {
        Error::RCGen { err }
    }
}

impl From<rustls::Error> for Error {
    fn from(err: rustls::Error) -> Self {
        Error::Rustls { err }
    }
}
