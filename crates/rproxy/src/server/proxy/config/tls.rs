use std::{
    fs::File,
    io::{Cursor, Read, Seek, Write},
    sync::OnceLock,
};

use atls::server::NestingAttestedTlsServer;
use base64::Engine;
use clap::Args;
use pkcs8::EncodePrivateKey;
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer},
};
use thiserror::Error;

use crate::config::ALREADY_VALIDATED;

static ATLS_KEY: OnceLock<rcgen::KeyPair> = OnceLock::new();
static TLS_CERT: OnceLock<Vec<rustls::pki_types::CertificateDer>> = OnceLock::new();
static TLS_KEY: OnceLock<rustls::pki_types::PrivateKeyDer> = OnceLock::new();

// ConfigTls -----------------------------------------------------------

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigTls {
    /// tls operation mode
    #[arg(
        default_value = "disabled",
        env = "RPROXY_TLS_MODE",
        help_heading = "tls",
        long("tls-mode"),
        name("tls_mode"),
        value_name = "mode"
    )]
    pub(crate) mode: ConfigTlsMode,

    #[command(flatten)]
    atls: ConfigTlsAttested,

    #[command(flatten)]
    tls: ConfigTlsStandard,
}

impl ConfigTls {
    pub(crate) fn validate(&self) -> Option<Vec<ConfigTlsError>> {
        let mut errs: Vec<ConfigTlsError> = vec![];

        if self.tls_enabled() {
            self.tls.validate(&mut errs);
        }

        if self.atls_enabled() {
            self.atls.validate(&mut errs);
        }

        if errs.is_empty() &&
            self.atls_enabled() &&
            let Err(err) = NestingAttestedTlsServer::new(
                self.tls_key(),
                self.tls_certificate(),
                self.atls_key(),
                atls::server::AttestationGenerator::with_no_attestation(),
                atls::server::AttestationVerifier::expect_none(),
            )
        {
            errs.push(ConfigTlsError::InvalidAtlsConfig { err: err.to_string() });
        }

        (!errs.is_empty()).then_some(errs)
    }

    #[inline]
    pub(crate) fn enabled(&self) -> bool {
        self.tls_enabled() || self.atls_enabled()
    }

    pub(crate) fn atls_enabled(&self) -> bool {
        match self.mode {
            ConfigTlsMode::Disabled | ConfigTlsMode::Standard => false,
            ConfigTlsMode::NestedPostHandshakeAttested => true,
        }
    }

    #[inline]
    pub(crate) fn atls_key(&self) -> &'static rcgen::KeyPair {
        self.atls.atls_key()
    }

    pub(crate) fn tls_enabled(&self) -> bool {
        match self.mode {
            ConfigTlsMode::Disabled => false,
            ConfigTlsMode::Standard | ConfigTlsMode::NestedPostHandshakeAttested => true,
        }
    }

    #[inline]
    pub(crate) fn tls_key(&self) -> &PrivateKeyDer<'static> {
        self.tls.tls_key()
    }

    #[inline]
    pub(crate) fn tls_certificate(&self) -> &Vec<rustls::pki_types::CertificateDer<'static>> {
        self.tls.tls_certificate()
    }
}

// ConfigTlsMode -------------------------------------------------------

#[derive(Clone, Debug, clap::ValueEnum)]
pub(crate) enum ConfigTlsMode {
    Disabled,
    Standard,
    NestedPostHandshakeAttested,
}

// ConfigTlsStandard ---------------------------------------------------

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigTlsStandard {
    /// path to standard tls certificate
    #[arg(
        default_value = "",
        env = "RPROXY_TLS_CERTIFICATE",
        help_heading = "tls",
        long("tls-certificate"),
        name("tls_certificate"),
        value_name = "path"
    )]
    pub(crate) tls_certificate: String,

    /// path to standard tls key
    #[arg(
        default_value = "",
        env = "RPROXY_TLS_KEY",
        help_heading = "tls",
        long("tls-key"),
        name("tls_key"),
        value_name = "path"
    )]
    pub(crate) tls_key: String,
}

