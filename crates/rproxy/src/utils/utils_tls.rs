use x509_parser::prelude::FromDer;

pub(crate) fn tls_certificate_validity_timestamps(
    cert: &Vec<rustls::pki_types::CertificateDer<'_>>,
) -> (i64, i64) {
    let mut valid_not_before = i64::MIN;
    let mut valid_not_after = i64::MAX;

    for cert in cert {
        if let Ok((_, cert)) = x509_parser::prelude::X509Certificate::from_der(cert) {
            let cert_valid_not_before = cert.validity.not_before.timestamp();
            valid_not_before = std::cmp::max(valid_not_before, cert_valid_not_before);

            let cert_valid_not_after = cert.validity.not_after.timestamp();
            valid_not_after = std::cmp::min(valid_not_after, cert_valid_not_after);
        }
    }

    (valid_not_after, valid_not_after)
}
