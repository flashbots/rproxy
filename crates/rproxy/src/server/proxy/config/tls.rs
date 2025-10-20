use std::{
    fs::File,
    io::{Cursor, Read},
    sync::OnceLock,
};

use base64::Engine;
use clap::Args;
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer},
};
use thiserror::Error;

use crate::config::ALREADY_VALIDATED;

static TLS_CERT: OnceLock<Vec<rustls::pki_types::CertificateDer>> = OnceLock::new();
static TLS_KEY: OnceLock<rustls::pki_types::PrivateKeyDer> = OnceLock::new();

// ConfigTls -----------------------------------------------------------

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigTls {
    /// path to tls certificate
    #[arg(
        default_value = "",
        env = "RPROXY_TLS_CERTIFICATE",
        help_heading = "tls",
        long("tls-certificate"),
        name("tls_certificate"),
        value_name = "path"
    )]
    pub(crate) certificate: String,

    /// path to tls key
    #[arg(
        default_value = "",
        env = "RPROXY_TLS_KEY",
        help_heading = "tls",
        long("tls-key"),
        name("tls_key"),
        value_name = "path"
    )]
    pub(crate) key: String,
}

impl ConfigTls {
    pub(crate) fn validate(&self) -> Option<Vec<ConfigTlsError>> {
        let mut errs: Vec<ConfigTlsError> = vec![];

        let mut cert: Option<Vec<CertificateDer>> = None;
        let key: Option<PrivateKeyDer> = None;

        // certificate
        {
            if self.certificate.is_empty() && !self.key.is_empty() {
                errs.push(ConfigTlsError::MissingCertificate);
            }

            if !self.certificate.is_empty() {
                match File::open(self.certificate.clone()) {
                    Err(err) => {
                        errs.push(ConfigTlsError::InvalidCertificateFile {
                            path: self.certificate.clone(),
                            err: err.to_string(),
                        });
                    }

                    Ok(mut file) => {
                        let mut raw = Vec::new();

                        if let Err(err) = file.read_to_end(&mut raw) {
                            errs.push(ConfigTlsError::InvalidCertificate {
                                path: self.certificate.clone(),
                                err: err.to_string(),
                            });
                        } else {
                            if let Ok(decoded) =
                                base64::engine::general_purpose::STANDARD.decode(&raw)
                            {
                                raw = decoded;
                            }

                            let mut reader = Cursor::new(raw);

                            match rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()
                            {
                                Err(err) => {
                                    errs.push(ConfigTlsError::InvalidCertificate {
                                        path: self.certificate.clone(),
                                        err: err.to_string(),
                                    });
                                }

                                Ok(res) => {
                                    cert = Some(res);
                                }
                            }
                        }
                    }
                }
            }
        }

        // key
        {
            if !self.certificate.is_empty() && self.key.is_empty() {
                errs.push(ConfigTlsError::MissingKey);
            }

            if !self.key.is_empty() {
                match File::open(self.key.clone()) {
                    Err(err) => {
                        errs.push(ConfigTlsError::InvalidKeyFile {
                            path: self.key.clone(),
                            err: err.to_string(),
                        });
                    }

                    Ok(mut file) => {
                        let mut raw = Vec::new();

                        if let Err(err) = file.read_to_end(&mut raw) {
                            errs.push(ConfigTlsError::InvalidKey {
                                path: self.certificate.clone(),
                                err: err.to_string(),
                            });
                        } else {
                            if let Ok(decoded) =
                                base64::engine::general_purpose::STANDARD.decode(&raw)
                            {
                                raw = decoded;
                            }

                            let mut reader = Cursor::new(raw);

                            match rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()
                            {
                                Err(err) => {
                                    errs.push(ConfigTlsError::InvalidKey {
                                        path: self.certificate.clone(),
                                        err: err.to_string(),
                                    });
                                }

                                Ok(res) => {
                                    cert = Some(res);
                                }
                            }
                        }
                    }
                }
            }
        }

        // certificate + key
        {
            if let (Some(cert), Some(key)) = (cert, key) &&
                let Err(err) =
                    ServerConfig::builder().with_no_client_auth().with_single_cert(cert, key)
            {
                errs.push(ConfigTlsError::InvalidPair {
                    path_cert: self.certificate.clone(),
                    path_key: self.key.clone(),
                    err: err.to_string(),
                });
            }
        }

        match errs.len() {
            0 => None,
            _ => Some(errs),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        !self.certificate.is_empty() && !self.key.is_empty()
    }

    pub(crate) fn key(&self) -> &PrivateKeyDer<'static> {
        TLS_KEY.get_or_init(|| {
            let mut file = File::open(self.key.clone()).expect(ALREADY_VALIDATED);
            let mut raw = Vec::new();
            file.read_to_end(&mut raw).expect(ALREADY_VALIDATED);

            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&raw) {
                raw = decoded;
            }

            let mut reader = Cursor::new(raw);

            rustls_pemfile::private_key(&mut reader)
                .expect(ALREADY_VALIDATED)
                .expect(ALREADY_VALIDATED)
                .clone_key()
        })
    }

    pub(crate) fn certificate(&self) -> &Vec<rustls::pki_types::CertificateDer<'static>> {
        TLS_CERT.get_or_init(|| {
            let mut file = File::open(self.certificate.clone()).expect(ALREADY_VALIDATED);
            let mut raw = Vec::new();
            file.read_to_end(&mut raw).expect(ALREADY_VALIDATED);

            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&raw) {
                raw = decoded;
            }

            let mut reader = Cursor::new(raw);

            rustls_pemfile::certs(&mut reader)
                .collect::<Result<Vec<_>, _>>()
                .expect(ALREADY_VALIDATED)
                .into_iter()
                .map(|c| c.into_owned())
                .collect::<Vec<_>>()
        })
    }
}

// ConfigTlsError ------------------------------------------------------

#[derive(Debug, Clone, Error)]
pub(crate) enum ConfigTlsError {
    #[error("invalid tls certificate at '{path}': {err}")]
    InvalidCertificate { path: String, err: String },

    #[error("invalid tls certificate file '{path}': {err}")]
    InvalidCertificateFile { path: String, err: String },

    #[error("path to tls certificate is missing")]
    MissingCertificate,

    #[error("invalid tls key at '{path}': {err}")]
    InvalidKey { path: String, err: String },

    #[error("invalid tls key file '{path}': {err}")]
    InvalidKeyFile { path: String, err: String },

    #[error("invalid tls pair '{path_cert}' / '{path_key}': {err}")]
    InvalidPair { path_cert: String, path_key: String, err: String },

    #[error("path to tls key is missing")]
    MissingKey,
}