impl ConfigTlsStandard {
    pub(crate) fn validate(&self, errs: &mut Vec<ConfigTlsError>) {
        let mut cert: Option<Vec<CertificateDer>> = None;
        let mut key: Option<PrivateKeyDer> = None;

        // certificate
        {
            if self.tls_certificate.is_empty() {
                errs.push(ConfigTlsError::MissingCertificate);
            } else {
                match File::open(self.tls_certificate.clone()) {
                    Err(err) => {
                        errs.push(ConfigTlsError::InvalidTlsCertificateFile {
                            path: self.tls_certificate.clone(),
                            err: err.to_string(),
                        });
                    }

                    Ok(mut file) => {
                        let mut raw = Vec::new();

                        if let Err(err) = file.read_to_end(&mut raw) {
                            errs.push(ConfigTlsError::InvalidTlsCertificateFile {
                                path: self.tls_certificate.clone(),
                                err: err.to_string(),
                            });
                        } else {
                            if let Ok(decoded) =
                                base64::engine::general_purpose::STANDARD.decode(raw.trim_ascii())
                            {
                                raw = decoded;
                            }

                            let mut reader = Cursor::new(raw);

                            match rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()
                            {
                                Err(err) => {
                                    errs.push(ConfigTlsError::InvalidTlsCertificate {
                                        path: self.tls_certificate.clone(),
                                        err: err.to_string(),
                                    });
                                }

                                Ok(res) => {
                                    if res.is_empty() {
                                        errs.push(ConfigTlsError::InvalidTlsCertificate {
                                            path: self.tls_certificate.clone(),
                                            err: String::from("the certificate is missing"),
                                        });
                                    }
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
            if self.tls_key.is_empty() {
                errs.push(ConfigTlsError::MissingTlsKey);
            } else {
                match File::open(self.tls_key.clone()) {
                    Err(err) => {
                        errs.push(ConfigTlsError::InvalidTlsKeyFile {
                            path: self.tls_key.clone(),
                            err: err.to_string(),
                        });
                    }

                    Ok(mut file) => {
                        let mut raw = Vec::new();

                        if let Err(err) = file.read_to_end(&mut raw) {
                            errs.push(ConfigTlsError::InvalidTlsKeyFile {
                                path: self.tls_certificate.clone(),
                                err: err.to_string(),
                            });
                        } else {
                            if let Ok(decoded) =
                                base64::engine::general_purpose::STANDARD.decode(raw.trim_ascii())
                            {
                                raw = decoded;
                            }

                            let mut reader = Cursor::new(raw);

                            match rustls_pemfile::private_key(&mut reader) {
                                Err(err) => {
                                    errs.push(ConfigTlsError::InvalidTlsKey {
                                        path: self.tls_certificate.clone(),
                                        err: err.to_string(),
                                    });
                                }

                                Ok(res) => {
                                    if res.is_none() {
                                        errs.push(ConfigTlsError::InvalidTlsKey {
                                            path: self.tls_certificate.clone(),
                                            err: String::from("the key is missing"),
                                        });
                                    }
                                    key = res;
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
                errs.push(ConfigTlsError::InvalidTlsPair {
                    path_cert: self.tls_certificate.clone(),
                    path_key: self.tls_key.clone(),
                    err: err.to_string(),
                });
            }
        }
    }

    pub(crate) fn tls_key(&self) -> &PrivateKeyDer<'static> {
        TLS_KEY.get_or_init(|| {
            let mut file = File::open(self.tls_key.clone()).expect(ALREADY_VALIDATED);
            let mut raw = Vec::new();
            file.read_to_end(&mut raw).expect(ALREADY_VALIDATED);

            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(raw.trim_ascii())
            {
                raw = decoded;
            }

            let mut reader = Cursor::new(raw);

            rustls_pemfile::private_key(&mut reader)
                .expect(ALREADY_VALIDATED)
                .expect(ALREADY_VALIDATED)
                .clone_key()
        })
    }

    pub(crate) fn tls_certificate(&self) -> &Vec<rustls::pki_types::CertificateDer<'static>> {
        TLS_CERT.get_or_init(|| {
            let mut file = File::open(self.tls_certificate.clone()).expect(ALREADY_VALIDATED);
            let mut raw = Vec::new();
            file.read_to_end(&mut raw).expect(ALREADY_VALIDATED);

            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(raw.trim_ascii())
            {
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

// ConfigTlsAttested ---------------------------------------------------

#[derive(Args, Clone, Debug)]
pub(crate) struct ConfigTlsAttested {
    /// path to atls key
    ///
    /// if missing or empty, new key will be generated and saved at the
    /// specified path
    #[arg(
        default_value = "",
        env = "RPROXY_TLS_ATTESTATION_KEY",
        help_heading = "tls",
        long("tls-attestation-key"),
        name("tls_attestation_key"),
        value_name = "path"
    )]
    pub(crate) tls_attestation_key: String,

    /// url of quote provider for atls
    ///
    /// if unset, the quote is generated by rproxy itself
    #[arg(
        default_value = "",
        env = "RPROXY_TLS_ATTESTATION_QUOTE_PROVIDER",
        help_heading = "tls",
        long("tls-attestation-quote-provider"),
        name("tls_attestation_quote_provider"),
        value_name = "url"
    )]
    pub(crate) tls_attestation_quote_provider: String,
}

impl ConfigTlsAttested {
    pub(crate) fn validate(&self, errs: &mut Vec<ConfigTlsError>) {
        let mut file = match File::open(self.tls_attestation_key.clone()) {
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    errs.push(ConfigTlsError::InvalidAtlsKeyFile {
                        path: self.tls_attestation_key.clone(),
                        err: err.to_string(),
                    });
                    return;
                }

                match File::create(self.tls_attestation_key.clone()) {
                    Err(err) => {
                        errs.push(ConfigTlsError::InvalidAtlsKeyFile {
                            path: self.tls_attestation_key.clone(),
                            err: err.to_string(),
                        });
                        return;
                    }

                    Ok(mut file) => {
                        let mut rng = rand_core::OsRng;
                        let key = ed25519_dalek::SigningKey::generate(&mut rng);

                        let pem = match key.to_pkcs8_pem(pkcs8::LineEnding::LF) {
                            Ok(pem) => pem,

                            Err(err) => {
                                errs.push(ConfigTlsError::FailedToGenerateAtlsKey {
                                    path: self.tls_attestation_key.clone(),
                                    err: err.to_string(),
                                });
                                return;
                            }
                        };

                        if let Err(err) = file.write_all(pem.as_bytes()) {
                            errs.push(ConfigTlsError::FailedToGenerateAtlsKey {
                                path: self.tls_attestation_key.clone(),
                                err: err.to_string(),
                            });
                            return;
                        }

                        if let Err(err) = file.rewind() {
                            errs.push(ConfigTlsError::FailedToGenerateAtlsKey {
                                path: self.tls_attestation_key.clone(),
                                err: err.to_string(),
                            });
                            return;
                        }

                        file
                    }
                }
            }

            Ok(file) => file,
        };

        let mut raw = String::new();
        if let Err(err) = file.read_to_string(&mut raw) {
            errs.push(ConfigTlsError::InvalidAtlsKeyFile {
                path: self.tls_attestation_key.clone(),
                err: err.to_string(),
            });
            return;
        }

        if let Err(err) = rcgen::KeyPair::from_pem(&raw) {
            errs.push(ConfigTlsError::InvalidAtlsKey {
                path: self.tls_attestation_key.clone(),
                err: err.to_string(),
            });
        }
    }

    pub(crate) fn atls_key(&self) -> &'static rcgen::KeyPair {
        ATLS_KEY.get_or_init(|| {
            let mut file = File::open(self.tls_attestation_key.clone()).expect(ALREADY_VALIDATED);
            let mut raw = String::new();
            file.read_to_string(&mut raw).expect(ALREADY_VALIDATED);

            rcgen::KeyPair::from_pem(&raw).expect(ALREADY_VALIDATED)
        })
    }
}

// ConfigTlsError ------------------------------------------------------

#[derive(Debug, Clone, Error)]
pub(crate) enum ConfigTlsError {
    #[error("invalid tls certificate in '{path}': {err}")]
    InvalidTlsCertificate { path: String, err: String },

    #[error("invalid tls certificate file '{path}': {err}")]
    InvalidTlsCertificateFile { path: String, err: String },

    #[error("path to tls certificate is missing")]
    MissingCertificate,

    #[error("invalid tls key in '{path}': {err}")]
    InvalidTlsKey { path: String, err: String },

    #[error("invalid tls key file '{path}': {err}")]
    InvalidTlsKeyFile { path: String, err: String },

    #[error("path to tls key is missing")]
    MissingTlsKey,

    #[error("invalid tls pair '{path_cert}' / '{path_key}': {err}")]
    InvalidTlsPair { path_cert: String, path_key: String, err: String },

    #[error("invalid atls configuration: {err}")]
    InvalidAtlsConfig { err: String },

    #[error("invalid atls key in '{path}': {err}")]
    InvalidAtlsKey { path: String, err: String },

    #[error("invalid atls key file '{path}': {err}")]
    InvalidAtlsKeyFile { path: String, err: String },

    #[error("failed to generate atls key file '{path}': {err}")]
    FailedToGenerateAtlsKey { path: String, err: String },
}
