use bcder::{Captured, Mode, decode::Constructed, encode::Values, string::BitString};
use bytes::Bytes;
use rustls::{
    crypto::aws_lc_rs::sign,
    pki_types::{CertificateDer, PrivateKeyDer},
};
use signature::Signer;
use tracing::debug;
use x509_certificate::{
    KeyAlgorithm,
    Sign,
    certificate::X509Certificate,
    rfc5280,
    signing::InMemorySigningKeyPair,
};

// self_sign_shadow_cert -----------------------------------------------

/// self_sign_shadow_cert creates a self-signed certificate that preserves
/// most of the original certificate fields (validity, subject/CN, SANs,
/// extensions, and so on)
pub(crate) fn self_sign_shadow_cert(
    new_key: &PrivateKeyDer<'_>,
    orig_cert: &Vec<CertificateDer<'_>>,
) -> Result<Vec<CertificateDer<'static>>, Box<dyn std::error::Error>> {
    let PrivateKeyDer::Pkcs8(new_key) = new_key else {
        return Err("only pkcs#8 private keys are supported".into());
    };

    let orig_cert = X509Certificate::from_der(
        orig_cert.first().ok_or("original tls certificate is empty")?.as_ref(),
    )?;

    let new_sig_key = sign::any_supported_type(&PrivateKeyDer::Pkcs8(new_key.clone_key()))?;

    let new_key_spki =
        new_sig_key.public_key().ok_or("private key does not expose subject public key info")?;
    let new_key_spki = Constructed::decode(new_key_spki.as_ref(), Mode::Der, |cons| {
        rfc5280::SubjectPublicKeyInfo::take_from(cons)
    })?;

    let new_key = InMemorySigningKeyPair::from_pkcs8_der(new_key.secret_pkcs8_der())?;

    let new_key_algo = new_key.key_algorithm().ok_or("new_key must use known algorithm")?;

    let mut new_sig_algo: rfc5280::AlgorithmIdentifier = new_key.signature_algorithm()?.into();

    let mut new_cert = orig_cert.as_ref().tbs_certificate.clone();
    new_cert.raw_data = None; // discard original raw-data

    new_cert.issuer = new_cert.subject.clone(); // b/c self-signed
    new_cert.signature = new_sig_algo.clone(); // use algo of the key
    new_cert.subject_public_key_info = new_key_spki;

    if new_key_algo == KeyAlgorithm::Ed25519 {
        // algorithm identifier params must be absent for ed25519
        let empty = rfc5280::AlgorithmParameter::from_captured(Captured::empty(Mode::Der));
        new_cert.signature.parameters = Some(empty.clone());
        new_cert.subject_public_key_info.algorithm.parameters = Some(empty.clone());
        new_sig_algo.parameters = Some(empty);
    }

    let mut new_cert_der = Vec::<u8>::new();
    new_cert.encode_ref().write_encoded(Mode::Der, &mut new_cert_der)?;
    let new_sig = new_key.try_sign(&new_cert_der)?;

    let pubkey = new_cert.subject_public_key_info.subject_public_key.octet_bytes();

    let cert = X509Certificate::from(rfc5280::Certificate {
        tbs_certificate: new_cert,
        signature_algorithm: new_sig_algo,
        signature: BitString::new(0, Bytes::copy_from_slice(new_sig.as_ref())),
    })
    .encode_der()?;

    debug!(pubkey = hex::encode(pubkey), "Generated self-signed certificate for atls");

    Ok(vec![CertificateDer::from(cert)])
}

// tests ===============================================================

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::OnceLock};

    use attested_tls_proxy::{
        AttestationGenerator,
        attestation::AttestationVerifier,
        attested_tls::{AttestedTlsServer, TlsCertAndKey},
    };
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

    use super::self_sign_shadow_cert;

    fn ensure_crypto_provider_installed() {
        static PROVIDER: OnceLock<()> = OnceLock::new();
        PROVIDER.get_or_init(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
    }

    fn load_orig_cert() -> Vec<CertificateDer<'static>> {
        ensure_crypto_provider_installed();
        let path = {
            let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.push("src/utils/testdata/orig.crt");
            path
        };
        CertificateDer::pem_file_iter(&path).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
    }

    fn load_new_key() -> PrivateKeyDer<'static> {
        ensure_crypto_provider_installed();
        let path = {
            let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.push("src/utils/testdata/new.key");
            path
        };
        PrivateKeyDer::from_pem_file(&path).unwrap()
    }

    #[test]
    fn self_signed_shadow_cert_is_accepted_by_attested_tls_server_new() {
        let orig_cert = load_orig_cert();
        let new_key = load_new_key();

        let shadow_cert = self_sign_shadow_cert(&new_key, &orig_cert).unwrap();

        let result = AttestedTlsServer::new(
            TlsCertAndKey { key: new_key, cert_chain: shadow_cert },
            AttestationGenerator::with_no_attestation(),
            AttestationVerifier::expect_none(),
            false,
        );

        assert!(result.is_ok(), "{}", result.err().unwrap().to_string());
    }
}
