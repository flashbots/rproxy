use x509_parser::prelude::{FromDer, *};

use crate::utils::ip_to_string;

// ---------------------------------------------------------------------

pub(crate) fn tls_certificate_validity_timestamps(
    cert: &Vec<rustls::pki_types::CertificateDer<'_>>,
) -> (i64, i64) {
    let mut valid_not_before = i64::MIN;
    let mut valid_not_after = i64::MAX;

    for cert in cert {
        if let Ok((_, cert)) = X509Certificate::from_der(cert) {
            let cert_valid_not_before = cert.validity.not_before.timestamp();
            valid_not_before = std::cmp::max(valid_not_before, cert_valid_not_before);

            let cert_valid_not_after = cert.validity.not_after.timestamp();
            valid_not_after = std::cmp::min(valid_not_after, cert_valid_not_after);
        }
    }

    (valid_not_after, valid_not_after)
}

// ---------------------------------------------------------------------

pub(crate) fn tls_certificate_subject_names(
    cert: &Vec<rustls::pki_types::CertificateDer<'_>>,
) -> Vec<String> {
    let Some(cert) = cert.first() else { return Vec::new() };

    let Ok((_, cert)) = X509Certificate::from_der(cert) else { return Vec::new() };

    let mut names: Vec<String> = cert
        .subject()
        .iter_rdn()
        .flat_map(|rdn| rdn.iter())
        .filter(|attr| attr.attr_type() == &oid_registry::OID_X509_COMMON_NAME)
        .filter_map(|attr| attr.as_str().ok().map(|s| s.to_string()))
        .collect();

    let Ok(Some(sans)) = cert.subject_alternative_name() else {
        return names;
    };

    names.extend(
        sans.value
            .general_names
            .iter()
            .map(|name| match name {
                GeneralName::DNSName(dns) => dns.to_string(),
                GeneralName::IPAddress(ip) => ip_to_string(ip),
                GeneralName::RFC822Name(email) => email.to_string(),
                GeneralName::URI(uri) => uri.to_string(),
                _ => String::new(),
            })
            .filter(|name| !name.is_empty()),
    );

    names
}
