// is_hop_by_hop_header ------------------------------------------------

use actix_web::http::header;

pub(crate) fn is_hop_by_hop_header(name: &header::HeaderName) -> bool {
    // Fast path: compare against known HeaderName constants (no allocation).
    // The original implementation only filtered these four (per its ASCII
    // lowercase match): connection, host, keep-alive, transfer-encoding.
    matches!(
        name,
        &header::CONNECTION |
            &header::HOST |
            &header::TRANSFER_ENCODING
    ) || name.as_str().eq_ignore_ascii_case("keep-alive")
}
